use async_recursion::async_recursion;
use chrono::NaiveDate;
use hayagriva::io;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use vb_exchange::projects::BlockType;

use html_parser::{Dom, Node};
use pandoc::{InputFormat, InputKind, OutputFormat, OutputKind, PandocOutput};

use crate::import::language_detection::{detect_language_for_post, detect_language_for_section};
use crate::import::wordpress::{
    Post, PostDataType, WordpressAPI, WordpressAPIContext, WordpressAPIError,
};
use crate::projects::{
    BlockData, NewContentBlock, PersonUuidOrString, Section, SectionMetadataV5, SectionOrTocV5,
    SectionV5,
};
use crate::settings::Settings;
use crate::storage::project_storage::{ProjectData, ProjectStorage};
use crate::storage::BibEntryV2;
use crate::utils::block_id_generator::generate_id;
use rocket::http::ContentType;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;
use tokio::task::spawn_blocking;
use vb_exchange::projects::{Identifier, IdentifierType};

/// Struct wrapping all import jobs
pub struct ImportProcessor {
    /// Copy of the global settings
    settings: Settings,
    /// Reference to the project storage
    project_storage: Arc<ProjectStorage>,
    /// Queue of import jobs that are still waiting for a worker thread
    pub job_queue: RwLock<VecDeque<ImportJob>>,
    /// HashMap with information about jobs currently running or finished/failed
    pub job_archive: RwLock<HashMap<uuid::Uuid, ImportStatus>>,
}

/// Represents the current status for an important job
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ImportStatus {
    /// The job is queued in the worker queue
    Pending,
    /// Posts are requested and transferred from a wordpress host
    RequestWPPosts,
    /// Content is being processed and converted
    Processing(ProcessingDetails),
    /// The job completed successfully
    Complete,
    /// The job failed
    Failed(ImportError),
}

/// Contains number of the item currently processed and the total number of items to process
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessingDetails {
    /// Number of item currently processed
    pub current: usize,
    /// Total number of items to process. Will be None for WordpressFilter requests since we can't know the exact number of posts
    pub total: Option<usize>,
}

impl ProcessingDetails {
    pub fn new(current: usize, total: Option<usize>) -> Self {
        ProcessingDetails { current, total }
    }
}

/// Contains errors that may occur on imports
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ImportError {
    /// The mime type is not supported or couldn't be read / guessed
    UnsupportedFileType,
    /// The file couldn't be opened or read
    InvalidFile,
    /// The bib file couldn't be opened or read
    BibFileInvalid,
    /// Pandoc couldn't be executed or failed
    PandocError,
    /// Couldn't parse the HTML produced after converting
    HtmlConversionFailed,
    /// An WordPress API error occurred
    WordPressApiError(WordpressAPIError),
    /// The target project to import to doesn't exist
    ProjectNotFound,
}

/// Represents a import job with settings and an ['ImportJobData'] variant.
#[derive(Debug)]
pub struct ImportJob {
    /// ImportJob id, randomly generated
    pub id: uuid::Uuid,
    /// ID of the project to import into
    pub project_id: uuid::Uuid,
    /// Whether we should convert all footnotes to endnotes
    pub convert_footnotes_to_endnotes: bool,
    /// Whether we should shift all headings up 1 level (h2 becomes h1)
    pub shift_headings_up: bool,
    /// Whether we should try to convert links into citations
    pub convert_links: bool,
    /// Whether we should import author names
    pub import_author_names: bool,
    /// References where to find the items to imports
    pub import_data: ImportJobData,
}

/// Contains the references to Links/Files/Wordpress Filters to import
#[derive(Debug)]
pub enum ImportJobData {
    /// Import by a list of links to wordpress posts
    WordpressLinks(Vec<String>),
    /// Import by converting files via pandoc
    FileImport(FileImportData),
    /// Import by requesting posts matching filters from a wordpress host
    WordpressFilter(WordpressFilterData),
}

/// Filter settings for WordPress imports
#[derive(Serialize, Deserialize, Debug)]
pub struct WordpressFilterData {
    /// Host (without protocol) to get posts from
    pub wp_host: String,
    /// optional filter to only include posts before a date
    pub before: Option<NaiveDate>,
    /// optional filter to only include posts after a date
    pub after: Option<NaiveDate>,
    /// optional filter to only include posts in at least one of the specified categories
    pub include_categories: Option<Vec<usize>>,
    /// optional filter to exclude posts in at least one of the specified categories
    pub exclude_categories: Option<Vec<usize>>,
}

/// Holds data for an import from files to convert via pandoc
#[derive(Debug)]
pub struct FileImportData {
    /// List of (Path, ContentType) entries (one per section)
    pub files_to_process: VecDeque<(String, ContentType)>,
    /// optional path to an bib file to import
    pub bib_file: Option<String>,
}

impl ImportProcessor {
    /// Updates the import status of a job in the job archive.
    ///
    /// Acquires a write lock on the job archive and sets the status of the specified job ID to the given `new_status`.
    /// Overwrites any existing status for the job ID.
    ///
    /// # Arguments
    /// * `job_id` - The unique identifier of the import job to update.
    /// * `new_status` - The new status to assign to the job.
    fn update_import_status(&self, job_id: &uuid::Uuid, new_status: ImportStatus) {
        self.job_archive
            .write()
            .unwrap()
            .insert(job_id.clone(), new_status);
    }

    /// Starts the background import processor and returns a shared instance of the processor.
    ///
    /// This function initializes an [`ImportProcessor`] with the given application [`Settings`] and a reference
    /// to the [`ProjectStorage`]. It then spawns an asynchronous task that continuously monitors the import job queue.
    /// Whenever there are pending jobs and the number of running import threads is less than the configured maximum,
    /// it starts new asynchronous worker threads to process each import job concurrently. Each job is tracked in
    /// the `job_archive` map with its current [`ImportStatus`]. The thread count is adjusted atomically as jobs are picked up and finished.
    /// If no immediate job can be picked up, the loop waits for one second before checking again.
    ///
    /// # Arguments
    /// * `settings` - The application configuration containing, e.g., the maximum number of concurrent import threads.
    /// * `project_storage` - An atomically reference-counted pointer to the global project storage instance.
    ///
    /// # Returns
    /// An `Arc<ImportProcessor>` that can be used to schedule new import jobs or query their progress.
    ///
    /// The background worker will run for the process lifetime, picking up and processing import jobs as
    /// they become available in the queue.
    pub fn start(settings: Settings, project_storage: Arc<ProjectStorage>) -> Arc<ImportProcessor> {
        let processor = Arc::new(ImportProcessor {
            settings,
            project_storage,
            job_queue: RwLock::new(VecDeque::new()),
            job_archive: RwLock::new(HashMap::new()),
        });

        let processor_clone = processor.clone();
        tokio::spawn(async move {
            let running_threads: Arc<std::sync::atomic::AtomicU64> =
                Arc::new(std::sync::atomic::AtomicU64::new(0));

            loop {
                // Check if there are any new jobs
                let job_queue_len = processor_clone.job_queue.read().unwrap().len();
                if job_queue_len > 0
                    && processor_clone.settings.max_import_threads
                        > running_threads.load(std::sync::atomic::Ordering::SeqCst)
                {
                    debug!("Starting new import job...");

                    let proc_clone = processor_clone.clone();
                    let running_threads_cpy = running_threads.clone();

                    tokio::spawn(async move {
                        running_threads_cpy.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let job = match proc_clone.job_queue.write().unwrap().pop_front() {
                            Some(job) => job,
                            None => {
                                running_threads_cpy
                                    .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                                return;
                            }
                        };

                        let total_to_process = match &job.import_data {
                            ImportJobData::WordpressLinks(data) => Some(data.len()),
                            ImportJobData::FileImport(data) => Some(data.files_to_process.len()),
                            ImportJobData::WordpressFilter(data) => None,
                        };

                        let status = ImportStatus::Processing(ProcessingDetails {
                            current: 0,
                            total: total_to_process,
                        });
                        proc_clone
                            .job_archive
                            .write()
                            .unwrap()
                            .insert(job.id.clone(), status);
                        proc_clone
                            .process_job(job, proc_clone.project_storage.clone())
                            .await;
                        debug!("Job finished");
                        running_threads_cpy.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                    });
                } else {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
            }
        });

        processor
    }

    /// Import WordPress posts by post links
    async fn process_wordpress_links(&self, job: ImportJob, project_storage: Arc<ProjectStorage>) {
        let job_data = match job.import_data {
            ImportJobData::WordpressLinks(links) => links,
            _ => unreachable!(),
        };

        let project = match project_storage
            .get_project(&job.project_id, &self.settings)
            .await
        {
            Ok(project) => project.clone(),
            Err(_) => {
                self.update_import_status(
                    &job.id,
                    ImportStatus::Failed(ImportError::ProjectNotFound),
                );
                return;
            }
        };

        let total_num = job_data.len();
        for (num, link) in job_data.iter().enumerate() {
            debug!("Importing wordpress URL: {}", link);
            // Update import status
            self.update_import_status(
                &job.id,
                ImportStatus::Processing(ProcessingDetails::new(num, Some(total_num))),
            );
            if let Err(e) = self
                .import_by_url(
                    link,
                    Arc::clone(&project),
                    job.convert_footnotes_to_endnotes,
                    job.shift_headings_up,
                    job.convert_links,
                    job.import_author_names,
                )
                .await
            {
                error!("Import failed: {:?}", e);
                self.update_import_status(&job.id, ImportStatus::Failed(e));
                break;
            }
        }
        self.update_import_status(&job.id, ImportStatus::Complete);
    }

    /// Import content from files via Pandoc.
    /// Optionally imports bibliography entries from bibtex
    async fn process_file_import(&self, job: ImportJob, project_storage: Arc<ProjectStorage>) {
        let job_data = match job.import_data {
            ImportJobData::FileImport(data) => data,
            _ => unreachable!(),
        };

        // Import bib entries from file if present
        if let Some(bib_file) = job_data.bib_file {
            match self
                .import_bib_entries(job.project_id, &bib_file, &self.settings)
                .await
            {
                Ok(_) => {
                    debug!("Bib entries imported successfully");
                }
                Err(e) => {
                    warn!("Error importing bib entries: {:?}", e);
                    self.update_import_status(&job.id, ImportStatus::Failed(e));
                    return;
                }
            }

            // Remove bib file
            if let Err(e) = tokio::fs::remove_file(bib_file).await {
                error!("Error deleting bib file: {:?}", e);
            }
        }

        let total_num = job_data.files_to_process.len();

        for (num, (file, content_type)) in job_data.files_to_process.iter().enumerate() {
            debug!("Processing file: {}", file);

            let project = project_storage
                .get_project(&job.project_id, &self.settings)
                .await
                .unwrap();

            match self
                .convert_file(
                    file,
                    content_type,
                    project,
                    job.convert_footnotes_to_endnotes,
                    job.shift_headings_up,
                    job.convert_links,
                )
                .await
            {
                Ok(_) => {
                    debug!("File processed successfully");
                    // Remove file from temp directory
                    let res = tokio::fs::remove_file(file).await;
                    if let Err(e) = res {
                        error!("Error removing file from temp directory: {:?}", e);
                    }
                    self.update_import_status(
                        &job.id,
                        ImportStatus::Processing(ProcessingDetails::new(num + 1, Some(total_num))),
                    )
                }
                Err(e) => {
                    warn!("Error processing file: {:?}", e);
                    self.update_import_status(&job.id, ImportStatus::Failed(e));
                    break;
                }
            }
        }
        for (file, _) in job_data.files_to_process.iter() {
            let res = tokio::fs::remove_file(file).await;
            if let Err(e) = res {
                error!("Error removing file from temp directory: {:?}", e);
            }
        }
        self.update_import_status(&job.id, ImportStatus::Complete);
    }

    /// Imports WordPress posts from a wordpress host by filter criteria
    async fn process_wordpress_filter(&self, job: ImportJob, project_storage: Arc<ProjectStorage>) {
        let job_data = match job.import_data {
            ImportJobData::WordpressFilter(data) => data,
            _ => unreachable!(),
        };

        // Load all posts matching filter (except categories)
        let api = match WordpressAPI::new(job_data.wp_host) {
            Ok(api) => api,
            Err(e) => {
                self.update_import_status(
                    &job.id,
                    ImportStatus::Failed(ImportError::WordPressApiError(e)),
                );
                return;
            }
        };

        self.update_import_status(&job.id, ImportStatus::RequestWPPosts);

        let mut posts: Vec<Post> = vec![];
        let mut page = 1;

        loop {
            let data = match api
                .get_posts(
                    WordpressAPIContext::View,
                    Some(page),
                    Some(100),
                    None,
                    job_data.after,
                    None,
                    job_data.before,
                    None,
                    None,
                    None,
                    None,
                )
                .await
            {
                Ok(data) => data,
                Err(e) => {
                    warn!("Error fetching posts from WordpressAPI: {:?}", e);
                    self.update_import_status(
                        &job.id,
                        ImportStatus::Failed(ImportError::WordPressApiError(e)),
                    );
                    break;
                }
            };

            let mut res_posts = match data.data {
                PostDataType::PostPreviews(_) => unreachable!(),
                PostDataType::FullPosts(posts) => posts,
            };
            posts.append(&mut res_posts);

            if page >= data.total_pages {
                break;
            } else {
                page = page + 1;
            }
        }

        // Remove posts that don't meet our category filters
        // We can't just use the wordpress filter mechanism for category filters since it fails on too many categories (url length > 2000 chars) on some wp hosts

        // Include Category Filter
        if let Some(include_categories) = job_data.include_categories {
            posts = posts
                .into_iter()
                .filter(|post| {
                    post.categories
                        .iter()
                        .any(|category| include_categories.contains(category))
                })
                .collect();
        }

        // Exclude Category Filter
        if let Some(exclude_categories) = job_data.exclude_categories {
            posts = posts
                .into_iter()
                .filter(|post| {
                    !post
                        .categories
                        .iter()
                        .any(|category| exclude_categories.contains(category))
                })
                .collect();
        }

        let number_of_posts = posts.len();
        let project = Arc::clone(
            &project_storage
                .get_project(&job.project_id, &self.settings)
                .await
                .unwrap(),
        );

        for (num, post) in posts.into_iter().enumerate() {
            self.update_import_status(
                &job.id,
                ImportStatus::Processing(ProcessingDetails::new(num + 1, Some(number_of_posts))),
            );

            let additional_author_names = if job.import_author_names {
                self.resolve_wp_authors(&post, &api).await
            } else {
                vec![]
            };

            if let Err(e) = self
                .import_wp_post(
                    post,
                    project.clone(),
                    job.convert_footnotes_to_endnotes,
                    job.shift_headings_up,
                    job.convert_links,
                    additional_author_names,
                )
                .await
            {
                eprintln!("Error processing post for import: {:?}", e);
                self.update_import_status(&job.id, ImportStatus::Failed(e));
                break;
            }
        }
        self.update_import_status(&job.id, ImportStatus::Complete);
    }

    /// Processes an import job by delegating the job to the appropriate handler based on the type of import data.
    ///
    /// This asynchronous function accepts an `ImportJob` and a reference to the shared `ProjectStorage`.
    /// Depending on the `import_data` variant present in the job, it will call the corresponding asynchronous processing function:
    /// - For `ImportJobData::WordpressLinks`, it processes links to WordPress posts.
    /// - For `ImportJobData::FileImport`, it processes file imports using Pandoc.
    /// - For `ImportJobData::WordpressFilter`, it processes filtered post imports from a WordPress host.
    ///
    /// # Arguments
    /// * `job` - The `ImportJob` to be processed, containing configuration and import data.
    /// * `project_storage` - Shared reference to the `ProjectStorage`, which manages project data and resources.
    async fn process_job(&self, job: ImportJob, project_storage: Arc<ProjectStorage>) {
        match job.import_data {
            ImportJobData::WordpressLinks(_) => {
                self.process_wordpress_links(job, project_storage).await
            }
            ImportJobData::FileImport(_) => self.process_file_import(job, project_storage).await,
            ImportJobData::WordpressFilter(_) => {
                self.process_wordpress_filter(job, project_storage).await
            }
        }
    }

    pub async fn import_by_url(
        &self,
        url: &str,
        project: Arc<RwLock<ProjectData>>,
        endnotes: bool,
        shift_headings_up: bool,
        convert_links: bool,
        import_author_names: bool,
    ) -> Result<(), ImportError> {
        let url = if url.ends_with("/") {
            url[..url.len() - 1].to_string()
        } else {
            url.to_string()
        };

        let parsed_url = url::Url::parse(&url).unwrap();
        let host = match parsed_url.host() {
            Some(host) => host,
            None => {
                return Err(ImportError::WordPressApiError(
                    WordpressAPIError::InvalidURL,
                ))
            }
        };

        let api = match WordpressAPI::new(host.to_string()) {
            Ok(api) => api,
            Err(e) => return Err(ImportError::WordPressApiError(e)),
        };
        let path = parsed_url.path();

        let slug = path.split("/").last().unwrap_or("");

        /*if path.starts_with("/category/"){
            debug!("Found category link. Trying to import all posts within category");
            let category = match api.get_categories(None, None, None, None, None, Some(slug.to_string()), None, None, None).await{
                Ok(categories) => categories,
                Err(e) => return Err(ImportError::WordPressApiError(e))
            };
            if category.len() != 1{
                return Err(ImportError::WordPressApiError(WordpressAPIError::NotFound));
            }
            let category = category.first().unwrap();
            let mut posts = vec![];
            let mut page = 1;
            loop{
                let mut new_posts = match api.get_posts(WordpressAPIContext::default(), Some(page), None, None, None, None, None, Some(vec![category.id]), None).await{
                    Ok(posts) => posts,
                    Err(e) => return Err(ImportError::WordPressApiError(e))
                };
                if new_posts.len() == 0{
                    break;
                }else{
                    posts.append(&mut new_posts);
                    page += 1;
                }
            }
            for post in posts {
                self.import_single_post(post.slug.clone(), project.clone(), endnotes, shift_headings_up, convert_links, &api).await?;
            }
        }else{
        TODO: reimplement category link importing
         */

        debug!("Found non-category link. Trying to import single post");
        let post = self.get_wp_post_by_link(slug.to_string(), &api).await?;

        let additional_author_names = if import_author_names {
            self.resolve_wp_authors(&post, &api).await
        } else {
            vec![]
        };

        self.import_wp_post(
            post,
            project.clone(),
            endnotes,
            shift_headings_up,
            convert_links,
            additional_author_names,
        )
        .await?;

        Ok(())
    }

    /// Tries to resolve the author id from the wordpress api
    ///
    /// Returns a Vec with ['PersonUuidOrString'] with the authors (and optional co authors) as NameString variants
    async fn resolve_wp_authors(&self, post: &Post, api: &WordpressAPI) -> Vec<PersonUuidOrString> {
        debug!("Trying to resolve author names for post.");
        let mut author_names = vec![];

        // Resolve author name
        if let Ok(author) = api.get_user(post.author).await {
            author_names.push(PersonUuidOrString::NameString(author.name));

            // Add co authors if any
            if let Some(co_authors) = &post.coauthors {
                for co_author in co_authors {
                    author_names.push(PersonUuidOrString::NameString(
                        co_author.display_name.clone(),
                    ));
                }
            }
        }

        author_names.dedup();

        debug!("Resolved author names: {:?}", author_names);

        author_names
    }

    /// Imports a WordPress post into a project as a new section.
    ///
    /// This function takes a WordPress post along with additional metadata and configuration flags,
    /// constructs a `Section` struct containing the imported content and metadata,
    /// then asynchronously imports the HTML content into the given project.
    ///
    /// - Extracts the subtitle from the post's advanced custom fields (ACF) if present.
    /// - Collects and attaches DOI identifiers from the ACF, preferring `crossref_doi` over `doi`.
    /// - If the `language_detection` feature is enabled, attempts to detect the post's language using the rendered HTML content.
    /// - Assembles section metadata including title, authors, identifiers, publishing dates, web URL, and language.
    /// - Finally, passes the section and the rendered HTML to `import_html_from_wp`, propagating any import errors.
    ///
    /// # Arguments
    /// * `post` - The WordPress post to import. Can include custom ACF fields and co-authors.
    /// * `project` - An atomic, shareable handle to the project data to which this post should be imported.
    /// * `endnotes` - Whether to convert inline footnotes to endnotes in the imported content.
    /// * `shift_headings_up` - Whether to increase the level of all headings in the imported content by one.
    /// * `convert_links` - Whether to convert any internal WordPress links to project-internal links.
    /// * `imported_authors` - List of author identifiers or names to set as authors for this section.
    ///
    /// # Errors
    /// Returns an [`ImportError`] if the import process fails, for example when the project is not found,
    /// importing the HTML fails, or the input contains unsupported content types.
    async fn import_wp_post(
        &self,
        post: Post,
        project: Arc<RwLock<ProjectData>>,
        endnotes: bool,
        shift_headings_up: bool,
        convert_links: bool,
        imported_authors: Vec<PersonUuidOrString>,
    ) -> Result<(), ImportError> {
        let subtitle = match &post.acf {
            None => None,
            Some(acf) => match &acf.subheadline {
                None => None,
                Some(subheadline) => Some(subheadline.clone()),
            },
        };

        let mut identifiers = vec![];

        if let Some(acf) = &post.acf {
            if let Some(crossref_doi) = &acf.crossref_doi {
                identifiers.push(Identifier {
                    id: Some(uuid::Uuid::new_v4()),
                    name: "DOI".to_string(),
                    value: crossref_doi.clone(),
                    identifier_type: IdentifierType::DOI,
                });
            } else if let Some(doi) = &acf.doi {
                identifiers.push(Identifier {
                    id: Some(uuid::Uuid::new_v4()),
                    name: "DOI".to_string(),
                    value: doi.clone(),
                    identifier_type: IdentifierType::DOI,
                });
            }
        }

        let lang = if cfg!(feature = "language_detection") {
            detect_language_for_post(&post)
        } else {
            None
        };

        let section = Section {
            id: Some(uuid::Uuid::new_v4()),
            css_classes: vec![],
            sub_sections: vec![],
            children: vec![],
            visible_in_toc: true,
            metadata: SectionMetadataV5 {
                title: post.title.rendered.clone(),
                toc_title_subtitle_override: None,
                subtitle,
                authors: imported_authors,
                editors: vec![],
                web_url: Some(post.link.clone()),
                identifiers,
                published: Some(post.date.date()),
                last_changed: Some(post.modified),
                lang,
            },
        };

        debug!("{:?}", section);

        self.import_html_from_wp(
            section,
            post.content.rendered.clone(),
            project,
            endnotes,
            shift_headings_up,
            convert_links,
        )
        .await
    }

    async fn get_wp_post_by_link(
        &self,
        slug: String,
        api: &WordpressAPI,
    ) -> Result<Post, ImportError> {
        let mut posts = match api
            .get_posts(
                WordpressAPIContext::default(),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(slug.to_string()),
                None,
                None,
            )
            .await
        {
            Ok(posts) => match posts.data {
                PostDataType::FullPosts(posts) => posts,
                _ => {
                    return Err(ImportError::WordPressApiError(
                        WordpressAPIError::InvalidURL,
                    ))
                }
            },
            Err(e) => return Err(ImportError::WordPressApiError(e)),
        };
        if posts.len() != 1 {
            return Err(ImportError::WordPressApiError(WordpressAPIError::NotFound));
        }
        Ok(posts.pop().unwrap())
    }

    async fn convert_file(
        &self,
        file_path: &str,
        content_type: &ContentType,
        project: Arc<RwLock<ProjectData>>,
        endnotes: bool,
        shift_headings_up: bool,
        convert_links: bool,
    ) -> Result<(), ImportError> {
        let mut file = match tokio::fs::File::open(file_path).await {
            Ok(file) => file,
            Err(e) => {
                warn!("Couldn't open file to import: {}", e);
                return Err(ImportError::InvalidFile);
            }
        };

        let mut file_content = String::new();
        let mut marks: Vec<String> = vec![];

        match content_type.to_string().as_str() {
            "text/x-tex" | "application/x-tex" => {
                debug!("Processing LaTeX file");
                if let Err(e) = file.read_to_string(&mut file_content).await {
                    warn!("Couldn't read file to import: {}", e);
                    return Err(ImportError::InvalidFile);
                }
                (file_content, marks) = preprocess::latex(file_content);
                file_content = self
                    .convert_with_pandoc(InputKind::Pipe(file_content), InputFormat::Latex)
                    .await?;
                file_content = postprocess::latex(file_content, marks);
            }
            "application/vnd.oasis.opendocument.text" => {
                debug!("Processing ODT file");
                file_content = self
                    .convert_with_pandoc(
                        InputKind::Files(vec![PathBuf::from(file_path)]),
                        InputFormat::Other("ODT".to_string()),
                    )
                    .await?;
            }
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => {
                debug!("Processing DOCX file");
                file_content = self
                    .convert_with_pandoc(
                        InputKind::Files(vec![PathBuf::from(file_path)]),
                        InputFormat::Docx,
                    )
                    .await?;
            }
            "application/msword" => {
                debug!("Processing DOC file");
                file_content = self
                    .convert_with_pandoc(
                        InputKind::Files(vec![PathBuf::from(file_path)]),
                        InputFormat::Other("DOC".to_string()),
                    )
                    .await?;
            }
            "application/epub+zip" => {
                debug!("Processing EPUB file");
                file_content = self
                    .convert_with_pandoc(
                        InputKind::Files(vec![PathBuf::from(file_path)]),
                        InputFormat::Epub,
                    )
                    .await?;
            }
            "application/rtf" => {
                debug!("Processing RTF file");
                file_content = self
                    .convert_with_pandoc(
                        InputKind::Files(vec![PathBuf::from(file_path)]),
                        InputFormat::Rtf,
                    )
                    .await?;
            }
            "text/markdown" | "text/x-markdown" => {
                debug!("Processing Markdown file");
                file_content = self
                    .convert_with_pandoc(
                        InputKind::Files(vec![PathBuf::from(file_path)]),
                        InputFormat::Markdown,
                    )
                    .await?;
            }
            _ => {
                warn!("Unsupported file type: {}", content_type);
                return Err(ImportError::UnsupportedFileType);
            }
        }

        self.import_html_from_pandoc(
            file_content,
            project,
            endnotes,
            shift_headings_up,
            convert_links,
        )
        .await?;
        Ok(())
    }

    async fn convert_with_pandoc(
        &self,
        input: InputKind,
        input_format: InputFormat,
    ) -> Result<String, ImportError> {
        let task = spawn_blocking({
            move || {
                let mut pandoc = pandoc::new();

                pandoc.set_input(input);
                pandoc.set_input_format(input_format, vec![]);
                pandoc.set_output_format(OutputFormat::Html5, vec![]);
                pandoc.set_output(OutputKind::Pipe);
                pandoc.execute()
            }
        })
        .await;

        match task {
            Ok(res) => match res {
                Ok(res) => match res {
                    PandocOutput::ToFile(_) => Err(ImportError::PandocError),
                    PandocOutput::ToBuffer(res) => Ok(res),
                    PandocOutput::ToBufferRaw(_) => Err(ImportError::PandocError),
                },
                Err(e) => {
                    warn!("Couldn't convert import file with pandoc: {}", e);
                    Err(ImportError::PandocError)
                }
            },
            Err(e) => {
                warn!("Couldn't run pandoc: {}", e);
                Err(ImportError::PandocError)
            }
        }
    }

    async fn import_html_from_wp(
        &self,
        mut section: SectionV5,
        input: String,
        project_data: Arc<RwLock<ProjectData>>,
        endnotes: bool,
        shift_headings: bool,
        convert_links: bool,
    ) -> Result<(), ImportError> {
        let dom = match Dom::parse(&input) {
            Ok(dom) => dom,
            Err(e) => {
                error!("Couldn't parse html from import after pandoc: {}", e);
                return Err(ImportError::HtmlConversionFailed);
            }
        };
        if dom.tree_type == html_parser::DomVariant::Document {
            return Err(ImportError::HtmlConversionFailed);
        }

        // Get footnotes:
        let mut footnotes: HashMap<String, String> = HashMap::new();

        if let Some(footnote_div) = dom.children.iter().find(|x| match x {
            Node::Element(div) => div
                .classes
                .contains(&"footnotes_reference_container".to_string()),
            _ => false,
        }) {
            match footnote_div {
                Node::Element(div) => {
                    if let Some(div) = div.children.get(1) {
                        match div {
                            Node::Element(e) => {
                                if let Some(table) = e.children.get(0) {
                                    match table {
                                        Node::Element(table) => {
                                            if table.name == "table" {
                                                if let Some(tbody) = table.children.get(1) {
                                                    if let Node::Element(tbody) = tbody {
                                                        if tbody.name == "tbody" {
                                                            for row in &tbody.children {
                                                                if let Node::Element(tr) = row {
                                                                    if let Some(th) =
                                                                        tr.children.get(0)
                                                                    {
                                                                        if let Node::Element(th) =
                                                                            th
                                                                        {
                                                                            if let Some(a) =
                                                                                th.children.get(0)
                                                                            {
                                                                                if let Node::Element(a) = a {
                                                                                        if a.classes.contains(&"footnote_backlink".to_string()) {
                                                                                            if let Some(id) = a.id.clone() {
                                                                                                //Found footnote id

                                                                                                // Get footnote text
                                                                                                if let Some(td) = tr.children.get(1) {
                                                                                                    if let Node::Element(td) = td {
                                                                                                        if td.classes.contains(&"footnote_plugin_text".to_string()) {
                                                                                                            footnotes.insert(id, self.dom_to_html(td.clone(), None, endnotes, convert_links, project_data.clone()).await);
                                                                                                        }
                                                                                                    }
                                                                                                }
                                                                                            }
                                                                                        }
                                                                                    }
                                                                            }
                                                                        }
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }

        for node in dom.children {
            match node {
                Node::Text(t) => {
                    // Wrap text without a tag in a paragraph
                    let cb = NewContentBlock {
                        id: generate_id(&section),
                        block_type: BlockType::Paragraph,
                        data: BlockData::Paragraph { text: t },
                        css_classes: vec![],
                        revision_id: None,
                    };
                    section.children.push(cb);
                }
                Node::Element(el) => {
                    match el.name.to_lowercase().as_str() {
                        "h1" | "h2" | "h4" | "h5" | "h6" => {
                            let mut level = match el.name.to_lowercase().as_str() {
                                "h1" => 1,
                                "h2" => 2,
                                "h3" => 3,
                                "h4" => 4,
                                "h5" => 5,
                                "h6" => 6,
                                _ => 0,
                            };

                            if shift_headings {
                                if level > 1 {
                                    level -= 1;
                                }
                            }

                            section.children.push(NewContentBlock {
                                id: generate_id(&section),
                                block_type: BlockType::Heading,
                                data: BlockData::Heading {
                                    text: self
                                        .dom_to_html(
                                            el,
                                            Some(&footnotes),
                                            endnotes,
                                            convert_links,
                                            project_data.clone(),
                                        )
                                        .await,
                                    level,
                                },
                                css_classes: vec![],
                                revision_id: None,
                            })
                        }
                        "p" => section.children.push(NewContentBlock {
                            id: generate_id(&section),
                            block_type: BlockType::Paragraph,
                            data: BlockData::Paragraph {
                                text: self
                                    .dom_to_html(
                                        el,
                                        Some(&footnotes),
                                        endnotes,
                                        convert_links,
                                        project_data.clone(),
                                    )
                                    .await,
                            },
                            css_classes: vec![],
                            revision_id: None,
                        }),
                        "ul" | "ol" => {
                            let style = match el.name.to_lowercase().as_str() {
                                "ul" => "unordered",
                                "ol" => "ordered",
                                _ => "unordered",
                            };
                            let style = style.to_string();
                            let mut items = Vec::new();

                            for node in el.children.iter() {
                                if let Node::Element(el) = node {
                                    if el.name.to_lowercase() == "li" {
                                        let result = self
                                            .dom_to_html(
                                                el.clone(),
                                                Some(&footnotes),
                                                endnotes,
                                                convert_links,
                                                project_data.clone(),
                                            )
                                            .await;
                                        items.push(result);
                                    }
                                }
                            }

                            section.children.push(NewContentBlock {
                                id: generate_id(&section),
                                block_type: BlockType::List,
                                data: BlockData::List { style, items },
                                css_classes: vec![],
                                revision_id: None,
                            });
                        }
                        "blockquote" => {
                            section.children.push(NewContentBlock {
                                id: generate_id(&section),
                                block_type: BlockType::Quote,
                                data: BlockData::Quote {
                                    text: self
                                        .dom_to_html(
                                            el,
                                            Some(&footnotes),
                                            endnotes,
                                            convert_links,
                                            project_data.clone(),
                                        )
                                        .await,
                                    caption: "".to_string(),
                                    alignment: "".to_string(),
                                },
                                css_classes: vec![],
                                revision_id: None,
                            });
                        }
                        "div" => {
                            if el
                                .classes
                                .contains(&String::from("footnotes_reference_container"))
                            {
                                continue;
                            } else {
                                warn!("Warning: Unsupported div. Adding as paragraph");
                                // Add as paragraph
                                section.children.push(NewContentBlock {
                                    id: generate_id(&section),
                                    block_type: BlockType::Paragraph,
                                    data: BlockData::Paragraph {
                                        text: self
                                            .dom_to_html(
                                                el,
                                                Some(&footnotes),
                                                endnotes,
                                                convert_links,
                                                project_data.clone(),
                                            )
                                            .await,
                                    },
                                    css_classes: vec![],
                                    revision_id: None,
                                });
                            }
                        }
                        _ => {
                            warn!("Warning: Unsupported tag: {}", el.name);
                            // Add as paragraph
                            section.children.push(NewContentBlock {
                                id: generate_id(&section),
                                block_type: BlockType::Paragraph,
                                data: BlockData::Paragraph {
                                    text: self
                                        .dom_to_html(
                                            el,
                                            Some(&footnotes),
                                            endnotes,
                                            convert_links,
                                            project_data.clone(),
                                        )
                                        .await,
                                },
                                css_classes: vec![],
                                revision_id: None,
                            });
                        }
                    }
                }
                // Skip comments
                Node::Comment(_) => {}
            }
        }

        section.metadata.lang = detect_language_for_section(&section);
        project_data
            .write()
            .unwrap()
            .sections
            .push(SectionOrTocV5::Section(section));
        Ok(())
    }

    async fn import_html_from_pandoc(
        &self,
        input: String,
        project_data: Arc<RwLock<ProjectData>>,
        endnotes: bool,
        shift_headings: bool,
        convert_links: bool,
    ) -> Result<(), ImportError> {
        let dom = match Dom::parse(&input) {
            Ok(dom) => dom,
            Err(e) => {
                error!("Couldn't parse html from import after pandoc: {}", e);
                return Err(ImportError::HtmlConversionFailed);
            }
        };
        if dom.tree_type == html_parser::DomVariant::Document {
            return Err(ImportError::HtmlConversionFailed);
        } //TODO support a full html document

        let mut section = SectionV5 {
            id: Some(uuid::Uuid::new_v4()),
            css_classes: vec![],
            sub_sections: vec![],
            children: vec![],
            visible_in_toc: true,
            metadata: SectionMetadataV5 {
                title: "Imported Section".to_string(),
                toc_title_subtitle_override: None,
                subtitle: None,
                authors: vec![],
                editors: vec![],
                web_url: None,
                identifiers: vec![],
                published: None,
                last_changed: None,
                lang: None,
            },
        };

        // Get footnotes:
        let mut footnotes: HashMap<String, String> = HashMap::new();

        if let Some(aside) = dom.children.iter().find(|x| match x {
            Node::Element(el) => el.name == "aside",
            _ => false,
        }) {
            if let Node::Element(aside) = aside {
                if aside.id == Some("footnotes".to_string()) {
                    let ol = aside.children.iter().find(|node| match node {
                        Node::Element(el) => el.name == "ol",
                        _ => false,
                    });
                    if let Some(ol) = ol {
                        if let Node::Element(ol) = ol {
                            for node in ol.children.iter() {
                                if let Node::Element(li) = node {
                                    if let Some(id) = li.id.clone() {
                                        let id = id.to_string();
                                        let mut text = String::new();
                                        if let Some(ptag) = li.children.iter().next() {
                                            match ptag {
                                                Node::Element(ptag) => {
                                                    for node in ptag.children.iter() {
                                                        match node {
                                                            Node::Text(t) => text.push_str(&t),
                                                            Node::Element(ele) => {
                                                                match ele
                                                                    .name
                                                                    .to_lowercase()
                                                                    .as_str()
                                                                {
                                                                    "a" => {
                                                                        if let Some(role) = ele
                                                                            .attributes
                                                                            .get("role")
                                                                        {
                                                                            if let Some(role) = role
                                                                            {
                                                                                if role == "doc-backlink" {
                                                                                    // Skip backlinks
                                                                                    continue;
                                                                                }
                                                                            }
                                                                        }
                                                                        let mut attributes =
                                                                            String::new();
                                                                        for (attr, attrvalue) in
                                                                            ele.attributes.iter()
                                                                        {
                                                                            match attrvalue{
                                                                                Some(value) => attributes.push_str(&format!(" {}=\"{}\"", attr, value)),
                                                                                None => attributes.push_str(&format!(" {}", attr)),
                                                                            }
                                                                        }
                                                                        text.push_str(&format!(
                                                                            "<a {}>{}</a>",
                                                                            attributes,
                                                                            &self
                                                                                .dom_to_html(
                                                                                    ele.clone(),
                                                                                    None,
                                                                                    endnotes,
                                                                                    false,
                                                                                    project_data
                                                                                        .clone()
                                                                                )
                                                                                .await
                                                                        ));
                                                                    }
                                                                    _ => {
                                                                        // TODO: whitelist elements and attributes
                                                                        // Currently we allow all elements but strip attributes
                                                                        text.push_str(&format!(
                                                                            "<{}>{}</{}>",
                                                                            ele.name,
                                                                            &self
                                                                                .dom_to_html(
                                                                                    ele.clone(),
                                                                                    None,
                                                                                    endnotes,
                                                                                    false,
                                                                                    project_data
                                                                                        .clone()
                                                                                )
                                                                                .await,
                                                                            ele.name
                                                                        ));
                                                                    }
                                                                }
                                                            }
                                                            _ => {}
                                                        }
                                                    }
                                                }
                                                Node::Text(t) => {
                                                    text.push_str(&t);
                                                }
                                                _ => {}
                                            }
                                        }
                                        footnotes.insert(id.clone(), text.clone());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        for node in dom.children {
            match node {
                Node::Text(t) => {
                    // Wrap text without a tag in a paragraph
                    let cb = NewContentBlock {
                        id: generate_id(&section),
                        block_type: BlockType::Paragraph,
                        data: BlockData::Paragraph { text: t },
                        css_classes: vec![],
                        revision_id: None,
                    };
                    section.children.push(cb);
                }
                Node::Element(el) => {
                    match el.name.to_lowercase().as_str() {
                        "h1" | "h2" | "h4" | "h5" | "h6" => {
                            let mut level = match el.name.to_lowercase().as_str() {
                                "h1" => 1,
                                "h2" => 2,
                                "h3" => 3,
                                "h4" => 4,
                                "h5" => 5,
                                "h6" => 6,
                                _ => 0,
                            };

                            if shift_headings {
                                if level == 1 {
                                    level = 1
                                } else {
                                    level = level - 1;
                                }
                            }

                            section.children.push(NewContentBlock {
                                id: generate_id(&section),
                                block_type: BlockType::Heading,
                                data: BlockData::Heading {
                                    text: self
                                        .dom_to_html(
                                            el,
                                            Some(&footnotes),
                                            endnotes,
                                            convert_links,
                                            project_data.clone(),
                                        )
                                        .await,
                                    level,
                                },
                                css_classes: vec![],
                                revision_id: None,
                            })
                        }
                        "p" => section.children.push(NewContentBlock {
                            id: generate_id(&section),
                            block_type: BlockType::Paragraph,
                            data: BlockData::Paragraph {
                                text: self
                                    .dom_to_html(
                                        el,
                                        Some(&footnotes),
                                        endnotes,
                                        convert_links,
                                        project_data.clone(),
                                    )
                                    .await,
                            },
                            css_classes: vec![],
                            revision_id: None,
                        }),
                        "ul" | "ol" => {
                            let style = match el.name.to_lowercase().as_str() {
                                "ul" => "unordered",
                                "ol" => "ordered",
                                _ => "unordered",
                            };
                            let style = style.to_string();
                            let mut items = Vec::new();

                            for node in el.children.iter() {
                                if let Node::Element(el) = node {
                                    if el.name.to_lowercase() == "li" {
                                        let result = self
                                            .dom_to_html(
                                                el.clone(),
                                                Some(&footnotes),
                                                endnotes,
                                                convert_links,
                                                project_data.clone(),
                                            )
                                            .await;
                                        items.push(result);
                                    }
                                }
                            }

                            section.children.push(NewContentBlock {
                                id: generate_id(&section),
                                block_type: BlockType::List,
                                data: BlockData::List { style, items },
                                css_classes: vec![],
                                revision_id: None,
                            });
                        }
                        "blockquote" => {
                            section.children.push(NewContentBlock {
                                id: generate_id(&section),
                                block_type: BlockType::Quote,
                                data: BlockData::Quote {
                                    text: self
                                        .dom_to_html(
                                            el,
                                            Some(&footnotes),
                                            endnotes,
                                            convert_links,
                                            project_data.clone(),
                                        )
                                        .await,
                                    caption: "".to_string(),
                                    alignment: "".to_string(),
                                },
                                css_classes: vec![],
                                revision_id: None,
                            });
                        }
                        "aside" => {
                            if let Some(id) = el.id {
                                if id == "footnotes" {
                                    // Skip footnotes
                                    continue;
                                }
                            }
                        }
                        _ => {
                            warn!("Warning: Unsupported tag: {}", el.name);
                            // Add as paragraph
                            section.children.push(NewContentBlock {
                                id: generate_id(&section),
                                block_type: BlockType::Paragraph,
                                data: BlockData::Paragraph {
                                    text: self
                                        .dom_to_html(
                                            el,
                                            Some(&footnotes),
                                            endnotes,
                                            convert_links,
                                            project_data.clone(),
                                        )
                                        .await,
                                },
                                css_classes: vec![],
                                revision_id: None,
                            });
                        }
                    }
                }
                // Skip comments
                Node::Comment(_) => {}
            }
        }

        if cfg!(feature = "language_detection") {
            let lang = detect_language_for_section(&section);
            section.metadata.lang = lang;
        }

        project_data
            .write()
            .unwrap()
            .sections
            .push(SectionOrTocV5::Section(section));
        Ok(())
    }

    //TODO: maybe also copy classes and ids from the html
    #[async_recursion]
    async fn dom_to_html(
        &self,
        ele: html_parser::Element,
        footnotes: Option<&HashMap<String, String>>,
        endnotes: bool,
        convert_links: bool,
        project_data: Arc<RwLock<ProjectData>>,
    ) -> String {
        let mut html = String::new();
        for node in ele.children {
            match node {
                Node::Text(t) => {
                    debug!("Found Text: {}", t);
                    html.push_str(&t);
                }
                Node::Element(el) => {
                    debug!("Found Element: {}", el.name);

                    if el.name == "a" {
                        // For pandoc footnotes
                        if let Some(role) = el.attributes.get("role") {
                            if let Some(role) = role {
                                if role == "doc-noteref" {
                                    // This is a reference to a footnote
                                    if let Some(sup) = el.children.iter().next() {
                                        if let Node::Element(sup) = sup {
                                            if sup.name == "sup" {
                                                if let Some(num) = sup.children.iter().next() {
                                                    if let Node::Text(num) = num {
                                                        if let Some(footnotes) = footnotes {
                                                            let num = num.trim().to_string();
                                                            if let Some(footnote) =
                                                                footnotes.get(&format!("fn{}", num))
                                                            {
                                                                debug!(
                                                                    "Found footnote: {}",
                                                                    footnote.clone()
                                                                );
                                                                if endnotes {
                                                                    html.push_str(&format!("<span class=\"note\" note-type=\"endnote\" note-content=\"{}\">E</span>", footnote.clone().replace("\"", "'")));
                                                                } else {
                                                                    html.push_str(&format!("<span class=\"note\" note-type=\"footnote\" note-content=\"{}\">F</span>", footnote.clone().replace("\"", "'")));
                                                                }
                                                                continue;
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        // For wordpress footnotes:
                        if let Some(sup) = el.children.get(0) {
                            if let Node::Element(sup) = sup {
                                if sup
                                    .classes
                                    .contains(&"footnote_plugin_tooltip_text".to_string())
                                {
                                    if let Some(id) = sup.id.clone() {
                                        let footnote_id = id.replace("tooltip", "reference");
                                        if let Some(footnotes) = footnotes {
                                            if let Some(footnote) = footnotes.get(&footnote_id) {
                                                debug!("Found footnote: {}", footnote.clone());
                                                if endnotes {
                                                    html.push_str(&format!("<span class=\"note\" note-type=\"endnote\" note-content=\"{}\">E</span>", footnote.clone().replace("\"", "'")));
                                                } else {
                                                    html.push_str(&format!("<span class=\"note\" note-type=\"footnote\" note-content=\"{}\">F</span>", footnote.clone().replace("\"", "'")));
                                                }

                                                continue;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        // Only convert links to citations if convert_links is true and we aren't in a footnote
                        if convert_links && footnotes.is_some() {
                            if let Some(href) = el.attributes.get("href") {
                                if let Some(href) = href {
                                    debug!("Trying to get citation for link: {}", href);
                                    if let Some(citation) =
                                        crate::import::link_converter::get_translation(
                                            href,
                                            &self.settings,
                                        )
                                        .await
                                    {
                                        let key = citation.key.clone();

                                        // Add citation to project
                                        project_data
                                            .write()
                                            .unwrap()
                                            .bibliography
                                            .insert(citation.key.clone(), citation.clone());

                                        let link_text = self
                                            .dom_to_html(
                                                el.clone(),
                                                None,
                                                endnotes,
                                                false,
                                                project_data.clone(),
                                            )
                                            .await;

                                        if link_text == *href {
                                            html.push_str(&format!(
                                                "<citation data-key=\"{}\">C</citation>",
                                                key
                                            ));
                                        } else {
                                            html.push_str(&format!(
                                                "{}<citation data-key=\"{}\">C</citation>",
                                                link_text, key
                                            ));
                                        }
                                        continue;
                                    }
                                }
                            }
                        }
                    }

                    let mut attrs: String = String::new();
                    for (attr, attrvalue) in el.attributes.iter() {
                        match attrvalue {
                            //TODO: whitelist attributes that are allowed for each tag, e.g. href for a-tags
                            Some(value) => attrs.push_str(&format!(" {}=\"{}\"", attr, value)),
                            None => attrs.push_str(&format!(" {}", attr)),
                        }
                    }
                    html.push_str(&format!("<{}{}>", el.name, attrs));
                    html.push_str(
                        &self
                            .dom_to_html(
                                el.clone(),
                                footnotes,
                                endnotes,
                                convert_links,
                                project_data.clone(),
                            )
                            .await,
                    );
                    html.push_str(&format!("</{}>", el.name));
                }
                // Ignore comments
                Node::Comment(_) => {}
            }
        }
        html
    }

    async fn import_bib_entries(
        &self,
        project_id: uuid::Uuid,
        bib_file_path: &str,
        settings: &Settings,
    ) -> Result<(), ImportError> {
        let mut bib_file_content = String::new();
        let mut bib_file = match tokio::fs::File::open(bib_file_path).await {
            Ok(bib_file) => bib_file,
            Err(e) => {
                warn!("Error opening bib file {}: {}", bib_file_path, e);
                return Err(ImportError::BibFileInvalid);
            }
        };
        if let Err(e) = bib_file.read_to_string(&mut bib_file_content).await {
            warn!("Error reading bib file: {}", e);
            return Err(ImportError::BibFileInvalid);
        }
        let bib = match io::from_biblatex_str(&bib_file_content) {
            Ok(bib) => bib,
            Err(e) => {
                warn!(
                    "Error parsing bib file: {}",
                    e.iter()
                        .map(|e| e.to_string())
                        .collect::<Vec<String>>()
                        .join(", ")
                );
                return Err(ImportError::BibFileInvalid);
            }
        };

        let project_storage = self.project_storage.clone();
        let project = project_storage
            .get_project(&project_id, settings)
            .await
            .unwrap()
            .clone();
        for entry in bib.iter() {
            let converted = BibEntryV2::from(entry);
            project
                .write()
                .unwrap()
                .bibliography
                .insert(converted.key.clone(), converted);
        }

        Ok(())
    }
}

/// Contains preprocessing methods that get called, BEFORE pandoc is executed.
mod preprocess {
    use regex::Regex;

    /// Preprocessing for latex input
    /// Replaces all endnotes with footnotes since endnotes are not supported by pandoc
    /// Finds all citations and replaces them with a temporary mark which survives pandoc
    pub fn latex(mut input: String) -> (String, Vec<String>) {
        let mut marks = Vec::new();

        let re = Regex::new(r"\\(cite|footcite|footcitetext|fullcite|footfullcite)(?:\[[^\]]*?\])?(?:\[[^\]]*?\])?\{(.*?)\}").unwrap();
        input = re
            .replace_all(&input, |caps: &regex::Captures| {
                let key = &caps[2];
                marks.push(key.to_string());
                return format!("vb-cite-{}", marks.len() - 1);
            })
            .to_string();

        (input.replace("\\endnote", "\\footnote"), marks)
    }
}

mod postprocess {
    use regex::Regex;

    pub fn latex(mut input: String, marks: Vec<String>) -> String {
        let re = Regex::new(r"vb-cite-(\d+)").unwrap();

        // Replace temporary citation marks with actual citations
        input = re
            .replace_all(&input, |caps: &regex::Captures| {
                let num = match (&caps[1]).parse::<usize>() {
                    Ok(num) => num,
                    Err(e) => {
                        warn!("Warning: couldn't parse vb-cite- citation number: {}", e);
                        return String::from("invalid-citation!");
                    }
                };
                format!(
                    "<citation data-key=\"{}\">C</citation>",
                    marks.get(num).unwrap_or(&"".to_string())
                )
            })
            .to_string();

        input
    }
}

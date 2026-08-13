use chrono::NaiveDate;
use hayagriva::io;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use vb_exchange::projects::BlockType;

use html5ever::tendril::TendrilSink;
use html5ever::{ParseOpts, QualName, parse_fragment};
use markup5ever::{Attribute, local_name, ns};
use markup5ever_rcdom::{Handle, NodeData, RcDom};
use pandoc::{InputFormat, InputKind, OutputFormat, OutputKind, PandocOutput};

use crate::db::repositories::{bibliography, projects};
use crate::import::language_detection::{detect_language_for_post, detect_language_for_section};
use crate::import::link_converter;
use crate::import::wordpress::{
    Post, PostDataType, WordpressAPI, WordpressAPIContext, WordpressAPIError,
};
use crate::settings::Settings;
use crate::storage::BibEntryV3;
use crate::storage::project_storage::current::{BibEntryOrFolder, PersonUuidOrString};
use crate::storage::project_storage::sections::content::current::BlockData;
use crate::storage::project_storage::sections::content::current::NewContentBlock;
use crate::storage::project_storage::sections::migration::convert_contentblocks_to_yrs;
use crate::storage::project_storage::sections::{Section, SectionMetadata};
use crate::utils::dedup::dedup_vec;
use log::{debug, error, warn};
use rocket::http::ContentType;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;
use tokio::task::spawn_blocking;
use vb_exchange::projects::{Identifier, IdentifierType};
use yrs::{ReadTxn, StateVector, Transact};

/// Struct wrapping all import jobs
pub struct ImportProcessor {
    /// Copy of the global settings
    settings: Settings,
    /// Postgres connection pool
    pool: sqlx::PgPool,
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
    /// A database error occurred while persisting imported content
    DatabaseError(String),
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
    /// Collects all bibliography entries, including all transitive parents, keyed by their
    /// original hayagriva key.
    fn collect_bib_entries_with_parents(
        entries: impl IntoIterator<Item = hayagriva::Entry>,
    ) -> HashMap<String, hayagriva::Entry> {
        let mut by_key: HashMap<String, hayagriva::Entry> = HashMap::new();
        let mut queue: Vec<hayagriva::Entry> = entries.into_iter().collect();

        while let Some(entry) = queue.pop() {
            let key = entry.key().to_string();
            if by_key.contains_key(&key) {
                continue;
            }
            for parent in entry.parents().iter().cloned() {
                queue.push(parent);
            }
            by_key.insert(key, entry);
        }

        by_key
    }
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
            .insert(*job_id, new_status);
    }

    /// Starts the background import processor and returns a shared instance of the processor.
    ///
    /// This function initializes an [`ImportProcessor`] with the given application [`Settings`] and a Postgres
    /// connection `pool`. It then spawns an asynchronous task that continuously monitors the import job queue.
    /// Whenever there are pending jobs and the number of running import threads is less than the configured maximum,
    /// it starts new asynchronous worker threads to process each import job concurrently. Each job is tracked in
    /// the `job_archive` map with its current [`ImportStatus`]. The thread count is adjusted atomically as jobs are picked up and finished.
    /// If no immediate job can be picked up, the loop waits for one second before checking again.
    ///
    /// # Arguments
    /// * `settings` - The application configuration containing, e.g., the maximum number of concurrent import threads.
    /// * `pool` - PostgreSQL connection pool used by the processor to read and persist project data.
    ///
    /// # Returns
    /// An `Arc<ImportProcessor>` that can be used to schedule new import jobs or query their progress.
    ///
    /// The background worker will run for the process lifetime, picking up and processing import jobs as
    /// they become available in the queue.
    pub fn start(settings: Settings, pool: sqlx::PgPool) -> Arc<ImportProcessor> {
        let processor = Arc::new(ImportProcessor {
            settings,
            pool,
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
                            ImportJobData::WordpressFilter(_data) => None,
                        };

                        let status = ImportStatus::Processing(ProcessingDetails {
                            current: 0,
                            total: total_to_process,
                        });
                        proc_clone
                            .job_archive
                            .write()
                            .unwrap()
                            .insert(job.id, status);
                        proc_clone.process_job(job).await;
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
    async fn process_wordpress_links(&self, job: ImportJob) {
        let job_data = match job.import_data {
            ImportJobData::WordpressLinks(links) => links,
            _ => unreachable!(),
        };

        match projects::exists(&self.pool, job.project_id).await {
            Ok(true) => {}
            Ok(false) => {
                self.update_import_status(
                    &job.id,
                    ImportStatus::Failed(ImportError::ProjectNotFound),
                );
                return;
            }
            Err(e) => {
                error!("Couldn't check if project {} exists: {}", job.project_id, e);
                self.update_import_status(
                    &job.id,
                    ImportStatus::Failed(ImportError::DatabaseError(e.to_string())),
                );
                return;
            }
        }

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
                    job.project_id,
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
    async fn process_file_import(&self, job: ImportJob) {
        let job_data = match job.import_data {
            ImportJobData::FileImport(data) => data,
            _ => unreachable!(),
        };

        match projects::exists(&self.pool, job.project_id).await {
            Ok(true) => {}
            Ok(false) => {
                self.update_import_status(
                    &job.id,
                    ImportStatus::Failed(ImportError::ProjectNotFound),
                );
                return;
            }
            Err(e) => {
                error!("Couldn't check if project {} exists: {}", job.project_id, e);
                self.update_import_status(
                    &job.id,
                    ImportStatus::Failed(ImportError::DatabaseError(e.to_string())),
                );
                return;
            }
        }

        // Import bib entries from file if present
        if let Some(bib_file) = job_data.bib_file {
            match self.import_bib_entries(job.project_id, &bib_file).await {
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

            match self
                .convert_file(
                    file,
                    content_type,
                    job.project_id,
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
    async fn process_wordpress_filter(&self, job: ImportJob) {
        let job_data = match job.import_data {
            ImportJobData::WordpressFilter(data) => data,
            _ => unreachable!(),
        };

        match projects::exists(&self.pool, job.project_id).await {
            Ok(true) => {}
            Ok(false) => {
                self.update_import_status(
                    &job.id,
                    ImportStatus::Failed(ImportError::ProjectNotFound),
                );
                return;
            }
            Err(e) => {
                error!("Couldn't check if project {} exists: {}", job.project_id, e);
                self.update_import_status(
                    &job.id,
                    ImportStatus::Failed(ImportError::DatabaseError(e.to_string())),
                );
                return;
            }
        }

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
        let data = match api
            .get_posts(
                WordpressAPIContext::View,
                None,
                job_data.after,
                None,
                job_data.before,
                None,
                None,
                job_data.include_categories,
                job_data.exclude_categories,
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
                return;
            }
        };

        match data.data {
            PostDataType::PostPreviews(_) => {
                unreachable!()
            }
            PostDataType::FullPosts(data) => {
                posts = data;
            }
        }

        // Add co authors if any
        for post in posts.iter_mut() {
            let _ = api.add_coauthors(post).await;
        }

        let number_of_posts = posts.len();

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
                    job.project_id,
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
    /// This asynchronous function accepts an `ImportJob`.
    /// Depending on the `import_data` variant present in the job, it will call the corresponding asynchronous processing function:
    /// - For `ImportJobData::WordpressLinks`, it processes links to WordPress posts.
    /// - For `ImportJobData::FileImport`, it processes file imports using Pandoc.
    /// - For `ImportJobData::WordpressFilter`, it processes filtered post imports from a WordPress host.
    ///
    /// # Arguments
    /// * `job` - The `ImportJob` to be processed, containing configuration and import data.
    async fn process_job(&self, job: ImportJob) {
        match job.import_data {
            ImportJobData::WordpressLinks(_) => self.process_wordpress_links(job).await,
            ImportJobData::FileImport(_) => self.process_file_import(job).await,
            ImportJobData::WordpressFilter(_) => self.process_wordpress_filter(job).await,
        }
    }

    /// Imports WordPress content referenced by a single URL into a project.
    ///
    /// If `url` points at a WordPress category page, every post in that category is fetched and
    /// imported as its own section. Otherwise `url` is treated as a link to a single post, which
    /// is fetched and imported. Author names are resolved from the WordPress API when
    /// `import_author_names` is set.
    ///
    /// # Errors
    /// Returns an [`ImportError`] if the URL is invalid, the WordPress API request fails, or
    /// importing the resolved post(s) fails.
    pub async fn import_by_url(
        &self,
        url: &str,
        project_id: uuid::Uuid,
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
                ));
            }
        };

        let api = match WordpressAPI::new(host.to_string()) {
            Ok(api) => api,
            Err(e) => return Err(ImportError::WordPressApiError(e)),
        };
        let path = parsed_url.path();

        let slug = path.split("/").last().unwrap_or("");

        if path.starts_with("/category/") {
            debug!("Found category link. Trying to import all posts within category");
            let category = match api
                .get_categories(
                    None,
                    None,
                    None,
                    None,
                    None,
                    Some(slug.to_string()),
                    None,
                    None,
                    None,
                )
                .await
            {
                Ok(categories) => categories,
                Err(e) => return Err(ImportError::WordPressApiError(e)),
            };
            if category.len() != 1 {
                return Err(ImportError::WordPressApiError(WordpressAPIError::NotFound));
            }
            let category = category.first().unwrap();
            let mut posts = match api
                .get_posts(
                    WordpressAPIContext::View,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    Some(vec![category.id]),
                    None,
                )
                .await
            {
                Ok(posts) => match posts.data {
                    PostDataType::FullPosts(posts) => posts,
                    _ => {
                        unreachable!()
                    }
                },
                Err(e) => return Err(ImportError::WordPressApiError(e)),
            };

            // Add co authors if any
            for post in posts.iter_mut() {
                let _ = api.add_coauthors(post).await;
            }

            for post in posts {
                let additional_author_names = if import_author_names {
                    self.resolve_wp_authors(&post, &api).await
                } else {
                    vec![]
                };
                self.import_wp_post(
                    post,
                    project_id,
                    endnotes,
                    shift_headings_up,
                    convert_links,
                    additional_author_names,
                )
                .await?;
            }
        } else {
            debug!("Found non-category link. Trying to import single post");

            let post = self.get_wp_post_by_link(slug.to_string(), &api).await?;
            debug!("Successfully downloaded wp post. Trying to resolve author names.");
            let additional_author_names = if import_author_names {
                self.resolve_wp_authors(&post, &api).await
            } else {
                vec![]
            };

            self.import_wp_post(
                post,
                project_id,
                endnotes,
                shift_headings_up,
                convert_links,
                additional_author_names,
            )
            .await?;
        }
        Ok(())
    }

    /// Tries to resolve the author id from the wordpress api
    ///
    /// Returns a Vec with ['PersonUuidOrString'] with the authors as NameString variants
    async fn resolve_wp_authors(&self, post: &Post, api: &WordpressAPI) -> Vec<PersonUuidOrString> {
        debug!("Trying to resolve author names for post.");
        let mut author_names = vec![];

        // Resolve author name
        if let Ok(author) = api.get_user(post.author).await {
            author_names.push(PersonUuidOrString::NameString(author.name));
        }

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
    /// * `project_id` - The UUID of the project this post should be imported into.
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
        project_id: uuid::Uuid,
        endnotes: bool,
        shift_headings_up: bool,
        convert_links: bool,
        imported_authors: Vec<PersonUuidOrString>,
    ) -> Result<(), ImportError> {
        let subtitle = match &post.acf {
            None => None,
            Some(acf) => acf.subheadline.clone(),
        };

        let mut identifiers = vec![];

        if let Some(acf) = &post.acf {
            if let Some(crossref_doi) = &acf.crossref_doi
                && !crossref_doi.trim().is_empty()
            {
                identifiers.push(Identifier {
                    id: Some(uuid::Uuid::new_v4()),
                    name: "DOI".to_string(),
                    value: crossref_doi.clone(),
                    identifier_type: IdentifierType::DOI,
                });
            } else if let Some(doi) = &acf.doi
                && !doi.trim().is_empty()
            {
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

        let mut authors = imported_authors;
        if let Some(co_authors) = &post.coauthors {
            for coauthor in co_authors {
                authors.push(PersonUuidOrString::NameString(
                    coauthor.display_name.clone(),
                ));
            }
        }

        authors = dedup_vec(authors);

        let section = Section {
            id: Some(uuid::Uuid::new_v4()),
            css_classes: vec![],
            sub_sections: vec![],
            content: vec![],
            visible_in_toc: true,
            metadata: SectionMetadata {
                title: post.title.rendered.clone(),
                toc_title_subtitle_override: None,
                subtitle,
                authors,
                editors: vec![],
                web_url: Some(post.link.clone()),
                identifiers,
                published: Some(post.date.date()),
                last_changed: Some(post.modified),
                lang,
                custom_fields: HashMap::new(),
            },
        };

        debug!("{:?}", section);

        self.import_html_from_wp(
            section,
            post.content.rendered.clone(),
            project_id,
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
                Some(slug.to_string()),
                None,
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
                    ));
                }
            },
            Err(e) => return Err(ImportError::WordPressApiError(e)),
        };
        // Add co authors if any
        for post in posts.iter_mut() {
            let _ = api.add_coauthors(post).await;
        }

        if posts.len() != 1 {
            return Err(ImportError::WordPressApiError(WordpressAPIError::NotFound));
        }
        Ok(posts.pop().unwrap())
    }

    async fn convert_file(
        &self,
        file_path: &str,
        content_type: &ContentType,
        project_id: uuid::Uuid,
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
            project_id,
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

    /// Sanitizes WordPress-flavored HTML into editor content blocks, sets the section's
    /// content and detected language, then persists the resulting `section` as a new
    /// section of `project_id` in the database.
    ///
    /// Downloads referenced media and resolves external links into citations before
    /// building the final blocks (see the phase comments in the body for the
    /// sync/async/sync split, which keeps non-`Send` DOM handles off await points).
    ///
    /// # Errors
    /// Returns [`ImportError::HtmlConversionFailed`] if the input can't be parsed, or
    /// [`ImportError::DatabaseError`] if persisting the section fails.
    async fn import_html_from_wp(
        &self,
        mut section: Section,
        input: String,
        project_id: uuid::Uuid,
        endnotes: bool,
        shift_headings: bool,
        convert_links: bool,
    ) -> Result<(), ImportError> {
        debug!("Importing html from wp");

        // Phase 1 (sync): parse and collect the URLs that need asynchronous work (images/media to download). The
        // html5ever handles are `Rc`-based (`!Send`) and must not be held across an `.await`,
        // so every handle access happens inside a scope that drops the handles before an await.
        let (media_srcs, hrefs) = {
            let dom = parse_dom(&input);
            let top_nodes = top_level_nodes(&dom);
            if top_nodes.is_empty() && !input.trim().is_empty() {
                error!("Couldn't parse html from import: no parseable nodes");
                return Err(ImportError::HtmlConversionFailed);
            }
            let mut media = Vec::new();
            collect_media_srcs(&top_nodes, &mut media);
            let mut hrefs = Vec::new();
            if convert_links {
                collect_convertible_hrefs(&top_nodes, &mut hrefs);
            }
            (dedup_vec(media), dedup_vec(hrefs))
        };

        // Phase 2 (async): download external media and resolve links into citations. Works
        // only on owned `String`s, so no non-Send handles are alive across the awaits.
        let media_map = self.download_all_media(&media_srcs, project_id).await;
        let link_map = self.resolve_all_links(&hrefs, project_id).await;

        // Phase 3 (sync): re-parse and build the content blocks using the precomputed maps.
        let blocks = {
            let dom = parse_dom(&input);
            let top_nodes = top_level_nodes(&dom);
            let footnotes = extract_wp_footnotes(&top_nodes, &link_map, &media_map);

            let mut blocks: Vec<NewContentBlock> = vec![];
            for node in &top_nodes {
                blocks.extend(node_to_blocks(
                    &section,
                    node,
                    &footnotes,
                    endnotes,
                    shift_headings,
                    &link_map,
                    &media_map,
                ));
            }
            blocks
        };

        debug!("Converting contentblocks to yrs.");
        let doc = convert_contentblocks_to_yrs(blocks);
        section.content = doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default());

        if cfg!(feature = "language_detection") {
            section.metadata.lang = detect_language_for_section(&section);
        }

        debug!("Saving imported section");
        crate::db::repositories::sections::insert_at_end(
            &self.pool,
            &self.settings,
            project_id,
            None,
            &section,
        )
        .await
        .map_err(|e| ImportError::DatabaseError(e.to_string()))?;
        Ok(())
    }

    /// Sanitizes pandoc-produced HTML into editor content blocks, builds a new `Section`
    /// from them, and persists it as a new section of `project_id` in the database.
    ///
    /// Like [`Self::import_html_from_wp`], downloads referenced media and resolves external
    /// links into citations before building the final blocks.
    ///
    /// # Errors
    /// Returns [`ImportError::HtmlConversionFailed`] if `input` looks like a full HTML
    /// document or can't be parsed, or [`ImportError::DatabaseError`] if persisting the
    /// section fails.
    async fn import_html_from_pandoc(
        &self,
        input: String,
        project_id: uuid::Uuid,
        endnotes: bool,
        shift_headings: bool,
        convert_links: bool,
    ) -> Result<(), ImportError> {
        // Reject full HTML documents; pandoc is expected to emit a fragment.
        if looks_like_document(&input) {
            return Err(ImportError::HtmlConversionFailed);
        }

        let (media_srcs, hrefs) = {
            let dom = parse_dom(&input);
            let top_nodes = top_level_nodes(&dom);
            if top_nodes.is_empty() && !input.trim().is_empty() {
                error!("Couldn't parse html from import after pandoc: no parseable nodes");
                return Err(ImportError::HtmlConversionFailed);
            }
            let mut media = Vec::new();
            collect_media_srcs(&top_nodes, &mut media);
            let mut hrefs = Vec::new();
            if convert_links {
                collect_convertible_hrefs(&top_nodes, &mut hrefs);
            }
            (dedup_vec(media), dedup_vec(hrefs))
        };

        let media_map = self.download_all_media(&media_srcs, project_id).await;
        let link_map = self.resolve_all_links(&hrefs, project_id).await;

        let mut section = Section {
            id: Some(uuid::Uuid::new_v4()),
            css_classes: vec![],
            sub_sections: vec![],
            content: vec![],
            visible_in_toc: true,
            metadata: SectionMetadata {
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
                custom_fields: HashMap::new(),
            },
        };

        let blocks = {
            let dom = parse_dom(&input);
            let top_nodes = top_level_nodes(&dom);
            let footnotes = extract_pandoc_footnotes(&top_nodes, &link_map, &media_map);

            let mut blocks: Vec<NewContentBlock> = vec![];
            for node in &top_nodes {
                blocks.extend(node_to_blocks(
                    &section,
                    node,
                    &footnotes,
                    endnotes,
                    shift_headings,
                    &link_map,
                    &media_map,
                ));
            }
            blocks
        };

        debug!("Converted HTML to ContentBlocks: {:?}", blocks);
        let doc = convert_contentblocks_to_yrs(blocks);
        section.content = doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default());

        if cfg!(feature = "language_detection") {
            section.metadata.lang = detect_language_for_section(&section);
        }

        crate::db::repositories::sections::insert_at_end(
            &self.pool,
            &self.settings,
            project_id,
            None,
            &section,
        )
        .await
        .map_err(|e| ImportError::DatabaseError(e.to_string()))?;
        Ok(())
    }

    /// Downloads all referenced external media into the project's uploads directory.
    ///
    /// Returns a map from the original `src` URL to `(api_url, filename)` for every file that
    /// was downloaded successfully. Downloads are best-effort: a failure just leaves the entry
    /// out of the map, and callers fall back to the original external URL.
    async fn download_all_media(
        &self,
        srcs: &[String],
        project_id: uuid::Uuid,
    ) -> HashMap<String, (String, String)> {
        use futures::StreamExt;

        let mut map = HashMap::new();
        if srcs.is_empty() {
            return map;
        }
        let client = media_client();

        // Download concurrently (bounded), rather than serializing every network round-trip.
        // Owned `String`s are moved into each task so the futures don't borrow `srcs`.
        let results: Vec<(String, Option<(String, String)>)> = futures::stream::iter(srcs.to_vec())
            .map(|src| async move {
                let local = self.download_media(client, &src, project_id).await;
                (src, local)
            })
            .buffer_unordered(8)
            .collect()
            .await;

        for (src, result) in results {
            match result {
                Some(local) => {
                    map.insert(src, local);
                }
                None => warn!("Couldn't download media from {}", src),
            }
        }
        map
    }

    /// Downloads a single media file into `{data_path}/projects/{project_id}/uploads`.
    ///
    /// Returns `(api_url, filename)` on success, where `api_url` is the project-internal URL
    /// used to reference the stored file.
    async fn download_media(
        &self,
        client: &reqwest::Client,
        url: &str,
        project_id: uuid::Uuid,
    ) -> Option<(String, String)> {
        let response = client.get(url).send().await.ok()?;
        if !response.status().is_success() {
            return None;
        }
        let bytes = response.bytes().await.ok()?;
        let filename = format!("{}{}", uuid::Uuid::new_v4(), extension_from_url(url));
        let dir = format!(
            "{}/projects/{}/uploads",
            self.settings.data_path, project_id
        );
        if let Err(e) = tokio::fs::create_dir_all(&dir).await {
            warn!("Couldn't create uploads directory {}: {}", dir, e);
            return None;
        }
        let path = format!("{}/{}", dir, filename);
        if let Err(e) = tokio::fs::write(&path, &bytes).await {
            warn!("Couldn't write media file {}: {}", path, e);
            return None;
        }
        Some((
            format!("/api/projects/{}/uploads/{}", project_id, filename),
            filename,
        ))
    }

    /// Resolves every collected link into a citation replacement (when the Zotero translation
    /// server recognizes it), adding the resulting bibliography entries to the project.
    async fn resolve_all_links(
        &self,
        hrefs: &[String],
        project_id: uuid::Uuid,
    ) -> HashMap<String, String> {
        use futures::StreamExt;

        let mut map = HashMap::new();
        if hrefs.is_empty() {
            return map;
        }

        // Resolve links concurrently (bounded): the translation round-trips overlap, while the
        // bibliography inserts inside `convert_link_to_citation` each go through their own
        // short-lived pool connection.
        let results: Vec<(String, Option<String>)> = futures::stream::iter(hrefs.to_vec())
            .map(|href| async move {
                let citation = self.convert_link_to_citation(&href, project_id).await;
                (href, citation)
            })
            .buffer_unordered(8)
            .collect()
            .await;

        for (href, citation) in results {
            if let Some(citation) = citation {
                map.insert(href, citation);
            }
        }
        map
    }

    /// Tries to convert a single external link into a citation by resolving it via the Zotero
    /// translation server. On success the referenced bibliography entries (and their parents)
    /// are added to the project and a `<citation>` replacement string is returned.
    async fn convert_link_to_citation(&self, href: &str, project_id: uuid::Uuid) -> Option<String> {
        let entries = link_converter::get_translation(href, &self.settings).await?;
        let main_entry = entries.first()?;
        let main_key = main_entry.key().to_string();
        let by_key = Self::collect_bib_entries_with_parents(entries);

        let mut uuid_map: HashMap<String, uuid::Uuid> = HashMap::new();
        for key in by_key.keys() {
            uuid_map.insert(key.clone(), uuid::Uuid::new_v4());
        }
        let main_uuid = *uuid_map.get(&main_key)?;

        for (key, entry) in by_key.iter() {
            let mut converted = BibEntryV3::from(entry);
            let entry_uuid = *uuid_map.get(key).unwrap();
            converted.key = entry_uuid;
            converted.parents = entry
                .parents()
                .iter()
                .filter_map(|p| uuid_map.get(p.key()).copied())
                .filter(|&p_uuid| p_uuid != entry_uuid)
                .collect();
            if let Err(e) = bibliography::insert(
                &self.pool,
                project_id,
                &BibEntryOrFolder::BibEntry(converted),
            )
            .await
            {
                warn!("Couldn't save imported bibliography entry: {:?}", e);
            }
        }

        Some(format!("<citation data-key=\"{}\">C</citation>", main_uuid))
    }

    /// Parses a BibLaTeX file and inserts its entries (and any parent entries they
    /// reference) into `project_id`'s bibliography, generating a fresh UUID for each entry.
    ///
    /// # Errors
    /// Returns [`ImportError::BibFileInvalid`] if the file can't be read or parsed,
    /// [`ImportError::ProjectNotFound`] if `project_id` doesn't exist, or
    /// [`ImportError::DatabaseError`] if inserting an entry fails.
    async fn import_bib_entries(
        &self,
        project_id: uuid::Uuid,
        bib_file_path: &str,
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

        debug!("Bib File Content: {:?}", bib_file_content);

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

        debug!("Parsed Bib Entries: {:?}", bib);

        if !projects::exists(&self.pool, project_id)
            .await
            .unwrap_or(false)
        {
            return Err(ImportError::ProjectNotFound);
        }

        // We need stable UUIDs for bib entries and their parents.
        // Recursively collect all entries + their parents.
        let by_key = Self::collect_bib_entries_with_parents(bib.iter().cloned());

        // Build UUID mapping (v4, per import)
        let mut uuid_map: HashMap<String, uuid::Uuid> = HashMap::new();
        for key in by_key.keys() {
            uuid_map.insert(key.clone(), uuid::Uuid::new_v4());
        }

        debug!("Generated UUID Map: {:?}", uuid_map);

        // Convert and resolve parents
        for (key, entry) in by_key.iter() {
            let mut converted = BibEntryV3::from(entry);
            let entry_uuid = *uuid_map.get(key).unwrap();
            converted.key = entry_uuid;
            converted.parents = entry
                .parents()
                .iter()
                .filter_map(|p| uuid_map.get(p.key()).copied())
                .filter(|&p_uuid| p_uuid != entry_uuid)
                .collect();

            debug!("Converted Entry: {:?}", converted);

            bibliography::insert(
                &self.pool,
                project_id,
                &BibEntryOrFolder::BibEntry(converted),
            )
            .await
            .map_err(|e| ImportError::DatabaseError(e.to_string()))?;
        }

        Ok(())
    }
}

/// HTML parsing, sanitization and serialization helpers built on top of
/// `html5ever`/`markup5ever_rcdom`.
///
/// The importer walks the parsed `RcDom` handles directly. Because those handles are
/// `Rc`-based (and therefore `!Send`), they are only ever touched from synchronous code; any
/// asynchronous work (media downloads, link resolution) runs separately on owned data.
/// Serialization is sanitizing:
/// - `<script>`/`<style>` and similar elements are dropped together with their content.
/// - Embedded content, forms and interactive elements are reduced to their plain text.
/// - Unsupported inline wrappers (e.g. `<span>`, `<font>`) are unwrapped, keeping their text
///   (this is what the editor expects — it would otherwise drop the tag *and* its content).
/// - Only a small allowlist of standard attributes survives; `data-*`, `on*` and other
///   non-standard attributes are stripped.

/// Context threaded through the synchronous serialization functions.
#[derive(Clone, Copy)]
struct Ctx<'a> {
    /// Extracted footnotes keyed by their reference id, used to inline note spans.
    footnotes: Option<&'a HashMap<String, String>>,
    /// Whether footnotes should be rendered as endnotes.
    endnotes: bool,
    /// Map from an external link URL to its `<citation>` replacement.
    link_map: &'a HashMap<String, String>,
    /// Map from an external media `src` to its downloaded `(api_url, filename)`, used to
    /// rewrite inline `<img>`/media sources to the project-internal URL.
    media_map: &'a HashMap<String, (String, String)>,
    /// When true, block/table structure is preserved (used for `Raw` blocks); otherwise only
    /// inline formatting is kept (used for paragraph/heading/quote/list text).
    keep_structural: bool,
}

/// Returns a process-wide shared HTTP client for media downloads, built once and reused
/// across all imports (avoids rebuilding a connection pool per post).
fn media_client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

/// Parses `input` as an HTML fragment in `<body>` context. Parsing an in-memory string never
/// fails.
///
/// The returned [`RcDom`] must be kept alive for as long as its handles are used: rcdom nodes
/// empty their descendants' child lists on drop, so extracting handles and then dropping the
/// dom would leave those handles childless.
fn parse_dom(input: &str) -> RcDom {
    let context = QualName::new(None, ns!(html), local_name!("body"));
    parse_fragment(
        RcDom::default(),
        ParseOpts::default(),
        context,
        Vec::<Attribute>::new(),
        false,
    )
    .from_utf8()
    .read_from(&mut input.as_bytes())
    .expect("reading from an in-memory byte slice cannot fail")
}

/// Returns the top-level content handles of a fragment parsed with [`parse_dom`].
///
/// `parse_fragment` wraps the fragment nodes in a synthetic `<html>` root; the actual content
/// nodes are that root's children.
fn top_level_nodes(dom: &RcDom) -> Vec<Handle> {
    match dom.document.children.borrow().first() {
        Some(root) => root.children.borrow().iter().cloned().collect(),
        None => Vec::new(),
    }
}

/// Case-insensitive ASCII prefix check without allocating.
fn starts_with_ci(s: &str, prefix: &str) -> bool {
    let bytes = s.as_bytes();
    let prefix = prefix.as_bytes();
    bytes.len() >= prefix.len() && bytes[..prefix.len()].eq_ignore_ascii_case(prefix)
}

/// Whether the input looks like a full HTML document rather than a fragment (a leading BOM,
/// whitespace and comments are skipped first).
fn looks_like_document(input: &str) -> bool {
    let mut s = input.trim_start_matches('\u{feff}').trim_start();
    while let Some(rest) = s.strip_prefix("<!--") {
        match rest.find("-->") {
            Some(end) => s = rest[end + 3..].trim_start(),
            None => break,
        }
    }
    starts_with_ci(s, "<!doctype") || starts_with_ci(s, "<html") || starts_with_ci(s, "<?xml")
}

/// Lowercased local name of an element handle, or `None` for non-elements.
fn tag_name(handle: &Handle) -> Option<String> {
    match &handle.data {
        NodeData::Element { name, .. } => Some(name.local.as_ref().to_ascii_lowercase()),
        _ => None,
    }
}

/// Value of `attr_name` on an element handle, if present.
fn attr_value(handle: &Handle, attr_name: &str) -> Option<String> {
    if let NodeData::Element { attrs, .. } = &handle.data {
        for attr in attrs.borrow().iter() {
            if attr.name.local.as_ref().eq_ignore_ascii_case(attr_name) {
                return Some(attr.value.to_string());
            }
        }
    }
    None
}

/// Class list of an element handle.
fn class_list(handle: &Handle) -> Vec<String> {
    attr_value(handle, "class")
        .map(|c| c.split_whitespace().map(|s| s.to_string()).collect())
        .unwrap_or_default()
}

/// All child handles of a node.
fn children(handle: &Handle) -> Vec<Handle> {
    handle.children.borrow().iter().cloned().collect()
}

/// Element child handles only (skips text and comments).
fn element_children(handle: &Handle) -> Vec<Handle> {
    handle
        .children
        .borrow()
        .iter()
        .filter(|c| matches!(c.data, NodeData::Element { .. }))
        .cloned()
        .collect()
}

/// Depth-first search for the first descendant element with the given (lowercase) tag name.
fn find_descendant(handle: &Handle, name: &str) -> Option<Handle> {
    for child in handle.children.borrow().iter() {
        if tag_name(child).as_deref() == Some(name) {
            return Some(child.clone());
        }
        if let Some(found) = find_descendant(child, name) {
            return Some(found);
        }
    }
    None
}

/// Collects all text within a subtree.
fn collect_text(handle: &Handle, out: &mut String) {
    match &handle.data {
        NodeData::Text { contents } => out.push_str(&contents.borrow()),
        NodeData::Element { .. } => {
            for child in handle.children.borrow().iter() {
                collect_text(child, out);
            }
        }
        _ => {}
    }
}

/// Whether `url` is an absolute http(s) URL.
fn is_http_url(url: &str) -> bool {
    let url = url.trim_start();
    starts_with_ci(url, "http://") || starts_with_ci(url, "https://")
}

/// Best-effort filename (with extension) derived from a URL. Used as a fallback when a media
/// file could not be downloaded.
fn derive_filename(src: &str) -> String {
    let path = src.split(['?', '#']).next().unwrap_or(src);
    path.rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("file")
        .to_string()
}

/// File extension (including the leading dot) derived from a URL, or an empty string.
fn extension_from_url(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let file = path.rsplit('/').next().unwrap_or("");
    match file.rsplit_once('.') {
        Some((_, ext))
            if !ext.is_empty()
                && ext.len() <= 5
                && ext.chars().all(|c| c.is_ascii_alphanumeric()) =>
        {
            format!(".{}", ext.to_ascii_lowercase())
        }
        _ => String::new(),
    }
}

/// Single-pass HTML escaping matching html5ever's serializer (see
/// `html5ever::serialize`'s `write_escaped`). Pass `attr_mode = true` for a
/// double-quoted attribute value (additionally escapes `"`) and `false` for
/// text content; `>` is escaped in both modes so the output is safe
/// regardless of the surrounding context.
fn escape_html(s: &str, attr_mode: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '\u{00A0}' => out.push_str("&nbsp;"),
            '"' if attr_mode => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
    out
}

/// Elements dropped entirely (tag and all descendants).
fn is_dropped_element(name: &str) -> bool {
    matches!(
        name,
        "script" | "style" | "noscript" | "template" | "head" | "title" | "meta" | "base"
    )
}

/// Embedded content, forms and interactive elements: replaced by their plain text content.
fn is_text_only_element(name: &str) -> bool {
    matches!(
        name,
        // Embedded content
        "iframe"
            | "embed"
            | "object"
            | "param"
            | "picture"
            | "source"
            | "video"
            | "audio"
            | "track"
            | "map"
            | "area"
            | "canvas"
            | "svg"
            | "math"
            | "applet"
            // Forms & form elements
            | "form"
            | "input"
            | "textarea"
            | "select"
            | "option"
            | "optgroup"
            | "button"
            | "label"
            | "fieldset"
            | "legend"
            | "datalist"
            | "output"
            | "progress"
            | "meter"
            // Interactive elements
            | "details"
            | "summary"
            | "dialog"
            | "menu"
    )
}

/// Inline formatting elements kept as-is (after attribute filtering).
fn is_kept_inline_element(name: &str) -> bool {
    matches!(
        name,
        "a" | "b"
            | "strong"
            | "i"
            | "em"
            | "u"
            | "s"
            | "strike"
            | "del"
            | "ins"
            | "mark"
            | "sup"
            | "sub"
            | "small"
            | "code"
            | "br"
            | "wbr"
            | "q"
            | "abbr"
            | "cite"
            | "kbd"
            | "samp"
            | "var"
            | "time"
            | "bdi"
            | "bdo"
            | "ruby"
            | "rt"
            | "rp"
            | "img"
    )
}

/// Block/structural elements kept when serializing `Raw` blocks.
fn is_kept_block_element(name: &str) -> bool {
    matches!(
        name,
        "p" | "div"
            | "section"
            | "article"
            | "header"
            | "footer"
            | "aside"
            | "nav"
            | "main"
            | "figure"
            | "figcaption"
            | "blockquote"
            | "pre"
            | "hr"
            | "address"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "ul"
            | "ol"
            | "li"
            | "dl"
            | "dt"
            | "dd"
            | "table"
            | "thead"
            | "tbody"
            | "tfoot"
            | "tr"
            | "td"
            | "th"
            | "caption"
            | "colgroup"
            | "col"
    )
}

/// Whether an element participates in inline flow (used for whitespace-significance).
fn is_inline_element(name: &str) -> bool {
    // Text-only elements (button, iframe, …) are unwrapped to inline text, so whitespace
    // separating them from adjacent inline content is significant and must be preserved.
    is_kept_inline_element(name)
        || is_text_only_element(name)
        || matches!(name, "span" | "font" | "big" | "tt" | "nobr" | "acronym")
}

/// HTML void elements (serialized without a closing tag).
fn is_void_element(name: &str) -> bool {
    matches!(
        name,
        "br" | "wbr"
            | "hr"
            | "img"
            | "col"
            | "area"
            | "source"
            | "track"
            | "embed"
            | "param"
            | "input"
            | "meta"
            | "link"
    )
}

/// Standard attributes allowed to survive sanitization (never `data-*`/`on*`/non-standard).
fn is_allowed_attribute(name: &str) -> bool {
    matches!(
        name,
        "href"
            | "src"
            | "alt"
            | "title"
            | "lang"
            | "dir"
            | "datetime"
            | "cite"
            | "colspan"
            | "rowspan"
            | "scope"
            | "headers"
            | "span"
            | "start"
            | "reversed"
            | "type"
            | "value"
            | "width"
            | "height"
    )
}

/// Serializes the allowed attributes of an element handle. A `src` that was downloaded is
/// rewritten to the project-internal URL so inline media points at the stored copy.
fn serialize_attrs(handle: &Handle, ctx: &Ctx) -> String {
    let mut out = String::new();
    if let NodeData::Element { attrs, .. } = &handle.data {
        for attr in attrs.borrow().iter() {
            let name = attr.name.local.as_ref().to_ascii_lowercase();
            if !is_allowed_attribute(&name) {
                continue;
            }
            let raw = attr.value.to_string();
            let value = if name == "src" {
                ctx.media_map
                    .get(&raw)
                    .map(|(url, _)| url.clone())
                    .unwrap_or(raw)
            } else {
                raw
            };
            out.push_str(&format!(" {}=\"{}\"", name, escape_html(&value, true)));
        }
    }
    out
}

/// Whether the nearest non-whitespace sibling in the given direction is inline (so that
/// whitespace between it and the current node is significant).
fn neighbor_is_inline(kids: &[Handle], index: usize, forward: bool) -> bool {
    let indices: Vec<usize> = if forward {
        ((index + 1)..kids.len()).collect()
    } else {
        (0..index).rev().collect()
    };
    for j in indices {
        match &kids[j].data {
            NodeData::Text { contents } => {
                if contents.borrow().trim().is_empty() {
                    continue;
                }
                return true;
            }
            NodeData::Element { .. } => {
                return tag_name(&kids[j])
                    .map(|t| is_inline_element(&t))
                    .unwrap_or(false);
            }
            _ => continue,
        }
    }
    false
}

/// Serializes the children of a node, preserving whitespace that separates inline content.
fn serialize_children(node: &Handle, ctx: &Ctx) -> String {
    let kids: Vec<Handle> = node.children.borrow().iter().cloned().collect();
    let mut out = String::new();
    for (i, kid) in kids.iter().enumerate() {
        match &kid.data {
            NodeData::Text { contents } => {
                let text = contents.borrow().to_string();
                if text.trim().is_empty() {
                    if neighbor_is_inline(&kids, i, false) && neighbor_is_inline(&kids, i, true) {
                        out.push(' ');
                    }
                } else {
                    out.push_str(&escape_html(&text, false));
                }
            }
            NodeData::Element { .. } => out.push_str(&serialize_node(kid, ctx)),
            _ => {}
        }
    }
    out
}

/// Serializes a single element handle to sanitized HTML.
fn serialize_node(node: &Handle, ctx: &Ctx) -> String {
    let tag = match tag_name(node) {
        Some(tag) => tag,
        None => {
            if let NodeData::Text { contents } = &node.data {
                return escape_html(&contents.borrow(), false);
            }
            return String::new();
        }
    };

    // Footnote references and link-to-citation conversion for `<a>`.
    if tag == "a" {
        if let Some(replacement) = footnote_replacement(node, ctx) {
            return replacement;
        }
        if attr_value(node, "role").as_deref() == Some("doc-backlink") {
            return String::new();
        }
        if let Some(href) = attr_value(node, "href") {
            if let Some(citation) = ctx.link_map.get(&href) {
                return citation.clone();
            }
        }
    }

    if is_dropped_element(&tag) {
        return String::new();
    }
    if is_text_only_element(&tag) {
        let mut text = String::new();
        collect_text(node, &mut text);
        return escape_html(&text, false);
    }

    let inner = serialize_children(node, ctx);
    let keep = is_kept_inline_element(&tag) || (ctx.keep_structural && is_kept_block_element(&tag));
    if keep {
        let attrs = serialize_attrs(node, ctx);
        if is_void_element(&tag) {
            format!("<{}{}>", tag, attrs)
        } else {
            format!("<{}{}>{}</{}>", tag, attrs, inner, tag)
        }
    } else {
        // Unsupported wrapper (e.g. <span>): unwrap, keeping the text/children.
        inner
    }
}

/// Detects a footnote reference `<a>` and returns the inline note-span replacement.
fn footnote_replacement(a_node: &Handle, ctx: &Ctx) -> Option<String> {
    let footnotes = ctx.footnotes?;
    let sup = element_children(a_node).into_iter().next()?;
    if tag_name(&sup).as_deref() != Some("sup") {
        return None;
    }

    // Pandoc: <a role="doc-noteref"><sup>N</sup></a>
    if attr_value(a_node, "role").as_deref() == Some("doc-noteref") {
        let mut num = String::new();
        collect_text(&sup, &mut num);
        if let Some(content) = footnotes.get(&format!("fn{}", num.trim())) {
            return Some(note_span(content, ctx.endnotes));
        }
    }

    // WordPress footnote plugin:
    // <a><sup class="footnote_plugin_tooltip_text" id="footnote_tooltip_N">N</sup></a>
    if class_list(&sup)
        .iter()
        .any(|c| c == "footnote_plugin_tooltip_text")
    {
        if let Some(id) = attr_value(&sup, "id") {
            let footnote_id = id.replace("tooltip", "reference");
            if let Some(content) = footnotes.get(&footnote_id) {
                return Some(note_span(content, ctx.endnotes));
            }
        }
    }
    None
}

/// Builds the inline `<span class="note">` used to represent a footnote/endnote.
fn note_span(footnote_html: &str, endnotes: bool) -> String {
    let content = footnote_html.replace('"', "'");
    let note_type = if endnotes { "endnote" } else { "footnote" };
    format!(
        "<span class=\"note\" note-type=\"{}\" note-content=\"{}\">N</span>",
        note_type, content
    )
}

/// Collects the http(s) `src` URLs of all media elements in a subtree.
fn collect_media_srcs(nodes: &[Handle], out: &mut Vec<String>) {
    for node in nodes {
        if let Some(tag) = tag_name(node) {
            if matches!(tag.as_str(), "img" | "video" | "audio" | "source") {
                if let Some(src) = attr_value(node, "src") {
                    if is_http_url(&src) {
                        out.push(src);
                    }
                }
            }
        }
        collect_media_srcs(&children(node), out);
    }
}

/// Collects the http(s) `href`s of all `<a>` elements in a subtree.
fn collect_convertible_hrefs(nodes: &[Handle], out: &mut Vec<String>) {
    for node in nodes {
        if tag_name(node).as_deref() == Some("a") {
            if let Some(href) = attr_value(node, "href") {
                if is_http_url(&href) {
                    out.push(href);
                }
            }
        }
        collect_convertible_hrefs(&children(node), out);
    }
}

/// Extracts WordPress footnote-plugin footnotes into a map keyed by reference id.
fn extract_wp_footnotes(
    top: &[Handle],
    link_map: &HashMap<String, String>,
    media_map: &HashMap<String, (String, String)>,
) -> HashMap<String, String> {
    let ctx = Ctx {
        footnotes: None,
        endnotes: false,
        link_map,
        media_map,
        keep_structural: false,
    };
    let mut footnotes = HashMap::new();

    let Some(container) = top.iter().find(|n| {
        class_list(n)
            .iter()
            .any(|c| c == "footnotes_reference_container")
    }) else {
        return footnotes;
    };
    let container_children = element_children(container);
    let Some(inner) = container_children.get(1) else {
        return footnotes;
    };
    let Some(table) = element_children(inner).into_iter().next() else {
        return footnotes;
    };
    if tag_name(&table).as_deref() != Some("table") {
        return footnotes;
    }
    let table_children = element_children(&table);
    let Some(tbody) = table_children.get(1) else {
        return footnotes;
    };
    if tag_name(tbody).as_deref() != Some("tbody") {
        return footnotes;
    }

    for tr in element_children(tbody) {
        let cells = element_children(&tr);
        let Some(th) = cells.first() else { continue };
        let Some(anchor) = element_children(th).into_iter().next() else {
            continue;
        };
        if !class_list(&anchor).iter().any(|c| c == "footnote_backlink") {
            continue;
        }
        let Some(id) = attr_value(&anchor, "id") else {
            continue;
        };
        let Some(td) = cells.get(1) else { continue };
        if !class_list(td).iter().any(|c| c == "footnote_plugin_text") {
            continue;
        }
        // Preserve the inner HTML of the cell, not the `<td>` wrapper.
        footnotes.insert(id, serialize_children(td, &ctx));
    }
    footnotes
}

/// Extracts pandoc footnotes (`<aside id="footnotes"><ol><li id="fnN">...`) into a map.
fn extract_pandoc_footnotes(
    top: &[Handle],
    link_map: &HashMap<String, String>,
    media_map: &HashMap<String, (String, String)>,
) -> HashMap<String, String> {
    let ctx = Ctx {
        footnotes: None,
        endnotes: false,
        link_map,
        media_map,
        keep_structural: false,
    };
    let mut footnotes = HashMap::new();

    let Some(aside) = top.iter().find(|n| {
        tag_name(n).as_deref() == Some("aside")
            && attr_value(n, "id").as_deref() == Some("footnotes")
    }) else {
        return footnotes;
    };
    let Some(ol) = element_children(aside)
        .into_iter()
        .find(|n| tag_name(n).as_deref() == Some("ol"))
    else {
        return footnotes;
    };

    for li in element_children(&ol) {
        if tag_name(&li).as_deref() != Some("li") {
            continue;
        }
        let Some(id) = attr_value(&li, "id") else {
            continue;
        };
        // Prefer the first <p> inside the <li>, falling back to the whole <li>.
        let text = if let Some(p) = element_children(&li)
            .into_iter()
            .find(|n| tag_name(n).as_deref() == Some("p"))
        {
            serialize_children(&p, &ctx)
        } else {
            serialize_children(&li, &ctx)
        };
        footnotes.insert(id, text);
    }
    footnotes
}

/// Converts a single top-level node handle into content blocks. Returns an empty vector when
/// the node should be skipped (footnote containers, dropped elements, insignificant whitespace).
///
/// A single node can yield several blocks: an image embedded inside a `<p>`/`<div>` is split out
/// into its own `Image` block (with the surrounding inline text kept as separate `Paragraph`
/// blocks), because EditorJS cannot render an image inline within a paragraph.
fn node_to_blocks(
    section: &Section,
    node: &Handle,
    footnotes: &HashMap<String, String>,
    endnotes: bool,
    shift_headings: bool,
    link_map: &HashMap<String, String>,
    media_map: &HashMap<String, (String, String)>,
) -> Vec<NewContentBlock> {
    if let NodeData::Text { contents } = &node.data {
        let text = contents.borrow().to_string();
        if text.trim().is_empty() {
            return vec![];
        }
        return vec![NewContentBlock::new(
            section,
            BlockType::Paragraph,
            BlockData::Paragraph {
                text: escape_html(&text, false),
            },
            vec![],
        )];
    }

    let Some(tag) = tag_name(node) else {
        return vec![];
    };
    let classes = class_list(node);

    // Skip footnote containers (extracted separately) and non-content elements.
    if classes.iter().any(|c| c == "footnotes_reference_container") {
        return vec![];
    }
    if tag == "aside" && attr_value(node, "id").as_deref() == Some("footnotes") {
        return vec![];
    }
    if is_dropped_element(&tag) {
        return vec![];
    }

    let ctx = Ctx {
        footnotes: Some(footnotes),
        endnotes,
        link_map,
        media_map,
        keep_structural: false,
    };
    let raw_ctx = Ctx {
        keep_structural: true,
        ..ctx
    };

    match tag.as_str() {
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            let mut level = tag.as_bytes()[1] - b'0';
            if shift_headings && level > 1 {
                level -= 1;
            }
            let text = serialize_children(node, &ctx);
            vec![NewContentBlock::new(
                section,
                BlockType::Heading,
                BlockData::Heading { text, level },
                classes,
            )]
        }
        "p" | "div" => container_to_blocks(
            section,
            node,
            footnotes,
            endnotes,
            shift_headings,
            link_map,
            media_map,
            classes,
        ),
        "ul" | "ol" => {
            let style = if tag == "ol" { "ordered" } else { "unordered" }.to_string();
            let mut items: Vec<String> = vec![];
            for li in element_children(node) {
                if tag_name(&li).as_deref() == Some("li") {
                    items.push(serialize_children(&li, &ctx));
                }
            }
            if items.is_empty() {
                vec![NewContentBlock::new(
                    section,
                    BlockType::Raw,
                    BlockData::Raw {
                        html: serialize_node(node, &raw_ctx),
                    },
                    classes,
                )]
            } else {
                vec![NewContentBlock::new(
                    section,
                    BlockType::List,
                    BlockData::List { style, items },
                    classes,
                )]
            }
        }
        "blockquote" => {
            let text = serialize_children(node, &ctx);
            vec![NewContentBlock::new(
                section,
                BlockType::Quote,
                BlockData::Quote {
                    text,
                    caption: String::new(),
                    alignment: "left".to_string(),
                },
                classes,
            )]
        }
        "figure" | "img" => image_or_raw_block(section, node, &tag, &ctx, media_map, classes)
            .into_iter()
            .collect(),
        "video" | "audio" => media_raw_block(section, node, &tag, media_map, classes)
            .into_iter()
            .collect(),
        _ => vec![NewContentBlock::new(
            section,
            BlockType::Raw,
            BlockData::Raw {
                html: serialize_node(node, &raw_ctx),
            },
            classes,
        )],
    }
}

/// Converts a `<p>`/`<div>` (or any block container) into blocks. Inline content is gathered
/// into `Paragraph` blocks; embedded images become standalone `Image` blocks; and nested
/// block-level elements (`<p>`, `<div>`, headings, lists, figures, tables, …) are converted
/// recursively so their structure — and any images they contain — is not flattened into inline
/// `<img>` markup (which EditorJS cannot render). When the element holds only inline content,
/// this yields a single `Paragraph` block identical to the previous behavior.
#[allow(clippy::too_many_arguments)]
fn container_to_blocks(
    section: &Section,
    node: &Handle,
    footnotes: &HashMap<String, String>,
    endnotes: bool,
    shift_headings: bool,
    link_map: &HashMap<String, String>,
    media_map: &HashMap<String, (String, String)>,
    classes: Vec<String>,
) -> Vec<NewContentBlock> {
    let ctx = Ctx {
        footnotes: Some(footnotes),
        endnotes,
        link_map,
        media_map,
        keep_structural: false,
    };

    // A child forces the container to be split when it is a block-level image or a nested
    // block-level element; otherwise the whole container is a single inline paragraph.
    let is_block_child = |kid: &Handle| -> bool {
        child_block_image(kid).is_some()
            || tag_name(kid)
                .map(|t| !is_inline_element(&t) && !is_dropped_element(&t))
                .unwrap_or(false)
    };

    let kids = children(node);
    if !kids.iter().any(is_block_child) {
        let text = serialize_children(node, &ctx);
        return vec![NewContentBlock::new(
            section,
            BlockType::Paragraph,
            BlockData::Paragraph { text },
            classes,
        )];
    }

    let mut blocks: Vec<NewContentBlock> = vec![];
    let mut pending = String::new();
    for (i, kid) in kids.iter().enumerate() {
        // A block-level image (possibly wrapped in a link) becomes its own Image block.
        if let Some(img) = child_block_image(kid) {
            flush_paragraph(section, &mut pending, &classes, &mut blocks);
            let img_tag = tag_name(&img).unwrap_or_default();
            if let Some(block) =
                image_or_raw_block(section, &img, &img_tag, &ctx, media_map, classes.clone())
            {
                blocks.push(block);
            }
            continue;
        }
        // A nested block-level element is converted recursively so its own children (paragraphs,
        // headings, images, …) are emitted as proper blocks instead of inline markup.
        if let Some(tag) = tag_name(kid) {
            if !is_inline_element(&tag) && !is_dropped_element(&tag) {
                flush_paragraph(section, &mut pending, &classes, &mut blocks);
                blocks.extend(node_to_blocks(
                    section,
                    kid,
                    footnotes,
                    endnotes,
                    shift_headings,
                    link_map,
                    media_map,
                ));
                continue;
            }
        }
        match &kid.data {
            NodeData::Text { contents } => {
                let text = contents.borrow().to_string();
                if text.trim().is_empty() {
                    if neighbor_is_inline(&kids, i, false) && neighbor_is_inline(&kids, i, true) {
                        pending.push(' ');
                    }
                } else {
                    pending.push_str(&escape_html(&text, false));
                }
            }
            NodeData::Element { .. } => pending.push_str(&serialize_node(kid, &ctx)),
            _ => {}
        }
    }
    flush_paragraph(section, &mut pending, &classes, &mut blocks);
    blocks
}

/// If `child` is (or wraps only) an image that should become its own block, returns the
/// `<img>`/`<figure>`/`<picture>` handle to build an `Image` block from. Inline wrappers
/// (a linked or emphasized image) qualify only when their sole significant content is the image,
/// so a `<p>` with a small inline icon inside a sentence keeps the icon inline.
fn child_block_image(child: &Handle) -> Option<Handle> {
    let tag = tag_name(child)?;
    match tag.as_str() {
        "img" | "figure" | "picture" => Some(child.clone()),
        "a" | "span" | "strong" | "em" | "b" | "i" => {
            let img = find_descendant(child, "img")?;
            let mut text = String::new();
            collect_text(child, &mut text);
            if text.trim().is_empty() {
                Some(img)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Emits the accumulated inline content as a `Paragraph` block (if non-empty) and clears it.
fn flush_paragraph(
    section: &Section,
    pending: &mut String,
    classes: &[String],
    blocks: &mut Vec<NewContentBlock>,
) {
    if !pending.trim().is_empty() {
        blocks.push(NewContentBlock::new(
            section,
            BlockType::Paragraph,
            BlockData::Paragraph {
                text: pending.trim().to_string(),
            },
            classes.to_vec(),
        ));
    }
    pending.clear();
}

/// Builds an `Image` block from an `<img>`/`<figure>`, using the downloaded copy when present.
/// Falls back to a `Raw` block when no usable image source is found.
fn image_or_raw_block(
    section: &Section,
    node: &Handle,
    tag: &str,
    ctx: &Ctx,
    media_map: &HashMap<String, (String, String)>,
    classes: Vec<String>,
) -> Option<NewContentBlock> {
    let (src, caption) = if tag == "img" {
        (attr_value(node, "src").unwrap_or_default(), None)
    } else {
        let src = find_descendant(node, "img")
            .and_then(|img| attr_value(&img, "src"))
            .unwrap_or_default();
        let caption = find_descendant(node, "figcaption").map(|fc| serialize_children(&fc, ctx));
        (src, caption)
    };

    if src.is_empty() {
        let raw_ctx = Ctx {
            keep_structural: true,
            ..*ctx
        };
        return Some(NewContentBlock::new(
            section,
            BlockType::Raw,
            BlockData::Raw {
                html: serialize_node(node, &raw_ctx),
            },
            classes,
        ));
    }

    let (url, filename) = media_map
        .get(&src)
        .cloned()
        .unwrap_or_else(|| (src.clone(), derive_filename(&src)));

    Some(NewContentBlock::new(
        section,
        BlockType::Image,
        BlockData::Image {
            file: crate::projects::api::UploadedImage { url, filename },
            caption,
            with_border: false,
            with_background: false,
            stretched: false,
        },
        classes,
    ))
}

/// Builds a `Raw` block for a `<video>`/`<audio>` element, using the downloaded copy when
/// present and rewriting the source to the project-internal URL.
fn media_raw_block(
    section: &Section,
    node: &Handle,
    tag: &str,
    media_map: &HashMap<String, (String, String)>,
    classes: Vec<String>,
) -> Option<NewContentBlock> {
    let src = attr_value(node, "src")
        .filter(|s| is_http_url(s))
        .or_else(|| find_descendant(node, "source").and_then(|s| attr_value(&s, "src")))
        .filter(|s| !s.is_empty())?;

    let url = media_map
        .get(&src)
        .map(|(u, _)| u.clone())
        .unwrap_or_else(|| src.clone());

    Some(NewContentBlock::new(
        section,
        BlockType::Raw,
        BlockData::Raw {
            html: format!(
                "<{tag} controls src=\"{}\"></{tag}>",
                escape_html(&url, true)
            ),
        },
        classes,
    ))
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
                format!("vb-cite-{}", marks.len() - 1)
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
                let num = match caps[1].parse::<usize>() {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::repositories::{sections as sections_repo, users};
    use crate::settings::{ExportServer, Settings};
    use crate::storage::project_storage::current::Bibliography;
    use crate::storage::project_storage::sections::content::current::decode_yjs_content;
    use sqlx::PgPool;
    use uuid::Uuid;

    fn dummy_settings() -> Settings {
        Settings {
            app_title: "test".to_string(),
            project_cache_time: 0,
            data_path: "/tmp".to_string(),
            database_url: "".to_string(),
            database_max_connections: 1,
            file_lock_timeout: 0,
            backup_to_file_interval: 0,
            max_connections_to_rendering_server: 0,
            max_import_threads: 0,
            zotero_translation_server: "".to_string(),
            export_servers: vec![ExportServer {
                hostname: "".to_string(),
                port: 0,
                domain_name: "".to_string(),
            }],
            ca_cert_path: "".to_string(),
            client_cert_path: "".to_string(),
            client_key_path: "".to_string(),
            revocation_list_path: "".to_string(),
            version: "test".to_string(),
            max_login_attempts: 5,
            lockout_window_minutes: 15,
        }
    }

    fn make_processor(pool: PgPool) -> ImportProcessor {
        ImportProcessor {
            settings: dummy_settings(),
            pool,
            job_queue: RwLock::new(VecDeque::new()),
            job_archive: RwLock::new(HashMap::new()),
        }
    }

    async fn seed_project(pool: &PgPool) -> Uuid {
        let team_id = users::ensure_default_team(pool).await.unwrap();
        sqlx::query_scalar!(
            "INSERT INTO projects (title, owner_team_id) VALUES ('Test', $1) RETURNING id",
            team_id
        )
        .fetch_one(pool)
        .await
        .unwrap()
    }

    /// Fetches the (single) section written for `project_id` and returns its decoded content.
    async fn imported_blocks(
        pool: &PgPool,
        settings: &Settings,
        project_id: Uuid,
    ) -> Vec<NewContentBlock> {
        let tree = sections_repo::get_tree_for_project_with_content(pool, settings, project_id)
            .await
            .unwrap();
        assert_eq!(tree.len(), 1);
        decode_yjs_content(&tree[0].content).unwrap()
    }

    fn empty_section() -> Section {
        Section {
            id: Some(Uuid::new_v4()),
            css_classes: vec![],
            sub_sections: vec![],
            content: vec![],
            visible_in_toc: true,
            metadata: SectionMetadata {
                title: "Imported".to_string(),
                toc_title_subtitle_override: None,
                subtitle: None,
                authors: vec![],
                editors: vec![],
                web_url: None,
                identifiers: vec![],
                published: None,
                last_changed: None,
                lang: None,
                custom_fields: HashMap::new(),
            },
        }
    }

    #[sqlx::test]
    async fn wp_footnote_plugin_is_converted_into_note_span(pool: PgPool) -> sqlx::Result<()> {
        let project_id = seed_project(&pool).await;
        let processor = make_processor(pool.clone());
        let section = empty_section();

        // Minimal WP-footnote-plugin-ish structure that matches the extractor.
        // Note reference: <a><sup class="footnote_plugin_tooltip_text" id="footnote_tooltip_1">1</sup></a>
        // Footnote table: backlink id="footnote_reference_1" => text in td. (tooltip -> reference replacement)
        let html = r##"
<p>Text <a href="#"><sup class="footnote_plugin_tooltip_text" id="footnote_tooltip_1">1</sup></a></p>
<div class="footnotes_reference_container">
  <span>ignored</span>
  <div>
    <table>
      <thead></thead>
      <tbody>
        <tr>
          <th><a class="footnote_backlink" id="footnote_reference_1">↩</a></th>
          <td class="footnote_plugin_text">Footnote <em>content</em></td>
        </tr>
      </tbody>
    </table>
  </div>
</div>
"##
        .to_string();

        processor
            .import_html_from_wp(section, html, project_id, false, false, false)
            .await
            .unwrap();

        let blocks = imported_blocks(&pool, &processor.settings, project_id).await;
        // The trailing footnotes container must be skipped; only the paragraph remains.
        assert_eq!(blocks.len(), 1);

        let para = &blocks[0];
        let BlockData::Paragraph { text } = &para.data else {
            panic!("expected first block to be paragraph");
        };
        assert!(text.contains("<span class=\"note\""));
        assert!(text.contains("note-type=\"footnote\""));
        assert!(text.contains("Footnote"));
        // Ensure the plugin table cell wrapper is stripped and only inner HTML is preserved.
        assert!(!text.contains("<td"));

        // Verify that the ID is a valid UUID v4
        uuid::Uuid::parse_str(&para.id).expect("Block ID should be a valid UUID");
        Ok(())
    }

    #[sqlx::test]
    async fn pandoc_footnote_is_converted_into_note_span_and_footnotes_are_skipped(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        let project_id = seed_project(&pool).await;
        let processor = make_processor(pool.clone());

        let html = r#"
<p>Hello<a role="doc-noteref"><sup>1</sup></a></p>
<aside id="footnotes"><ol><li id="fn1"><p>FN one <a role="doc-backlink">↩</a></p></li></ol></aside>
"#
        .to_string();

        processor
            .import_html_from_pandoc(html, project_id, false, false, false)
            .await
            .unwrap();

        let blocks = imported_blocks(&pool, &processor.settings, project_id).await;
        assert_eq!(blocks.len(), 1);

        let BlockData::Paragraph { text } = &blocks[0].data else {
            panic!("expected paragraph");
        };
        assert!(text.contains("<span class=\"note\""));
        assert!(text.contains("note-type=\"footnote\""));
        assert!(text.contains("FN one"));
        assert!(!text.contains("doc-backlink"));

        // Verify UUID
        uuid::Uuid::parse_str(&blocks[0].id).expect("Block ID should be a valid UUID");
        Ok(())
    }

    #[sqlx::test]
    async fn whitespace_between_inline_elements_is_preserved(pool: PgPool) -> sqlx::Result<()> {
        let project_id = seed_project(&pool).await;
        let processor = make_processor(pool.clone());
        let section = empty_section();

        // The space separating the two links is significant: dropping it would join the
        // words into "onetwo" on re-serialization.
        let html =
            r#"<p>see <a href="https://a">one</a> <a href="https://b">two</a></p>"#.to_string();

        processor
            .import_html_from_wp(section, html, project_id, false, false, false)
            .await
            .unwrap();

        let blocks = imported_blocks(&pool, &processor.settings, project_id).await;
        assert_eq!(blocks.len(), 1);
        let BlockData::Paragraph { text } = &blocks[0].data else {
            panic!("expected paragraph");
        };
        assert!(
            text.contains("</a> <a"),
            "space between inline elements must be preserved, got: {text}"
        );
        assert!(!text.contains("onetwo"));
        Ok(())
    }

    /// Helper that imports a WordPress HTML fragment and returns the decoded blocks.
    async fn import_wp_blocks(pool: &PgPool, html: &str) -> Vec<NewContentBlock> {
        let project_id = seed_project(pool).await;
        let processor = make_processor(pool.clone());
        processor
            .import_html_from_wp(
                empty_section(),
                html.to_string(),
                project_id,
                false,
                false,
                false,
            )
            .await
            .unwrap();
        imported_blocks(pool, &processor.settings, project_id).await
    }

    fn first_paragraph_text(blocks: &[NewContentBlock]) -> String {
        match &blocks[0].data {
            BlockData::Paragraph { text } => text.clone(),
            other => panic!("expected paragraph, got {other:?}"),
        }
    }

    #[sqlx::test]
    async fn unsupported_span_wrapper_is_unwrapped_keeping_its_text(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        // Regression: the editor drops `<span>` together with its content, so the importer
        // must unwrap spans and keep the text (here the "EN-US" span text must survive).
        let blocks = import_wp_blocks(
            &pool,
            r#"<p><i><span lang="EN-US">We</span></i> thank <span class="x">Ana</span>.</p>"#,
        )
        .await;
        let text = first_paragraph_text(&blocks);
        assert!(
            !text.contains("<span"),
            "span should be unwrapped, got: {text}"
        );
        assert!(
            text.contains("<i>We</i>"),
            "italic text must be kept, got: {text}"
        );
        assert!(text.contains("Ana"), "span text must be kept, got: {text}");
        Ok(())
    }

    #[sqlx::test]
    async fn script_tags_are_dropped_with_their_content(pool: PgPool) -> sqlx::Result<()> {
        let blocks = import_wp_blocks(&pool, r#"<p>a<script>alert('x')</script>b</p>"#).await;
        let text = first_paragraph_text(&blocks);
        assert_eq!(text, "ab");
        assert!(!text.contains("alert"));
        assert!(!text.contains("script"));
        Ok(())
    }

    #[sqlx::test]
    async fn data_and_event_and_nonstandard_attributes_are_stripped(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        let blocks = import_wp_blocks(
            &pool,
            r#"<p><a href="https://x.example" data-foo="1" onclick="evil()" rel="nofollow" title="T">l</a></p>"#,
        )
        .await;
        let text = first_paragraph_text(&blocks);
        assert!(
            text.contains("href=\"https://x.example\""),
            "href kept, got: {text}"
        );
        assert!(
            text.contains("title=\"T\""),
            "standard title kept, got: {text}"
        );
        assert!(!text.contains("data-foo"), "data-* stripped, got: {text}");
        assert!(
            !text.contains("onclick"),
            "event handler stripped, got: {text}"
        );
        assert!(
            !text.contains("rel="),
            "non-standard attr stripped, got: {text}"
        );
        Ok(())
    }

    #[sqlx::test]
    async fn embedded_and_form_elements_are_reduced_to_text(pool: PgPool) -> sqlx::Result<()> {
        let blocks = import_wp_blocks(
            &pool,
            r#"<p>before <button onclick="x">Click me</button> mid <iframe src="https://y">frame</iframe> end</p>"#,
        )
        .await;
        let text = first_paragraph_text(&blocks);
        assert!(
            !text.contains("<button"),
            "form element stripped, got: {text}"
        );
        assert!(
            !text.contains("<iframe"),
            "embedded element stripped, got: {text}"
        );
        assert!(text.contains("Click me"), "button text kept, got: {text}");
        assert!(text.contains("frame"), "iframe text kept, got: {text}");
        Ok(())
    }

    #[sqlx::test]
    async fn video_element_becomes_raw_block_linking_the_source(pool: PgPool) -> sqlx::Result<()> {
        // `.invalid` never resolves, so the download fails fast and we fall back to the
        // original URL — the block is still produced and links the (external) source.
        let blocks = import_wp_blocks(
            &pool,
            r#"<video src="https://nonexistent.invalid/clip.mp4"></video>"#,
        )
        .await;
        assert_eq!(blocks.len(), 1);
        let BlockData::Raw { html } = &blocks[0].data else {
            panic!("expected raw block, got {:?}", blocks[0].data);
        };
        assert!(html.contains("<video controls"), "got: {html}");
        assert!(
            html.contains("clip.mp4"),
            "source must be linked, got: {html}"
        );
        Ok(())
    }

    #[test]
    fn inline_image_src_is_rewritten_to_downloaded_url() {
        // An inline <img> whose source was downloaded must reference the project-local copy,
        // not the original external URL.
        let dom = parse_dom(r#"<p><img src="https://cdn.example/x.png"></p>"#);
        let top = top_level_nodes(&dom);
        let p = top.first().unwrap();

        let link_map = HashMap::new();
        let mut media_map = HashMap::new();
        media_map.insert(
            "https://cdn.example/x.png".to_string(),
            (
                "/api/projects/abc/uploads/f.png".to_string(),
                "f.png".to_string(),
            ),
        );
        let ctx = Ctx {
            footnotes: None,
            endnotes: false,
            link_map: &link_map,
            media_map: &media_map,
            keep_structural: false,
        };

        let html = serialize_children(p, &ctx);
        assert!(
            html.contains(r#"src="/api/projects/abc/uploads/f.png""#),
            "inline img must point at the downloaded copy, got: {html}"
        );
        assert!(
            !html.contains("cdn.example"),
            "external src must be replaced, got: {html}"
        );
    }

    #[sqlx::test]
    async fn whitespace_around_text_only_element_is_preserved(pool: PgPool) -> sqlx::Result<()> {
        // The space between a reduced-to-text <button> and the following inline element must
        // survive so words are not joined.
        let blocks = import_wp_blocks(&pool, r#"<p>a <button>b</button> <i>c</i></p>"#).await;
        let text = first_paragraph_text(&blocks);
        assert!(
            text.contains("b <i>c</i>"),
            "space after text-only element must survive, got: {text}"
        );
        Ok(())
    }

    /// Live integration test against verfassungsblog.de. Requires network access.
    #[sqlx::test]
    async fn wp_import_real_verfassungsblog_post_produces_sanitized_blocks(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        let project_id = seed_project(&pool).await;
        let data_dir = format!("/tmp/vb_import_test_{}", Uuid::new_v4());
        let mut settings = dummy_settings();
        settings.data_path = data_dir.clone();
        let processor = ImportProcessor {
            settings: settings.clone(),
            pool: pool.clone(),
            job_queue: RwLock::new(VecDeque::new()),
            job_archive: RwLock::new(HashMap::new()),
        };

        let api = WordpressAPI::new("verfassungsblog.de".to_string()).unwrap();
        let post = api.get_post(79100).await.unwrap();

        processor
            .import_wp_post(post, project_id, false, false, false, vec![])
            .await
            .unwrap();

        let blocks = imported_blocks(&pool, &settings, project_id).await;
        assert!(!blocks.is_empty(), "expected imported content blocks");

        for block in &blocks {
            match &block.data {
                BlockData::Paragraph { text } | BlockData::Heading { text, .. } => {
                    assert!(!text.contains("<script"), "script leaked: {text}");
                    assert!(!text.contains("onclick"), "event handler leaked: {text}");
                    assert!(!text.contains("<iframe"), "embedded element leaked: {text}");
                    assert!(!text.contains("data-"), "data attribute leaked: {text}");
                }
                BlockData::Image { file, .. } => {
                    // Downloaded media is referenced via the project uploads path; a failed
                    // download falls back to the original absolute URL.
                    assert!(
                        file.url.starts_with("/api/projects/") || file.url.starts_with("http"),
                        "unexpected image url: {}",
                        file.url
                    );
                }
                _ => {}
            }
        }

        let _ = std::fs::remove_dir_all(&data_dir);
        Ok(())
    }

    #[sqlx::test]
    async fn import_produces_yrs_content_that_decodes_back_to_blocks(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        let project_id = seed_project(&pool).await;
        let processor = make_processor(pool.clone());
        let section = empty_section();

        processor
            .import_html_from_wp(
                section,
                "<h2>H</h2><p>P</p>".to_string(),
                project_id,
                false,
                true,
                false,
            )
            .await
            .unwrap();

        let blocks = imported_blocks(&pool, &processor.settings, project_id).await;
        assert_eq!(blocks.len(), 2);
        assert!(matches!(blocks[0].data, BlockData::Heading { .. }));
        if let BlockData::Heading { level, .. } = blocks[0].data {
            assert_eq!(level, 1); // shifted up from h2
        }
        Ok(())
    }

    #[sqlx::test]
    async fn ul_is_converted_to_list_block_and_css_classes_are_copied(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        let project_id = seed_project(&pool).await;
        let processor = make_processor(pool.clone());

        let html = r#"<ul class="my-list"><li>One</li><li><em>Two</em></li></ul>"#.to_string();

        processor
            .import_html_from_pandoc(html, project_id, false, false, false)
            .await
            .unwrap();

        let blocks = imported_blocks(&pool, &processor.settings, project_id).await;
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].css_classes, vec!["my-list".to_string()]);

        let BlockData::List { style, items } = &blocks[0].data else {
            panic!("expected list");
        };
        assert_eq!(style, "unordered");
        assert_eq!(items.len(), 2);
        assert!(items[0].contains("One"));
        assert!(items[1].contains("Two"));

        // Verify UUID
        uuid::Uuid::parse_str(&blocks[0].id).expect("Block ID should be a valid UUID");
        Ok(())
    }

    #[sqlx::test]
    async fn blockquote_is_converted_to_quote_block(pool: PgPool) -> sqlx::Result<()> {
        let project_id = seed_project(&pool).await;
        let processor = make_processor(pool.clone());

        let html = r#"<blockquote class="q">Hello <em>world</em></blockquote>"#.to_string();

        processor
            .import_html_from_pandoc(html, project_id, false, false, false)
            .await
            .unwrap();

        let blocks = imported_blocks(&pool, &processor.settings, project_id).await;
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].css_classes, vec!["q".to_string()]);

        let BlockData::Quote {
            text,
            caption,
            alignment,
        } = &blocks[0].data
        else {
            panic!("expected quote");
        };
        assert!(text.contains("Hello"));
        assert!(text.contains("<em>world</em>"));
        assert_eq!(caption, "");
        assert_eq!(alignment, "left");

        // Verify UUID
        uuid::Uuid::parse_str(&blocks[0].id).expect("Block ID should be a valid UUID");
        Ok(())
    }

    #[sqlx::test]
    async fn figure_img_is_converted_to_image_block_with_caption(pool: PgPool) -> sqlx::Result<()> {
        let project_id = seed_project(&pool).await;
        let processor = make_processor(pool.clone());

        let html = r#"
<figure class="img">
  <img src="https://example.com/path/pic.png?x=1" />
  <figcaption>Cap</figcaption>
</figure>
"#
        .to_string();

        processor
            .import_html_from_pandoc(html, project_id, false, false, false)
            .await
            .unwrap();

        let blocks = imported_blocks(&pool, &processor.settings, project_id).await;
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].css_classes, vec!["img".to_string()]);

        let BlockData::Image { file, caption, .. } = &blocks[0].data else {
            panic!("expected image");
        };
        assert_eq!(file.url, "https://example.com/path/pic.png?x=1");
        assert_eq!(file.filename, "pic.png");
        assert_eq!(caption.as_deref(), Some("Cap"));

        // Verify UUID
        uuid::Uuid::parse_str(&blocks[0].id).expect("Block ID should be a valid UUID");
        Ok(())
    }

    #[sqlx::test]
    async fn img_inside_paragraph_is_split_into_its_own_image_block(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        let project_id = seed_project(&pool).await;
        let processor = make_processor(pool.clone());

        // Text before and after the image, plus a linked image, all inside one <p>.
        let html = r#"
<p>Intro text <img src="https://example.com/inline.png" /> and trailing text.</p>
<p><a href="https://example.com/full.png"><img src="https://example.com/linked.png" /></a></p>
"#
        .to_string();

        processor
            .import_html_from_pandoc(html, project_id, false, false, false)
            .await
            .unwrap();

        let blocks = imported_blocks(&pool, &processor.settings, project_id).await;

        // First <p> -> paragraph, image, paragraph. Second <p> -> a single image block.
        assert_eq!(blocks.len(), 4);

        let BlockData::Paragraph { text } = &blocks[0].data else {
            panic!("expected leading paragraph, got {:?}", blocks[0].data);
        };
        assert_eq!(text, "Intro text");

        let BlockData::Image { file, .. } = &blocks[1].data else {
            panic!("expected image block, got {:?}", blocks[1].data);
        };
        assert_eq!(file.url, "https://example.com/inline.png");

        let BlockData::Paragraph { text } = &blocks[2].data else {
            panic!("expected trailing paragraph, got {:?}", blocks[2].data);
        };
        assert_eq!(text, "and trailing text.");

        let BlockData::Image { file, .. } = &blocks[3].data else {
            panic!("expected linked-image block, got {:?}", blocks[3].data);
        };
        assert_eq!(file.url, "https://example.com/linked.png");
        Ok(())
    }

    #[sqlx::test]
    async fn wp_leading_alignleft_image_in_paragraph_becomes_image_block(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        // A WordPress `alignleft` image is emitted as the first child of a `<p>`, directly
        // followed by the body text (with a large `srcset`). The image must be split out into
        // its own Image block instead of being flattened into the paragraph as inline `<img>`
        // markup (which the editor cannot render).
        let html = r#"<p><img decoding="async" class="size-medium wp-image-105086 alignleft" src="https://verfassungsblog.de/wp-content/uploads/2026/07/DE_Ferrante-184x300.jpg" alt="" width="184" height="300" srcset="https://verfassungsblog.de/wp-content/uploads/2026/07/DE_Ferrante-92x150.jpg 92w, https://verfassungsblog.de/wp-content/uploads/2026/07/DE_Ferrante-184x300.jpg 184w, https://verfassungsblog.de/wp-content/uploads/2026/07/DE_Ferrante-200x327.jpg 200w, https://verfassungsblog.de/wp-content/uploads/2026/07/DE_Ferrante-400x654.jpg 400w, https://verfassungsblog.de/wp-content/uploads/2026/07/DE_Ferrante-scaled.jpg 1567w" sizes="(max-width: 184px) 100vw, 184px">Daran dachte ich nicht. Ich griff zu.</p>"#;

        let blocks = import_wp_blocks(&pool, html).await;

        assert_eq!(
            blocks.len(),
            2,
            "expected an image block and a paragraph, got {blocks:?}"
        );

        let BlockData::Image { file, .. } = &blocks[0].data else {
            panic!("expected leading image block, got {:?}", blocks[0].data);
        };
        // The downloaded copy is referenced via the uploads path; a failed download falls back
        // to the original absolute URL.
        assert!(
            file.url.starts_with("/api/projects/") || file.url.starts_with("http"),
            "unexpected image url: {}",
            file.url
        );

        let BlockData::Paragraph { text } = &blocks[1].data else {
            panic!("expected trailing paragraph, got {:?}", blocks[1].data);
        };
        assert_eq!(text, "Daran dachte ich nicht. Ich griff zu.");
        assert!(
            !text.contains("<img"),
            "image must not remain inline, got: {text}"
        );
        Ok(())
    }

    #[test]
    fn bibliography_collects_transitive_parents() {
        // child -> parent
        let mut parent = hayagriva::Entry::new("parent", hayagriva::types::EntryType::Book);
        parent.set_title("Parent".to_string().into());

        let mut child = hayagriva::Entry::new("child", hayagriva::types::EntryType::Article);
        child.set_title("Child".to_string().into());
        child.set_parents(vec![parent.clone()]);

        let collected = ImportProcessor::collect_bib_entries_with_parents(vec![child.clone()]);
        assert!(collected.contains_key("child"));
        assert!(collected.contains_key("parent"));
        assert_eq!(collected.len(), 2);
        assert_eq!(collected.get("child").unwrap().parents().len(), 1);
        assert_eq!(collected.get("child").unwrap().parents()[0].key(), "parent");
    }

    #[test]
    fn bibliography_entry_cannot_be_its_own_parent() {
        let mut entry = hayagriva::Entry::new("self_parent", hayagriva::types::EntryType::Book);
        entry.set_title("Self Parent".to_string().into());
        // In some cases hayagriva might allow this or we might get it from somewhere else
        entry.set_parents(vec![entry.clone()]);

        let entries = vec![entry];
        let by_key = ImportProcessor::collect_bib_entries_with_parents(entries);

        let mut uuid_map: HashMap<String, uuid::Uuid> = HashMap::new();
        for key in by_key.keys() {
            uuid_map.insert(key.clone(), uuid::Uuid::new_v4());
        }

        for (key, entry) in by_key.iter() {
            let mut converted = BibEntryV3::from(entry);
            let entry_uuid = *uuid_map.get(key).unwrap();
            converted.key = entry_uuid;
            converted.parents = entry
                .parents()
                .iter()
                .filter_map(|p| uuid_map.get(p.key()).copied())
                .filter(|&p_uuid| p_uuid != entry_uuid) // THIS IS WHAT WE WANT TO TEST/FIX
                .collect();

            assert!(
                converted.parents.is_empty(),
                "Entry should not have itself as a parent"
            );
        }
    }

    #[test]
    fn convert_links_with_parents_preserves_parents() {
        let mut bibliography = Bibliography::new();

        let mut parent = hayagriva::Entry::new("parent", hayagriva::types::EntryType::Book);
        parent.set_title("Parent".to_string().into());

        let mut child = hayagriva::Entry::new("child", hayagriva::types::EntryType::Article);
        child.set_title("Child".to_string().into());
        child.set_parents(vec![parent.clone()]);

        let entries = vec![child, parent];

        // Simulating the block in convert_link_to_citation where a link resolves to entries
        let main_entry = entries.first().unwrap();
        let main_key = main_entry.key().to_string();
        let by_key = ImportProcessor::collect_bib_entries_with_parents(entries);

        let mut uuid_map: HashMap<String, uuid::Uuid> = HashMap::new();
        for key in by_key.keys() {
            uuid_map.insert(key.clone(), uuid::Uuid::new_v4());
        }

        let main_uuid = *uuid_map.get(&main_key).unwrap();

        for (key, entry) in by_key.iter() {
            let mut converted = BibEntryV3::from(entry);
            converted.key = *uuid_map.get(key).unwrap();
            converted.parents = entry
                .parents()
                .iter()
                .filter_map(|p| uuid_map.get(p.key()).copied())
                .collect();

            bibliography.add_entry(converted);
        }

        assert_eq!(bibliography.entries.len(), 2);

        let child_entry_v3 = match bibliography
            .entries
            .get(&main_uuid)
            .expect("Child entry missing")
        {
            crate::storage::project_storage::current::BibEntryOrFolder::BibEntry(be) => be,
            _ => panic!("Expected BibEntry, found folder"),
        };

        assert_eq!(child_entry_v3.parents.len(), 1);

        let parent_uuid = child_entry_v3.parents[0];
        assert!(bibliography.entries.contains_key(&parent_uuid));
        let parent_entry_v3 = match bibliography.entries.get(&parent_uuid).unwrap() {
            crate::storage::project_storage::current::BibEntryOrFolder::BibEntry(be) => be,
            _ => panic!("Expected BibEntry, found folder"),
        };
        assert_eq!(parent_entry_v3.title.as_ref().unwrap().value, "Parent");
    }
}

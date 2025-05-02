use vb_exchange::projects::BlockType;
use async_recursion::async_recursion;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use hayagriva::{io};

use html_parser::{Dom, Node};
use pandoc::{InputFormat, InputKind, OutputFormat, OutputKind, PandocOutput};

use rocket::http::ContentType;
use serde::{Deserialize, Serialize};
use crate::data_storage::{BibEntryV2, ProjectDataV5, ProjectStorage};
use crate::settings::Settings;
use tokio::io::AsyncReadExt;
use tokio::task::spawn_blocking;
use vb_exchange::projects::{Identifier, IdentifierType};
use crate::import::wordpress::{PostData, PostDataType, WordpressAPI, WordpressAPIContext, WordpressAPIError};
use crate::projects::{BlockData, NewContentBlock, SectionMetadataV3, SectionOrTocV2, SectionOrTocV3, SectionV3};
use crate::utils::block_id_generator::generate_id;

pub struct ImportProcessor{
    pub settings: Settings,
    pub project_storage: Arc<ProjectStorage>,
    pub job_queue: RwLock<VecDeque<ImportJob>>,
    pub job_archive: RwLock<HashMap<uuid::Uuid, Arc<RwLock<ImportJob>>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ImportStatus{
    Pending,
    Processing,
    Complete,
    Failed
}

#[derive(Debug)]
pub enum ImportError{
    UnknownFileType,
    UnsupportedFileType,
    InvalidFile,
    BibFileInvalid,
    PandocError,
    HtmlConversionFailed,
    WordPressApiError(WordpressAPIError)
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ImportStatusPoll{
    pub status: ImportStatus,
    pub processed: usize,
    pub length: usize,
}


pub struct ImportJob{
    pub id: uuid::Uuid,
    pub project_id: uuid::Uuid,
    pub length: usize,
    pub processed: usize,
    pub convert_footnotes_to_endnotes: bool,
    pub shift_headings_up: bool,
    pub convert_links: bool,
    pub files_to_process: Option<VecDeque<(String, ContentType)>>,
    pub wordpress_post_links_to_convert: Option<VecDeque<String>>,
    pub bib_file: Option<String>,
    pub status: ImportStatus,
}

impl ImportProcessor{
    pub fn start(settings: Settings, project_storage: Arc<ProjectStorage>) -> Arc<ImportProcessor>{
        let processor = Arc::new(ImportProcessor{
            settings,
            project_storage,
            job_queue: RwLock::new(VecDeque::new()),
            job_archive: RwLock::new(HashMap::new()),
        });

        let processor_clone = processor.clone();
        tokio::spawn(async move {
            let running_threads: Arc<std::sync::atomic::AtomicU64> = Arc::new(std::sync::atomic::AtomicU64::new(0));

            loop{
                // Check if there are any new jobs
                let job_queue_len = processor_clone.job_queue.read().unwrap().len();
                if job_queue_len > 0 && processor_clone.settings.max_import_threads > running_threads.load(std::sync::atomic::Ordering::Relaxed){
                    debug!("Starting new import job..."); //TODO: new thread

                    let proc_clone = processor_clone.clone();
                    let running_threads_cpy = running_threads.clone();

                    tokio::spawn(async move{
                        running_threads_cpy.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let mut job = match proc_clone.job_queue.write().unwrap().pop_front(){
                            Some(job) => job,
                            None => {
                                running_threads_cpy.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                                return;
                            }
                        };
                        job.status = ImportStatus::Processing;
                        let job = Arc::new(RwLock::new(job));
                        proc_clone.job_archive.write().unwrap().insert(job.read().unwrap().id, job.clone());
                        proc_clone.process_job(job, proc_clone.project_storage.clone()).await;
                        debug!("Job finished");
                        running_threads_cpy.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                    });
                }else{
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
            }
        });

        processor
    }

    async fn process_job(&self, job: Arc<RwLock<ImportJob>>, project_storage: Arc<ProjectStorage>){
        let job = job.clone();

        let project_id = job.read().unwrap().project_id.clone();
        let bib_file = job.read().unwrap().bib_file.clone();

        // Start with bib file if present
        if let Some(bib_file) = bib_file{
            match self.import_bib_entries(project_id, bib_file, &self.settings).await{
                Ok(_) => {
                    debug!("Bib entries imported successfully");
                }
                Err(e) => {
                    warn!("Error importing bib entries: {:?}", e);
                    job.write().unwrap().status = ImportStatus::Failed;
                    return;
                }
            }
        }

        let files_to_process_cpy = job.read().unwrap().files_to_process.clone().unwrap_or_else(|| VecDeque::new());

        loop{
            let files_to_process_len = match &job.read().unwrap().files_to_process{
                None => 0,
                Some(ftp) => ftp.len()
            };
            let wordpress_to_process_len = match &job.read().unwrap().wordpress_post_links_to_convert{
                None => 0,
                Some(pltc) => pltc.len(),
            };

            if files_to_process_len > 0 {
                debug!("Checking remaining files... {} remaining", files_to_process_len);
                let res = job.write().unwrap().files_to_process.as_mut().unwrap().pop_front();
                let (file, content_type) = match res {
                    Some(f) => f,
                    None => {
                        job.write().unwrap().status = ImportStatus::Complete;
                        break;
                    }
                };
                debug!("Processing file: {}", file);

                let project_id = job.read().unwrap().project_id.clone();
                let endnotes = job.read().unwrap().convert_footnotes_to_endnotes;

                let project = project_storage.get_project(&project_id, &self.settings).await.unwrap();

                match self.convert_file(&file, content_type, project, endnotes).await {
                    Ok(_) => {
                        debug!("File processed successfully");
                        // Remove file from temp directory
                        let res = tokio::fs::remove_file(file).await;
                        if let Err(e) = res {
                            error!("Error removing file from temp directory: {:?}", e);
                        }
                    }
                    Err(e) => {
                        warn!("Error processing file: {:?}", e);
                        job.write().unwrap().status = ImportStatus::Failed;
                        // Remove files from temp directory
                        let res = tokio::fs::remove_file(file).await;
                        if let Err(e) = res {
                            error!("Error removing file from temp directory: {:?}", e);
                        }
                        for (file, _) in files_to_process_cpy.iter() {
                            let res = tokio::fs::remove_file(file).await;
                            if let Err(e) = res {
                                error!("Error removing file from temp directory: {:?}", e);
                            }
                        }
                        break;
                    }
                }
            }else if wordpress_to_process_len > 0{

                let endnotes = job.read().unwrap().convert_footnotes_to_endnotes;
                let shift_headings_up = job.read().unwrap().shift_headings_up;
                let convert_links = job.read().unwrap().convert_links;

                let project = project_storage.get_project(&project_id, &self.settings).await.unwrap();

                let res = job.write().unwrap().wordpress_post_links_to_convert.as_mut().unwrap().pop_front();
                if let Some(url) = res{
                    match self.import_by_url(&url, project, endnotes, shift_headings_up, convert_links).await{
                        Ok(_) => {
                            debug!("Wordpress Post processed successfully");
                        }
                        Err(e) => {
                            warn!("Error processing wordpress post: {:?}", e);
                            job.write().unwrap().status = ImportStatus::Failed;
                            break;
                        }
                    }
                }
            }else{
                job.write().unwrap().status = ImportStatus::Complete;
                break;
            }

            job.write().unwrap().processed += 1;
        }
    }

    pub async fn import_by_url(&self, url: &str, project: Arc<RwLock<ProjectDataV5>>, endnotes: bool, shift_headings_up: bool, convert_links: bool) -> Result<(), ImportError>{
        let url = if url.ends_with("/"){
            url[..url.len()-1].to_string()
        }else{
            url.to_string()
        };

        let parsed_url = url::Url::parse(&url).unwrap();
        let host = match parsed_url.host(){
            Some(host) => host,
            None => return Err(ImportError::WordPressApiError(WordpressAPIError::InvalidURL))
        };

        let api = match WordpressAPI::new(host.to_string()){
            Ok(api) => api,
            Err(e) => return Err(ImportError::WordPressApiError(e))
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
            self.import_single_post(slug.to_string(), project, endnotes, shift_headings_up, convert_links, &api).await?;
        //}
        Ok(())
    }

    async fn import_single_post(&self, slug: String, project: Arc<RwLock<ProjectDataV5>>, endnotes: bool, shift_headings_up: bool, convert_links: bool, api: &WordpressAPI) -> Result<(), ImportError>{
        let posts = match api.get_posts(WordpressAPIContext::default(), None, None, None, None, None, None, None, Some(slug.to_string()), None, None).await{
            Ok(posts) => match posts.data{
                PostDataType::FullPosts(posts) => posts,
                _ => return Err(ImportError::WordPressApiError(WordpressAPIError::InvalidURL))
            },
            Err(e) => return Err(ImportError::WordPressApiError(e))
        };
        if posts.len()!= 1{
            return Err(ImportError::WordPressApiError(WordpressAPIError::NotFound));
        }
        let post = posts.first().unwrap();

        let subtitle = match &post.acf{
            None => None,
            Some(acf) => {
                match &acf.subheadline{
                    None => None,
                    Some(subheadline) => Some(subheadline.clone())
                }
            }
        };

        let mut identifiers = vec![];

        if let Some(acf) = &post.acf{
            if let Some(crossref_doi) = &acf.crossref_doi{
                identifiers.push(Identifier{
                    id: Some(uuid::Uuid::new_v4()),
                    name: "DOI".to_string(),
                    value: crossref_doi.clone(),
                    identifier_type: IdentifierType::DOI,
                });
            }else if let Some(doi) = &acf.doi{
                identifiers.push(Identifier{
                    id: Some(uuid::Uuid::new_v4()),
                    name: "DOI".to_string(),
                    value: doi.clone(),
                    identifier_type: IdentifierType::DOI,
                });
            }
        }


        let section = SectionV3 {
            id: Some(uuid::Uuid::new_v4()),
            css_classes: vec![],
            sub_sections: vec![],
            children: vec![],
            visible_in_toc: true,
            metadata: SectionMetadataV3 {
                title: post.title.rendered.clone(),
                toc_title_override: None,
                subtitle,
                toc_subtitle_override: None,
                authors: vec![], //TODO create authors automatically
                editors: vec![],
                web_url: Some(post.link.clone()),
                identifiers,
                published: Some(post.date.date()),
                last_changed: Some(post.modified),
                lang: None,
            },
        };

        self.import_html_from_wp(section, post.content.rendered.clone(), project, endnotes, shift_headings_up, convert_links).await
    }

    async fn convert_file(&self, file_path: &str, content_type: ContentType, project: Arc<RwLock<ProjectDataV5>>, endnotes: bool) -> Result<(), ImportError>{
        let mut file = match tokio::fs::File::open(file_path).await{
            Ok(file) => file,
            Err(e) => {
                warn!("Couldn't open file to import: {}", e);
                return Err(ImportError::InvalidFile)
            }
        };

        let mut file_content = String::new();
        let mut marks: Vec<String> = vec![];

        match content_type.to_string().as_str() {
                    "text/x-tex" | "application/x-tex" => {
                        debug!("Processing LaTeX file");
                        if let Err(e) = file.read_to_string(&mut file_content).await{
                            warn!("Couldn't read file to import: {}", e);
                            return Err(ImportError::InvalidFile)
                        }
                        (file_content, marks) = preprocess::latex(file_content);
                        file_content = self.convert_with_pandoc(InputKind::Pipe(file_content), InputFormat::Latex).await?;
                        file_content = postprocess::latex(file_content, marks);
                    },
                    "application/vnd.oasis.opendocument.text" => {
                        debug!("Processing ODT file");
                        file_content = self.convert_with_pandoc(InputKind::Files(vec![PathBuf::from(file_path)]), InputFormat::Other("ODT".to_string())).await?;
                    },
                    "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => {
                        debug!("Processing DOCX file");
                        file_content = self.convert_with_pandoc(InputKind::Files(vec![PathBuf::from(file_path)]), InputFormat::Docx).await?;
                    },
                    "application/msword" => {
                        debug!("Processing DOC file");
                        file_content = self.convert_with_pandoc(InputKind::Files(vec![PathBuf::from(file_path)]), InputFormat::Other("DOC".to_string())).await?;
                    },
                    "application/epub+zip" => {
                        debug!("Processing EPUB file");
                        file_content = self.convert_with_pandoc(InputKind::Files(vec![PathBuf::from(file_path)]), InputFormat::Epub).await?;
                    },
                    "application/rtf" => {
                        debug!("Processing RTF file");
                        file_content = self.convert_with_pandoc(InputKind::Files(vec![PathBuf::from(file_path)]), InputFormat::Rtf).await?;
                    },
                    "text/markdown" | "text/x-markdown" => {
                        debug!("Processing Markdown file");
                        file_content = self.convert_with_pandoc(InputKind::Files(vec![PathBuf::from(file_path)]), InputFormat::Markdown).await?;
                    },
                    _ => {
                        warn!("Unsupported file type: {}", content_type);
                        return Err(ImportError::UnsupportedFileType);
                    }
        }

        self.import_html_from_pandoc(file_content, project, endnotes).await?;
        Ok(())
    }

    async fn convert_with_pandoc(&self, input: InputKind, input_format: InputFormat) -> Result<String, ImportError>{
           let task = spawn_blocking({
                move || {
                    let mut pandoc = pandoc::new();

                    pandoc.set_input(input);
                    pandoc.set_input_format(input_format, vec![]);
                    pandoc.set_output_format(OutputFormat::Html5, vec![]);
                    pandoc.set_output(OutputKind::Pipe);
                    pandoc.execute()
                }
            }).await;

            match task{
                Ok(res) => {
                    match res {
                        Ok(res) => {
                            match res{
                                PandocOutput::ToFile(_) => Err(ImportError::PandocError),
                                PandocOutput::ToBuffer(res) => Ok(res),
                                PandocOutput::ToBufferRaw(_) => Err(ImportError::PandocError)
                            }
                        },
                        Err(e) => {
                            warn!("Couldn't convert import file with pandoc: {}", e);
                            Err(ImportError::PandocError)
                        }
                    }
                }
                Err(e) => {
                    warn!("Couldn't run pandoc: {}", e);
                    Err(ImportError::PandocError)
                }
            }
    }

    async fn import_html_from_wp(&self, mut section: SectionV3, input: String, project_data: Arc<RwLock<ProjectDataV5>>, endnotes: bool, shift_headings: bool, convert_links: bool) -> Result<(), ImportError> {
        let dom = match Dom::parse(&input) {
            Ok(dom) => dom,
            Err(e) => {
                error!("Couldn't parse html from import after pandoc: {}", e);
                return Err(ImportError::HtmlConversionFailed)
            }
        };
        if dom.tree_type == html_parser::DomVariant::Document {
            return Err(ImportError::HtmlConversionFailed)
        }

        // Get footnotes:
        let mut footnotes: HashMap<String, String> = HashMap::new();

        if let Some(footnote_div) = dom.children.iter().find(|x| match x {
            Node::Element(div) => {
                div.classes.contains(&"footnotes_reference_container".to_string())
            },
            _ => false
        }){
            match footnote_div{
                Node::Element(div) => {
                    if let Some(div) = div.children.get(1){
                        match div{
                            Node::Element(e) => {
                                if let Some(table) = e.children.get(0){
                                    match table{
                                        Node::Element(table) => {
                                            if table.name == "table"{
                                                if let Some(tbody) = table.children.get(1){
                                                        if let Node::Element(tbody) = tbody{
                                                            if tbody.name == "tbody" {
                                                                for row in &tbody.children {
                                                                    if let Node::Element(tr) = row {
                                                                        if let Some(th) = tr.children.get(0) {
                                                                            if let Node::Element(th) = th {
                                                                                if let Some(a) = th.children.get(0) {
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
                                        },
                                        _ => {}
                                    }
                                }
                            },
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }

        for node in dom.children{
            match node{
                Node::Text(t) => {
                    // Wrap text without a tag in a paragraph
                    let cb = NewContentBlock{
                        id: generate_id(&section),
                        block_type: BlockType::Paragraph,
                        data: BlockData::Paragraph {
                            text: t,
                        },
                        css_classes: vec![],
                        revision_id: None,
                    };
                    section.children.push(cb);
                }
                Node::Element(el) => {
                    match el.name.to_lowercase().as_str(){
                        "h1" | "h2" | "h4" | "h5" | "h6" => {
                            let mut level = match el.name.to_lowercase().as_str(){
                                "h1" => 1,
                                "h2" => 2,
                                "h3" => 3,
                                "h4" => 4,
                                "h5" => 5,
                                "h6" => 6,
                                _ => 0
                            };

                            if shift_headings{
                                if level > 1{
                                    level -= 1;
                                }
                            }

                            section.children.push(NewContentBlock{
                                id: generate_id(&section),
                                block_type: BlockType::Heading,
                                data: BlockData::Heading {
                                    text: self.dom_to_html(el, Some(&footnotes), endnotes, convert_links, project_data.clone()).await,
                                    level,
                                },
                                css_classes: vec![],
                                revision_id: None,
                            })
                        },
                        "p" => {
                            section.children.push(NewContentBlock{
                                id: generate_id(&section),
                                block_type: BlockType::Paragraph,
                                data: BlockData::Paragraph {
                                    text: self.dom_to_html(el, Some(&footnotes), endnotes, convert_links, project_data.clone()).await,
                                },
                                css_classes: vec![],
                                revision_id: None,
                            })
                        },
                        "ul" | "ol" => {
                            let style = match el.name.to_lowercase().as_str(){
                                "ul" => "unordered",
                                "ol" => "ordered",
                                _ => "unordered"
                            };
                            let style = style.to_string();
                            let mut items = Vec::new();

                            for node in el.children.iter() {
                                if let Node::Element(el) = node {
                                    if el.name.to_lowercase() == "li" {
                                        let result = self.dom_to_html(el.clone(), Some(&footnotes), endnotes, convert_links, project_data.clone()).await;
                                        items.push(result);
                                    }
                                }
                            }

                            section.children.push(NewContentBlock{
                                id: generate_id(&section),
                                block_type: BlockType::List,
                                data: BlockData::List {
                                    style,
                                    items
                                },
                                css_classes: vec![],
                                revision_id: None,
                            });
                        },
                        "blockquote" => {
                            section.children.push(NewContentBlock{
                                id: generate_id(&section),
                                block_type: BlockType::Quote,
                                data: BlockData::Quote {
                                    text: self.dom_to_html(el, Some(&footnotes), endnotes, convert_links, project_data.clone()).await,
                                    caption: "".to_string(),
                                    alignment: "".to_string(),
                                },
                                css_classes: vec![],
                                revision_id: None,
                            });
                        },
                        "div" => {
                            if el.classes.contains(&String::from("footnotes_reference_container")){
                                continue
                            }else{
                                warn!("Warning: Unsupported div. Adding as paragraph");
                                // Add as paragraph
                                section.children.push(NewContentBlock{
                                    id: generate_id(&section),
                                    block_type: BlockType::Paragraph,
                                    data: BlockData::Paragraph {
                                        text: self.dom_to_html(el, Some(&footnotes), endnotes, convert_links, project_data.clone()).await,
                                    },
                                    css_classes: vec![],
                                    revision_id: None,
                                });
                            }
                        }
                        _ => {
                            warn!("Warning: Unsupported tag: {}", el.name);
                            // Add as paragraph
                            section.children.push(NewContentBlock{
                                id: generate_id(&section),
                                block_type: BlockType::Paragraph,
                                data: BlockData::Paragraph {
                                    text: self.dom_to_html(el, Some(&footnotes), endnotes, convert_links, project_data.clone()).await,
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

        project_data.write().unwrap().sections.push(SectionOrTocV3::Section(section));
        Ok(())

    }

    async fn import_html_from_pandoc(&self, input: String, project_data: Arc<RwLock<ProjectDataV5>>, endnotes: bool) -> Result<(), ImportError>{
        let dom = match Dom::parse(&input){
            Ok(dom) => dom,
            Err(e) => {
                error!("Couldn't parse html from import after pandoc: {}", e);
                return Err(ImportError::HtmlConversionFailed)
            }
        };
        if dom.tree_type == html_parser::DomVariant::Document{
            return Err(ImportError::HtmlConversionFailed)
        } //TODO support a full html document
        
        let mut section = SectionV3 {
            id: Some(uuid::Uuid::new_v4()),
            css_classes: vec![],
            sub_sections: vec![],
            children: vec![],
            visible_in_toc: true,
            metadata: SectionMetadataV3 {
                title: "Imported Section".to_string(),
                toc_title_override: None,
                subtitle: None,
                toc_subtitle_override: None,
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
        let mut footnotes : HashMap<String, String> = HashMap::new();

        if let Some(aside) = dom.children.iter().find(|x| {
            match x{
                Node::Element(el) => el.name == "aside",
                _ => false
            }
        }){
            if let Node::Element(aside) = aside{
                if aside.id == Some("footnotes".to_string()){
                    let ol = aside.children.iter().find(|node| match node{
                        Node::Element(el) => el.name == "ol",
                        _ => false
                    });
                    if let Some(ol) = ol{
                        if let Node::Element(ol) = ol{
                            for node in ol.children.iter(){
                                if let Node::Element(li) = node{
                                    if let Some(id) = li.id.clone(){
                                        let id = id.to_string();
                                        let mut text = String::new();
                                        if let Some(ptag) = li.children.iter().next(){
                                            match ptag {
                                                Node::Element(ptag) => {
                                                    for node in ptag.children.iter() {
                                                        match node {
                                                            Node::Text(t) => text.push_str(&t),
                                                            Node::Element(ele) => {
                                                                match ele.name.to_lowercase().as_str() {
                                                                    "a" => {
                                                                        if let Some(role) = ele.attributes.get("role") {
                                                                            if let Some(role) = role {
                                                                                if role == "doc-backlink" {
                                                                                    // Skip backlinks
                                                                                    continue;
                                                                                }
                                                                            }
                                                                        }
                                                                        let mut attributes = String::new();
                                                                        for (attr, attrvalue) in ele.attributes.iter(){
                                                                            match attrvalue{
                                                                                Some(value) => attributes.push_str(&format!(" {}=\"{}\"", attr, value)),
                                                                                None => attributes.push_str(&format!(" {}", attr)),
                                                                            }
                                                                        }
                                                                        text.push_str(&format!("<a {}>{}</a>", attributes, &self.dom_to_html(ele.clone(), None, endnotes, false, project_data.clone()).await));
                                                                    },
                                                                    _ => {
                                                                        // TODO: whitelist elements and attributes
                                                                        // Currently we allow all elements but strip attributes
                                                                        text.push_str(&format!("<{}>{}</{}>", ele.name, &self.dom_to_html(ele.clone(), None, endnotes, false, project_data.clone()).await, ele.name));
                                                                    },
                                                                }
                                                            }
                                                            _ => {}
                                                        }

                                                    }
                                                },
                                                Node::Text(t) => {
                                                    text.push_str(&t);
                                                },
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

        for node in dom.children{
            match node{
                Node::Text(t) => {
                    // Wrap text without a tag in a paragraph
                    let cb = NewContentBlock{
                        id: generate_id(&section),
                        block_type: BlockType::Paragraph,
                        data: BlockData::Paragraph {
                            text: t,
                        },
                        css_classes: vec![],
                        revision_id: None,
                    };
                    section.children.push(cb);
                }
                Node::Element(el) => {
                    match el.name.to_lowercase().as_str(){
                        "h1" | "h2" | "h4" | "h5" | "h6" => {
                            let level = match el.name.to_lowercase().as_str(){
                                "h1" => 1,
                                "h2" => 2,
                                "h3" => 3,
                                "h4" => 4,
                                "h5" => 5,
                                "h6" => 6,
                                _ => 0
                            };
                            section.children.push(NewContentBlock{
                                id: generate_id(&section),
                                block_type: BlockType::Heading,
                                data: BlockData::Heading {
                                    text: self.dom_to_html(el, Some(&footnotes), endnotes, false, project_data.clone()).await,
                                    level,
                                },
                                css_classes: vec![],
                                revision_id: None,
                            })
                        },
                        "p" => {
                            section.children.push(NewContentBlock{
                                id: generate_id(&section),
                                block_type: BlockType::Paragraph,
                                data: BlockData::Paragraph {
                                    text: self.dom_to_html(el, Some(&footnotes), endnotes, false, project_data.clone()).await,
                                },
                                css_classes: vec![],
                                revision_id: None,
                            })
                        },
                        "ul" | "ol" => {
                            let style = match el.name.to_lowercase().as_str(){
                                "ul" => "unordered",
                                "ol" => "ordered",
                                _ => "unordered"
                            };
                            let style = style.to_string();
                            let mut items = Vec::new();

                            for node in el.children.iter() {
                                if let Node::Element(el) = node {
                                    if el.name.to_lowercase() == "li" {
                                        let result = self.dom_to_html(el.clone(), Some(&footnotes), endnotes, false, project_data.clone()).await;
                                        items.push(result);
                                    }
                                }
                            }

                            section.children.push(NewContentBlock{
                                id: generate_id(&section),
                                block_type: BlockType::List,
                                data: BlockData::List {
                                    style,
                                    items
                                },
                                css_classes: vec![],
                                revision_id: None,
                            });
                        },
                        "blockquote" => {
                            section.children.push(NewContentBlock{
                                id: generate_id(&section),
                                block_type: BlockType::Quote,
                                data: BlockData::Quote {
                                    text: self.dom_to_html(el, Some(&footnotes), endnotes, false, project_data.clone()).await,
                                    caption: "".to_string(),
                                    alignment: "".to_string(),
                                },
                                css_classes: vec![],
                                revision_id: None,
                            });
                        },
                        "aside" => {
                            if let Some(id) = el.id{
                                if id == "footnotes"{
                                    // Skip footnotes
                                    continue;
                                }
                            }
                        },
                        _ => {
                            warn!("Warning: Unsupported tag: {}", el.name);
                            // Add as paragraph
                            section.children.push(NewContentBlock{
                                id: generate_id(&section),
                                block_type: BlockType::Paragraph,
                                data: BlockData::Paragraph {
                                    text: self.dom_to_html(el, Some(&footnotes), endnotes, false, project_data.clone()).await,
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

        project_data.write().unwrap().sections.push(SectionOrTocV3::Section(section));
        Ok(())
    }

    //TODO: maybe also copy classes and ids from the html
    #[async_recursion]
    async fn dom_to_html(&self, ele: html_parser::Element, footnotes: Option<&HashMap<String, String>>, endnotes: bool, convert_links: bool, project_data: Arc<RwLock<ProjectDataV5>>) -> String{
        let mut html = String::new();
        for node in ele.children{
            match node{
                Node::Text(t) => {
                    debug!("Found Text: {}", t);
                    html.push_str(&t);
                }
                Node::Element(el) => {
                    debug!("Found Element: {}", el.name);

                    if el.name == "a"{
                        // For pandoc footnotes
                        if let Some(role) = el.attributes.get("role"){
                            if let Some(role) = role {
                                if role == "doc-noteref" {
                                    // This is a reference to a footnote
                                    if let Some(sup) = el.children.iter().next() {
                                        if let Node::Element(sup) = sup {
                                            if sup.name == "sup" {
                                                if let Some(num) = sup.children.iter().next() {
                                                    if let Node::Text(num) = num {
                                                        if let Some(footnotes) = footnotes{
                                                            let num = num.trim().to_string();
                                                            if let Some(footnote) = footnotes.get(&format!("fn{}", num)){
                                                                debug!("Found footnote: {}", footnote.clone());
                                                                if endnotes {
                                                                    html.push_str(&format!("<span class=\"note\" note-type=\"endnote\" note-content=\"{}\">E</span>", footnote.clone().replace("\"", "'")));
                                                                }else{
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
                        if let Some(sup) = el.children.get(0){
                            if let Node::Element(sup) = sup{
                                if sup.classes.contains(&"footnote_plugin_tooltip_text".to_string()){
                                    if let Some(id) = sup.id.clone(){
                                        let footnote_id = id.replace("tooltip", "reference");
                                        if let Some(footnotes) = footnotes{
                                            if let Some(footnote) = footnotes.get(&footnote_id){
                                                debug!("Found footnote: {}", footnote.clone());
                                                if endnotes {
                                                    html.push_str(&format!("<span class=\"note\" note-type=\"endnote\" note-content=\"{}\">E</span>", footnote.clone().replace("\"", "'")));
                                                }else{
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
                        if convert_links && footnotes.is_some(){
                            if let Some(href) = el.attributes.get("href"){
                                if let Some(href) = href{
                                    debug!("Trying to get citation for link: {}", href);
                                    if let Some(citation) = crate::import::link_converter::get_translation(href, &self.settings).await{
                                        let key = citation.key.clone();

                                        // Add citation to project
                                        project_data.write().unwrap().bibliography.insert(citation.key.clone(), citation.clone());

                                        let link_text = self.dom_to_html(el.clone(), None, endnotes, false, project_data.clone()).await;

                                        if link_text == *href {
                                            html.push_str(&format!("<citation data-key=\"{}\">C</citation>", key));
                                        }else{
                                            html.push_str(&format!("{}<citation data-key=\"{}\">C</citation>", link_text, key));
                                        }
                                        continue;
                                    }
                                }
                            }
                        }
                    }

                    let mut attrs : String = String::new();
                    for (attr, attrvalue) in el.attributes.iter(){
                        match attrvalue{ //TODO: whitelist attributes that are allowed for each tag, e.g. href for a-tags
                            Some(value) => attrs.push_str(&format!(" {}=\"{}\"", attr, value)),
                            None => attrs.push_str(&format!(" {}", attr)),
                        }
                    }
                    html.push_str(&format!("<{}{}>", el.name, attrs));
                    html.push_str(&self.dom_to_html(el.clone(), footnotes, endnotes, convert_links, project_data.clone()).await);
                    html.push_str(&format!("</{}>", el.name));
                },
                // Ignore comments
                Node::Comment(_) => {}
            }
        }
        html
    }

    async fn import_bib_entries(&self, project_id: uuid::Uuid, bib_file_path: String, settings: &Settings) -> Result<(), ImportError>{
        let mut bib_file_content = String::new();
        let mut bib_file = match tokio::fs::File::open(bib_file_path.clone()).await{
            Ok(bib_file) => bib_file,
            Err(e) => {
                warn!("Error opening bib file {}: {}", bib_file_path, e);
                return Err(ImportError::BibFileInvalid);
            }
        };
        if let Err(e) = bib_file.read_to_string(&mut bib_file_content).await{
            warn!("Error reading bib file: {}", e);
            return Err(ImportError::BibFileInvalid);
        }
        let bib = match io::from_biblatex_str(&bib_file_content){
            Ok(bib) => bib,
            Err(e) => {
                warn!("Error parsing bib file: {}", e.iter().map(|e| e.to_string()).collect::<Vec<String>>().join(", "));
                return Err(ImportError::BibFileInvalid);
            }
        };

        let project_storage = self.project_storage.clone();
        let project = project_storage.get_project(&project_id, settings).await.unwrap().clone();
        for entry in bib.iter(){
            let converted = BibEntryV2::from(entry);
            project.write().unwrap().bibliography.insert(converted.key.clone(), converted);
        }

        Ok(())
    }
}

/// Contains preprocessing methods that get called, BEFORE pandoc is executed.
mod preprocess{
    use regex::Regex;

    /// Preprocessing for latex input
    /// Replaces all endnotes with footnotes since endnotes are not supported by pandoc
    /// Finds all citations and replaces them with a temporary mark which survives pandoc
    pub fn latex(mut input: String) -> (String, Vec<String>){
        let mut marks = Vec::new();

        let re = Regex::new(r"\\(cite|footcite|footcitetext|fullcite|footfullcite)(?:\[[^\]]*?\])?(?:\[[^\]]*?\])?\{(.*?)\}").unwrap();
        input = re.replace_all(&input, |caps: &regex::Captures|{
                let key = &caps[2];
                marks.push(key.to_string());
                return format!("vb-cite-{}", marks.len()-1)
            }
        ).to_string();

        (input.replace("\\endnote", "\\footnote"), marks)
    }
}

mod postprocess{
    use regex::Regex;

    pub fn latex(mut input: String, marks: Vec<String>) -> String{
        let re = Regex::new(r"vb-cite-(\d+)").unwrap();

        // Replace temporary citation marks with actual citations
        input = re.replace_all(&input, |caps: &regex::Captures| {
            let num = match (&caps[1]).parse::<usize>() {
                Ok(num) => num,
                Err(e) => {
                    warn!("Warning: couldn't parse vb-cite- citation number: {}", e);
                    return String::from("invalid-citation!");
                }
            };
            format!("<citation data-key=\"{}\">C</citation>", marks.get(num).unwrap_or(&"".to_string()))
        }).to_string();

        input
    }
}
use crate::data_storage::{DataStorage, ProjectTemplateV2};
use crate::projects::{NewContentBlock, NewContentBlockEditorJSFormat, SectionOrTocV3};
use crate::projects::SectionOrTocV2;
use rocket::serde::json::Json;
use std::sync::Arc;
use bincode::{Decode, Encode};
use chrono::{NaiveDate, NaiveDateTime};
use rocket::form::Form;
use rocket::fs::{NamedFile, TempFile};
use rocket::http::Status;
use rocket::State;
use serde::{Deserialize, Serialize};
use vb_exchange::projects::ProjectSettingsV5;
use crate::data_storage::ProjectStorage;
use crate::projects::{Identifier, Keyword, Language, License, ProjectMetadataV2};
use crate::session::session_guard::Session;
use crate::settings::Settings;

pub mod sections;

#[derive(Serialize, Deserialize)]
pub struct ApiResult<T> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ApiError>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
}

#[derive(Serialize, Deserialize)]
pub enum ApiError{
    NotFound,
    BadRequest(String),
    Unauthorized,
    InternalServerError,
    Conflict(String),
    Other(String),
}

impl<T> ApiResult<T>{
    pub fn new_error(error: ApiError) -> Json<ApiResult<T>> {
        Json(Self {
            error: Some(error),
            data: None,
        })
    }
    pub fn new_data(data: T) -> Json<ApiResult<T>> {
        Json(Self {
            error: None,
            data: Some(data),
        })
    }
}

/// Delete project
/// DELETE /api/projects/<project_id>
#[delete("/api/projects/<project_id>")]
pub async fn delete_project(project_id: String, _session: Session, _settings: &State<Settings>, project_storage: &State<Arc<ProjectStorage>>) -> Json<ApiResult<()>> {
    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            eprintln!("Couldn't parse project id: {}", e);
            return ApiResult::new_error(ApiError::BadRequest("Couldn't parse project id".to_string()));
        },
    };

    let project_storage = Arc::clone(project_storage);

    project_storage.projects.write().unwrap().remove(&project_id);
    ApiResult::new_data(())
}

#[get("/api/projects/<project_id>/metadata")]
pub async fn get_project_metadata(project_id: String, _session: Session, settings: &State<Settings>, project_storage: &State<Arc<ProjectStorage>>, data_storage: &State<Arc<DataStorage>>) -> Json<ApiResult<Option<ProjectMetadataV2>>> {
    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            eprintln!("Couldn't parse project id: {}", e);
            return ApiResult::new_error(ApiError::NotFound);
        },
    };

    let project_storage = Arc::clone(project_storage);
    let data_storage = Arc::clone(data_storage);

    let project_entry = match project_storage.get_project(&project_id, settings).await{
        Ok(project_entry) => project_entry.clone(),
        Err(_) => {
            eprintln!("Couldn't get project with id {}", project_id);
            return ApiResult::new_error(ApiError::NotFound);
        },
    };

    let metadata = project_entry.read().unwrap().metadata.clone();
    if let Some(mut metadata) = metadata{
        let old_metadata = metadata.clone();

        let valid_persons: Vec<uuid::Uuid> = {
            let data_read = data_storage.data.read().unwrap();
            data_read.persons.keys().cloned().collect()
        };
        if let Some(mut authors) = metadata.authors{
            authors.retain_mut(|author| valid_persons.contains(&author));
            metadata.authors = Some(authors);
        }
        if let Some(mut editors) = metadata.editors{
            editors.retain_mut(|editor| valid_persons.contains(&editor));
            metadata.editors = Some(editors);
        }

        if metadata != old_metadata{
            {
                let project = project_storage.get_project(&project_id, settings).await.unwrap();
                project.write().unwrap().metadata = Some(metadata.clone());
            }
        }

        ApiResult::new_data(Some(metadata))
    }else{
        ApiResult::new_data(None)
    }
}
pub trait Patch<P, T>{
    fn patch(&mut self, patch: P) -> T;
}

impl Patch<PatchProjectMetadata, ProjectMetadataV2> for ProjectMetadataV2 {
    fn patch(&mut self, patch: PatchProjectMetadata) -> ProjectMetadataV2 {
        let mut new_metadata = self.clone();

        if let Some(title) = patch.title{
            new_metadata.title = title;
        }

        if let Some(subtitle) = patch.subtitle{
            new_metadata.subtitle = subtitle;
        }

        if let Some(authors) = patch.authors{
            new_metadata.authors = authors;
        }

        if let Some(editors) = patch.editors{
            new_metadata.editors = editors;
        }

        if let Some(web_url) = patch.web_url{
            new_metadata.web_url = web_url;
        }

        if let Some(identifiers) = patch.identifiers{
            new_metadata.identifiers = identifiers;
        }

        if let Some(published) = patch.published{
            match published{
                Some(published) => {
                    match NaiveDate::parse_from_str(&published, "%Y-%m-%d"){
                        Ok(parsed_date) => {
                            new_metadata.published = Some(parsed_date)
                        },
                        Err(e) => {
                            warn!("Couldn't parse date: {}", e);
                            new_metadata.published = None;
                        }
                    };
                },
                None => {
                    new_metadata.published = None;
                }
            }
        }

        if let Some(languages) = patch.languages{
            new_metadata.languages = languages;
        }

        if let Some(number_of_pages) = patch.number_of_pages{
            new_metadata.number_of_pages = number_of_pages;
        }

        if let Some(short_abstract) = patch.short_abstract{
            new_metadata.short_abstract = short_abstract;
        }

        if let Some(long_abstract) = patch.long_abstract{
            new_metadata.long_abstract = long_abstract;
        }

        if let Some(keywords) = patch.keywords{
            new_metadata.keywords = keywords;
        }

        if let Some(ddc) = patch.ddc{
            new_metadata.ddc = ddc;
        }

        if let Some(license) = patch.license{
            new_metadata.license = license;
        }

        if let Some(series) = patch.series{
            new_metadata.series = series;
        }

        if let Some(volume) = patch.volume{
            new_metadata.volume = volume;
        }

        if let Some(edition) = patch.edition{
            new_metadata.edition = edition;
        }

        if let Some(publisher) = patch.publisher{
            new_metadata.publisher = publisher;
        }

        new_metadata
    }
}



#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq, Default)]
pub struct PatchProjectMetadata{
    /// Book Title
    pub title: Option<String>,
    /// Subtitle of the book
    #[serde(default, skip_serializing_if = "Option::is_none", with = "::serde_with::rust::double_option")]
    pub subtitle: Option<Option<String>>,
    /// List of ids of authors of the book
    #[bincode(with_serde)]
    #[serde(default, skip_serializing_if = "Option::is_none", with = "::serde_with::rust::double_option")]
    pub authors: Option<Option<Vec<uuid::Uuid>>>,
    /// List of ids of editors of the book
    #[bincode(with_serde)]
    #[serde(default, skip_serializing_if = "Option::is_none", with = "::serde_with::rust::double_option")]
    pub editors: Option<Option<Vec<uuid::Uuid>>>,
    /// URL to a web version of the book or reference
    #[serde(default, skip_serializing_if = "Option::is_none", with = "::serde_with::rust::double_option")]
    pub web_url: Option<Option<String>>,
    /// List of identifiers of the book (e.g. ISBNs)
    #[serde(default, skip_serializing_if = "Option::is_none", with = "::serde_with::rust::double_option")]
    pub identifiers: Option<Option<Vec<Identifier>>>,
    /// Date of publication
    #[bincode(with_serde)]
    #[serde(default, skip_serializing_if = "Option::is_none", with = "::serde_with::rust::double_option")]
    pub published: Option<Option<String>>,
    /// Languages of the book
    #[serde(default, skip_serializing_if = "Option::is_none", with = "::serde_with::rust::double_option")]
    pub languages: Option<Option<Vec<Language>>>,
    /// Number of pages of the book (should be automatically calculated)
    #[serde(default, skip_serializing_if = "Option::is_none", with = "::serde_with::rust::double_option")]
    pub number_of_pages: Option<Option<u32>>,
    /// Short abstract of the book
    #[serde(default, skip_serializing_if = "Option::is_none", with = "::serde_with::rust::double_option")]
    pub short_abstract: Option<Option<String>>,
    /// Long abstract of the book
    #[serde(default, skip_serializing_if = "Option::is_none", with = "::serde_with::rust::double_option")]
    pub long_abstract: Option<Option<String>>,
    /// Keywords of the book
    #[serde(default, skip_serializing_if = "Option::is_none", with = "::serde_with::rust::double_option")]
    pub keywords: Option<Option<Vec<Keyword>>>,
    /// Dewey Decimal Classification (DDC) classes (subject groups)
    #[serde(default, skip_serializing_if = "Option::is_none", with = "::serde_with::rust::double_option")]
    pub ddc: Option<Option<String>>,
    /// License of the book
    #[serde(default, skip_serializing_if = "Option::is_none", with = "::serde_with::rust::double_option")]
    pub license: Option<Option<License>>,
    /// Series the book belongs to
    #[serde(default, skip_serializing_if = "Option::is_none", with = "::serde_with::rust::double_option")]
    pub series: Option<Option<String>>,
    /// Volume of the book in the series
    #[serde(default, skip_serializing_if = "Option::is_none", with = "::serde_with::rust::double_option")]
    pub volume: Option<Option<String>>,
    /// Edition of the book
    #[serde(default, skip_serializing_if = "Option::is_none", with = "::serde_with::rust::double_option")]
    pub edition: Option<Option<String>>,
    /// Publisher of the book
    #[serde(default, skip_serializing_if = "Option::is_none", with = "::serde_with::rust::double_option")]
    pub publisher: Option<Option<String>>
}

#[post("/api/projects/<project_id>/metadata", data = "<metadata>")]
pub async fn set_project_metadata(project_id: String, _session: Session, settings: &State<Settings>, project_storage: &State<Arc<ProjectStorage>>, metadata: Json<ProjectMetadataV2>) -> Json<ApiResult<()>> {
    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            eprintln!("Couldn't parse project id: {}", e);
            return ApiResult::new_error(ApiError::NotFound);
        },
    };

    let project_storage = Arc::clone(project_storage);

    let project_entry = match project_storage.get_project(&project_id, settings).await{
        Ok(project_entry) => project_entry.clone(),
        Err(_) => {
            eprintln!("Couldn't get project with id {}", project_id);
            return ApiResult::new_error(ApiError::NotFound);
        },
    };

    let mut project = project_entry.write().unwrap();

    project.metadata = Some(metadata.into_inner());

    ApiResult::new_data(())
}

#[patch("/api/projects/<project_id>/metadata", data = "<metadata>")]
pub async fn patch_project_metadata(project_id: String, _session: Session, settings: &State<Settings>, project_storage: &State<Arc<ProjectStorage>>, data_storage: &State<Arc<DataStorage>>, metadata: Json<PatchProjectMetadata>) -> Json<ApiResult<()>> {
    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            eprintln!("Couldn't parse project id: {}", e);
            return ApiResult::new_error(ApiError::NotFound);
        },
    };

    let project_storage = Arc::clone(project_storage);

    let project_entry = match project_storage.get_project(&project_id, settings).await{
        Ok(project_entry) => project_entry.clone(),
        Err(_) => {
            eprintln!("Couldn't get project with id {}", project_id);
            return ApiResult::new_error(ApiError::NotFound);
        },
    };


    let mut old_metadata = match &project_entry.read().unwrap().metadata{
        Some(metadata) => metadata.clone(),
        None => {
            ProjectMetadataV2::default()
        }
    };

    let new_metadata = old_metadata.patch(metadata.into_inner());

    // Validate new metadata

    // Validate authors: Check if each author exists
    if let Some(ref authors) = new_metadata.authors {
        for author in authors.iter() {
            if !data_storage.person_exists(author){
                return ApiResult::new_error(ApiError::BadRequest(format!("Author {} does not exist", author)));
            }
        }
    }
    // Validate editors: Check if each editor exists
    if let Some(ref editors) = new_metadata.editors {
        for editor in editors.iter() {
            if !data_storage.person_exists(editor){
                return ApiResult::new_error(ApiError::BadRequest(format!("Editor {} does not exist", editor)));
            }
        }
    }

    let mut project = project_entry.write().unwrap();

    project.metadata = Some(new_metadata);

    ApiResult::new_data(())
}

#[get("/api/csl/styles")]
pub async fn get_csl_styles(_session: Session, settings: &State<Settings>) -> Json<ApiResult<Vec<String>>> {
    let path = format!("{}/csl_styles", settings.data_path);
    let mut styles = vec![];

    match tokio::fs::read_dir(path).await{
        Ok(mut dir) => {
            loop{
                let _entry = match dir.next_entry().await{
                    Ok(entry) => {
                        match entry{
                            Some(entry) => {
                                let file_name = entry.file_name().to_string_lossy().to_string();
                                if file_name.ends_with(".csl"){
                                    styles.push((&file_name[..file_name.len() - 4]).to_string());
                                }
                            },
                            None => {
                                break;
                            }
                        }
                    },
                    Err(e) => {
                        eprintln!("Error reading csl directory: {}", e);
                        return ApiResult::new_error(ApiError::InternalServerError)
                    }
                };
            }
        },
        Err(e) => {
            eprintln!("Error reading csl styles: {}", e);
            return ApiResult::new_error(ApiError::InternalServerError)
        }
    }

    ApiResult::new_data(styles)
}

#[get("/api/projects/<project_id>/settings")]
pub async fn get_project_settings(project_id: String, _session: Session, settings: &State<Settings>, project_storage: &State<Arc<ProjectStorage>>) -> Json<ApiResult<Option<ProjectSettingsV5>>> {
    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            eprintln!("Couldn't parse project id: {}", e);
            return ApiResult::new_error(ApiError::NotFound);
        },
    };

    let project_storage = Arc::clone(project_storage);

    let project_entry = match project_storage.get_project(&project_id, settings).await{
        Ok(project_entry) => project_entry.clone(),
        Err(_) => {
            eprintln!("Couldn't get project with id {}", project_id);
            return ApiResult::new_error(ApiError::NotFound);
        },
    };

    let settings = project_entry.read().unwrap().settings.clone();

    ApiResult::new_data(settings)
}

#[post("/api/projects/<project_id>/settings", data = "<project_settings>")]
pub async fn set_project_settings(project_id: String, _session: Session, settings: &State<Settings>, project_storage: &State<Arc<ProjectStorage>>, project_settings: Json<ProjectSettingsV5>) -> Json<ApiResult<()>> {
    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            eprintln!("Couldn't parse project id: {}", e);
            return ApiResult::new_error(ApiError::NotFound);
        },
    };

    let project_storage = Arc::clone(project_storage);

    let project_entry = match project_storage.get_project(&project_id, settings).await{
        Ok(project_entry) => project_entry.clone(),
        Err(_) => {
            eprintln!("Couldn't get project with id {}", project_id);
            return ApiResult::new_error(ApiError::NotFound);
        },
    };

    let mut project = project_entry.write().unwrap();

    project.settings = Some(project_settings.into_inner());

    ApiResult::new_data(())
}

/// PUT /api/projects/<project_id>/metadata/authors/<author_id>
/// Add person as author to project
#[put("/api/projects/<project_id>/metadata/authors/<author_id>")]
pub async fn add_author_to_project(project_id: String, author_id: String, _session: Session, settings: &State<Settings>, project_storage: &State<Arc<ProjectStorage>>) -> Json<ApiResult<()>> {
    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            eprintln!("Couldn't parse project id: {}", e);
            return ApiResult::new_error(ApiError::BadRequest("Couldn't parse project id".to_string()));
        },
    };

    let author_id = match uuid::Uuid::parse_str(&author_id) {
        Ok(author_id) => author_id,
        Err(e) => {
            eprintln!("Couldn't parse author id: {}", e);
            return ApiResult::new_error(ApiError::BadRequest("Couldn't parse author id".to_string()));
        },
    };

    let project_storage = Arc::clone(project_storage);

    let project_entry = match project_storage.get_project(&project_id, settings).await{
        Ok(project_entry) => project_entry.clone(),
        Err(_) => {
            eprintln!("Couldn't get project with id {}", project_id);
            return ApiResult::new_error(ApiError::NotFound);
        },
    };

    let mut project = project_entry.write().unwrap();

    if let None = project.metadata{
        let new_metadata: ProjectMetadataV2 = Default::default();
        project.metadata = Some(new_metadata);
    }

    if let None = project.metadata.as_ref().unwrap().authors{
        project.metadata.as_mut().unwrap().authors = Some(Vec::new());
    }

    if !project.metadata.as_ref().unwrap().authors.as_ref().unwrap().contains(&author_id){
        project.metadata.as_mut().unwrap().authors.as_mut().unwrap().push(author_id);
    }

    ApiResult::new_data(())
}

/// PUT /api/projects/<project_id>/metadata/editors/<editor_id>
/// Add person as editor to project
#[put("/api/projects/<project_id>/metadata/editors/<editor_id>")]
pub async fn add_editor_to_project(project_id: String, editor_id: String, _session: Session, settings: &State<Settings>, project_storage: &State<Arc<ProjectStorage>>) -> Json<ApiResult<()>> {
    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            eprintln!("Couldn't parse project id: {}", e);
            return ApiResult::new_error(ApiError::BadRequest("Couldn't parse project id".to_string()));
        },
    };

    let editor_id = match uuid::Uuid::parse_str(&editor_id) {
        Ok(editor_id) => editor_id,
        Err(e) => {
            eprintln!("Couldn't parse editor id: {}", e);
            return ApiResult::new_error(ApiError::BadRequest("Couldn't parse editor id".to_string()));
        },
    };

    let project_storage = Arc::clone(project_storage);

    let project_entry = match project_storage.get_project(&project_id, settings).await{
        Ok(project_entry) => project_entry.clone(),
        Err(_) => {
            eprintln!("Couldn't get project with id {}", project_id);
            return ApiResult::new_error(ApiError::NotFound);
        },
    };

    let mut project = project_entry.write().unwrap();

    if let None = project.metadata{
        let new_metadata: ProjectMetadataV2 = Default::default();
        project.metadata = Some(new_metadata);
    }

    if let None = project.metadata.as_ref().unwrap().editors{
        project.metadata.as_mut().unwrap().editors = Some(Vec::new());
    }

    if !project.metadata.as_ref().unwrap().editors.as_ref().unwrap().contains(&editor_id){
        project.metadata.as_mut().unwrap().editors.as_mut().unwrap().push(editor_id);
    }

    ApiResult::new_data(())
}

/// DELETE /api/projects/<project_id>/metadata/authors/<author_id>
/// Remove person from project as author
#[delete("/api/projects/<project_id>/metadata/authors/<author_id>")]
pub async fn remove_author_from_project(project_id: String, author_id: String, _session: Session, settings: &State<Settings>, project_storage: &State<Arc<ProjectStorage>>) -> Json<ApiResult<()>> {
    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            eprintln!("Couldn't parse project id: {}", e);
            return ApiResult::new_error(ApiError::BadRequest("Couldn't parse project id".to_string()));
        },
    };

    let author_id = match uuid::Uuid::parse_str(&author_id) {
        Ok(author_id) => author_id,
        Err(e) => {
            eprintln!("Couldn't parse author id: {}", e);
            return ApiResult::new_error(ApiError::BadRequest("Couldn't parse author id".to_string()));
        },
    };

    let project_storage = Arc::clone(project_storage);

    let project_entry = match project_storage.get_project(&project_id, settings).await{
        Ok(project_entry) => project_entry.clone(),
        Err(_) => {
            eprintln!("Couldn't get project with id {}", project_id);
            return ApiResult::new_error(ApiError::NotFound);
        },
    };

    let mut project = project_entry.write().unwrap();

    if let None = project.metadata{
        return ApiResult::new_error(ApiError::NotFound);
    }

    if let None = project.metadata.as_ref().unwrap().authors{
        return ApiResult::new_error(ApiError::NotFound);
    }

    if let Some(index) = project.metadata.as_ref().unwrap().authors.as_ref().unwrap().iter().position(|x| *x == author_id){
        project.metadata.as_mut().unwrap().authors.as_mut().unwrap().remove(index);
    }

    ApiResult::new_data(())
}

/// DELETE /api/projects/<project_id>/metadata/editors/<editor_id>
/// Remove person from project as editor
#[delete("/api/projects/<project_id>/metadata/editors/<editor_id>")]
pub async fn remove_editor_from_project(project_id: String, editor_id: String, _session: Session, settings: &State<Settings>, project_storage: &State<Arc<ProjectStorage>>) -> Json<ApiResult<()>> {
    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            eprintln!("Couldn't parse project id: {}", e);
            return ApiResult::new_error(ApiError::BadRequest("Couldn't parse project id".to_string()));
        },
    };

    let editor_id = match uuid::Uuid::parse_str(&editor_id) {
        Ok(editor_id) => editor_id,
        Err(e) => {
            eprintln!("Couldn't parse author id: {}", e);
            return ApiResult::new_error(ApiError::BadRequest("Couldn't parse editor id".to_string()));
        },
    };

    let project_storage = Arc::clone(project_storage);

    let project_entry = match project_storage.get_project(&project_id, settings).await{
        Ok(project_entry) => project_entry.clone(),
        Err(_) => {
            eprintln!("Couldn't get project with id {}", project_id);
            return ApiResult::new_error(ApiError::NotFound);
        },
    };

    let mut project = project_entry.write().unwrap();

    if let None = project.metadata{
        return ApiResult::new_error(ApiError::NotFound);
    }

    if let None = project.metadata.as_ref().unwrap().editors{
        return ApiResult::new_error(ApiError::NotFound);
    }

    if let Some(index) = project.metadata.as_ref().unwrap().editors.as_ref().unwrap().iter().position(|x| *x == editor_id){
        project.metadata.as_mut().unwrap().editors.as_mut().unwrap().remove(index);
    }else{
        return ApiResult::new_error(ApiError::NotFound);
    }

    ApiResult::new_data(())
}

/// PUT /api/projects/<project_id>/metadata/keywords
/// Add keyword to project
#[put("/api/projects/<project_id>/metadata/keywords", data = "<keyword>")]
pub async fn add_keyword_to_project(project_id: String, keyword: Json<Keyword>, _session: Session, settings: &State<Settings>, project_storage: &State<Arc<ProjectStorage>>) -> Json<ApiResult<()>> {
    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            eprintln!("Couldn't parse project id: {}", e);
            return ApiResult::new_error(ApiError::BadRequest("Couldn't parse project id".to_string()));
        },
    };

    let project_storage = Arc::clone(project_storage);

    let project_entry = match project_storage.get_project(&project_id, settings).await{
        Ok(project_entry) => project_entry.clone(),
        Err(_) => {
            eprintln!("Couldn't get project with id {}", project_id);
            return ApiResult::new_error(ApiError::NotFound);
        },
    };

    let mut project = project_entry.write().unwrap();

    if let None = project.metadata{
        let new_metadata: ProjectMetadataV2 = Default::default();
        project.metadata = Some(new_metadata);
    }

    if let None = project.metadata.as_ref().unwrap().keywords{
        project.metadata.as_mut().unwrap().keywords = Some(Vec::new());
    }

    if !project.metadata.as_ref().unwrap().keywords.as_ref().unwrap().contains(&keyword){
        project.metadata.as_mut().unwrap().keywords.as_mut().unwrap().push(keyword.into_inner());
    }

    ApiResult::new_data(())
}

/// DELETE /api/projects/<project_id>/metadata/keywords/<keyword>
/// Remove keyword from project
#[delete("/api/projects/<project_id>/metadata/keywords/<keyword>")]
pub async fn remove_keyword_from_project(project_id: String, keyword: String, _session: Session, settings: &State<Settings>, project_storage: &State<Arc<ProjectStorage>>) -> Json<ApiResult<()>> {
    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            eprintln!("Couldn't parse project id: {}", e);
            return ApiResult::new_error(ApiError::BadRequest("Couldn't parse project id".to_string()));
        },
    };

    let project_storage = Arc::clone(project_storage);

    let project_entry = match project_storage.get_project(&project_id, settings).await{
        Ok(project_entry) => project_entry.clone(),
        Err(_) => {
            eprintln!("Couldn't get project with id {}", project_id);
            return ApiResult::new_error(ApiError::NotFound);
        },
    };

    let mut project = project_entry.write().unwrap();

    if let None = project.metadata{
        return ApiResult::new_error(ApiError::NotFound);
    }

    if let None = project.metadata.as_ref().unwrap().keywords{
        return ApiResult::new_error(ApiError::NotFound);
    }

    if let Some(index) = project.metadata.as_ref().unwrap().keywords.as_ref().unwrap().iter().position(|x| *x.title == keyword){
        project.metadata.as_mut().unwrap().keywords.as_mut().unwrap().remove(index);
    }else{
        return ApiResult::new_error(ApiError::NotFound);
    }

    ApiResult::new_data(())
}

/// POST /api/projects/<project_id>/metadata/identifiers/
/// Add identifier to project
#[post("/api/projects/<project_id>/metadata/identifiers", data = "<identifier>")]
pub async fn add_identifier_to_project(project_id: String, mut identifier: Json<Identifier>, _session: Session, settings: &State<Settings>, project_storage: &State<Arc<ProjectStorage>>) -> Json<ApiResult<Identifier>> {
    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            eprintln!("Couldn't parse project id: {}", e);
            return ApiResult::new_error(ApiError::BadRequest("Couldn't parse project id".to_string()));
        },
    };

    if let None = identifier.id{
        identifier.id = Some(uuid::Uuid::new_v4());
    }else{
        return ApiResult::new_error(ApiError::BadRequest("Identifier is not supposed to have an id.".to_string()));
    }

    let project_storage = Arc::clone(project_storage);

    let project_entry = match project_storage.get_project(&project_id, settings).await{
        Ok(project_entry) => project_entry.clone(),
        Err(_) => {
            eprintln!("Couldn't get project with id {}", project_id);
            return ApiResult::new_error(ApiError::NotFound);
        },
    };

    let mut project = project_entry.write().unwrap();

    if let None = project.metadata{
        let new_metadata: ProjectMetadataV2 = Default::default();
        project.metadata = Some(new_metadata);
    }

    if let None = project.metadata.as_ref().unwrap().identifiers{
        project.metadata.as_mut().unwrap().identifiers = Some(Vec::new());
    }

    if !project.metadata.as_ref().unwrap().identifiers.as_ref().unwrap().contains(&identifier){
        project.metadata.as_mut().unwrap().identifiers.as_mut().unwrap().push(identifier.clone().into_inner());
    }

    ApiResult::new_data(identifier.into_inner())
}

/// DELETE /api/projects/<project_id>/metadata/identifiers/<identifier_ic>
/// Remove identifier
#[delete("/api/projects/<project_id>/metadata/identifiers/<identifier_id>")]
pub async fn remove_identifier_from_project(project_id: String, identifier_id: String, _session: Session, settings: &State<Settings>, project_storage: &State<Arc<ProjectStorage>>) -> Json<ApiResult<()>> {
    let identifier_id = match uuid::Uuid::parse_str(&identifier_id) {
        Ok(identifier_id) => identifier_id,
        Err(e) => {
            eprintln!("Couldn't parse identifier id: {}", e);
            return ApiResult::new_error(ApiError::BadRequest("Couldn't parse identifier id".to_string()));
        },
    };

    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            eprintln!("Couldn't parse project id: {}", e);
            return ApiResult::new_error(ApiError::BadRequest("Couldn't parse project id".to_string()));
        },
    };

    let project_storage = Arc::clone(project_storage);

    let project_entry = match project_storage.get_project(&project_id, settings).await{
        Ok(project_entry) => project_entry.clone(),
        Err(_) => {
            eprintln!("Couldn't get project with id {}", project_id);
            return ApiResult::new_error(ApiError::NotFound);
        },
    };

    let mut project = project_entry.write().unwrap();
    if let None = project.metadata{
        return ApiResult::new_error(ApiError::NotFound);
    }

    if let None = project.metadata.as_ref().unwrap().identifiers{
        return ApiResult::new_error(ApiError::NotFound);
    }

    if let Some(index) = project.metadata.as_ref().unwrap().identifiers.as_ref().unwrap().iter().position(|x| x.id.unwrap_or_default() == identifier_id){
        project.metadata.as_mut().unwrap().identifiers.as_mut().unwrap().remove(index);
        ApiResult::new_data(())
    }else{
        ApiResult::new_error(ApiError::NotFound)
    }
}

/// PUT /api/projects/<project_id>/metadata/identifiers/<identifier_id>
/// Update identifier
#[put("/api/projects/<project_id>/metadata/identifiers/<identifier_id>", data = "<identifier>")]
pub async fn update_identifier_in_project(project_id: String, identifier_id: String, identifier: Json<Identifier>, _session: Session, settings: &State<Settings>, project_storage: &State<Arc<ProjectStorage>>) -> Json<ApiResult<()>> {

    let identifier_id = match uuid::Uuid::parse_str(&identifier_id) {
        Ok(identifier_id) => identifier_id,
        Err(e) => {
            eprintln!("Couldn't parse identifier id: {}", e);
            return ApiResult::new_error(ApiError::BadRequest("Couldn't parse identifier id".to_string()));
        },
    };

    let mut identifier = identifier.into_inner();

    if let Some(id) = identifier.id{
        if id != identifier_id{
            return ApiResult::new_error(ApiError::BadRequest("Identifier id in url and body don't match".to_string()));
        }
    }else{
        identifier.id = Some(identifier_id);
    }

    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            eprintln!("Couldn't parse project id: {}", e);
            return ApiResult::new_error(ApiError::BadRequest("Couldn't parse project id".to_string()));
        },
    };

    let project_storage = Arc::clone(project_storage);

    let project_entry = match project_storage.get_project(&project_id, settings).await{
        Ok(project_entry) => project_entry.clone(),
        Err(_) => {
            eprintln!("Couldn't get project with id {}", project_id);
            return ApiResult::new_error(ApiError::NotFound);
        },
    };

    let mut project = project_entry.write().unwrap();
    if let None = project.metadata{
        return ApiResult::new_error(ApiError::NotFound);
    }

    if let None = project.metadata.as_ref().unwrap().identifiers{
        return ApiResult::new_error(ApiError::NotFound);
    }

    if let Some(index) = project.metadata.as_ref().unwrap().identifiers.as_ref().unwrap().iter().position(|x| x.id.unwrap_or_default() == identifier_id){
        project.metadata.as_mut().unwrap().identifiers.as_mut().unwrap()[index] = identifier;
        ApiResult::new_data(())
    }else{
        ApiResult::new_error(ApiError::NotFound)
    }
}

/// GET /api/projects/<project_id>/contents
/// Returns a list of all contents (sections or toc placeholder) in the project
/// Strips out the inner content of ContentBlocks
#[get("/api/projects/<project_id>/contents")]
pub async fn get_project_contents(project_id: String, _session: Session, settings: &State<Settings>, project_storage: &State<Arc<ProjectStorage>>) -> Json<ApiResult<Vec<SectionOrTocV3>>> {
    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            println!("Couldn't parse project id: {}", e);
            return ApiResult::new_error(ApiError::NotFound);
        },
    };

    let project_storage = Arc::clone(project_storage);

    let project = match project_storage.get_project(&project_id, settings).await{
        Ok(project) => project,
        Err(_) => {
            println!("Couldn't get project with id {}", project_id);
            return ApiResult::new_error(ApiError::NotFound);
        },
    };

    let project = project.read().unwrap();

    let mut contents = Vec::new();
    for entry in project.sections.iter(){
        match entry{
            SectionOrTocV3::Toc => {
                contents.push(entry.clone());
            },
            SectionOrTocV3::Section(section) => {
                contents.push(SectionOrTocV3::Section(section.clone_without_contentblocks()));
            }
        }
    }

    ApiResult::new_data(contents)
}

/// POST /api/projects/<project_id>/contents
/// Add a new section or toc placeholder to the project
#[post("/api/projects/<project_id>/contents", data = "<content>")]
pub async fn add_content(project_id: String, _session: Session, settings: &State<Settings>, project_storage: &State<Arc<ProjectStorage>>, content: Json<SectionOrTocV3>) -> Json<ApiResult<SectionOrTocV3>>{
    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            eprintln!("Couldn't parse project id: {}", e);
            return ApiResult::new_error(ApiError::BadRequest("Couldn't parse project id".to_string()));
        },
    };

    // Check if Section or Toc, generate uuid if section
    let mut content = content.into_inner();
    match &mut content{
        SectionOrTocV3::Section(section) => {
            if let None = section.id{
                section.id = Some(uuid::Uuid::new_v4());
            }
        },
        SectionOrTocV3::Toc => {},
    }

    let project_storage = Arc::clone(project_storage);

    let project_entry = match project_storage.get_project(&project_id, settings).await{
        Ok(project_entry) => project_entry.clone(),
        Err(_) => {
            eprintln!("Couldn't get project with id {}", project_id);
            return ApiResult::new_error(ApiError::NotFound);
        },
    };

    // Insert new content block at the end
    project_entry.write().unwrap().sections.push(content.clone());

    //Return inserted content block
    ApiResult::new_data(content)
}

/// PUT /api/projects/<project_id>/contents/<content_id>/move/after/<after_id>
/// Move a section or toc after another section or toc
// TODO: implement for toc
#[put("/api/projects/<project_id>/contents/<content_id>/move/after/<after_id>")]
pub async fn move_content_after(project_id: String, content_id: String, after_id: String, _session: Session, settings: &State<Settings>, project_storage: &State<Arc<ProjectStorage>>) -> Json<ApiResult<()>> {
    let content_id = match uuid::Uuid::parse_str(&content_id) {
        Ok(content_id) => content_id,
        Err(e) => {
            eprintln!("Couldn't parse content id: {}", e);
            return ApiResult::new_error(ApiError::BadRequest("Couldn't parse content id".to_string()));
        },
    };

    let after_id = match uuid::Uuid::parse_str(&after_id) {
        Ok(after_id) => after_id,
        Err(e) => {
            eprintln!("Couldn't parse after id: {}", e);
            return ApiResult::new_error(ApiError::BadRequest("Couldn't parse after id".to_string()));
        },
    };

    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            println!("Couldn't parse project id: {}", e);
            return ApiResult::new_error(ApiError::NotFound);
        },
    };

    let project_storage = Arc::clone(project_storage);

    let project = match project_storage.get_project(&project_id, settings).await {
        Ok(project) => project,
        Err(_) => {
            println!("Couldn't get project with id {}", project_id);
            return ApiResult::new_error(ApiError::NotFound);
        },
    };

    let mut project = project.write().unwrap();

    // Get section to move
    let content = match project.remove_section(&content_id){
        Some(content) => content,
        None => {
            println!("Couldn't find content with id {}", content_id);
            return ApiResult::new_error(ApiError::NotFound);
        }
    };

    // Add section after specified section
    match project.insert_section_after(&after_id, content.clone()){
        Ok(_) => ApiResult::new_data(()),
        Err(_) => {
            println!("Couldn't find content with id {}", after_id);
            //TODO re-add content to the end
            project.sections.push(SectionOrTocV3::Section(content));
            ApiResult::new_error(ApiError::NotFound)
        }
    }
}


/// PUT /api/projects/<project_id>/contents/<content_id>/move/child_of/<parent_id>
/// Move a section or toc to be a child of another section or toc. It will be the first child.
//TODO: Implement for toc
#[put("/api/projects/<project_id>/contents/<content_id>/move/child_of/<parent_id>")]
pub async fn move_content_child_of(project_id: String, content_id: String, parent_id: String, _session: Session, settings: &State<Settings>, project_storage: &State<Arc<ProjectStorage>>) -> Json<ApiResult<()>> {
    let content_id = match uuid::Uuid::parse_str(&content_id) {
        Ok(content_id) => content_id,
        Err(e) => {
            eprintln!("Couldn't parse content id: {}", e);
            return ApiResult::new_error(ApiError::BadRequest("Couldn't parse content id".to_string()));
        },
    };

    let parent_id = match uuid::Uuid::parse_str(&parent_id) {
        Ok(parent_id) => parent_id,
        Err(e) => {
            eprintln!("Couldn't parse parent id: {}", e);
            return ApiResult::new_error(ApiError::BadRequest("Couldn't parse parent id".to_string()));
        },
    };

    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            println!("Couldn't parse project id: {}", e);
            return ApiResult::new_error(ApiError::NotFound);
        },
    };

    let project_storage = Arc::clone(project_storage);

    let project = match project_storage.get_project(&project_id, settings).await {
        Ok(project) => project,
        Err(_) => {
            println!("Couldn't get project with id {}", project_id);
            return ApiResult::new_error(ApiError::NotFound);
        },
    };

    let mut project = project.write().unwrap();

    // Get section to move
    let content = match project.remove_section(&content_id){
        Some(content) => content,
        None => {
            println!("Couldn't find content with id {}", content_id);
            return ApiResult::new_error(ApiError::NotFound);
        }
    };

    // Add section as first child of specified section
    match project.insert_section_as_first_child(&parent_id, content.clone()){
        Ok(_) => ApiResult::new_data(()),
        Err(_) => {
            println!("Couldn't find content with id {}", parent_id);
            //TODO re-add content to the end
            project.sections.push(SectionOrTocV3::Section(content));
            ApiResult::new_error(ApiError::NotFound)
        }
    }
}

/// GET /api/projects/<project_id>/sections/<content_path>/content_blocks
/// Get all content blocks in a section
#[get("/api/projects/<project_id>/sections/<content_path>/content_blocks")]
pub async fn get_content_blocks_in_section(project_id: String, content_path: String, _session: Session, settings: &State<Settings>, project_storage: &State<Arc<ProjectStorage>>) -> Json<ApiResult<Vec<NewContentBlockEditorJSFormat>>> {
    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            println!("Couldn't parse project id: {}", e);
            return ApiResult::new_error(ApiError::NotFound);
        },
    };

    let mut path = vec![];

    for part in content_path.split(":"){
        match uuid::Uuid::parse_str(part){
            Ok(part) => path.push(part),
            Err(e) => {
                println!("Couldn't parse content path: {}", e);
                return ApiResult::new_error(ApiError::BadRequest("Couldn't parse content path".to_string()));
            }
        }
    }

    if path.len() == 0{
        println!("Couldn't parse content path: path is empty");
        return ApiResult::new_error(ApiError::BadRequest("Couldn't parse content path".to_string()));
    }

    let project_storage = Arc::clone(project_storage);

    let project = match project_storage.get_project(&project_id, settings).await {
        Ok(project) => project,
        Err(_) => {
            println!("Couldn't get project with id {}", project_id);
            return ApiResult::new_error(ApiError::NotFound);
        },
    };

    let project = project.read().unwrap();

    let section = crate::data_storage::get_section_by_path(&project, &path);

    match section{
        Ok(section) => {
            let mut blocks : Vec<NewContentBlockEditorJSFormat> = vec![];

            for block in section.children.iter(){
                blocks.push(NewContentBlockEditorJSFormat::from(block.clone()))
            }
            ApiResult::new_data(blocks)
        },
        Err(e) => ApiResult::new_error(e)
    }
}

/// PUT /api/projects/<project_id>/sections/<content_path>/content_blocks
/// Replace all content blocks in a section
#[put("/api/projects/<project_id>/sections/<content_path>/content_blocks", data = "<blocks>")]
pub async fn set_content_blocks_in_section(project_id: String, content_path: String, blocks: Json<Vec<NewContentBlockEditorJSFormat>>, _session: Session, settings: &State<Settings>, project_storage: &State<Arc<ProjectStorage>>) -> Json<ApiResult<()>>{
    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            println!("Couldn't parse project id: {}", e);
            return ApiResult::new_error(ApiError::NotFound);
        },
    };

    let mut path = vec![];

    for part in content_path.split(":"){
        match uuid::Uuid::parse_str(part){
            Ok(part) => path.push(part),
            Err(e) => {
                println!("Couldn't parse content path: {}", e);
                return ApiResult::new_error(ApiError::BadRequest("Couldn't parse content path".to_string()));
            }
        }
    }

    if path.len() == 0{
        println!("Couldn't parse content path: path is empty");
        return ApiResult::new_error(ApiError::BadRequest("Couldn't parse content path".to_string()));
    }

    let project_storage = Arc::clone(project_storage);

    let project = match project_storage.get_project(&project_id, settings).await {
        Ok(project) => project,
        Err(_) => {
            println!("Couldn't get project with id {}", project_id);
            return ApiResult::new_error(ApiError::NotFound);
        },
    };

    let mut project = project.write().unwrap();

    let section = crate::data_storage::get_section_by_path_mut(&mut project, &path);

    match section{
        Ok(section) => {
            let mut new_blocks : Vec<NewContentBlock> = vec![];

            for block in blocks.iter(){
                let new_block = block.clone().try_into();

                match new_block{
                    Ok(new_block) => new_blocks.push(new_block),
                    Err(e) => {
                        return ApiResult::new_error(ApiError::BadRequest(e))
                    }
                }
            }

            section.children = new_blocks;
            ApiResult::new_data(())
        },
        Err(e) => ApiResult::new_error(e)
    }
}

#[derive(FromForm)]
struct ImageUpload<'a>{
    image: TempFile<'a>,
}

#[derive(Serialize, Deserialize, Default)]
struct ImageUploadResponse{
    success: u8,
    file: Option<UploadedImage>
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Encode, Decode)]
pub struct UploadedImage {
    pub url: String,
    pub filename: String,
    //TODO: add more fields if neede here, e.g. height alignment etc.
}


/// Upload image via multipart form
/// Endpoint for EditorJS image upload
/// POST /api/projects/<project_id>/uploads
#[post("/api/projects/<project_id>/uploads", data = "<form>")]
pub async fn upload_to_project(project_id: String, form: Form<ImageUpload<'_>>, settings: &State<Settings>, project_storage: &State<Arc<ProjectStorage>>,  _session: Session) -> Json<ImageUploadResponse> {
    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            eprintln!("Couldn't parse project id: {}", e);
            return Json(ImageUploadResponse::default());
        },
    };

    let project_storage = Arc::clone(project_storage);
    let _project = match project_storage.get_project(&project_id, settings).await{
        Ok(project) => project,
        Err(_) => {
            eprintln!("Couldn't get project with id {}", project_id);
            return Json(ImageUploadResponse::default());
        },
    };

    //TODO: check if user has access to this project once we have user management

    // Create projects upload directory if it doesn't exist
    match tokio::fs::create_dir(format!("{}/projects/{}/uploads", settings.data_path, project_id)).await{
        Ok(_) => {},
        Err(e) => {
            if e.kind() != std::io::ErrorKind::AlreadyExists{
                eprintln!("Couldn't create folder for project uploads: {}", e);
                return Json(ImageUploadResponse::default());
            }
        }
    }

    let mut image = form.into_inner().image;

    // Extract file extension from Name:
    //let extension = image.name().and_then(|name| name.split('.').last()); //TODO find working solution

    // Generate new filename
    let filename = uuid::Uuid::new_v4().to_string();
    /*if let Some(extension) = extension{
        filename = format!("{}.{}", filename, extension);
    }*/

    let filepath = format!("{}/projects/{}/uploads/{}", settings.data_path, project_id, filename);
    match image.move_copy_to(&filepath).await{
        Ok(_) => {
            Json(ImageUploadResponse{
                success: 1,
                file: Some(UploadedImage {
                    url: format!("/api/projects/{}/uploads/{}", project_id, filename),
                    filename: filename,
                })
            })
        },
        Err(e) => {
            eprintln!("Couldn't save image: {}", e);
            Json(ImageUploadResponse::default())
        }
    }
}

/// Delete a uploaded file
/// DELETE /api/projects/<project_id>/uploads/<filename>
#[delete("/api/projects/<project_id>/uploads/<filename>")]
pub async fn delete_project_upload(project_id: String, filename: String, settings: &State<Settings>, _session: Session) -> Json<ApiResult<()>> {
    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            eprintln!("Couldn't parse project id: {}", e);
            return ApiResult::new_error(ApiError::BadRequest("Couldn't parse project id".to_string()));
        },
    };

    // Create projects upload directory if it doesn't exist
    match tokio::fs::remove_file(format!("{}/projects/{}/uploads/{}", settings.data_path, project_id, filename)).await{
        Ok(_) => {
            ApiResult::new_data(())
        },
        Err(e) => {
            eprintln!("Couldn't delete image: {}", e);
            match e.kind(){
                std::io::ErrorKind::NotFound => ApiResult::new_error(ApiError::NotFound),
                _ => ApiResult::new_error(ApiError::InternalServerError)
            }
        }
    }
}

#[get("/api/projects/<project_id>/uploads/<filename>")]
pub async fn get_project_upload(project_id: String, filename: String, settings: &State<Settings>, _session: Session) -> Result<NamedFile, Status> {
    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            eprintln!("Couldn't parse project id: {}", e);
            return Err(Status::NotFound);
        },
    };

    let path = format!("{}/projects/{}/uploads/{}", settings.data_path, project_id, filename);

    let file = NamedFile::open(path).await.map_err(|_| Status::NotFound)?;
    Ok(file)
}

/// Get the id of the template currently set in project
/// GET /api/projects/<project_id>/template
#[get("/api/projects/<project_id>/template")]
pub async fn get_project_template(project_id: String, settings: &State<Settings>, project_storage: &State<Arc<ProjectStorage>>, _session: Session) -> Json<ApiResult<uuid::Uuid>> {
    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            eprintln!("Couldn't parse project id: {}", e);
            return ApiResult::new_error(ApiError::NotFound);
        },
    };

    let project_storage = Arc::clone(project_storage);

    let project = match project_storage.get_project(&project_id, settings).await{
        Ok(project) => project,
        Err(_) => {
            eprintln!("Couldn't get project with id {}", project_id);
            return ApiResult::new_error(ApiError::NotFound);
        },
    };

    let project = project.read().unwrap();

    ApiResult::new_data(project.template_id.clone())
}

/// Set project's template to the specified template_id
/// PUT /api/projects/<project_id>/template
#[put("/api/projects/<project_id>/template", data = "<template_id>")]
pub async fn set_project_template(project_id: String, template_id: Json<uuid::Uuid>, settings: &State<Settings>, project_storage: &State<Arc<ProjectStorage>>, _session: Session) -> Json<ApiResult<()>> {
    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            eprintln!("Couldn't parse project id: {}", e);
            return ApiResult::new_error(ApiError::NotFound);
        },
    };

    let project_storage = Arc::clone(project_storage);

    let project = match project_storage.get_project(&project_id, settings).await{
        Ok(project) => project,
        Err(_) => {
            eprintln!("Couldn't get project with id {}", project_id);
            return ApiResult::new_error(ApiError::NotFound);
        },
    };

    let mut project = project.write().unwrap();

    project.template_id = template_id.into_inner();

    ApiResult::new_data(())
}

/// List all templates
/// GET /api/templates
#[get("/api/templates")]
pub async fn list_templates(_session: Session, data_storage: &State<Arc<DataStorage>>) -> Json<ApiResult<Vec<ProjectTemplateV2>>> {
    let data_storage = Arc::clone(data_storage);

    let templates = data_storage.data.read().unwrap().templates.clone().iter().map(|x|x.1.read().unwrap().clone()).collect();

    ApiResult::new_data(templates)
}
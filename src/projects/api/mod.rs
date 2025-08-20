use crate::data_storage::DataStorage;
use crate::projects::{Identifier, Keyword, License, ProjectMetadata};
use crate::projects::{
    NewContentBlock, NewContentBlockEditorJSFormat, PersonUuidOrString, SectionOrTocV5,
};
use crate::session::session_guard::Session;
use crate::settings::Settings;
use crate::storage::project_storage::current::{get_section_by_path, get_section_by_path_mut};
use crate::storage::project_storage::{ProjectStorage, ProjectStorageError};
use crate::storage::ProjectTemplateV2;
use crate::utils::api_helpers::{ApiErrorType, APIResult};
use bincode::{Decode, Encode};
use chrono::NaiveDate;
use language::Language;
use rocket::form::Form;
use rocket::fs::{NamedFile, TempFile};
use rocket::http::Status;
use rocket::serde::json::Json;
use rocket::State;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use vb_exchange::projects::ProjectSettingsV5;

pub mod sections;
pub mod get;
pub mod patch;

/// DEPRECATED!
/// General return type of API Routes
/// One of the fields error or data must be Some
///
/// Return data: Some(()) if api call succeeded and you don't want to return anything
#[deprecated(note = "Please use `crate::utils::api_helpers::APIResult` instead.")]
#[derive(Serialize, Deserialize)]
pub struct DeprecatedApiResult<T> {
    /// Error occured
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<DeprecatedApiError>,
    /// Return response data or Some(()) if succeeded but no return data
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
}

/// DEPRECATED! Errors that may occur when calling api routes
#[deprecated]
#[derive(Serialize, Deserialize)]
pub enum DeprecatedApiError {
    /// The requested resource doesn't exist
    NotFound,
    /// The request couldn't be fulfilled due to user error, see string
    BadRequest(String),
    /// You didn't send a valid session cookie
    Unauthorized,
    /// Something seriously went wrong, admin will find infos in logs
    InternalServerError,
    /// e.g. Folder/File with this name already exists
    Conflict(String),
    /// Other error, specified with a string
    Other(String),
}

impl<T> DeprecatedApiResult<T> {
    /// Creates a JSON response with an ['ApiResult'] where the error field is set to the error provided
    pub fn new_error(error: DeprecatedApiError) -> Json<DeprecatedApiResult<T>> {
        Json(Self {
            error: Some(error),
            data: None,
        })
    }
    /// Creates a JSON response with an ['ApiResult'] where the data field is set to the data provided
    pub fn new_data(data: T) -> Json<DeprecatedApiResult<T>> {
        Json(Self {
            error: None,
            data: Some(data),
        })
    }
}

/// Delete project
/// DELETE /api/projects/<project_id>
#[delete("/api/projects/<project_id>")]
pub async fn delete_project(
    project_id: String,
    _session: Session,
    settings: &State<Settings>,
    project_storage: &State<Arc<ProjectStorage>>,
) -> Json<DeprecatedApiResult<()>> {
    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            eprintln!("Couldn't parse project id: {}", e);
            return DeprecatedApiResult::new_error(DeprecatedApiError::BadRequest(
                "Couldn't parse project id".to_string(),
            ));
        }
    };

    let project_storage = Arc::clone(project_storage);

    match project_storage.delete_project(&project_id, settings).await {
        Ok(_) => DeprecatedApiResult::new_data(()),
        Err(e) => match e {
            ProjectStorageError::ProjectNotFound => {
                DeprecatedApiResult::new_error(DeprecatedApiError::BadRequest("Project not Found".to_string()))
            }
            _ => DeprecatedApiResult::new_error(DeprecatedApiError::InternalServerError),
        },
    }
}

#[get("/api/projects/<project_id>/metadata")]
pub async fn get_project_metadata(
    project_id: String,
    _session: Session,
    settings: &State<Settings>,
    project_storage: &State<Arc<ProjectStorage>>,
    data_storage: &State<Arc<DataStorage>>,
) -> Json<DeprecatedApiResult<Option<ProjectMetadata>>> {
    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            eprintln!("Couldn't parse project id: {}", e);
            return DeprecatedApiResult::new_error(DeprecatedApiError::NotFound);
        }
    };

    let project_storage = Arc::clone(project_storage);
    let data_storage = Arc::clone(data_storage);

    let project_entry = match project_storage.get_project(&project_id, settings).await {
        Ok(project_entry) => project_entry.clone(),
        Err(_) => {
            eprintln!("Couldn't get project with id {}", project_id);
            return DeprecatedApiResult::new_error(DeprecatedApiError::NotFound);
        }
    };

    let metadata = project_entry.read().unwrap().metadata.clone();
    if let Some(mut metadata) = metadata {
        let old_metadata = metadata.clone();

        let valid_persons: Vec<uuid::Uuid> = {
            let data_read = data_storage.data.read().unwrap();
            data_read.persons.keys().cloned().collect()
        };
        if let Some(mut authors) = metadata.authors {
            authors.retain_mut(|author| match author {
                PersonUuidOrString::PersonUuid(uuid) => valid_persons.contains(uuid),
                PersonUuidOrString::NameString(_) => true,
            });
            metadata.authors = Some(authors);
        }
        if let Some(mut editors) = metadata.editors {
            editors.retain_mut(|editor| match editor {
                PersonUuidOrString::PersonUuid(uuid) => valid_persons.contains(uuid),
                PersonUuidOrString::NameString(_) => true,
            });
            metadata.editors = Some(editors);
        }

        if metadata != old_metadata {
            {
                let project = project_storage
                    .get_project(&project_id, settings)
                    .await
                    .unwrap();
                project.write().unwrap().metadata = Some(metadata.clone());
            }
        }

        DeprecatedApiResult::new_data(Some(metadata))
    } else {
        DeprecatedApiResult::new_data(None)
    }
}

/// Trait for HTTP PATCH routes
pub trait Patch<P, T> {
    /// Update type T with data from P
    fn patch(&mut self, patch: P) -> T;
}

impl<P, T> Patch<Option<P>, Option<T>> for Option<T>
where T: Patch<P, T> + Default + Clone{
    fn patch(&mut self, patch: Option<P>) -> Option<T> {
        match self{
            None => {
                match patch{
                    None => None,
                    Some(patch) => {
                        Some(T::default().patch(patch))
                    }
                }
            }
            Some(mself) => {
                match patch{
                    Some(patch) => {
                        Some(mself.patch(patch))
                    },
                    None => {
                        Some(mself.clone())
                    }
                }
            }
        }
    }
}

#[post("/api/projects/<project_id>/metadata", data = "<metadata>")]
pub async fn set_project_metadata(
    project_id: String,
    _session: Session,
    settings: &State<Settings>,
    project_storage: &State<Arc<ProjectStorage>>,
    metadata: Json<ProjectMetadata>,
) -> Json<DeprecatedApiResult<()>> {
    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            eprintln!("Couldn't parse project id: {}", e);
            return DeprecatedApiResult::new_error(DeprecatedApiError::NotFound);
        }
    };

    let project_storage = Arc::clone(project_storage);

    let project_entry = match project_storage.get_project(&project_id, settings).await {
        Ok(project_entry) => project_entry.clone(),
        Err(_) => {
            eprintln!("Couldn't get project with id {}", project_id);
            return DeprecatedApiResult::new_error(DeprecatedApiError::NotFound);
        }
    };

    let mut project = project_entry.write().unwrap();

    project.metadata = Some(metadata.into_inner());

    DeprecatedApiResult::new_data(())
}


/// GET /api/csl/styles
/// Returns a list of all csl styles available
///
/// Returns:
/// ApiResult with a list of strings containing the csl style filenames
#[get("/api/csl/styles")]
pub async fn get_csl_styles(
    _session: Session,
    settings: &State<Settings>,
) -> Json<DeprecatedApiResult<Vec<String>>> {
    let path = format!("{}/csl_styles", settings.data_path);
    let mut styles = vec![];

    match tokio::fs::read_dir(path).await {
        Ok(mut dir) => loop {
            let _entry = match dir.next_entry().await {
                Ok(entry) => match entry {
                    Some(entry) => {
                        let file_name = entry.file_name().to_string_lossy().to_string();
                        if file_name.ends_with(".csl") {
                            styles.push((&file_name[..file_name.len() - 4]).to_string());
                        }
                    }
                    None => {
                        break;
                    }
                },
                Err(e) => {
                    eprintln!("Error reading csl directory: {}", e);
                    return DeprecatedApiResult::new_error(DeprecatedApiError::InternalServerError);
                }
            };
        },
        Err(e) => {
            eprintln!("Error reading csl styles: {}", e);
            return DeprecatedApiResult::new_error(DeprecatedApiError::InternalServerError);
        }
    }

    DeprecatedApiResult::new_data(styles)
}

#[get("/api/projects/<project_id>/settings")]
pub async fn get_project_settings(
    project_id: String,
    _session: Session,
    settings: &State<Settings>,
    project_storage: &State<Arc<ProjectStorage>>,
) -> Json<DeprecatedApiResult<Option<ProjectSettingsV5>>> {
    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            eprintln!("Couldn't parse project id: {}", e);
            return DeprecatedApiResult::new_error(DeprecatedApiError::NotFound);
        }
    };

    let project_storage = Arc::clone(project_storage);

    let project_entry = match project_storage.get_project(&project_id, settings).await {
        Ok(project_entry) => project_entry.clone(),
        Err(_) => {
            eprintln!("Couldn't get project with id {}", project_id);
            return DeprecatedApiResult::new_error(DeprecatedApiError::NotFound);
        }
    };

    let settings = project_entry.read().unwrap().settings.clone();

    DeprecatedApiResult::new_data(settings)
}

#[post("/api/projects/<project_id>/settings", data = "<project_settings>")]
pub async fn set_project_settings(
    project_id: String,
    _session: Session,
    settings: &State<Settings>,
    project_storage: &State<Arc<ProjectStorage>>,
    project_settings: Json<ProjectSettingsV5>,
) -> Json<DeprecatedApiResult<()>> {
    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            eprintln!("Couldn't parse project id: {}", e);
            return DeprecatedApiResult::new_error(DeprecatedApiError::NotFound);
        }
    };

    let project_storage = Arc::clone(project_storage);

    let project_entry = match project_storage.get_project(&project_id, settings).await {
        Ok(project_entry) => project_entry.clone(),
        Err(_) => {
            eprintln!("Couldn't get project with id {}", project_id);
            return DeprecatedApiResult::new_error(DeprecatedApiError::NotFound);
        }
    };

    let mut project = project_entry.write().unwrap();

    project.settings = Some(project_settings.into_inner());

    DeprecatedApiResult::new_data(())
}

/// PUT /api/projects/<project_id>/metadata/authors/<author_id>
/// Add person as author to project
#[put("/api/projects/<project_id>/metadata/authors/<author_id>")]
pub async fn add_author_to_project(
    project_id: String,
    author_id: String,
    _session: Session,
    settings: &State<Settings>,
    project_storage: &State<Arc<ProjectStorage>>,
) -> Json<DeprecatedApiResult<()>> {
    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            eprintln!("Couldn't parse project id: {}", e);
            return DeprecatedApiResult::new_error(DeprecatedApiError::BadRequest(
                "Couldn't parse project id".to_string(),
            ));
        }
    };

    let author_id = match uuid::Uuid::parse_str(&author_id) {
        Ok(author_id) => author_id,
        Err(e) => {
            eprintln!("Couldn't parse author id: {}", e);
            return DeprecatedApiResult::new_error(DeprecatedApiError::BadRequest(
                "Couldn't parse author id".to_string(),
            ));
        }
    };

    let project_storage = Arc::clone(project_storage);

    let project_entry = match project_storage.get_project(&project_id, settings).await {
        Ok(project_entry) => project_entry.clone(),
        Err(_) => {
            eprintln!("Couldn't get project with id {}", project_id);
            return DeprecatedApiResult::new_error(DeprecatedApiError::NotFound);
        }
    };

    let mut project = project_entry.write().unwrap();

    if let None = project.metadata {
        let new_metadata: ProjectMetadata = Default::default();
        project.metadata = Some(new_metadata);
    }

    if let None = project.metadata.as_ref().unwrap().authors {
        project.metadata.as_mut().unwrap().authors = Some(Vec::new());
    }

    if !project
        .metadata
        .as_ref()
        .unwrap()
        .authors
        .clone()
        .unwrap()
        .contains(&PersonUuidOrString::PersonUuid(author_id.clone()))
    {
        project
            .metadata
            .as_mut()
            .unwrap()
            .authors
            .as_mut()
            .unwrap()
            .push(PersonUuidOrString::PersonUuid(author_id));
    }

    DeprecatedApiResult::new_data(())
}

/// PUT /api/projects/<project_id>/metadata/editors/<editor_id>
/// Add person as editor to project
#[put("/api/projects/<project_id>/metadata/editors/<editor_id>")]
pub async fn add_editor_to_project(
    project_id: String,
    editor_id: String,
    _session: Session,
    settings: &State<Settings>,
    project_storage: &State<Arc<ProjectStorage>>,
) -> Json<DeprecatedApiResult<()>> {
    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            eprintln!("Couldn't parse project id: {}", e);
            return DeprecatedApiResult::new_error(DeprecatedApiError::BadRequest(
                "Couldn't parse project id".to_string(),
            ));
        }
    };

    let editor_id = match uuid::Uuid::parse_str(&editor_id) {
        Ok(editor_id) => editor_id,
        Err(e) => {
            eprintln!("Couldn't parse editor id: {}", e);
            return DeprecatedApiResult::new_error(DeprecatedApiError::BadRequest(
                "Couldn't parse editor id".to_string(),
            ));
        }
    };

    let project_storage = Arc::clone(project_storage);

    let project_entry = match project_storage.get_project(&project_id, settings).await {
        Ok(project_entry) => project_entry.clone(),
        Err(_) => {
            eprintln!("Couldn't get project with id {}", project_id);
            return DeprecatedApiResult::new_error(DeprecatedApiError::NotFound);
        }
    };

    let mut project = project_entry.write().unwrap();

    if let None = project.metadata {
        let new_metadata: ProjectMetadata = Default::default();
        project.metadata = Some(new_metadata);
    }

    if let None = project.metadata.as_ref().unwrap().editors {
        project.metadata.as_mut().unwrap().editors = Some(Vec::new());
    }

    if !project
        .metadata
        .as_ref()
        .unwrap()
        .editors
        .as_ref()
        .unwrap()
        .contains(&PersonUuidOrString::PersonUuid(editor_id))
    {
        project
            .metadata
            .as_mut()
            .unwrap()
            .editors
            .as_mut()
            .unwrap()
            .push(PersonUuidOrString::PersonUuid(editor_id));
    }

    DeprecatedApiResult::new_data(())
}

/// DELETE /api/projects/<project_id>/metadata/authors/<author_id>
/// Remove person from project as author
#[delete("/api/projects/<project_id>/metadata/authors/<author_id>")]
pub async fn remove_author_from_project(
    project_id: String,
    author_id: String,
    _session: Session,
    settings: &State<Settings>,
    project_storage: &State<Arc<ProjectStorage>>,
) -> Json<DeprecatedApiResult<()>> {
    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            eprintln!("Couldn't parse project id: {}", e);
            return DeprecatedApiResult::new_error(DeprecatedApiError::BadRequest(
                "Couldn't parse project id".to_string(),
            ));
        }
    };

    let author_id = match uuid::Uuid::parse_str(&author_id) {
        Ok(author_id) => author_id,
        Err(e) => {
            eprintln!("Couldn't parse author id: {}", e);
            return DeprecatedApiResult::new_error(DeprecatedApiError::BadRequest(
                "Couldn't parse author id".to_string(),
            ));
        }
    };

    let project_storage = Arc::clone(project_storage);

    let project_entry = match project_storage.get_project(&project_id, settings).await {
        Ok(project_entry) => project_entry.clone(),
        Err(_) => {
            eprintln!("Couldn't get project with id {}", project_id);
            return DeprecatedApiResult::new_error(DeprecatedApiError::NotFound);
        }
    };

    let mut project = project_entry.write().unwrap();

    if let None = project.metadata {
        return DeprecatedApiResult::new_error(DeprecatedApiError::NotFound);
    }

    if let None = project.metadata.as_ref().unwrap().authors {
        return DeprecatedApiResult::new_error(DeprecatedApiError::NotFound);
    }

    if let Some(index) = project
        .metadata
        .as_ref()
        .unwrap()
        .authors
        .as_ref()
        .unwrap()
        .iter()
        .position(|x| *x == PersonUuidOrString::PersonUuid(author_id))
    {
        project
            .metadata
            .as_mut()
            .unwrap()
            .authors
            .as_mut()
            .unwrap()
            .remove(index);
    }

    DeprecatedApiResult::new_data(())
}

/// DELETE /api/projects/<project_id>/metadata/editors/<editor_id>
/// Remove person from project as editor
#[delete("/api/projects/<project_id>/metadata/editors/<editor_id>")]
pub async fn remove_editor_from_project(
    project_id: String,
    editor_id: String,
    _session: Session,
    settings: &State<Settings>,
    project_storage: &State<Arc<ProjectStorage>>,
) -> Json<DeprecatedApiResult<()>> {
    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            eprintln!("Couldn't parse project id: {}", e);
            return DeprecatedApiResult::new_error(DeprecatedApiError::BadRequest(
                "Couldn't parse project id".to_string(),
            ));
        }
    };

    let editor_id = match uuid::Uuid::parse_str(&editor_id) {
        Ok(editor_id) => editor_id,
        Err(e) => {
            eprintln!("Couldn't parse author id: {}", e);
            return DeprecatedApiResult::new_error(DeprecatedApiError::BadRequest(
                "Couldn't parse editor id".to_string(),
            ));
        }
    };

    let project_storage = Arc::clone(project_storage);

    let project_entry = match project_storage.get_project(&project_id, settings).await {
        Ok(project_entry) => project_entry.clone(),
        Err(_) => {
            eprintln!("Couldn't get project with id {}", project_id);
            return DeprecatedApiResult::new_error(DeprecatedApiError::NotFound);
        }
    };

    let mut project = project_entry.write().unwrap();

    if let None = project.metadata {
        return DeprecatedApiResult::new_error(DeprecatedApiError::NotFound);
    }

    if let None = project.metadata.as_ref().unwrap().editors {
        return DeprecatedApiResult::new_error(DeprecatedApiError::NotFound);
    }

    if let Some(index) = project
        .metadata
        .as_ref()
        .unwrap()
        .editors
        .as_ref()
        .unwrap()
        .iter()
        .position(|x| *x == PersonUuidOrString::PersonUuid(editor_id))
    {
        project
            .metadata
            .as_mut()
            .unwrap()
            .editors
            .as_mut()
            .unwrap()
            .remove(index);
    } else {
        return DeprecatedApiResult::new_error(DeprecatedApiError::NotFound);
    }

    DeprecatedApiResult::new_data(())
}

/// PUT /api/projects/<project_id>/metadata/keywords
/// Add keyword to project
#[put("/api/projects/<project_id>/metadata/keywords", data = "<keyword>")]
pub async fn add_keyword_to_project(
    project_id: String,
    keyword: Json<Keyword>,
    _session: Session,
    settings: &State<Settings>,
    project_storage: &State<Arc<ProjectStorage>>,
) -> Json<DeprecatedApiResult<()>> {
    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            eprintln!("Couldn't parse project id: {}", e);
            return DeprecatedApiResult::new_error(DeprecatedApiError::BadRequest(
                "Couldn't parse project id".to_string(),
            ));
        }
    };

    let project_storage = Arc::clone(project_storage);

    let project_entry = match project_storage.get_project(&project_id, settings).await {
        Ok(project_entry) => project_entry.clone(),
        Err(_) => {
            eprintln!("Couldn't get project with id {}", project_id);
            return DeprecatedApiResult::new_error(DeprecatedApiError::NotFound);
        }
    };

    let mut project = project_entry.write().unwrap();

    if let None = project.metadata {
        let new_metadata: ProjectMetadata = Default::default();
        project.metadata = Some(new_metadata);
    }

    if let None = project.metadata.as_ref().unwrap().keywords {
        project.metadata.as_mut().unwrap().keywords = Some(Vec::new());
    }

    if !project
        .metadata
        .as_ref()
        .unwrap()
        .keywords
        .as_ref()
        .unwrap()
        .contains(&keyword)
    {
        project
            .metadata
            .as_mut()
            .unwrap()
            .keywords
            .as_mut()
            .unwrap()
            .push(keyword.into_inner());
    }

    DeprecatedApiResult::new_data(())
}

/// DELETE /api/projects/<project_id>/metadata/keywords/<keyword>
/// Remove keyword from project
#[delete("/api/projects/<project_id>/metadata/keywords/<keyword>")]
pub async fn remove_keyword_from_project(
    project_id: String,
    keyword: String,
    _session: Session,
    settings: &State<Settings>,
    project_storage: &State<Arc<ProjectStorage>>,
) -> Json<DeprecatedApiResult<()>> {
    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            eprintln!("Couldn't parse project id: {}", e);
            return DeprecatedApiResult::new_error(DeprecatedApiError::BadRequest(
                "Couldn't parse project id".to_string(),
            ));
        }
    };

    let project_storage = Arc::clone(project_storage);

    let project_entry = match project_storage.get_project(&project_id, settings).await {
        Ok(project_entry) => project_entry.clone(),
        Err(_) => {
            eprintln!("Couldn't get project with id {}", project_id);
            return DeprecatedApiResult::new_error(DeprecatedApiError::NotFound);
        }
    };

    let mut project = project_entry.write().unwrap();

    if let None = project.metadata {
        return DeprecatedApiResult::new_error(DeprecatedApiError::NotFound);
    }

    if let None = project.metadata.as_ref().unwrap().keywords {
        return DeprecatedApiResult::new_error(DeprecatedApiError::NotFound);
    }

    if let Some(index) = project
        .metadata
        .as_ref()
        .unwrap()
        .keywords
        .as_ref()
        .unwrap()
        .iter()
        .position(|x| *x.title == keyword)
    {
        project
            .metadata
            .as_mut()
            .unwrap()
            .keywords
            .as_mut()
            .unwrap()
            .remove(index);
    } else {
        return DeprecatedApiResult::new_error(DeprecatedApiError::NotFound);
    }

    DeprecatedApiResult::new_data(())
}

/// POST /api/projects/<project_id>/metadata/identifiers/
/// Add identifier to project
#[post(
    "/api/projects/<project_id>/metadata/identifiers",
    data = "<identifier>"
)]
pub async fn add_identifier_to_project(
    project_id: String,
    mut identifier: Json<Identifier>,
    _session: Session,
    settings: &State<Settings>,
    project_storage: &State<Arc<ProjectStorage>>,
) -> Json<DeprecatedApiResult<Identifier>> {
    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            eprintln!("Couldn't parse project id: {}", e);
            return DeprecatedApiResult::new_error(DeprecatedApiError::BadRequest(
                "Couldn't parse project id".to_string(),
            ));
        }
    };

    if let None = identifier.id {
        identifier.id = Some(uuid::Uuid::new_v4());
    } else {
        return DeprecatedApiResult::new_error(DeprecatedApiError::BadRequest(
            "Identifier is not supposed to have an id.".to_string(),
        ));
    }

    let project_storage = Arc::clone(project_storage);

    let project_entry = match project_storage.get_project(&project_id, settings).await {
        Ok(project_entry) => project_entry.clone(),
        Err(_) => {
            eprintln!("Couldn't get project with id {}", project_id);
            return DeprecatedApiResult::new_error(DeprecatedApiError::NotFound);
        }
    };

    let mut project = project_entry.write().unwrap();

    if let None = project.metadata {
        let new_metadata: ProjectMetadata = Default::default();
        project.metadata = Some(new_metadata);
    }

    if let None = project.metadata.as_ref().unwrap().identifiers {
        project.metadata.as_mut().unwrap().identifiers = Some(Vec::new());
    }

    if !project
        .metadata
        .as_ref()
        .unwrap()
        .identifiers
        .as_ref()
        .unwrap()
        .contains(&identifier)
    {
        project
            .metadata
            .as_mut()
            .unwrap()
            .identifiers
            .as_mut()
            .unwrap()
            .push(identifier.clone().into_inner());
    }

    DeprecatedApiResult::new_data(identifier.into_inner())
}

/// DELETE /api/projects/<project_id>/metadata/identifiers/<identifier_ic>
/// Remove identifier
#[delete("/api/projects/<project_id>/metadata/identifiers/<identifier_id>")]
pub async fn remove_identifier_from_project(
    project_id: String,
    identifier_id: String,
    _session: Session,
    settings: &State<Settings>,
    project_storage: &State<Arc<ProjectStorage>>,
) -> Json<DeprecatedApiResult<()>> {
    let identifier_id = match uuid::Uuid::parse_str(&identifier_id) {
        Ok(identifier_id) => identifier_id,
        Err(e) => {
            eprintln!("Couldn't parse identifier id: {}", e);
            return DeprecatedApiResult::new_error(DeprecatedApiError::BadRequest(
                "Couldn't parse identifier id".to_string(),
            ));
        }
    };

    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            eprintln!("Couldn't parse project id: {}", e);
            return DeprecatedApiResult::new_error(DeprecatedApiError::BadRequest(
                "Couldn't parse project id".to_string(),
            ));
        }
    };

    let project_storage = Arc::clone(project_storage);

    let project_entry = match project_storage.get_project(&project_id, settings).await {
        Ok(project_entry) => project_entry.clone(),
        Err(_) => {
            eprintln!("Couldn't get project with id {}", project_id);
            return DeprecatedApiResult::new_error(DeprecatedApiError::NotFound);
        }
    };

    let mut project = project_entry.write().unwrap();
    if let None = project.metadata {
        return DeprecatedApiResult::new_error(DeprecatedApiError::NotFound);
    }

    if let None = project.metadata.as_ref().unwrap().identifiers {
        return DeprecatedApiResult::new_error(DeprecatedApiError::NotFound);
    }

    if let Some(index) = project
        .metadata
        .as_ref()
        .unwrap()
        .identifiers
        .as_ref()
        .unwrap()
        .iter()
        .position(|x| x.id.unwrap_or_default() == identifier_id)
    {
        project
            .metadata
            .as_mut()
            .unwrap()
            .identifiers
            .as_mut()
            .unwrap()
            .remove(index);
        DeprecatedApiResult::new_data(())
    } else {
        DeprecatedApiResult::new_error(DeprecatedApiError::NotFound)
    }
}

/// PUT /api/projects/<project_id>/metadata/identifiers/<identifier_id>
/// Update identifier
#[put(
    "/api/projects/<project_id>/metadata/identifiers/<identifier_id>",
    data = "<identifier>"
)]
pub async fn update_identifier_in_project(
    project_id: String,
    identifier_id: String,
    identifier: Json<Identifier>,
    _session: Session,
    settings: &State<Settings>,
    project_storage: &State<Arc<ProjectStorage>>,
) -> Json<DeprecatedApiResult<()>> {
    let identifier_id = match uuid::Uuid::parse_str(&identifier_id) {
        Ok(identifier_id) => identifier_id,
        Err(e) => {
            eprintln!("Couldn't parse identifier id: {}", e);
            return DeprecatedApiResult::new_error(DeprecatedApiError::BadRequest(
                "Couldn't parse identifier id".to_string(),
            ));
        }
    };

    let mut identifier = identifier.into_inner();

    if let Some(id) = identifier.id {
        if id != identifier_id {
            return DeprecatedApiResult::new_error(DeprecatedApiError::BadRequest(
                "Identifier id in url and body don't match".to_string(),
            ));
        }
    } else {
        identifier.id = Some(identifier_id);
    }

    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            eprintln!("Couldn't parse project id: {}", e);
            return DeprecatedApiResult::new_error(DeprecatedApiError::BadRequest(
                "Couldn't parse project id".to_string(),
            ));
        }
    };

    let project_storage = Arc::clone(project_storage);

    let project_entry = match project_storage.get_project(&project_id, settings).await {
        Ok(project_entry) => project_entry.clone(),
        Err(_) => {
            eprintln!("Couldn't get project with id {}", project_id);
            return DeprecatedApiResult::new_error(DeprecatedApiError::NotFound);
        }
    };

    let mut project = project_entry.write().unwrap();
    if let None = project.metadata {
        return DeprecatedApiResult::new_error(DeprecatedApiError::NotFound);
    }

    if let None = project.metadata.as_ref().unwrap().identifiers {
        return DeprecatedApiResult::new_error(DeprecatedApiError::NotFound);
    }

    if let Some(index) = project
        .metadata
        .as_ref()
        .unwrap()
        .identifiers
        .as_ref()
        .unwrap()
        .iter()
        .position(|x| x.id.unwrap_or_default() == identifier_id)
    {
        project
            .metadata
            .as_mut()
            .unwrap()
            .identifiers
            .as_mut()
            .unwrap()[index] = identifier;
        DeprecatedApiResult::new_data(())
    } else {
        DeprecatedApiResult::new_error(DeprecatedApiError::NotFound)
    }
}

/// GET /api/projects/<project_id>/contents
/// Returns a list of all contents (sections or toc placeholder) in the project
/// Strips out the inner content of ContentBlocks
#[get("/api/projects/<project_id>/contents")]
pub async fn get_project_contents(
    project_id: String,
    _session: Session,
    settings: &State<Settings>,
    project_storage: &State<Arc<ProjectStorage>>,
) -> Json<DeprecatedApiResult<Vec<SectionOrTocV5>>> {
    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            println!("Couldn't parse project id: {}", e);
            return DeprecatedApiResult::new_error(DeprecatedApiError::NotFound);
        }
    };

    let project_storage = Arc::clone(project_storage);

    let project = match project_storage.get_project(&project_id, settings).await {
        Ok(project) => project,
        Err(_) => {
            println!("Couldn't get project with id {}", project_id);
            return DeprecatedApiResult::new_error(DeprecatedApiError::NotFound);
        }
    };

    let project = project.read().unwrap();

    let mut contents = Vec::new();
    for entry in project.sections.iter() {
        match entry {
            SectionOrTocV5::Toc => {
                contents.push(entry.clone());
            }
            SectionOrTocV5::Section(section) => {
                contents.push(SectionOrTocV5::Section(
                    section.clone_without_contentblocks(),
                ));
            }
        }
    }

    DeprecatedApiResult::new_data(contents)
}

/// POST /api/projects/<project_id>/contents
/// Add a new section or toc placeholder to the project
#[post("/api/projects/<project_id>/contents", data = "<content>")]
pub async fn add_content(
    project_id: String,
    _session: Session,
    settings: &State<Settings>,
    project_storage: &State<Arc<ProjectStorage>>,
    content: Json<SectionOrTocV5>,
) -> Json<DeprecatedApiResult<SectionOrTocV5>> {
    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            eprintln!("Couldn't parse project id: {}", e);
            return DeprecatedApiResult::new_error(DeprecatedApiError::BadRequest(
                "Couldn't parse project id".to_string(),
            ));
        }
    };

    // Check if Section or Toc, generate uuid if section
    let mut content = content.into_inner();
    match &mut content {
        SectionOrTocV5::Section(section) => {
            if let None = section.id {
                section.id = Some(uuid::Uuid::new_v4());
            }
        }
        SectionOrTocV5::Toc => {}
    }

    let project_storage = Arc::clone(project_storage);

    let project_entry = match project_storage.get_project(&project_id, settings).await {
        Ok(project_entry) => project_entry.clone(),
        Err(_) => {
            eprintln!("Couldn't get project with id {}", project_id);
            return DeprecatedApiResult::new_error(DeprecatedApiError::NotFound);
        }
    };

    // Insert new content block at the end
    project_entry
        .write()
        .unwrap()
        .sections
        .push(content.clone());

    //Return inserted content block
    DeprecatedApiResult::new_data(content)
}

/// PUT /api/projects/<project_id>/contents/<content_id>/move/after/<after_id>
/// Move a section or toc after another section or toc
// TODO: implement for toc
#[put("/api/projects/<project_id>/contents/<content_id>/move/after/<after_id>")]
pub async fn move_content_after(
    project_id: String,
    content_id: String,
    after_id: String,
    _session: Session,
    settings: &State<Settings>,
    project_storage: &State<Arc<ProjectStorage>>,
) -> Json<DeprecatedApiResult<()>> {
    let content_id = match uuid::Uuid::parse_str(&content_id) {
        Ok(content_id) => content_id,
        Err(e) => {
            eprintln!("Couldn't parse content id: {}", e);
            return DeprecatedApiResult::new_error(DeprecatedApiError::BadRequest(
                "Couldn't parse content id".to_string(),
            ));
        }
    };

    let after_id = match uuid::Uuid::parse_str(&after_id) {
        Ok(after_id) => after_id,
        Err(e) => {
            eprintln!("Couldn't parse after id: {}", e);
            return DeprecatedApiResult::new_error(DeprecatedApiError::BadRequest(
                "Couldn't parse after id".to_string(),
            ));
        }
    };

    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            println!("Couldn't parse project id: {}", e);
            return DeprecatedApiResult::new_error(DeprecatedApiError::NotFound);
        }
    };

    let project_storage = Arc::clone(project_storage);

    let project = match project_storage.get_project(&project_id, settings).await {
        Ok(project) => project,
        Err(_) => {
            println!("Couldn't get project with id {}", project_id);
            return DeprecatedApiResult::new_error(DeprecatedApiError::NotFound);
        }
    };

    let mut project = project.write().unwrap();

    // Get section to move
    let content = match project.remove_section(&content_id) {
        Some(content) => content,
        None => {
            println!("Couldn't find content with id {}", content_id);
            return DeprecatedApiResult::new_error(DeprecatedApiError::NotFound);
        }
    };

    // Add section after specified section
    match project.insert_section_after(&after_id, content.clone()) {
        Ok(_) => DeprecatedApiResult::new_data(()),
        Err(_) => {
            println!("Couldn't find content with id {}", after_id);
            //TODO re-add content to the end
            project.sections.push(SectionOrTocV5::Section(content));
            DeprecatedApiResult::new_error(DeprecatedApiError::NotFound)
        }
    }
}

/// PUT /api/projects/<project_id>/contents/<content_id>/move/child_of/<parent_id>
/// Move a section or toc to be a child of another section or toc. It will be the first child.
//TODO: Implement for toc
#[put("/api/projects/<project_id>/contents/<content_id>/move/child_of/<parent_id>")]
pub async fn move_content_child_of(
    project_id: String,
    content_id: String,
    parent_id: String,
    _session: Session,
    settings: &State<Settings>,
    project_storage: &State<Arc<ProjectStorage>>,
) -> Json<DeprecatedApiResult<()>> {
    let content_id = match uuid::Uuid::parse_str(&content_id) {
        Ok(content_id) => content_id,
        Err(e) => {
            eprintln!("Couldn't parse content id: {}", e);
            return DeprecatedApiResult::new_error(DeprecatedApiError::BadRequest(
                "Couldn't parse content id".to_string(),
            ));
        }
    };

    let parent_id = match uuid::Uuid::parse_str(&parent_id) {
        Ok(parent_id) => parent_id,
        Err(e) => {
            eprintln!("Couldn't parse parent id: {}", e);
            return DeprecatedApiResult::new_error(DeprecatedApiError::BadRequest(
                "Couldn't parse parent id".to_string(),
            ));
        }
    };

    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            println!("Couldn't parse project id: {}", e);
            return DeprecatedApiResult::new_error(DeprecatedApiError::NotFound);
        }
    };

    let project_storage = Arc::clone(project_storage);

    let project = match project_storage.get_project(&project_id, settings).await {
        Ok(project) => project,
        Err(_) => {
            println!("Couldn't get project with id {}", project_id);
            return DeprecatedApiResult::new_error(DeprecatedApiError::NotFound);
        }
    };

    let mut project = project.write().unwrap();

    // Get section to move
    let content = match project.remove_section(&content_id) {
        Some(content) => content,
        None => {
            println!("Couldn't find content with id {}", content_id);
            return DeprecatedApiResult::new_error(DeprecatedApiError::NotFound);
        }
    };

    // Add section as first child of specified section
    match project.insert_section_as_first_child(&parent_id, content.clone()) {
        Ok(_) => DeprecatedApiResult::new_data(()),
        Err(_) => {
            println!("Couldn't find content with id {}", parent_id);
            //TODO re-add content to the end
            project.sections.push(SectionOrTocV5::Section(content));
            DeprecatedApiResult::new_error(DeprecatedApiError::NotFound)
        }
    }
}

/// GET /api/projects/<project_id>/sections/<content_path>/content_blocks
/// Get all content blocks in a section
#[get("/api/projects/<project_id>/sections/<content_path>/content_blocks")]
pub async fn get_content_blocks_in_section(
    project_id: String,
    content_path: String,
    _session: Session,
    settings: &State<Settings>,
    project_storage: &State<Arc<ProjectStorage>>,
) -> APIResult<Vec<NewContentBlockEditorJSFormat>> {
    let project_id = uuid::Uuid::parse_str(&project_id)?;

    let mut path = vec![];

    for part in content_path.split(":") {
        path.push(uuid::Uuid::parse_str(part)?);
    }

    if path.len() == 0 {
        println!("Couldn't parse content path: path is empty");
        return Err(ApiErrorType::UnparsableParameter("content_path".to_string()).into());
    }

    let project_storage = Arc::clone(project_storage);

    let project = project_storage.get_project(&project_id, settings).await?;

    let project = project.read().unwrap();

    let section = get_section_by_path(&project, &path)?;

    let mut blocks: Vec<NewContentBlockEditorJSFormat> = vec![];

    for block in section.children.iter() {
        blocks.push(NewContentBlockEditorJSFormat::from(block.clone()))
    }
    Ok(blocks.into())
}

/// PUT /api/projects/<project_id>/sections/<content_path>/content_blocks
/// Replace all content blocks in a section
#[put(
    "/api/projects/<project_id>/sections/<content_path>/content_blocks",
    data = "<blocks>"
)]
pub async fn set_content_blocks_in_section(
    project_id: String,
    content_path: String,
    blocks: Json<Vec<NewContentBlockEditorJSFormat>>,
    _session: Session,
    settings: &State<Settings>,
    project_storage: &State<Arc<ProjectStorage>>,
) -> APIResult<()> {
    let project_id = uuid::Uuid::parse_str(&project_id)?;

    let mut path = vec![];

    for part in content_path.split(":") {
        path.push(uuid::Uuid::parse_str(part)?);
    }

    if path.len() == 0 {
        println!("Couldn't parse content path: path is empty");
        return Err(ApiErrorType::UnparsableParameter("content_path".to_string()).into());
    }

    let project_storage = Arc::clone(project_storage);

    let project = project_storage.get_project(&project_id, settings).await?;

    let mut project = project.write().unwrap();

    let section = get_section_by_path_mut(&mut project, &path)?;

    let mut new_blocks: Vec<NewContentBlock> = vec![];

    for block in blocks.iter() {
        let new_block: NewContentBlock = block.clone().try_into().map_err(|e| {
            ApiErrorType::UnparsableParameter(e)
        })?;
        new_blocks.push(new_block);
    }

    section.children = new_blocks;
    Ok(().into())
}

#[derive(FromForm)]
struct ImageUpload<'a> {
    image: TempFile<'a>,
}

#[derive(Serialize, Deserialize, Default)]
struct ImageUploadResponse {
    success: u8,
    file: Option<UploadedImage>,
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
pub async fn upload_to_project(
    project_id: String,
    form: Form<ImageUpload<'_>>,
    settings: &State<Settings>,
    project_storage: &State<Arc<ProjectStorage>>,
    _session: Session,
) -> Json<ImageUploadResponse> {
    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            eprintln!("Couldn't parse project id: {}", e);
            return Json(ImageUploadResponse::default());
        }
    };

    let project_storage = Arc::clone(project_storage);
    let _project = match project_storage.get_project(&project_id, settings).await {
        Ok(project) => project,
        Err(_) => {
            eprintln!("Couldn't get project with id {}", project_id);
            return Json(ImageUploadResponse::default());
        }
    };

    //TODO: check if user has access to this project once we have user management

    // Create projects upload directory if it doesn't exist
    match tokio::fs::create_dir(format!(
        "{}/projects/{}/uploads",
        settings.data_path, project_id
    ))
    .await
    {
        Ok(_) => {}
        Err(e) => {
            if e.kind() != std::io::ErrorKind::AlreadyExists {
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

    let filepath = format!(
        "{}/projects/{}/uploads/{}",
        settings.data_path, project_id, filename
    );
    match image.move_copy_to(&filepath).await {
        Ok(_) => Json(ImageUploadResponse {
            success: 1,
            file: Some(UploadedImage {
                url: format!("/api/projects/{}/uploads/{}", project_id, filename),
                filename: filename,
            }),
        }),
        Err(e) => {
            eprintln!("Couldn't save image: {}", e);
            Json(ImageUploadResponse::default())
        }
    }
}

/// Delete a uploaded file
/// DELETE /api/projects/<project_id>/uploads/<filename>
#[delete("/api/projects/<project_id>/uploads/<filename>")]
pub async fn delete_project_upload(
    project_id: String,
    filename: String,
    settings: &State<Settings>,
    _session: Session,
) -> Json<DeprecatedApiResult<()>> {
    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            eprintln!("Couldn't parse project id: {}", e);
            return DeprecatedApiResult::new_error(DeprecatedApiError::BadRequest(
                "Couldn't parse project id".to_string(),
            ));
        }
    };

    // Create projects upload directory if it doesn't exist
    match tokio::fs::remove_file(format!(
        "{}/projects/{}/uploads/{}",
        settings.data_path, project_id, filename
    ))
    .await
    {
        Ok(_) => DeprecatedApiResult::new_data(()),
        Err(e) => {
            eprintln!("Couldn't delete image: {}", e);
            match e.kind() {
                std::io::ErrorKind::NotFound => DeprecatedApiResult::new_error(DeprecatedApiError::NotFound),
                _ => DeprecatedApiResult::new_error(DeprecatedApiError::InternalServerError),
            }
        }
    }
}

#[get("/api/projects/<project_id>/uploads/<filename>")]
pub async fn get_project_upload(
    project_id: String,
    filename: String,
    settings: &State<Settings>,
    _session: Session,
) -> Result<NamedFile, Status> {
    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            eprintln!("Couldn't parse project id: {}", e);
            return Err(Status::NotFound);
        }
    };

    let path = format!(
        "{}/projects/{}/uploads/{}",
        settings.data_path, project_id, filename
    );

    let file = NamedFile::open(path).await.map_err(|_| Status::NotFound)?;
    Ok(file)
}

/// Get the id of the template currently set in project
/// GET /api/projects/<project_id>/template
#[get("/api/projects/<project_id>/template")]
pub async fn get_project_template(
    project_id: String,
    settings: &State<Settings>,
    project_storage: &State<Arc<ProjectStorage>>,
    _session: Session,
) -> Json<DeprecatedApiResult<uuid::Uuid>> {
    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            eprintln!("Couldn't parse project id: {}", e);
            return DeprecatedApiResult::new_error(DeprecatedApiError::NotFound);
        }
    };

    let project_storage = Arc::clone(project_storage);

    let project = match project_storage.get_project(&project_id, settings).await {
        Ok(project) => project,
        Err(_) => {
            eprintln!("Couldn't get project with id {}", project_id);
            return DeprecatedApiResult::new_error(DeprecatedApiError::NotFound);
        }
    };

    let project = project.read().unwrap();

    DeprecatedApiResult::new_data(project.template_id.clone())
}

/// Set project's template to the specified template_id
/// PUT /api/projects/<project_id>/template
#[put("/api/projects/<project_id>/template", data = "<template_id>")]
pub async fn set_project_template(
    project_id: String,
    template_id: Json<uuid::Uuid>,
    settings: &State<Settings>,
    project_storage: &State<Arc<ProjectStorage>>,
    _session: Session,
) -> Json<DeprecatedApiResult<()>> {
    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            eprintln!("Couldn't parse project id: {}", e);
            return DeprecatedApiResult::new_error(DeprecatedApiError::NotFound);
        }
    };

    let project_storage = Arc::clone(project_storage);

    let project = match project_storage.get_project(&project_id, settings).await {
        Ok(project) => project,
        Err(_) => {
            eprintln!("Couldn't get project with id {}", project_id);
            return DeprecatedApiResult::new_error(DeprecatedApiError::NotFound);
        }
    };

    let mut project = project.write().unwrap();

    project.template_id = template_id.into_inner();

    DeprecatedApiResult::new_data(())
}

/// List all templates
/// GET /api/templates
#[get("/api/templates")]
pub async fn list_templates(
    _session: Session,
    data_storage: &State<Arc<DataStorage>>,
) -> Json<DeprecatedApiResult<Vec<ProjectTemplateV2>>> {
    let data_storage = Arc::clone(data_storage);

    let templates = data_storage
        .data
        .read()
        .unwrap()
        .templates
        .clone()
        .iter()
        .map(|x| x.1.read().unwrap().clone())
        .collect();

    DeprecatedApiResult::new_data(templates)
}

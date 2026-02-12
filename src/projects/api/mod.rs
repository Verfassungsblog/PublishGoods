use crate::data_storage::DataStorage;
use crate::session::session_guard::Session;
use crate::settings::Settings;
use crate::storage::project_storage::current::{
    get_section_by_path, get_section_by_path_mut, PersonUuidOrString,
};
use crate::storage::project_storage::sections::content::current::{
    NewContentBlock, NewContentBlockEditorJSFormat,
};
use crate::storage::project_storage::sections::Section;
use crate::storage::project_storage::{ProjectMetadata, ProjectStorage, ProjectStorageError};
use crate::storage::ProjectTemplateV2;
use crate::utils::api_helpers::{APIResult, ApiErrorType};
use bincode::{Decode, Encode};
use rocket::form::Form;
use rocket::fs::{NamedFile, TempFile};
use rocket::http::Status;
use rocket::serde::json::Json;
use rocket::State;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use vb_exchange::projects::ProjectSettingsV5;
use vb_exchange::projects::{Identifier, Keyword};

pub mod bibliography;
pub mod get;
pub mod patch;
pub mod sections;

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
            ProjectStorageError::ProjectNotFound => DeprecatedApiResult::new_error(
                DeprecatedApiError::BadRequest("Project not Found".to_string()),
            ),
            _ => DeprecatedApiResult::new_error(DeprecatedApiError::InternalServerError),
        },
    }
}

/// Trait for HTTP PATCH routes
pub trait Patch<P, T> {
    /// Update type T with data from P
    fn patch(&mut self, patch: P) -> T;
}

impl<P, T> Patch<Option<P>, Option<T>> for Option<T>
where
    T: Patch<P, T> + Default + Clone,
{
    fn patch(&mut self, patch: Option<P>) -> Option<T> {
        match self {
            None => match patch {
                None => None,
                Some(patch) => Some(T::default().patch(patch)),
            },
            Some(mself) => match patch {
                Some(patch) => Some(mself.patch(patch)),
                None => Some(mself.clone()),
            },
        }
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
) -> Json<DeprecatedApiResult<Vec<Section>>> {
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
        contents.push(entry.clone_without_content());
    }

    DeprecatedApiResult::new_data(contents)
}

/// POST /api/projects/<project_id>/contents
/// Add a new section to the project
#[post("/api/projects/<project_id>/contents", data = "<content>")]
pub async fn add_content(
    project_id: String,
    _session: Session,
    settings: &State<Settings>,
    project_storage: &State<Arc<ProjectStorage>>,
    content: Json<Section>,
) -> Json<DeprecatedApiResult<Section>> {
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
    if let None = content.id {
        content.id = Some(uuid::Uuid::new_v4());
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
            project.sections.push(content);
            DeprecatedApiResult::new_error(DeprecatedApiError::NotFound)
        }
    }
}

/// PUT /api/projects/<project_id>/contents/<content_id>/move/child_of/<parent_id>
/// Move a section or toc to be a child of another section or toc. It will be the first child.
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
            project.sections.push(content);
            DeprecatedApiResult::new_error(DeprecatedApiError::NotFound)
        }
    }
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
) -> APIResult<()> {
    let project_id = uuid::Uuid::parse_str(&project_id)?;

    tokio::fs::remove_file(format!(
        "{}/projects/{}/uploads/{}",
        settings.data_path, project_id, filename
    ))
    .await?;

    Ok(().into())
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

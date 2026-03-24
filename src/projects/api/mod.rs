use crate::data_storage::DataStorage;
use crate::session::session_guard::Session;
use crate::settings::Settings;
use crate::storage::project_storage::sections::Section;
use crate::storage::project_storage::{ProjectStorage, ProjectStorageError};
use crate::storage::ProjectTemplateV2;
use crate::utils::api_helpers::{APIResponse, APIResult, ApiError, ApiErrorType};
use bincode::{Decode, Encode};
use rocket::form::Form;
use rocket::fs::{NamedFile, TempFile};
use rocket::http::Status;
use rocket::serde::json::Json;
use rocket::State;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

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
) -> APIResult<()> {
    let project_id = uuid::Uuid::parse_str(&project_id)?;

    let project_storage = Arc::clone(project_storage);
    project_storage
        .delete_project(&project_id, settings)
        .await?;
    Ok(().into())
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
) -> APIResult<Vec<Section>> {
    let project_id = uuid::Uuid::parse_str(&project_id)?;

    let project_storage = Arc::clone(project_storage);
    let project = project_storage.get_project(&project_id, settings).await?;

    let project = project.read().unwrap();

    let mut contents = Vec::new();
    for entry in project.sections.iter() {
        contents.push(entry.clone_without_content());
    }

    Ok(contents.into())
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
) -> APIResult<Section> {
    let project_id = uuid::Uuid::parse_str(&project_id)?;

    // Check if Section or Toc, generate uuid if section
    let mut content = content.into_inner();
    if let None = content.id {
        content.id = Some(uuid::Uuid::new_v4());
    }

    let project_storage = Arc::clone(project_storage);
    let project_entry = project_storage.get_project(&project_id, settings).await?;

    // Insert new content block at the end
    project_entry
        .write()
        .unwrap()
        .sections
        .push(content.clone());

    //Return inserted content block
    Ok(content.into())
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
) -> APIResult<()> {
    let content_id = uuid::Uuid::parse_str(&content_id)?;
    let after_id = uuid::Uuid::parse_str(&after_id)?;
    let project_id = uuid::Uuid::parse_str(&project_id)?;

    let project_storage = Arc::clone(project_storage);
    let project = project_storage.get_project(&project_id, settings).await?;

    let mut project = project.write().unwrap();

    // Get section to move
    let content = match project.remove_section(&content_id) {
        Some(content) => content,
        None => {
            println!("Couldn't find content with id {}", content_id);
            return Err(ApiErrorType::ResourceNotFound("content".to_string()).into());
        }
    };

    // Add section after specified section
    match project.insert_section_after(&after_id, content.clone()) {
        Ok(_) => Ok(().into()),
        Err(_) => {
            println!("Couldn't find content with id {}", after_id);
            project.sections.push(content);
            Err(ApiErrorType::ResourceNotFound("content".to_string()).into())
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
) -> APIResult<()> {
    let content_id = uuid::Uuid::parse_str(&content_id)?;
    let parent_id = uuid::Uuid::parse_str(&parent_id)?;
    let project_id = uuid::Uuid::parse_str(&project_id)?;

    let project_storage = Arc::clone(project_storage);
    let project = project_storage.get_project(&project_id, settings).await?;

    let mut project = project.write().unwrap();

    // Get section to move
    let content = match project.remove_section(&content_id) {
        Some(content) => content,
        None => {
            println!("Couldn't find content with id {}", content_id);
            return Err(ApiErrorType::ResourceNotFound("content".to_string()).into());
        }
    };

    // Add section as first child of specified section
    match project.insert_section_as_first_child(&parent_id, content.clone()) {
        Ok(_) => Ok(().into()),
        Err(_) => {
            println!("Couldn't find content with id {}", parent_id);
            //TODO re-add content to the end
            project.sections.push(content);
            Err(ApiErrorType::ResourceNotFound("content".to_string()).into())
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
) -> APIResult<uuid::Uuid> {
    let project_id = uuid::Uuid::parse_str(&project_id)?;

    let project_storage = Arc::clone(project_storage);
    let project = project_storage.get_project(&project_id, settings).await?;

    let project = project.read().unwrap();

    Ok(project.template_id.clone().into())
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
) -> APIResult<()> {
    let project_id = uuid::Uuid::parse_str(&project_id)?;

    let project_storage = Arc::clone(project_storage);
    let project = project_storage.get_project(&project_id, settings).await?;

    let mut project = project.write().unwrap();
    project.template_id = template_id.into_inner();

    Ok(().into())
}

/// List all templates
/// GET /api/templates
#[get("/api/templates")]
pub async fn list_templates(
    _session: Session,
    data_storage: &State<Arc<DataStorage>>,
) -> APIResult<Vec<ProjectTemplateV2>> {
    let data_storage = Arc::clone(data_storage);

    let templates: Vec<ProjectTemplateV2> = data_storage
        .data
        .templates
        .iter()
        .map(|x| x.value().read().unwrap().clone())
        .collect();

    Ok(templates.into())
}

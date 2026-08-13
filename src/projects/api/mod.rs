use crate::session::session_guard::Session;
use crate::settings::Settings;
use crate::storage::project_storage::sections::Section;
use crate::utils::api_helpers::APIResult;
use bincode::{Decode, Encode};
use rocket::State;
use rocket::form::Form;
use rocket::fs::{NamedFile, TempFile};
use rocket::http::Status;
use rocket::serde::json::Json;
use serde::{Deserialize, Serialize};

pub mod bibliography;
pub mod get;
pub mod patch;
pub mod sections;

/// Delete project
/// DELETE /api/projects/<project_id>
#[delete("/api/projects/<project_id>")]
pub async fn delete_project(
    project_id: String,
    _session: Session,
    settings: &State<Settings>,
    pool: &State<sqlx::PgPool>,
) -> APIResult<()> {
    let project_id = uuid::Uuid::parse_str(&project_id)?;
    let pool = pool.inner();

    // Section CRDT content lives in a flat directory keyed by section id, untouched by the
    // cascading SQL delete below — enumerate and remove those files first. If the lookup
    // fails we bail out here rather than deleting the project anyway: doing so with an
    // (incorrectly) empty id list would orphan the on-disk content files with no DB row left
    // to ever reference them again for cleanup.
    let section_ids =
        crate::db::repositories::sections::get_all_ids_for_project(pool, project_id).await?;
    for section_id in section_ids {
        if let Err(e) = crate::db::section_content::delete(settings.inner(), section_id).await {
            warn!(
                "Couldn't delete CRDT content file for section {}: {}",
                section_id, e
            );
        }
    }

    crate::db::repositories::projects::delete(pool, project_id).await?;

    // Best-effort cleanup of the project's upload directory (unrelated to the DB row).
    let uploads_path = format!("{}/projects/{}", settings.data_path, project_id);
    if let Err(e) = tokio::fs::remove_dir_all(&uploads_path).await
        && e.kind() != std::io::ErrorKind::NotFound
    {
        warn!(
            "Couldn't delete upload directory for project {}: {}",
            project_id, e
        );
    }

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
            None => patch.map(|patch| T::default().patch(patch)),
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
    pool: &State<sqlx::PgPool>,
) -> APIResult<Vec<Section>> {
    let project_id = uuid::Uuid::parse_str(&project_id)?;
    let contents =
        crate::db::repositories::sections::get_tree_for_project(pool.inner(), project_id).await?;
    Ok(contents.into())
}

/// POST /api/projects/<project_id>/contents
/// Add a new section to the project
#[post("/api/projects/<project_id>/contents", data = "<content>")]
pub async fn add_content(
    project_id: String,
    _session: Session,
    settings: &State<Settings>,
    pool: &State<sqlx::PgPool>,
    content: Json<Section>,
) -> APIResult<Section> {
    let project_id = uuid::Uuid::parse_str(&project_id)?;

    // Check if Section or Toc, generate uuid if section
    let mut content = content.into_inner();
    if content.id.is_none() {
        content.id = Some(uuid::Uuid::new_v4());
    }

    // Insert new content block at the end
    crate::db::repositories::sections::insert_at_end(
        pool.inner(),
        settings.inner(),
        project_id,
        None,
        &content,
    )
    .await?;

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
    pool: &State<sqlx::PgPool>,
) -> APIResult<()> {
    let content_id = uuid::Uuid::parse_str(&content_id)?;
    let after_id = uuid::Uuid::parse_str(&after_id)?;
    let project_id = uuid::Uuid::parse_str(&project_id)?;

    crate::db::repositories::sections::move_after(pool.inner(), project_id, content_id, after_id)
        .await?;
    Ok(().into())
}

/// PUT /api/projects/<project_id>/contents/<content_id>/move/child_of/<parent_id>
/// Move a section or toc to be a child of another section or toc. It will be the first child.
#[put("/api/projects/<project_id>/contents/<content_id>/move/child_of/<parent_id>")]
pub async fn move_content_child_of(
    project_id: String,
    content_id: String,
    parent_id: String,
    _session: Session,
    pool: &State<sqlx::PgPool>,
) -> APIResult<()> {
    let content_id = uuid::Uuid::parse_str(&content_id)?;
    let parent_id = uuid::Uuid::parse_str(&parent_id)?;
    let project_id = uuid::Uuid::parse_str(&project_id)?;

    crate::db::repositories::sections::move_child_of(
        pool.inner(),
        project_id,
        content_id,
        parent_id,
    )
    .await?;
    Ok(().into())
}

#[derive(FromForm)]
pub struct ImageUpload<'a> {
    image: TempFile<'a>,
}

#[derive(Serialize, Deserialize, Default)]
pub struct ImageUploadResponse {
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
    pool: &State<sqlx::PgPool>,
    _session: Session,
) -> Json<ImageUploadResponse> {
    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            eprintln!("Couldn't parse project id: {}", e);
            return Json(ImageUploadResponse::default());
        }
    };

    match crate::db::repositories::projects::exists(pool.inner(), project_id).await {
        Ok(true) => {}
        _ => {
            eprintln!("Couldn't get project with id {}", project_id);
            return Json(ImageUploadResponse::default());
        }
    }

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
                filename,
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
    pool: &State<sqlx::PgPool>,
    _session: Session,
) -> APIResult<Option<uuid::Uuid>> {
    let project_id = uuid::Uuid::parse_str(&project_id)?;
    let template_id =
        crate::db::repositories::projects::get_template_id(pool.inner(), project_id).await?;
    Ok(template_id.into())
}

/// Set project's template to the specified template_id
/// PUT /api/projects/<project_id>/template
#[put("/api/projects/<project_id>/template", data = "<template_id>")]
pub async fn set_project_template(
    project_id: String,
    template_id: Json<uuid::Uuid>,
    pool: &State<sqlx::PgPool>,
    _session: Session,
) -> APIResult<()> {
    let project_id = uuid::Uuid::parse_str(&project_id)?;
    crate::db::repositories::projects::update_template(
        pool.inner(),
        project_id,
        Some(template_id.into_inner()),
    )
    .await?;
    Ok(().into())
}

/// List all templates
/// GET /api/templates
#[get("/api/templates")]
pub async fn list_templates(
    _session: Session,
    pool: &State<sqlx::PgPool>,
) -> APIResult<Vec<crate::db::repositories::templates::Template>> {
    let templates = crate::db::repositories::templates::list_all(pool.inner()).await?;
    Ok(templates.into())
}

use crate::session::session_guard::Session;
use crate::storage::data_storage::DataStorage;
use crate::storage::ProjectTemplateV2;
use crate::utils::api_helpers::{APIResponse, APIResult, ApiErrorType};
use rocket::form::Form;
use rocket::fs::{NamedFile, TempFile};
use rocket::http::Status;
use rocket::serde::json::Json;
use rocket::State;
use std::fs;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;
use vb_exchange::export_formats::{ExportFormat, ExportStep};

/// Contains API endpoints for the templates editor.

/// GET /api/templates/<template_id>
/// Get a template by its id.
#[get("/api/templates/<template_id>")]
pub async fn get_template(
    _session: Session,
    template_id: String,
    data_storage: &State<Arc<DataStorage>>,
) -> APIResult<ProjectTemplateV2> {
    //Parse template_id to uuid
    let template_id = match Uuid::parse_str(&template_id) {
        Ok(template_id) => template_id,
        Err(e) => {
            eprintln!("Couldn't parse template id: {}", e);
            return Err(ApiErrorType::UnparsableParameter("uuid".to_string()).into());
        }
    };

    let data_storage = data_storage;

    // Get template from data storage
    let lock = data_storage.data.read().unwrap();
    let template = lock.templates.get(&template_id);
    template.map_or_else(
        || Err(ApiErrorType::ResourceNotFound("template".to_string()).into()),
        |template| Ok(template.clone().read().unwrap().clone().into()),
    )
}

/// POST /api/templates/<template_id>
/// Update a template by its id.
/// The template id in the url must match the id in the body.
/// Can't be used to create a new template.
#[post("/api/templates/<template_id>", data = "<template>")]
pub async fn update_template(
    _session: Session,
    template_id: String,
    template: Json<ProjectTemplateV2>,
    data_storage: &State<Arc<DataStorage>>,
) -> APIResult<()> {
    //Parse template_id to uuid
    let template_id = match Uuid::parse_str(&template_id) {
        Ok(template_id) => template_id,
        Err(e) => {
            eprintln!("Couldn't parse template id: {}", e);
            return Err(ApiErrorType::UnparsableParameter("uuid".to_string()).into());
        }
    };

    let mut template = template.into_inner();
    // Generate a new version id since we updated the template
    template.version = Some(uuid::Uuid::new_v4());

    let data_storage = data_storage;

    // Check if template exists, otherwise return 404
    let lock = data_storage.data.read().unwrap();
    if !lock.templates.contains_key(&template_id) {
        return Err(ApiErrorType::ResourceNotFound("template".to_string()).into());
    }

    // Check if id in template matches id in url
    if template_id != template.id {
        return Err(ApiErrorType::Other(
            "Template id in url does not match template id in body, id change is not supported."
                .to_string(),
        )
        .into());
    }

    *lock.templates.get(&template_id).unwrap().write().unwrap() = template;

    Ok(().into())
}

#[derive(serde::Deserialize, serde::Serialize, Debug)]
pub struct AssetList {
    pub assets: Vec<Asset>,
}

#[derive(serde::Deserialize, serde::Serialize, Debug)]
pub struct AssetFolder {
    /// Path to the folder to identify uniquily, e.g. folder1.folder2
    pub path: String,
    /// Name of the folder, unique inside the parent folder
    pub name: String,
    /// Subfolders and files inside this folder
    pub assets: Vec<Asset>,
}

#[derive(serde::Deserialize, serde::Serialize, Debug)]
pub struct AssetFile {
    /// Path to the file to identify uniquily, e.g. folder1.folder2.file1
    pub path: String,
    /// Name of the file, unique inside the parent folder
    pub name: String,
    /// Mime type of the file to determine if editable in browser, e.g. "text/plain" TODO: auto detect mime type
    pub mime_type: Option<String>,
}

#[derive(serde::Deserialize, serde::Serialize, Debug)]
pub enum Asset {
    Folder(AssetFolder),
    File(AssetFile),
}

#[derive(serde::Deserialize, serde::Serialize)]
pub struct NewAssetFolder {
    pub name: String,
}

#[derive(FromForm)]
pub struct NewAssetFile<'r> {
    pub file: TempFile<'r>,
}

pub fn sanitize_path(path: &str) -> String {
    // Entfernen von `../` und `./`
    let path = path.replace("../", "").replace("./", "");

    // Remove leading / if present
    let path = if path.starts_with("/") {
        &path[1..]
    } else {
        &path
    };

    // Erlaubte Zeichen sind alphanumerische Zeichen, Unterstrich, Bindestrich, Punkt und Schrägstrich
    let allowed_chars =
        |c: &char| c.is_alphanumeric() || *c == '_' || *c == '-' || *c == '.' || *c == '/';
    path.chars().filter(allowed_chars).collect()
}

/// Safely combines a base path with a user input path.
pub fn safe_path_combine(base_path: &str, user_input: &str) -> Result<PathBuf, ()> {
    let sanitized_input = sanitize_path(user_input);
    if sanitized_input.is_empty() {
        return Err(());
    }
    let base = Path::new(base_path);
    let full_path = base.join(sanitized_input);

    // Sicherstellen, dass der resultierende Pfad im Basisverzeichnis bleibt
    if !full_path.starts_with(base) {
        return Err(());
    }

    Ok(full_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_safe_path_combine_valid_path() {
        let base_path = "/data/templates/template1/assets";
        let user_input = "folder1/file1.txt";
        let expected_result = Ok(PathBuf::from(
            "/data/templates/template1/assets/folder1/file1.txt",
        ));

        let result = safe_path_combine(base_path, user_input);

        assert_eq!(result, expected_result);
    }

    #[test]
    fn test_safe_path_combine_evil_path() {
        let base_path = "/data/templates/template1/assets";
        let user_input = "../folder1/file1.txt";
        let expected_result = Ok(PathBuf::from(
            "/data/templates/template1/assets/folder1/file1.txt",
        ));

        let result = safe_path_combine(base_path, user_input);

        assert_eq!(result, expected_result);
    }

    #[test]
    fn test_safe_path_combine_root_folder() {
        let base_path = "/data/templates/template1/assets";
        let user_input = "/folder1/file1.txt";
        let expected_result = Ok(PathBuf::from(
            "/data/templates/template1/assets/folder1/file1.txt",
        ));

        let result = safe_path_combine(base_path, user_input);

        assert_eq!(result, expected_result);
    }
}

/// POST /api/templates/<template_id>/assets/file
/// Creates a new asset in the global assets folder of the template
#[post("/api/templates/<template_id>/assets/file", data = "<asset>")]
pub async fn create_file_asset(
    _session: Session,
    template_id: String,
    asset: Form<NewAssetFile<'_>>,
    data_storage: &State<Arc<DataStorage>>,
) -> APIResult<()> {
    //Parse template_id to uuid
    let template_id = match Uuid::parse_str(&template_id) {
        Ok(template_id) => template_id,
        Err(e) => {
            eprintln!("Couldn't parse template id: {}", e);
            return Err(ApiErrorType::UnparsableParameter("uuid".to_string()).into());
        }
    };

    let mut file = asset.into_inner().file;

    let filename = match file.raw_name() {
        Some(name) => name,
        None => {
            eprintln!("No file name provided");
            return Err(ApiErrorType::Other("No file name provided".to_string()).into());
        }
    };

    let filename = sanitize_path(filename.dangerous_unsafe_unsanitized_raw().as_str());

    println!("Filename: {}", filename);

    let mut path;
    let mut i = 0;

    loop {
        path = if i == 0 {
            format!("data/templates/{}/assets/{}", template_id, filename)
        } else {
            let filename_splitted = filename.split('.').collect::<Vec<&str>>();

            let new_filename = if filename_splitted.len() == 1 {
                // File has no extension, add number to end
                format!("{}_{}", filename, i)
            } else {
                // Get all parts except the last one
                let filename_without_extension = filename_splitted
                    .clone()
                    .iter()
                    .take(filename_splitted.len() - 1)
                    .map(|s| format!("{}.", s))
                    .collect::<String>();
                format!(
                    "{}_{}.{}",
                    filename_without_extension,
                    i,
                    filename_splitted.last().unwrap()
                )
            };

            format!("data/templates/{}/assets/{}", template_id, new_filename)
        };
        // Check if file already exists
        if Path::new(&path).exists() {
            i += 1;
        } else {
            break;
        }
    }
    match file.copy_to(path).await {
        Ok(_) => {
            if let Err(()) = data_storage.update_template_version_id(template_id).await {
                return Err(ApiErrorType::ResourceNotFound("template".to_string()).into());
            }
            return Ok(().into());
        }
        Err(e) => {
            eprintln!("Error copying file: {}", e);
            return Err(ApiErrorType::InternalServerError.into());
        }
    }
}

#[derive(serde::Deserialize, serde::Serialize)]
pub struct DeleteAssetRequest {
    pub paths: Vec<String>,
}

/// DELETE /api/templates/<template_id>/assets/<path>
/// Deletes an asset in the global assets folder of the template
#[delete("/api/templates/<template_id>/assets", data = "<paths>")]
pub async fn delete_assets(
    _session: Session,
    template_id: String,
    paths: Json<DeleteAssetRequest>,
    data_storage: &State<Arc<DataStorage>>,
) -> APIResult<()> {
    //Parse template_id to uuid
    let template_id = match Uuid::parse_str(&template_id) {
        Ok(template_id) => template_id,
        Err(e) => {
            eprintln!("Couldn't parse template id: {}", e);
            return Err(ApiErrorType::UnparsableParameter("uuid".to_string()).into());
        }
    };

    let base_path_raw = Path::new(&format!("data/templates/{}/assets", template_id))
        .canonicalize()
        .unwrap();
    let base_path = base_path_raw.to_str().unwrap();

    for path in &paths.paths {
        let path = match safe_path_combine(&base_path, &path) {
            Ok(path) => path,
            Err(_) => {
                eprintln!("Error deleting asset, invalid path.");
                return Err(ApiErrorType::Other("Invalid path".to_string()).into());
            }
        };

        // Check if directory or file
        if path.is_dir() {
            match tokio::fs::remove_dir_all(path).await {
                Ok(_) => (),
                Err(_) => {
                    eprintln!("Error deleting asset.");
                    return Err(ApiErrorType::InternalServerError.into());
                }
            }
        } else {
            match tokio::fs::remove_file(path).await {
                Ok(_) => (),
                Err(_) => {
                    eprintln!("Error deleting asset.");
                    return Err(ApiErrorType::InternalServerError.into());
                }
            }
        }
    }

    if let Err(()) = data_storage.update_template_version_id(template_id).await {
        return Err(ApiErrorType::ResourceNotFound("template".to_string()).into());
    }

    Ok(().into())
}

/// POST /api/templates/<template_id>/assets/folder
/// Creates a new asset in the global assets folder of the template
#[post("/api/templates/<template_id>/assets/folder", data = "<asset>")]
pub async fn create_folder_asset(
    _session: Session,
    template_id: String,
    asset: Json<NewAssetFolder>,
    data_storage: &State<Arc<DataStorage>>,
) -> APIResult<()> {
    //Parse template_id to uuid
    let template_id = match Uuid::parse_str(&template_id) {
        Ok(template_id) => template_id,
        Err(e) => {
            eprintln!("Couldn't parse template id: {}", e);
            return Err(ApiErrorType::UnparsableParameter("uuid".to_string()).into());
        }
    };

    let name = sanitize_path(&asset.name);

    // Get the path to the global assets folder
    let path = format!("data/templates/{}/assets/{}", template_id, name);

    // Create the folder
    let res = tokio::task::spawn_blocking(move || match fs::create_dir(&path) {
        Ok(_) => Ok(().into()),
        Err(e) => match e.kind() {
            io::ErrorKind::AlreadyExists => {
                Err(ApiErrorType::Other("Folder already exists".to_string()).into())
            }
            _ => {
                eprintln!("Error creating folder: {}", e);
                Err(ApiErrorType::InternalServerError.into())
            }
        },
    })
    .await;

    match res {
        Ok(res) => {
            if let Err(()) = data_storage.update_template_version_id(template_id).await {
                return Err(ApiErrorType::ResourceNotFound("template".to_string()).into());
            }
            res
        }
        Err(e) => {
            eprintln!("Error creating folder: {}", e);
            Err(ApiErrorType::InternalServerError.into())
        }
    }
}

/// GET /api/templates/<template_id>/assets
/// List all global assets saved for the template
#[get("/api/templates/<template_id>/assets")]
pub async fn get_assets(_session: Session, template_id: String) -> APIResult<AssetList> {
    //Parse template_id to uuid
    let template_id = match Uuid::parse_str(&template_id) {
        Ok(template_id) => template_id,
        Err(e) => {
            eprintln!("Couldn't parse template id: {}", e);
            return Err(ApiErrorType::UnparsableParameter("uuid".to_string()).into());
        }
    };

    // Get all entries in the global assets folder (via async fs) inside data/templates/<template_id>/assets
    let res = tokio::task::spawn_blocking(move || {
        let path = format!("data/templates/{}/assets", template_id);
        match get_assets_recursive(&path, None) {
            Ok(assets) => Ok(AssetList { assets }.into()),
            Err(e) => {
                eprintln!("Error getting assets: {}", e);
                Err(ApiErrorType::InternalServerError.into())
            }
        }
    })
    .await;

    match res {
        Ok(assets) => assets,
        Err(e) => {
            eprintln!("Error getting assets: {}", e);
            Err(ApiErrorType::InternalServerError.into())
        }
    }
}

fn get_assets_recursive(
    current_path: &str,
    path_to_asset: Option<&String>,
) -> Result<Vec<Asset>, io::Error> {
    let mut assets: Vec<Asset> = Vec::new();
    let entries = fs::read_dir(current_path)?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();

        let path_to_asset = match path_to_asset {
            Some(path) => format!(
                "{}/{}",
                path,
                entry.file_name().to_string_lossy().to_string()
            ),
            None => entry.file_name().to_string_lossy().to_string(),
        };

        if path.is_dir() {
            let folder = AssetFolder {
                name: entry.file_name().to_string_lossy().to_string(),
                assets: get_assets_recursive(&path.to_string_lossy(), Some(&path_to_asset))?,
                path: path_to_asset.clone(),
            };
            assets.push(Asset::Folder(folder));
        } else {
            let file = AssetFile {
                name: entry.file_name().to_string_lossy().to_string(),
                mime_type: None, //TODO: remove if not needed
                path: path_to_asset,
            };
            assets.push(Asset::File(file));
        }
    }

    Ok(assets)
}

/// GET /api/templates/<template_id>/assets/files/<path>
/// Get an specific File asset in the global assets folder of the template
#[get("/api/templates/<template_id>/assets/files/<path..>")]
pub async fn get_asset_file(
    _session: Session,
    template_id: String,
    path: PathBuf,
) -> Result<NamedFile, Status> {
    //Parse template_id to uuid
    let template_id = match Uuid::parse_str(&template_id) {
        Ok(template_id) => template_id,
        Err(e) => {
            eprintln!("Couldn't parse template id: {}", e);
            return Err(Status::NotFound);
        }
    };

    // Get the path to the global assets folder
    let path = match safe_path_combine(
        &format!("data/templates/{}/assets", template_id),
        &path.to_string_lossy(),
    ) {
        //TODO use path to data directory from config
        Ok(path) => path,
        Err(_) => {
            eprintln!("Error getting asset, invalid path.");
            return Err(Status::BadRequest);
        }
    };

    match NamedFile::open(path).await {
        Ok(file) => Ok(file),
        Err(e) => {
            eprintln!("Error getting asset: {}", e);
            Err(Status::NotFound)
        }
    }
}

#[derive(serde::Deserialize)]
pub struct UpdateAssetRequest {
    pub content: String,
}

/// PUT /api/templates/<template_id>/assets/files/<path>
/// Updates a text-based asset in the global assets folder of the template
/// The asset must be a text-based file, e.g. .txt, .html, .css, .js
#[put(
    "/api/templates/<template_id>/assets/files/<path..>",
    data = "<content>"
)]
pub async fn update_asset_file(
    _session: Session,
    template_id: String,
    path: PathBuf,
    content: Json<UpdateAssetRequest>,
    data_storage: &State<Arc<DataStorage>>,
) -> APIResult<()> {
    //Parse template_id to uuid
    let template_id = match Uuid::parse_str(&template_id) {
        Ok(template_id) => template_id,
        Err(e) => {
            eprintln!("Couldn't parse template id: {}", e);
            return Err(ApiErrorType::UnparsableParameter("uuid".to_string()).into());
        }
    };

    // Get the path to the global assets folder
    let path = match safe_path_combine(
        &format!("data/templates/{}/assets", template_id),
        &path.to_string_lossy(),
    ) {
        //TODO use path to data directory from config
        Ok(path) => path,
        Err(_) => {
            eprintln!("Error updating asset, invalid path.");
            return Err(ApiErrorType::BadRequest("Invalid path".to_string()).into());
        }
    };

    // Check if file exists
    if !path.exists() {
        return Err(ApiErrorType::ResourceNotFound("asset".to_string()).into());
    }

    if let Err(()) = data_storage.update_template_version_id(template_id).await {
        return Err(ApiErrorType::ResourceNotFound("template".to_string()).into());
    }

    // Update the file
    match tokio::fs::write(&path, content.into_inner().content).await {
        Ok(_) => Ok(().into()),
        Err(e) => {
            eprintln!("Error updating asset: {}", e);
            Err(ApiErrorType::InternalServerError.into())
        }
    }
}

/// POST /api/templates/<template_id>/assets/move
/// Moves an asset in the global assets folder of the template
#[post("/api/templates/<template_id>/assets/move", data = "<asset>")]
pub async fn move_asset(
    _session: Session,
    template_id: String,
    asset: Json<MoveAssetRequest>,
    data_storage: &State<Arc<DataStorage>>,
) -> APIResult<()> {
    //Parse template_id to uuid
    let template_id = match Uuid::parse_str(&template_id) {
        Ok(template_id) => template_id,
        Err(e) => {
            eprintln!("Couldn't parse template id: {}", e);
            return Err(ApiErrorType::UnparsableParameter("uuid".to_string()).into());
        }
    };
    let base_path = format!("data/templates/{}/assets", template_id);
    let base_path = Path::new(&base_path).canonicalize().unwrap();

    let old_path = match safe_path_combine(&base_path.to_str().unwrap(), &asset.old_path) {
        Ok(path) => path,
        Err(_) => {
            eprintln!("Error moving asset, invalid path.");
            return Err(ApiErrorType::BadRequest("Invalid path".to_string()).into());
        }
    };

    let new_path = match safe_path_combine(&base_path.to_str().unwrap(), &asset.new_path) {
        Ok(path) => path,
        Err(_) => {
            eprintln!("Error moving asset, invalid path.");
            return Err(ApiErrorType::BadRequest("Invalid path".to_string()).into());
        }
    };

    // If not overwriting, ensure target doesn't exist
    if !asset.overwrite && new_path.exists() {
        return Err(ApiErrorType::Conflict("Target path already exists".to_string()).into());
    }

    // Move the asset asynchronously
    if let Err(e) = tokio::fs::rename(&old_path, &new_path).await {
        eprintln!("Error moving asset: {}", e);
        return Err(ApiErrorType::InternalServerError.into());
    }

    if let Err(()) = data_storage.update_template_version_id(template_id).await {
        return Err(ApiErrorType::ResourceNotFound("template".to_string()).into());
    }

    Ok(().into())
}

#[post("/api/templates/<template_id>/export_formats", data = "<data>")]
pub async fn add_export_format(
    _session: Session,
    template_id: String,
    data_storage: &State<Arc<DataStorage>>,
    data: Json<ExportFormat>,
) -> APIResult<ExportFormat> {
    // Clone data storage
    let data_storage = data_storage;

    let template_id = match Uuid::parse_str(&template_id) {
        Ok(template_id) => template_id,
        Err(e) => {
            eprintln!("Couldn't parse template id: {}", e);
            return Err(ApiErrorType::UnparsableParameter("uuid".to_string()).into());
        }
    };

    // Get the format to be added
    let format = data.into_inner();

    // Add folder in file system
    let base_path = format!("data/templates/{}/formats", template_id);
    let base_path = Path::new(&base_path).canonicalize().unwrap();

    let new_path = match safe_path_combine(&base_path.to_str().unwrap(), &format.slug) {
        Ok(path) => path,
        Err(_) => {
            eprintln!("Error creating export Format, invalid slug.");
            return Err(ApiErrorType::BadRequest("Invalid Slug".to_string()).into());
        }
    };

    if new_path.exists() {
        return Err(ApiErrorType::Conflict(
            "An export format with this slug already exists.".to_string(),
        )
        .into());
    }

    let template_exists;
    {
        let lock = data_storage.data.read().unwrap();
        template_exists = match lock.templates.get(&template_id) {
            Some(template) => {
                template
                    .write()
                    .unwrap()
                    .export_formats
                    .insert(format.slug.clone(), format.clone());
                true
            }
            None => false,
        };
    }

    if !template_exists {
        return Err(ApiErrorType::ResourceNotFound("template".to_string()).into());
    }

    match tokio::fs::create_dir_all(new_path).await {
        Ok(_) => {
            if let Err(()) = data_storage.update_template_version_id(template_id).await {
                return Err(ApiErrorType::ResourceNotFound("template".to_string()).into());
            }
            Ok(format.into())
        }
        Err(e) => {
            eprintln!("Couldn't create folder for new export format: {}", e);
            Err(ApiErrorType::InternalServerError.into())
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct ExportFormatMetadata {
    pub name: String,
    pub slug: String,
    pub preview_pdf_path: Option<String>,
}

/// UPDATE /api/templates/<template_id>/export_formats/<slug>
/// Updates the export_format metadata
#[post(
    "/api/templates/<template_id>/export_formats/<slug>/metadata",
    data = "<data>"
)]
pub async fn update_export_format_metadata(
    _session: Session,
    slug: String,
    template_id: String,
    data_storage: &State<Arc<DataStorage>>,
    data: Json<ExportFormatMetadata>,
) -> APIResult<()> {
    // Clone data storage
    let data_storage = data_storage;

    let template_id = match Uuid::parse_str(&template_id) {
        Ok(template_id) => template_id,
        Err(e) => {
            eprintln!("Couldn't parse template id: {}", e);
            return Err(ApiErrorType::UnparsableParameter("uuid".to_string()).into());
        }
    };

    // Get template
    let template_entry = match data_storage
        .data
        .read()
        .unwrap()
        .templates
        .get(&template_id)
    {
        None => return Err(ApiErrorType::ResourceNotFound("template".to_string()).into()),
        Some(template) => Arc::clone(template),
    };

    // Move files on disk if slug changed
    if slug != data.slug {
        let base_path = format!("data/templates/{}/formats", template_id);
        let base_path = Path::new(&base_path).canonicalize().unwrap();

        let old_path = match safe_path_combine(&base_path.to_str().unwrap(), &slug) {
            Ok(path) => path,
            Err(_) => {
                eprintln!("Error moving export Format, invalid slug.");
                return Err(ApiErrorType::BadRequest("Invalid Slug".to_string()).into());
            }
        };
        let new_path = match safe_path_combine(&base_path.to_str().unwrap(), &data.slug) {
            Ok(path) => path,
            Err(_) => {
                eprintln!("Error moving export Format, invalid slug.");
                return Err(ApiErrorType::BadRequest("Invalid Slug".to_string()).into());
            }
        };

        if new_path.exists() {
            return Err(ApiErrorType::Conflict(
                "An export format with this slug already exists.".to_string(),
            )
            .into());
        }

        if let Err(e) = tokio::fs::rename(old_path, new_path).await {
            eprintln!("Couldn't rename export format: {}", e);
            return Err(ApiErrorType::InternalServerError.into());
        }
    }

    if slug != data.slug {
        let mut entry = match template_entry.write().unwrap().export_formats.remove(&slug) {
            None => return Err(ApiErrorType::ResourceNotFound("export_format".to_string()).into()),
            Some(entry) => entry,
        };
        entry.slug = data.slug.clone();
        entry.name = data.name.clone();
        entry.preview_pdf_path = data.preview_pdf_path.clone();

        template_entry
            .write()
            .unwrap()
            .export_formats
            .insert(entry.slug.clone(), entry);
    } else {
        match template_entry
            .write()
            .unwrap()
            .export_formats
            .get_mut(&slug)
        {
            None => return Err(ApiErrorType::ResourceNotFound("export_format".to_string()).into()),
            Some(export_format) => {
                export_format.name = data.name.clone();
                export_format.preview_pdf_path = data.preview_pdf_path.clone();
            }
        }
    }

    Ok(().into())
}

/// DELETE /api/templates/<template_id>/export_formats/<slug>
/// Deletes export format with slug <slug> in template with <template_id>
#[delete("/api/templates/<template_id>/export_formats/<slug>")]
pub async fn delete_export_format(
    _session: Session,
    template_id: String,
    data_storage: &State<Arc<DataStorage>>,
    slug: String,
) -> APIResult<()> {
    let data_storage = data_storage;

    let template_id = match Uuid::parse_str(&template_id) {
        Ok(template_id) => template_id,
        Err(e) => {
            eprintln!("Couldn't parse template id: {}", e);
            return Err(ApiErrorType::UnparsableParameter("uuid".to_string()).into());
        }
    };
    let slug = sanitize_path(&slug);

    let template = {
        let templates_guard = data_storage.data.read().unwrap();
        let templates = &templates_guard.templates;
        // This scope ensures that we drop the lock as soon as we finish using it
        match templates.get(&template_id) {
            Some(template) => template.clone(),
            None => return Err(ApiErrorType::ResourceNotFound("template".to_string()).into()),
        }
    };

    let remove_result = {
        let mut template_write = template.write().unwrap();
        template_write.export_formats.remove(&slug)
    };

    match remove_result {
        Some(_) => {
            //Remove folder:
            let base_path = format!("data/templates/{}/formats/", template_id);
            let safe_path = safe_path_combine(base_path.as_str(), &slug);
            match safe_path {
                Ok(path) => match tokio::fs::remove_dir_all(path).await {
                    Ok(_) => {
                        if let Err(()) = data_storage.update_template_version_id(template_id).await
                        {
                            return Err(
                                ApiErrorType::ResourceNotFound("template".to_string()).into()
                            );
                        }
                        Ok(().into())
                    }
                    Err(e) => {
                        eprintln!("Couldn't delete physical folder for export format: {}", e);
                        Err(ApiErrorType::InternalServerError.into())
                    }
                },
                Err(_) => {
                    eprintln!("Couldn't delete physical folder for export format. Couldn't create safe_path");
                    Err(ApiErrorType::BadRequest("Invalid Slug".to_string()).into())
                }
            }
        }
        None => Err(ApiErrorType::ResourceNotFound("export_format".to_string()).into()),
    }
}

/// GET /api/templates/<template_id>/export_formats/<slug>/assets
/// List all assets of the export_format
#[get("/api/templates/<template_id>/export_formats/<slug>/assets")]
pub async fn get_assets_for_export_format(
    _session: Session,
    template_id: String,
    slug: String,
) -> APIResult<AssetList> {
    //Parse template_id to uuid
    let template_id = match Uuid::parse_str(&template_id) {
        Ok(template_id) => template_id,
        Err(e) => {
            eprintln!("Couldn't parse template id: {}", e);
            return Err(ApiErrorType::UnparsableParameter("uuid".to_string()).into());
        }
    };
    let slug = sanitize_path(&slug);

    // Get all entries in the assets folder (via async fs) inside data/templates/<template_id>/assets
    let res = tokio::task::spawn_blocking(move || {
        let path = format!("data/templates/{}/formats/{}", template_id, slug);
        match get_assets_recursive(&path, None) {
            Ok(assets) => Ok(AssetList { assets }.into()),
            Err(e) => {
                eprintln!("Error getting assets: {}", e);
                Err(ApiErrorType::InternalServerError.into())
            }
        }
    })
    .await;

    match res {
        Ok(assets) => assets,
        Err(e) => {
            eprintln!("Error getting assets: {}", e);
            Err(ApiErrorType::InternalServerError.into())
        }
    }
}

/// GET /api/templates/<template_id>/export_formats/<slug>/assets/files/<path>
/// Get an specific File asset in the folder of the export format
#[get("/api/templates/<template_id>/export_formats/<slug>/assets/files/<path..>")]
pub async fn get_asset_file_for_export_format(
    _session: Session,
    template_id: String,
    path: PathBuf,
    slug: String,
) -> Result<NamedFile, Status> {
    //Parse template_id to uuid
    let template_id = match Uuid::parse_str(&template_id) {
        Ok(template_id) => template_id,
        Err(e) => {
            eprintln!("Couldn't parse template id: {}", e);
            return Err(Status::NotFound);
        }
    };
    let slug = sanitize_path(&slug);

    // Get the path to the export format folder
    let path = match safe_path_combine(
        &format!("data/templates/{}/formats/{}", template_id, slug),
        &path.to_string_lossy(),
    ) {
        //TODO use path to data directory from config
        Ok(path) => path,
        Err(_) => {
            eprintln!("Error getting asset, invalid path.");
            return Err(Status::BadRequest);
        }
    };

    match NamedFile::open(path).await {
        Ok(file) => Ok(file),
        Err(e) => {
            eprintln!("Error getting asset: {}", e);
            Err(Status::NotFound)
        }
    }
}

/// POST /api/templates/<template_id>/export_formats/<slug>/assets/file
/// Creates a new asset in the export format folder
#[post(
    "/api/templates/<template_id>/export_formats/<slug>/assets/file",
    data = "<asset>"
)]
pub async fn create_file_asset_for_export_format(
    _session: Session,
    template_id: String,
    slug: String,
    asset: Form<NewAssetFile<'_>>,
    data_storage: &State<Arc<DataStorage>>,
) -> APIResult<()> {
    //Parse template_id to uuid
    let template_id = match Uuid::parse_str(&template_id) {
        Ok(template_id) => template_id,
        Err(e) => {
            eprintln!("Couldn't parse template id: {}", e);
            return Err(ApiErrorType::UnparsableParameter("uuid".to_string()).into());
        }
    };

    let slug = sanitize_path(&slug);

    let mut file = asset.into_inner().file;

    let filename = match file.raw_name() {
        Some(name) => name,
        None => {
            eprintln!("No file name provided");
            return Err(ApiErrorType::BadRequest("No file name provided".to_string()).into());
        }
    };

    let filename = sanitize_path(filename.dangerous_unsafe_unsanitized_raw().as_str());

    let mut path;
    let mut i = 0;

    loop {
        path = if i == 0 {
            format!(
                "data/templates/{}/formats/{}/{}",
                template_id, slug, filename
            )
        } else {
            let filename_splitted = filename.split('.').collect::<Vec<&str>>();

            let new_filename = if filename_splitted.len() == 1 {
                // File has no extension, add number to end
                format!("{}_{}", filename, i)
            } else {
                // Get all parts except the last one
                let filename_without_extension = filename_splitted
                    .clone()
                    .iter()
                    .take(filename_splitted.len() - 1)
                    .map(|s| format!("{}.", s))
                    .collect::<String>();
                format!(
                    "{}_{}.{}",
                    filename_without_extension,
                    i,
                    filename_splitted.last().unwrap()
                )
            };

            format!(
                "data/templates/{}/formats/{}/{}",
                template_id, slug, new_filename
            )
        };
        // Check if file already exists
        if Path::new(&path).exists() {
            i += 1;
        } else {
            break;
        }
    }
    match file.copy_to(path).await {
        Ok(_) => {
            if let Err(()) = data_storage.update_template_version_id(template_id).await {
                return Err(ApiErrorType::ResourceNotFound("template".to_string()).into());
            }
            return Ok(().into());
        }
        Err(e) => {
            eprintln!("Error copying file: {}", e);
            return Err(ApiErrorType::InternalServerError.into());
        }
    }
}

/// DELETE /api/templates/<template_id>/export_formats/<slug>/assets
/// Deletes an asset in the export format folder of the template
#[delete(
    "/api/templates/<template_id>/export_formats/<slug>/assets",
    data = "<paths>"
)]
pub async fn delete_assets_for_export_format(
    _session: Session,
    template_id: String,
    paths: Json<DeleteAssetRequest>,
    slug: String,
    data_storage: &State<Arc<DataStorage>>,
) -> APIResult<()> {
    //Parse template_id to uuid
    let template_id = match Uuid::parse_str(&template_id) {
        Ok(template_id) => template_id,
        Err(e) => {
            eprintln!("Couldn't parse template id: {}", e);
            return Err(ApiErrorType::UnparsableParameter("uuid".to_string()).into());
        }
    };

    let slug = sanitize_path(&slug);

    let base_path_raw = Path::new(&format!("data/templates/{}/formats/{}", template_id, slug))
        .canonicalize()
        .unwrap();
    let base_path = base_path_raw.to_str().unwrap();

    for path in &paths.paths {
        let path = match safe_path_combine(&base_path, &path) {
            Ok(path) => path,
            Err(_) => {
                eprintln!("Error deleting asset, invalid path.");
                return Err(ApiErrorType::BadRequest("Invalid path".to_string()).into());
            }
        };

        // Check if directory or file
        if path.is_dir() {
            match tokio::fs::remove_dir_all(path).await {
                Ok(_) => (),
                Err(e) => {
                    eprintln!("Error deleting asset: {} ", e);
                    return Err(ApiErrorType::InternalServerError.into());
                }
            }
        } else {
            match tokio::fs::remove_file(path).await {
                Ok(_) => (),
                Err(e) => {
                    eprintln!("Error deleting asset: {}", e);
                    return Err(ApiErrorType::InternalServerError.into());
                }
            }
        }
    }
    if let Err(()) = data_storage.update_template_version_id(template_id).await {
        return Err(ApiErrorType::ResourceNotFound("template".to_string()).into());
    }

    Ok(().into())
}

/// POST /api/templates/<template_id>/export_formats/<slug>/assets/folder
/// Creates a new asset in the export format folder of the template
#[post(
    "/api/templates/<template_id>/export_formats/<slug>/assets/folder",
    data = "<asset>"
)]
pub async fn create_folder_asset_for_export_format(
    _session: Session,
    template_id: String,
    asset: Json<NewAssetFolder>,
    slug: String,
    data_storage: &State<Arc<DataStorage>>,
) -> APIResult<()> {
    //Parse template_id to uuid
    let template_id = match Uuid::parse_str(&template_id) {
        Ok(template_id) => template_id,
        Err(e) => {
            eprintln!("Couldn't parse template id: {}", e);
            return Err(ApiErrorType::UnparsableParameter("uuid".to_string()).into());
        }
    };

    let slug = sanitize_path(&slug);

    let name = sanitize_path(&asset.name);

    // Get the path to the global assets folder
    let path = format!("data/templates/{}/formats/{}/{}", template_id, slug, name);

    // Create the folder
    let res = tokio::task::spawn_blocking(move || match fs::create_dir(&path) {
        Ok(_) => Ok(().into()),
        Err(e) => match e.kind() {
            io::ErrorKind::AlreadyExists => {
                Err(ApiErrorType::Conflict("Folder already exists".to_string()).into())
            }
            _ => {
                eprintln!("Error creating folder: {}", e);
                Err(ApiErrorType::InternalServerError.into())
            }
        },
    })
    .await;

    match res {
        Ok(res) => {
            if let Err(()) = data_storage.update_template_version_id(template_id).await {
                return Err(ApiErrorType::ResourceNotFound("template".to_string()).into());
            }
            res
        }
        Err(e) => {
            eprintln!("Error creating folder: {}", e);
            Err(ApiErrorType::InternalServerError.into())
        }
    }
}

/// PUT /api/templates/<template_id>/export_formats/<slug>/assets/files/<path>
/// Updates a text-based asset in the export format folder of the template
/// The asset must be a text-based file, e.g. .txt, .html, .css, .js
#[put(
    "/api/templates/<template_id>/export_formats/<slug>/assets/files/<path..>",
    data = "<content>"
)]
pub async fn update_asset_file_for_export_format(
    _session: Session,
    template_id: String,
    path: PathBuf,
    content: Json<UpdateAssetRequest>,
    slug: String,
    data_storage: &State<Arc<DataStorage>>,
) -> APIResult<()> {
    //Parse template_id to uuid
    let template_id = match Uuid::parse_str(&template_id) {
        Ok(template_id) => template_id,
        Err(e) => {
            eprintln!("Couldn't parse template id: {}", e);
            return Err(ApiErrorType::UnparsableParameter("uuid".to_string()).into());
        }
    };

    let slug = sanitize_path(&slug);

    // Get the path to the global assets folder
    let path = match safe_path_combine(
        &format!("data/templates/{}/formats/{}", template_id, slug),
        &path.to_string_lossy(),
    ) {
        //TODO use path to data directory from config
        Ok(path) => path,
        Err(_) => {
            eprintln!("Error updating asset, invalid path.");
            return Err(ApiErrorType::BadRequest("Invalid path".to_string()).into());
        }
    };

    // Check if file exists
    if !path.exists() {
        return Err(ApiErrorType::ResourceNotFound("asset".to_string()).into());
    }

    // Update the file
    match tokio::fs::write(&path, content.into_inner().content).await {
        Ok(_) => {
            if let Err(()) = data_storage.update_template_version_id(template_id).await {
                return Err(ApiErrorType::ResourceNotFound("template".to_string()).into());
            }
            Ok(().into())
        }
        Err(e) => {
            eprintln!("Error updating asset: {}", e);
            Err(ApiErrorType::InternalServerError.into())
        }
    }
}

/// POST /api/templates/<template_id>/export_formats/<slug>/assets/move
/// Moves an asset in the export_format folder of the template
#[post(
    "/api/templates/<template_id>/export_formats/<slug>/assets/move",
    data = "<asset>"
)]
pub async fn move_asset_for_export_format(
    _session: Session,
    template_id: String,
    asset: Json<MoveAssetRequest>,
    slug: String,
    data_storage: &State<Arc<DataStorage>>,
) -> APIResult<()> {
    //Parse template_id to uuid
    let template_id = match Uuid::parse_str(&template_id) {
        Ok(template_id) => template_id,
        Err(e) => {
            eprintln!("Couldn't parse template id: {}", e);
            return Err(ApiErrorType::UnparsableParameter("uuid".to_string()).into());
        }
    };

    let slug = sanitize_path(&slug);

    let base_path = format!("data/templates/{}/formats/{}", template_id, slug);
    let base_path = Path::new(&base_path).canonicalize().unwrap();

    let old_path = match safe_path_combine(&base_path.to_str().unwrap(), &asset.old_path) {
        Ok(path) => path,
        Err(_) => {
            eprintln!("Error moving asset, invalid path.");
            return Err(ApiErrorType::BadRequest("Invalid path".to_string()).into());
        }
    };

    let new_path = match safe_path_combine(&base_path.to_str().unwrap(), &asset.new_path) {
        Ok(path) => path,
        Err(_) => {
            eprintln!("Error moving asset, invalid path.");
            return Err(ApiErrorType::BadRequest("Invalid path".to_string()).into());
        }
    };

    // If not overwriting, ensure target doesn't exist
    if !asset.overwrite && new_path.exists() {
        return Err(ApiErrorType::Conflict("Target path already exists".to_string()).into());
    }

    if let Err(e) = tokio::fs::rename(&old_path, &new_path).await {
        eprintln!("Error moving asset: {}", e);
        return Err(ApiErrorType::InternalServerError.into());
    }

    if let Err(()) = data_storage.update_template_version_id(template_id).await {
        return Err(ApiErrorType::ResourceNotFound("template".to_string()).into());
    }

    Ok(().into())
}

#[derive(serde::Deserialize)]
pub struct MoveAssetRequest {
    pub overwrite: bool,
    pub old_path: String,
    pub new_path: String,
}

/// POST /api/templates/<template_id>/export_formats/<slug>/export_steps/
/// Creates new Export Step
#[post(
    "/api/templates/<template_id>/export_formats/<slug>/export_steps",
    data = "<step>"
)]
pub async fn create_export_step(
    _session: Session,
    template_id: String,
    slug: String,
    step: Json<ExportStep>,
    data_storage: &State<Arc<DataStorage>>,
) -> APIResult<ExportStep> {
    //Parse template_id to uuid
    let template_id = match Uuid::parse_str(&template_id) {
        Ok(template_id) => template_id,
        Err(e) => {
            eprintln!("Couldn't parse template id: {}", e);
            return Err(ApiErrorType::UnparsableParameter("uuid".to_string()).into());
        }
    };

    let slug = sanitize_path(&slug);

    let data_storage = Arc::clone(data_storage);
    let template = match data_storage
        .data
        .read()
        .unwrap()
        .templates
        .get(&template_id)
    {
        Some(template) => template.clone(),
        None => {
            eprintln!("Couldn't find template");
            return Err(ApiErrorType::ResourceNotFound("template".to_string()).into());
        }
    };

    let mut step = step.into_inner();
    step.id = Some(uuid::Uuid::new_v4());

    match template.write().unwrap().export_formats.get_mut(&slug) {
        None => {
            eprintln!("Couldn't find export format");
            return Err(ApiErrorType::ResourceNotFound("export_format".to_string()).into());
        }
        Some(export_format) => export_format.export_steps.push(step.clone()),
    }
    if let Err(()) = data_storage.update_template_version_id(template_id).await {
        return Err(ApiErrorType::ResourceNotFound("template".to_string()).into());
    }

    return Ok(step.into());
}

#[derive(serde::Deserialize)]
pub struct MoveExportStepRequest {
    /// Element to moved behind. Set to None to move to first position
    pub move_after: Option<uuid::Uuid>,
}

/// POST /api/templates/<template_id>/export_formats/<slug>/export_steps/<step_id>/move
/// Moves a export step to a specified position
#[post(
    "/api/templates/<template_id>/export_formats/<slug>/export_steps/<step_id>/move",
    data = "<movedata>"
)]
pub async fn move_export_step(
    _session: Session,
    template_id: String,
    slug: String,
    step_id: String,
    movedata: Json<MoveExportStepRequest>,
    data_storage: &State<Arc<DataStorage>>,
) -> APIResult<()> {
    //Parse template_id to uuid
    let template_id = match Uuid::parse_str(&template_id) {
        Ok(template_id) => template_id,
        Err(e) => {
            eprintln!("Couldn't parse template id: {}", e);
            return Err(ApiErrorType::UnparsableParameter("uuid".to_string()).into());
        }
    };
    //Parse step_id to uuid
    let step_id = match Uuid::parse_str(&step_id) {
        Ok(step_id) => step_id,
        Err(e) => {
            eprintln!("Couldn't parse step id: {}", e);
            return Err(ApiErrorType::UnparsableParameter("uuid".to_string()).into());
        }
    };

    let slug = sanitize_path(&slug);
    let move_after = movedata.move_after;

    let data_storage = Arc::clone(data_storage);
    let template = match data_storage
        .data
        .read()
        .unwrap()
        .templates
        .get(&template_id)
    {
        Some(template) => template.clone(),
        None => {
            eprintln!("Couldn't find template");
            return Err(ApiErrorType::ResourceNotFound("template".to_string()).into());
        }
    };

    match template.write().unwrap().export_formats.get_mut(&slug) {
        None => {
            eprintln!("Couldn't find export format");
            return Err(ApiErrorType::ResourceNotFound("export_format".to_string()).into());
        }
        Some(export_format) => {
            // Find export step and move it after move_after:
            let step_index = export_format
                .export_steps
                .iter()
                .position(|step| step.id == Some(step_id));
            let step_index = match step_index {
                Some(index) => index,
                None => {
                    return Err(ApiErrorType::ResourceNotFound("export_step".to_string()).into())
                }
            };
            let step = export_format.export_steps.remove(step_index);
            // Find new position:
            let new_index = match move_after {
                Some(move_after) => {
                    match export_format
                        .export_steps
                        .iter()
                        .position(|step| step.id == Some(move_after))
                    {
                        None => {
                            return Err(
                                ApiErrorType::ResourceNotFound("export_step".to_string()).into()
                            )
                        }
                        Some(index) => index + 1,
                    }
                }
                None => 0 as usize,
            };
            export_format.export_steps.insert(new_index, step);
        }
    }

    if let Err(()) = data_storage.update_template_version_id(template_id).await {
        return Err(ApiErrorType::ResourceNotFound("template".to_string()).into());
    }
    return Ok(().into());
}

/// PUT /api/templates/<template_id>/export_formats/<slug>/export_steps/<step_id>
/// Updates a export step
#[post(
    "/api/templates/<template_id>/export_formats/<slug>/export_steps/<step_id>",
    data = "<step>"
)]
pub async fn update_export_step(
    _session: Session,
    template_id: String,
    slug: String,
    step_id: String,
    step: Json<ExportStep>,
    data_storage: &State<Arc<DataStorage>>,
) -> APIResult<()> {
    //Parse template_id to uuid
    let template_id = match Uuid::parse_str(&template_id) {
        Ok(template_id) => template_id,
        Err(e) => {
            eprintln!("Couldn't parse template id: {}", e);
            return Err(ApiErrorType::UnparsableParameter("uuid".to_string()).into());
        }
    };

    let slug = sanitize_path(&slug);

    let data_storage = Arc::clone(data_storage);
    let template = match data_storage
        .data
        .read()
        .unwrap()
        .templates
        .get(&template_id)
    {
        Some(template) => template.clone(),
        None => {
            eprintln!("Couldn't find template");
            return Err(ApiErrorType::ResourceNotFound("template".to_string()).into());
        }
    };

    let parameter_step_id = match Uuid::parse_str(&step_id) {
        Ok(id) => id,
        Err(e) => {
            eprintln!("Couldn't parse step id: {}", e);
            return Err(ApiErrorType::UnparsableParameter("uuid".to_string()).into());
        }
    };

    let step = step.into_inner();
    let step_id = match step.id {
        Some(id) => id,
        None => return Err(ApiErrorType::BadRequest(String::from("Missing step_id")).into()),
    };
    if parameter_step_id != step_id {
        return Err(
            ApiErrorType::BadRequest(String::from("step_id mismatches in data and url")).into(),
        );
    }

    match template.write().unwrap().export_formats.get_mut(&slug) {
        None => {
            eprintln!("Couldn't find export format");
            return Err(ApiErrorType::ResourceNotFound("export_format".to_string()).into());
        }
        Some(export_format) => {
            // Find export_step and update
            let index = match export_format
                .export_steps
                .iter()
                .position(|x| x.id == Some(step_id))
            {
                Some(id) => id,
                None => {
                    return Err(ApiErrorType::ResourceNotFound("export_step".to_string()).into())
                }
            };

            match export_format.export_steps.get_mut(index) {
                Some(old_step) => *old_step = step,
                None => {
                    return Err(ApiErrorType::ResourceNotFound("export_step".to_string()).into())
                }
            }
        }
    }
    if let Err(()) = data_storage.update_template_version_id(template_id).await {
        return Err(ApiErrorType::ResourceNotFound("template".to_string()).into());
    }
    Ok(().into())
}

#[delete("/api/templates/<template_id>/export_formats/<slug>/export_steps/<step_id>")]
pub async fn delete_export_step(
    _session: Session,
    template_id: String,
    slug: String,
    step_id: String,
    data_storage: &State<Arc<DataStorage>>,
) -> APIResult<()> {
    //Parse template_id and step_id to uuid
    let template_id = match Uuid::parse_str(&template_id) {
        Ok(template_id) => template_id,
        Err(e) => {
            eprintln!("Couldn't parse template id: {}", e);
            return Err(ApiErrorType::UnparsableParameter("uuid".to_string()).into());
        }
    };

    let step_id = match Uuid::parse_str(&step_id) {
        Ok(step_id) => step_id,
        Err(e) => {
            eprintln!("Couldn't parse step id: {}", e);
            return Err(ApiErrorType::UnparsableParameter("uuid".to_string()).into());
        }
    };

    let slug = sanitize_path(&slug);

    let data_storage = Arc::clone(data_storage);
    let template = match data_storage
        .data
        .read()
        .unwrap()
        .templates
        .get(&template_id)
    {
        Some(template) => template.clone(),
        None => {
            eprintln!("Couldn't find template");
            return Err(ApiErrorType::ResourceNotFound("template".to_string()).into());
        }
    };

    match template.write().unwrap().export_formats.get_mut(&slug) {
        Some(export_format) => {
            export_format
                .export_steps
                .retain(|step| step.id != Some(step_id));
        }
        None => {
            eprintln!("Couldn't find export format");
            return Err(ApiErrorType::ResourceNotFound("export_format".to_string()).into());
        }
    };
    if let Err(()) = data_storage.update_template_version_id(template_id).await {
        return Err(ApiErrorType::ResourceNotFound("template".to_string()).into());
    }
    return Ok(().into());
}

#[get("/api/templates/<template_id>/export_formats/<slug>/export_steps")]
pub async fn get_export_steps(
    _session: Session,
    template_id: String,
    slug: String,
    data_storage: &State<Arc<DataStorage>>,
) -> APIResult<Vec<ExportStep>> {
    //Parse template_id to uuid
    let template_id = match Uuid::parse_str(&template_id) {
        Ok(template_id) => template_id,
        Err(e) => {
            eprintln!("Couldn't parse template id: {}", e);
            return Err(ApiErrorType::UnparsableParameter("uuid".to_string()).into());
        }
    };

    let slug = sanitize_path(&slug);

    let data_storage = data_storage;
    let template = match data_storage
        .data
        .read()
        .unwrap()
        .templates
        .get(&template_id)
    {
        Some(template) => template.clone(),
        None => {
            eprintln!("Couldn't find template");
            return Err(ApiErrorType::ResourceNotFound("template".to_string()).into());
        }
    };

    let export_steps = match template.read().unwrap().export_formats.get(&slug) {
        Some(export_format) => export_format.export_steps.clone(),
        None => {
            eprintln!("Couldn't find export format");
            return Err(ApiErrorType::ResourceNotFound("export_format".to_string()).into());
        }
    };
    if let Err(()) = data_storage.update_template_version_id(template_id).await {
        return Err(ApiErrorType::ResourceNotFound("template".to_string()).into());
    }
    return Ok(export_steps.into());
}

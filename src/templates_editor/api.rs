use crate::db::repositories::templates::{self, Template};
use crate::permissions::AuthorizedTemplate;
use crate::storage::ProjectTemplateV2;
use crate::utils::api_helpers::{APIResult, ApiError, ApiErrorType};
use rocket::State;
use rocket::form::Form;
use rocket::fs::{NamedFile, TempFile};
use rocket::http::Status;
use rocket::serde::json::Json;
use sqlx::PgPool;
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::path::{Component, Path};
use uuid::Uuid;
use vb_exchange::export_formats::{ExportFormat, ExportStep};

/// Contains API endpoints for the templates editor.

/// GET /api/templates/<_>
/// Get a template by its id.
#[get("/api/templates/<_>")]
pub async fn get_template(auth: AuthorizedTemplate, pool: &State<PgPool>) -> APIResult<Template> {
    Ok(templates::get(pool.inner(), auth.id).await?.into())
}

/// Maps a failed [`AuthorizedTemplate`] guard for routes that serve files (and so can't return
/// an [`ApiError`]): unknown/inaccessible templates and unparsable ids are 404, anything else 500.
fn file_route_status(error: ApiError) -> Status {
    match error.error {
        ApiErrorType::ResourceNotFound(_) | ApiErrorType::UnparsableParameter(_) => {
            Status::NotFound
        }
        _ => Status::InternalServerError,
    }
}

/// Canonicalizes one of a template's directories. A template has no files on disk until
/// something is first written, so a missing directory is `ResourceNotFound`, not a panic.
fn template_dir(path: &str) -> Result<PathBuf, ApiError> {
    Path::new(path).canonicalize().map_err(|e| match e.kind() {
        io::ErrorKind::NotFound => ApiErrorType::ResourceNotFound("asset".to_string()).into(),
        _ => ApiError::from(e),
    })
}

/// POST /api/templates/<_>
/// Update a template by its id.
/// The template id in the url must match the id in the body.
/// Can't be used to create a new template.
#[post("/api/templates/<_>", data = "<template>")]
pub async fn update_template(
    auth: AuthorizedTemplate,
    template: Json<ProjectTemplateV2>,
    pool: &State<PgPool>,
) -> APIResult<()> {
    let template_id = auth.id;

    let mut template = template.into_inner();

    // Check if id in template matches id in url
    if template_id != template.id {
        return Err(ApiErrorType::Other(
            "Template id in url does not match template id in body, id change is not supported."
                .to_string(),
        )
        .into());
    }

    // Formats saved before slugs were validated may have unusable ones: derive valid slugs for
    // them instead of making the whole template unsaveable.
    let renamed = normalize_export_format_slugs(&mut template.export_formats);

    templates::replace(
        pool.inner(),
        template_id,
        &template.name,
        &template.description,
        &template.export_formats,
    )
    .await?;

    for (old_slug, new_slug) in renamed {
        move_legacy_format_dir(template_id, &old_slug, &new_slug).await;
    }

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

/// Reduces a user-supplied path to `/`-separated plain components of allowed characters
/// (alphanumerics, `_`, `-`, `.`). Empty, `.` and `..` components and a leading `/` are
/// dropped, so the result can never be absolute or climb out of the directory it's joined to.
pub fn sanitize_path(path: &str) -> String {
    let allowed_chars = |c: &char| c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '/');
    let path: String = path.chars().filter(allowed_chars).collect();
    path.split('/')
        .filter(|component| !matches!(*component, "" | "." | ".."))
        .collect::<Vec<_>>()
        .join("/")
}

/// Validates the `<slug>` of an export format route before it is used as a directory name or
/// DB key: it must be a single plain path component, see [`is_valid_slug`]. Slugs are never
/// rewritten, an invalid one is a `BadRequest`.
fn checked_slug(slug: &str) -> Result<String, ApiError> {
    if is_valid_slug(slug) {
        Ok(slug.to_string())
    } else {
        Err(ApiErrorType::BadRequest("Invalid Slug".to_string()).into())
    }
}

/// An export format slug names a single directory and is used verbatim as the DB key, so it
/// must be a single normal path component (not `.` or `..`) that survives [`sanitize_path`]
/// unchanged.
fn is_valid_slug(slug: &str) -> bool {
    !slug.is_empty() && !slug.contains('/') && sanitize_path(slug) == slug
}

/// Derives a valid slug from an invalid `slug`: the part of it [`sanitize_path`] keeps (the
/// directory the format's files were stored in so far), else one made from the format's `name`,
/// else `"format"`.
fn generate_slug(slug: &str, name: &str) -> String {
    let from_name = name.trim().replace(char::is_whitespace, "-");
    [slug, &from_name]
        .into_iter()
        .map(|s| sanitize_path(s).replace('/', "-"))
        .find(|candidate| is_valid_slug(candidate))
        .unwrap_or_else(|| "format".to_string())
}

/// Replaces every invalid slug in `formats` by a generated one that is unique among the
/// formats (valid slugs are never touched) and re-keys the map accordingly. Returns the
/// `(old, new)` slug pairs that changed.
fn normalize_export_format_slugs(
    formats: &mut HashMap<String, ExportFormat>,
) -> Vec<(String, String)> {
    let (valid, mut invalid): (Vec<_>, Vec<_>) = std::mem::take(formats)
        .into_values()
        .partition(|format| is_valid_slug(&format.slug));
    formats.extend(
        valid
            .into_iter()
            .map(|format| (format.slug.clone(), format)),
    );
    invalid.sort_by(|a, b| a.slug.cmp(&b.slug));

    let mut renamed = Vec::new();
    for mut format in invalid {
        let old_slug = format.slug.clone();
        let base = generate_slug(&old_slug, &format.name);
        let mut slug = base.clone();
        let mut counter = 2;
        while formats.contains_key(&slug) {
            slug = format!("{base}-{counter}");
            counter += 1;
        }
        format.slug = slug.clone();
        formats.insert(slug.clone(), format);
        renamed.push((old_slug, slug));
    }
    renamed
}

/// Moves the files of a format whose invalid slug was replaced to the directory of its new
/// slug. Best effort: the old directory is where [`sanitize_path`] put the files, which is
/// usually already the new one. Never overwrites an existing directory.
async fn move_legacy_format_dir(template_id: Uuid, old_slug: &str, new_slug: &str) {
    let base = format!("data/templates/{template_id}/formats");
    let (Ok(old_path), Ok(new_path)) = (
        safe_path_combine(&base, old_slug),
        safe_path_combine(&base, new_slug),
    ) else {
        return;
    };
    if old_path == new_path
        || !tokio::fs::try_exists(&old_path).await.unwrap_or(false)
        || tokio::fs::try_exists(&new_path).await.unwrap_or(true)
    {
        return;
    }
    if let Err(e) = tokio::fs::rename(&old_path, &new_path).await {
        error!(
            "Couldn't move files of export format {old_slug:?} to {new_slug:?} in template {template_id}: {e}"
        );
    }
}

/// Safely combines a base path with a user input path.
pub fn safe_path_combine(base_path: &str, user_input: &str) -> Result<PathBuf, ()> {
    let sanitized_input = sanitize_path(user_input);
    if sanitized_input.is_empty() {
        return Err(());
    }
    // `starts_with` compares components without resolving `..`, so only plain components are
    // allowed (sanitize_path already drops the others, this keeps the invariant local).
    if !Path::new(&sanitized_input)
        .components()
        .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
    {
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
    fn test_is_valid_slug() {
        assert!(is_valid_slug("pdf-print_v2.1"));
        assert!(!is_valid_slug(""));
        assert!(!is_valid_slug("a/b"));
        assert!(!is_valid_slug("../../x"));
        assert!(!is_valid_slug("a b"));
        assert!(!is_valid_slug("."));
        assert!(!is_valid_slug(".."));
    }

    fn format(slug: &str, name: &str) -> ExportFormat {
        ExportFormat {
            slug: slug.to_string(),
            name: name.to_string(),
            export_steps: vec![],
            output_files: vec![],
            preview_pdf_path: None,
        }
    }

    #[test]
    fn test_generate_slug() {
        assert_eq!(generate_slug("print pdf", "Print"), "printpdf");
        assert_eq!(generate_slug("a/b", "x"), "a-b");
        assert_eq!(generate_slug("..", "My Format"), "My-Format");
        assert_eq!(generate_slug("///", "!!!"), "format");
    }

    #[test]
    fn test_normalize_export_format_slugs() {
        let mut formats = HashMap::new();
        for f in [
            format("pdf", "Pdf"),
            format("p df", "A"),
            format("pdf ", "B"),
            format("", "Epub"),
        ] {
            formats.insert(f.slug.clone(), f);
        }

        let renamed = normalize_export_format_slugs(&mut formats);

        assert_eq!(formats.len(), 4);
        assert!(
            formats
                .iter()
                .all(|(k, f)| *k == f.slug && is_valid_slug(k))
        );
        assert_eq!(formats["pdf"].name, "Pdf");
        assert_eq!(renamed.len(), 3);
        assert!(formats.contains_key("Epub"));
    }

    #[test]
    fn test_sanitize_path_never_leaves_parent_dir_components() {
        for evil in [
            "..",
            "a/..",
            "x/../..",
            "....//",
            ".....///..",
            ".....///.....///.....///..",
            "/../etc/passwd",
            "a/./../b",
        ] {
            let sanitized = sanitize_path(evil);
            assert!(
                Path::new(&sanitized)
                    .components()
                    .all(|c| matches!(c, Component::Normal(_))),
                "{evil:?} -> {sanitized:?}"
            );
        }
        assert_eq!(sanitize_path(".....///.."), ".....");
        assert_eq!(sanitize_path("/a/./b//c"), "a/b/c");
    }

    #[test]
    fn test_checked_slug_rejects_rewritable_slugs() {
        assert_eq!(checked_slug("pdf-print").unwrap(), "pdf-print");
        for evil in [".....///..", "../..", "a/b", "..", ".", ""] {
            assert!(checked_slug(evil).is_err(), "{evil:?}");
        }
    }

    #[test]
    fn test_safe_path_combine_stays_inside_base() {
        let base_path = "/data/templates/template1/assets";
        assert_eq!(safe_path_combine(base_path, ".."), Err(()));
        for evil in ["a/..", "x/../..", ".....///..///other/assets/x"] {
            let path = safe_path_combine(base_path, evil).unwrap();
            assert!(path.starts_with(base_path), "{evil:?} -> {path:?}");
            assert!(path.components().all(|c| c != Component::ParentDir));
        }
    }

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

/// POST /api/templates/<_>/assets/file
/// Creates a new asset in the global assets folder of the template
#[post("/api/templates/<_>/assets/file", data = "<asset>")]
pub async fn create_file_asset(
    auth: AuthorizedTemplate,
    asset: Form<NewAssetFile<'_>>,
    pool: &State<PgPool>,
) -> APIResult<()> {
    let template_id = auth.id;

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
            templates::touch_version(pool.inner(), template_id).await?;
            Ok(().into())
        }
        Err(e) => {
            eprintln!("Error copying file: {}", e);
            Err(ApiErrorType::InternalServerError.into())
        }
    }
}

#[derive(serde::Deserialize, serde::Serialize)]
pub struct DeleteAssetRequest {
    pub paths: Vec<String>,
}

/// DELETE /api/templates/<_>/assets/<path>
/// Deletes an asset in the global assets folder of the template
#[delete("/api/templates/<_>/assets", data = "<paths>")]
pub async fn delete_assets(
    auth: AuthorizedTemplate,
    paths: Json<DeleteAssetRequest>,
    pool: &State<PgPool>,
) -> APIResult<()> {
    let template_id = auth.id;

    let base_path_raw = template_dir(&format!("data/templates/{}/assets", template_id))?;
    let base_path = base_path_raw.to_str().unwrap();

    for path in &paths.paths {
        let path = match safe_path_combine(base_path, path) {
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

    templates::touch_version(pool.inner(), template_id).await?;

    Ok(().into())
}

/// POST /api/templates/<_>/assets/folder
/// Creates a new asset in the global assets folder of the template
#[post("/api/templates/<_>/assets/folder", data = "<asset>")]
pub async fn create_folder_asset(
    auth: AuthorizedTemplate,
    asset: Json<NewAssetFolder>,
    pool: &State<PgPool>,
) -> APIResult<()> {
    let template_id = auth.id;

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
            templates::touch_version(pool.inner(), template_id).await?;
            res
        }
        Err(e) => {
            eprintln!("Error creating folder: {}", e);
            Err(ApiErrorType::InternalServerError.into())
        }
    }
}

/// GET /api/templates/<_>/assets
/// List all global assets saved for the template
#[get("/api/templates/<_>/assets")]
pub async fn get_assets(auth: AuthorizedTemplate) -> APIResult<AssetList> {
    let template_id = auth.id;

    let global_asset_path = PathBuf::from(format!("data/templates/{}/assets", template_id));

    if !tokio::fs::try_exists(&global_asset_path).await? {
        tokio::fs::create_dir_all(&global_asset_path).await?;
    }

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
            Some(path) => format!("{}/{}", path, entry.file_name().to_string_lossy()),
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

/// GET /api/templates/<_>/assets/files/<path>
/// Get an specific File asset in the global assets folder of the template
#[get("/api/templates/<_>/assets/files/<path..>")]
pub async fn get_asset_file(
    auth: Result<AuthorizedTemplate, ApiError>,
    path: PathBuf,
) -> Result<NamedFile, Status> {
    let template_id = auth.map_err(file_route_status)?.id;

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

/// PUT /api/templates/<_>/assets/files/<path>
/// Updates a text-based asset in the global assets folder of the template
/// The asset must be a text-based file, e.g. .txt, .html, .css, .js
#[put("/api/templates/<_>/assets/files/<path..>", data = "<content>")]
pub async fn update_asset_file(
    auth: AuthorizedTemplate,
    path: PathBuf,
    content: Json<UpdateAssetRequest>,
    pool: &State<PgPool>,
) -> APIResult<()> {
    let template_id = auth.id;

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

    templates::touch_version(pool.inner(), template_id).await?;

    // Update the file
    match tokio::fs::write(&path, content.into_inner().content).await {
        Ok(_) => Ok(().into()),
        Err(e) => {
            eprintln!("Error updating asset: {}", e);
            Err(ApiErrorType::InternalServerError.into())
        }
    }
}

/// POST /api/templates/<_>/assets/move
/// Moves an asset in the global assets folder of the template
#[post("/api/templates/<_>/assets/move", data = "<asset>")]
pub async fn move_asset(
    auth: AuthorizedTemplate,
    asset: Json<MoveAssetRequest>,
    pool: &State<PgPool>,
) -> APIResult<()> {
    let template_id = auth.id;
    let base_path = template_dir(&format!("data/templates/{}/assets", template_id))?;

    let old_path = match safe_path_combine(base_path.to_str().unwrap(), &asset.old_path) {
        Ok(path) => path,
        Err(_) => {
            eprintln!("Error moving asset, invalid path.");
            return Err(ApiErrorType::BadRequest("Invalid path".to_string()).into());
        }
    };

    let new_path = match safe_path_combine(base_path.to_str().unwrap(), &asset.new_path) {
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

    templates::touch_version(pool.inner(), template_id).await?;

    Ok(().into())
}

/// POST /api/templates/<_>/export_formats
/// Adds a new export format to the template and creates its asset directory on disk.
#[post("/api/templates/<_>/export_formats", data = "<data>")]
pub async fn add_export_format(
    auth: AuthorizedTemplate,
    pool: &State<PgPool>,
    data: Json<ExportFormat>,
) -> APIResult<ExportFormat> {
    let template_id = auth.id;

    // Get the format to be added
    let format = data.into_inner();
    if !is_valid_slug(&format.slug) {
        return Err(ApiErrorType::BadRequest("Invalid Slug".to_string()).into());
    }

    // Add folder in file system
    let base_path = format!("data/templates/{}/formats", template_id);
    let base_path = Path::new(&base_path);

    let new_path = match safe_path_combine(base_path.to_str().unwrap(), &format.slug) {
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

    templates::insert_export_format(pool.inner(), template_id, &format).await?;

    match tokio::fs::create_dir_all(new_path).await {
        Ok(_) => {
            templates::touch_version(pool.inner(), template_id).await?;
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

/// UPDATE /api/templates/<_>/export_formats/<slug>
/// Updates the export_format metadata
#[post("/api/templates/<_>/export_formats/<slug>/metadata", data = "<data>")]
pub async fn update_export_format_metadata(
    auth: AuthorizedTemplate,
    slug: String,
    pool: &State<PgPool>,
    data: Json<ExportFormatMetadata>,
) -> APIResult<()> {
    let template_id = auth.id;
    let old_slug = checked_slug(&slug)?;
    let new_slug = checked_slug(&data.slug)?;

    // An export format without files has no directory to move. The files are moved first so a
    // failing DB update (unknown format, slug conflict) can undo it, leaving both unchanged.
    let mut moved = None;
    if old_slug != new_slug {
        // not canonicalized: the formats directory doesn't exist yet if no format has files
        let base_path = format!("data/templates/{}/formats", template_id);
        let (Ok(old_path), Ok(new_path)) = (
            safe_path_combine(&base_path, &old_slug),
            safe_path_combine(&base_path, &new_slug),
        ) else {
            return Err(ApiErrorType::BadRequest("Invalid Slug".to_string()).into());
        };

        if !templates::export_format_exists(pool.inner(), template_id, &old_slug).await? {
            return Err(ApiErrorType::ResourceNotFound("export_format".to_string()).into());
        }

        if new_path.exists() {
            return Err(ApiErrorType::Conflict(
                "An export format with this slug already exists.".to_string(),
            )
            .into());
        }

        if tokio::fs::try_exists(&old_path).await.unwrap_or(false) {
            if let Err(e) = tokio::fs::rename(&old_path, &new_path).await {
                eprintln!("Couldn't rename export format: {}", e);
                return Err(ApiErrorType::InternalServerError.into());
            }
            moved = Some((old_path, new_path));
        }
    }

    if let Err(e) = templates::update_export_format_metadata(
        pool.inner(),
        template_id,
        &old_slug,
        &new_slug,
        &data.name,
        data.preview_pdf_path.as_deref(),
    )
    .await
    {
        if let Some((old_path, new_path)) = moved
            && let Err(e) = tokio::fs::rename(&new_path, &old_path).await
        {
            error!("Couldn't undo the move of export format {old_slug:?} to {new_slug:?}: {e}");
        }
        return Err(e.into());
    }

    Ok(().into())
}

/// DELETE /api/templates/<_>/export_formats/<slug>
/// Deletes export format with slug <slug> in the template
#[delete("/api/templates/<_>/export_formats/<slug>")]
pub async fn delete_export_format(
    auth: AuthorizedTemplate,
    pool: &State<PgPool>,
    slug: String,
) -> APIResult<()> {
    let template_id = auth.id;
    let slug = checked_slug(&slug)?;

    templates::delete_export_format(pool.inner(), template_id, &slug).await?;

    //Remove folder:
    let base_path = format!("data/templates/{}/formats/", template_id);
    let safe_path = safe_path_combine(base_path.as_str(), &slug);
    match safe_path {
        Ok(path) => match tokio::fs::remove_dir_all(path).await {
            Ok(_) => {
                templates::touch_version(pool.inner(), template_id).await?;
                Ok(().into())
            }
            Err(e) => {
                eprintln!("Couldn't delete physical folder for export format: {}", e);
                Err(ApiErrorType::InternalServerError.into())
            }
        },
        Err(_) => {
            eprintln!(
                "Couldn't delete physical folder for export format. Couldn't create safe_path"
            );
            Err(ApiErrorType::BadRequest("Invalid Slug".to_string()).into())
        }
    }
}

/// GET /api/templates/<_>/export_formats/<slug>/assets
/// List all assets of the export_format
#[get("/api/templates/<_>/export_formats/<slug>/assets")]
pub async fn get_assets_for_export_format(
    auth: AuthorizedTemplate,
    slug: String,
) -> APIResult<AssetList> {
    let template_id = auth.id;
    let slug = checked_slug(&slug)?;

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

/// GET /api/templates/<_>/export_formats/<slug>/assets/files/<path>
/// Get an specific File asset in the folder of the export format
#[get("/api/templates/<_>/export_formats/<slug>/assets/files/<path..>")]
pub async fn get_asset_file_for_export_format(
    auth: Result<AuthorizedTemplate, ApiError>,
    path: PathBuf,
    slug: String,
) -> Result<NamedFile, Status> {
    let template_id = auth.map_err(file_route_status)?.id;
    let slug = checked_slug(&slug).map_err(|_| Status::NotFound)?;

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

/// POST /api/templates/<_>/export_formats/<slug>/assets/file
/// Creates a new asset in the export format folder
#[post(
    "/api/templates/<_>/export_formats/<slug>/assets/file",
    data = "<asset>"
)]
pub async fn create_file_asset_for_export_format(
    auth: AuthorizedTemplate,
    slug: String,
    asset: Form<NewAssetFile<'_>>,
    pool: &State<PgPool>,
) -> APIResult<()> {
    let template_id = auth.id;

    let slug = checked_slug(&slug)?;

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
            templates::touch_version(pool.inner(), template_id).await?;
            Ok(().into())
        }
        Err(e) => {
            eprintln!("Error copying file: {}", e);
            Err(ApiErrorType::InternalServerError.into())
        }
    }
}

/// DELETE /api/templates/<_>/export_formats/<slug>/assets
/// Deletes an asset in the export format folder of the template
#[delete("/api/templates/<_>/export_formats/<slug>/assets", data = "<paths>")]
pub async fn delete_assets_for_export_format(
    auth: AuthorizedTemplate,
    paths: Json<DeleteAssetRequest>,
    slug: String,
    pool: &State<PgPool>,
) -> APIResult<()> {
    let template_id = auth.id;

    let slug = checked_slug(&slug)?;

    let base_path_raw = template_dir(&format!("data/templates/{}/formats/{}", template_id, slug))?;
    let base_path = base_path_raw.to_str().unwrap();

    for path in &paths.paths {
        let path = match safe_path_combine(base_path, path) {
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
    templates::touch_version(pool.inner(), template_id).await?;

    Ok(().into())
}

/// POST /api/templates/<_>/export_formats/<slug>/assets/folder
/// Creates a new asset in the export format folder of the template
#[post(
    "/api/templates/<_>/export_formats/<slug>/assets/folder",
    data = "<asset>"
)]
pub async fn create_folder_asset_for_export_format(
    auth: AuthorizedTemplate,
    asset: Json<NewAssetFolder>,
    slug: String,
    pool: &State<PgPool>,
) -> APIResult<()> {
    let template_id = auth.id;

    let slug = checked_slug(&slug)?;

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
            templates::touch_version(pool.inner(), template_id).await?;
            res
        }
        Err(e) => {
            eprintln!("Error creating folder: {}", e);
            Err(ApiErrorType::InternalServerError.into())
        }
    }
}

/// PUT /api/templates/<_>/export_formats/<slug>/assets/files/<path>
/// Updates a text-based asset in the export format folder of the template
/// The asset must be a text-based file, e.g. .txt, .html, .css, .js
#[put(
    "/api/templates/<_>/export_formats/<slug>/assets/files/<path..>",
    data = "<content>"
)]
pub async fn update_asset_file_for_export_format(
    auth: AuthorizedTemplate,
    path: PathBuf,
    content: Json<UpdateAssetRequest>,
    slug: String,
    pool: &State<PgPool>,
) -> APIResult<()> {
    let template_id = auth.id;

    let slug = checked_slug(&slug)?;

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
            templates::touch_version(pool.inner(), template_id).await?;
            Ok(().into())
        }
        Err(e) => {
            eprintln!("Error updating asset: {}", e);
            Err(ApiErrorType::InternalServerError.into())
        }
    }
}

/// POST /api/templates/<_>/export_formats/<slug>/assets/move
/// Moves an asset in the export_format folder of the template
#[post(
    "/api/templates/<_>/export_formats/<slug>/assets/move",
    data = "<asset>"
)]
pub async fn move_asset_for_export_format(
    auth: AuthorizedTemplate,
    asset: Json<MoveAssetRequest>,
    slug: String,
    pool: &State<PgPool>,
) -> APIResult<()> {
    let template_id = auth.id;

    let slug = checked_slug(&slug)?;

    let base_path = template_dir(&format!("data/templates/{}/formats/{}", template_id, slug))?;

    let old_path = match safe_path_combine(base_path.to_str().unwrap(), &asset.old_path) {
        Ok(path) => path,
        Err(_) => {
            eprintln!("Error moving asset, invalid path.");
            return Err(ApiErrorType::BadRequest("Invalid path".to_string()).into());
        }
    };

    let new_path = match safe_path_combine(base_path.to_str().unwrap(), &asset.new_path) {
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

    templates::touch_version(pool.inner(), template_id).await?;

    Ok(().into())
}

#[derive(serde::Deserialize)]
pub struct MoveAssetRequest {
    pub overwrite: bool,
    pub old_path: String,
    pub new_path: String,
}

/// POST /api/templates/<_>/export_formats/<slug>/export_steps/
/// Creates new Export Step
#[post(
    "/api/templates/<_>/export_formats/<slug>/export_steps",
    data = "<step>"
)]
pub async fn create_export_step(
    auth: AuthorizedTemplate,
    slug: String,
    step: Json<ExportStep>,
    pool: &State<PgPool>,
) -> APIResult<ExportStep> {
    let template_id = auth.id;

    let slug = checked_slug(&slug)?;

    let mut step = step.into_inner();
    step.id = Some(uuid::Uuid::new_v4());

    let mut steps = templates::get_export_steps(pool.inner(), template_id, &slug).await?;
    steps.push(step.clone());
    templates::update_export_steps(pool.inner(), template_id, &slug, &steps).await?;
    templates::touch_version(pool.inner(), template_id).await?;

    Ok(step.into())
}

#[derive(serde::Deserialize)]
pub struct MoveExportStepRequest {
    /// Element to moved behind. Set to None to move to first position
    pub move_after: Option<uuid::Uuid>,
}

/// POST /api/templates/<_>/export_formats/<slug>/export_steps/<step_id>/move
/// Moves a export step to a specified position
#[post(
    "/api/templates/<_>/export_formats/<slug>/export_steps/<step_id>/move",
    data = "<movedata>"
)]
pub async fn move_export_step(
    auth: AuthorizedTemplate,
    slug: String,
    step_id: String,
    movedata: Json<MoveExportStepRequest>,
    pool: &State<PgPool>,
) -> APIResult<()> {
    let template_id = auth.id;
    //Parse step_id to uuid
    let step_id = match Uuid::parse_str(&step_id) {
        Ok(step_id) => step_id,
        Err(e) => {
            eprintln!("Couldn't parse step id: {}", e);
            return Err(ApiErrorType::UnparsableParameter("uuid".to_string()).into());
        }
    };

    let slug = checked_slug(&slug)?;
    let move_after = movedata.move_after;

    let mut steps = templates::get_export_steps(pool.inner(), template_id, &slug).await?;

    // Find export step and move it after move_after:
    let step_index = steps.iter().position(|step| step.id == Some(step_id));
    let step_index = match step_index {
        Some(index) => index,
        None => {
            return Err(ApiErrorType::ResourceNotFound("export_step".to_string()).into());
        }
    };
    let step = steps.remove(step_index);
    // Find new position:
    let new_index = match move_after {
        Some(move_after) => match steps.iter().position(|step| step.id == Some(move_after)) {
            None => {
                return Err(ApiErrorType::ResourceNotFound("export_step".to_string()).into());
            }
            Some(index) => index + 1,
        },
        None => 0_usize,
    };
    steps.insert(new_index, step);

    templates::update_export_steps(pool.inner(), template_id, &slug, &steps).await?;
    templates::touch_version(pool.inner(), template_id).await?;
    Ok(().into())
}

/// PUT /api/templates/<_>/export_formats/<slug>/export_steps/<step_id>
/// Updates a export step
#[post(
    "/api/templates/<_>/export_formats/<slug>/export_steps/<step_id>",
    data = "<step>"
)]
pub async fn update_export_step(
    auth: AuthorizedTemplate,
    slug: String,
    step_id: String,
    step: Json<ExportStep>,
    pool: &State<PgPool>,
) -> APIResult<()> {
    let template_id = auth.id;

    let slug = checked_slug(&slug)?;

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

    let mut steps = templates::get_export_steps(pool.inner(), template_id, &slug).await?;
    let index = match steps.iter().position(|x| x.id == Some(step_id)) {
        Some(id) => id,
        None => {
            return Err(ApiErrorType::ResourceNotFound("export_step".to_string()).into());
        }
    };
    match steps.get_mut(index) {
        Some(old_step) => *old_step = step,
        None => {
            return Err(ApiErrorType::ResourceNotFound("export_step".to_string()).into());
        }
    }

    templates::update_export_steps(pool.inner(), template_id, &slug, &steps).await?;
    templates::touch_version(pool.inner(), template_id).await?;
    Ok(().into())
}

/// DELETE /api/templates/<_>/export_formats/<slug>/export_steps/<step_id>
/// Removes an export step from an export format's step list.
#[delete("/api/templates/<_>/export_formats/<slug>/export_steps/<step_id>")]
pub async fn delete_export_step(
    auth: AuthorizedTemplate,
    slug: String,
    step_id: String,
    pool: &State<PgPool>,
) -> APIResult<()> {
    //Parse template_id and step_id to uuid
    let template_id = auth.id;

    let step_id = match Uuid::parse_str(&step_id) {
        Ok(step_id) => step_id,
        Err(e) => {
            eprintln!("Couldn't parse step id: {}", e);
            return Err(ApiErrorType::UnparsableParameter("uuid".to_string()).into());
        }
    };

    let slug = checked_slug(&slug)?;

    let mut steps = templates::get_export_steps(pool.inner(), template_id, &slug).await?;
    steps.retain(|step| step.id != Some(step_id));
    templates::update_export_steps(pool.inner(), template_id, &slug, &steps).await?;
    templates::touch_version(pool.inner(), template_id).await?;
    Ok(().into())
}

/// GET /api/templates/<_>/export_formats/<slug>/export_steps
/// Returns the ordered list of export steps for the given export format.
#[get("/api/templates/<_>/export_formats/<slug>/export_steps")]
pub async fn get_export_steps(
    auth: AuthorizedTemplate,
    slug: String,
    pool: &State<PgPool>,
) -> APIResult<Vec<ExportStep>> {
    let template_id = auth.id;

    let slug = checked_slug(&slug)?;

    let export_steps = templates::get_export_steps(pool.inner(), template_id, &slug).await?;
    Ok(export_steps.into())
}

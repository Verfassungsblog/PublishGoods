use crate::db::repositories::templates::{self, Template};
use crate::permissions::AuthorizedTemplate;
use crate::settings::Settings;
use crate::storage::ProjectTemplateV2;
use crate::templates_editor::git;
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
fn template_dir(path: &Path) -> Result<PathBuf, ApiError> {
    path.canonicalize().map_err(|e| match e.kind() {
        io::ErrorKind::NotFound => ApiErrorType::ResourceNotFound("asset".to_string()).into(),
        _ => ApiError::from(e),
    })
}

/// The global assets of a template.
fn assets_dir(settings: &Settings, template_id: Uuid) -> PathBuf {
    settings.template_dir(template_id).join("assets")
}

/// The directory all export formats of a template keep their files in.
fn formats_dir(settings: &Settings, template_id: Uuid) -> PathBuf {
    settings.template_dir(template_id).join("formats")
}

/// The files of one export format. `slug` must be valid, see [`checked_slug`].
fn format_dir(settings: &Settings, template_id: Uuid, slug: &str) -> PathBuf {
    formats_dir(settings, template_id).join(slug)
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
    settings: &State<Settings>,
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

    let lock = git::lock(template_id).await;

    templates::replace(
        pool.inner(),
        template_id,
        &template.name,
        &template.description,
        &template.export_formats,
    )
    .await?;

    for (old_slug, new_slug) in renamed {
        move_legacy_format_dir(settings, template_id, &old_slug, &new_slug).await;
    }

    git::commit(pool.inner(), settings, template_id, lock, "Update template").await;

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
/// unchanged. Nor may it be a `.git` name, whose files are never versioned, see
/// [`git::is_git_dir_name`].
fn is_valid_slug(slug: &str) -> bool {
    !slug.is_empty()
        && !slug.contains('/')
        && sanitize_path(slug) == slug
        && !git::is_git_dir_name(slug)
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
async fn move_legacy_format_dir(
    settings: &Settings,
    template_id: Uuid,
    old_slug: &str,
    new_slug: &str,
) {
    let base = formats_dir(settings, template_id);
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
pub fn safe_path_combine(base_path: impl AsRef<Path>, user_input: &str) -> Result<PathBuf, ()> {
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

    let base = base_path.as_ref();
    let full_path = base.join(sanitized_input);

    // Sicherstellen, dass der resultierende Pfad im Basisverzeichnis bleibt
    if !full_path.starts_with(base) {
        return Err(());
    }

    Ok(full_path)
}

/// Whether a sanitized relative path has a `.git` component, see [`git::is_git_dir_name`].
fn has_git_component(sanitized_path: &str) -> bool {
    sanitized_path.split('/').any(git::is_git_dir_name)
}

/// Like [`safe_path_combine`], for paths a template file is written to (created, edited or
/// moved to): rejects paths with a `.git` component, see [`git::is_git_dir_name`]. Such files
/// would silently be left out of the template's history. Reading, deleting and moving away
/// existing ones stays possible.
fn safe_new_path_combine(base_path: impl AsRef<Path>, user_input: &str) -> Result<PathBuf, ()> {
    // Check what is actually used: sanitizing can turn `.g!it` into `.git`.
    if has_git_component(&sanitize_path(user_input)) {
        return Err(());
    }
    safe_path_combine(base_path, user_input)
}

/// Records a change of the template: new version, and a commit of the current state to its git
/// repository (a failing commit is only logged, see [`git::commit`]). The commit is made even if
/// the version couldn't be bumped, so the repository doesn't miss a change that is on disk; the
/// error is returned afterwards. `lock` is the template's lock, taken before the change was made.
async fn record_change(
    pool: &PgPool,
    settings: &Settings,
    template_id: Uuid,
    lock: git::TemplateLock,
    message: &str,
) -> Result<(), ApiError> {
    let touched = templates::touch_version(pool, template_id).await;
    git::commit(pool, settings, template_id, lock, message).await;
    touched?;
    Ok(())
}

/// `Ok(name)` unless `name` (sanitized user input for a new file or folder) is empty or has a
/// `.git` component, see [`git::is_git_dir_name`].
fn checked_new_name(name: String) -> Result<String, ApiError> {
    if name.is_empty() || has_git_component(&name) {
        return Err(ApiErrorType::BadRequest("Invalid or reserved name".to_string()).into());
    }
    Ok(name)
}

/// The name `filename` is saved under on its `attempt`-th try (0 is the name itself): a number
/// goes in front of the extension, `up.css` becomes `up._1.css`.
fn numbered_name(filename: &str, attempt: u32) -> String {
    if attempt == 0 {
        return filename.to_string();
    }
    match filename.rsplit_once('.') {
        Some((stem, extension)) => format!("{stem}._{attempt}.{extension}"),
        None => format!("{filename}_{attempt}"),
    }
}

/// The name an upload with the client-supplied file name `raw` is saved under: its last path
/// component (some browsers send `C:\Users\x\style.css`), sanitized. Rocket's sanitized
/// `FileName::as_str` would strip the extension, which assets are referenced by. Empty if
/// nothing usable is left, see [`checked_new_name`].
fn upload_file_name(raw: &str) -> String {
    let last = raw.rsplit(['/', '\\']).next().unwrap_or_default();
    sanitize_path(last)
}

/// Saves an uploaded file into the directory `prefix` of the template, under a numbered name if
/// the file name is taken. `no_name` is the error for an upload without a file name.
async fn save_upload(
    pool: &PgPool,
    settings: &Settings,
    template_id: Uuid,
    prefix: &str,
    mut file: TempFile<'_>,
    no_name: ApiErrorType,
) -> APIResult<()> {
    let dir = settings.template_dir(template_id).join(prefix);
    let filename = match file.raw_name() {
        Some(name) => name,
        None => {
            eprintln!("No file name provided");
            return Err(no_name.into());
        }
    };
    let filename = checked_new_name(upload_file_name(
        filename.dangerous_unsafe_unsanitized_raw().as_str(),
    ))?;

    // Before looking for a free name, or another upload could take it meanwhile.
    let lock = git::lock(template_id).await;
    let (saved_as, path) = (0..)
        .map(|attempt| numbered_name(&filename, attempt))
        .map(|name| (format!("{prefix}/{name}"), dir.join(name)))
        .find(|(_, path)| !path.exists())
        .expect("an unbounded range");
    match file.copy_to(&path).await {
        Ok(_) => {
            record_change(
                pool,
                settings,
                template_id,
                lock,
                &format!("Add asset {saved_as}"),
            )
            .await?;
            Ok(().into())
        }
        Err(e) => {
            eprintln!("Error copying file: {}", e);
            // The name was free under the lock, so whatever is there is the partial copy.
            // Left behind, the next commit would record it and the renderer would use it.
            if let Err(e) = tokio::fs::remove_file(&path).await
                && e.kind() != io::ErrorKind::NotFound
            {
                eprintln!("Error removing partial upload {}: {}", path.display(), e);
            }
            Err(ApiErrorType::InternalServerError.into())
        }
    }
}

/// Creates the folder `name` (user input) in the directory `prefix` of the template. `exists` is
/// the error for a folder that already exists.
async fn create_folder(
    pool: &PgPool,
    settings: &Settings,
    template_id: Uuid,
    prefix: &str,
    name: &str,
    exists: ApiErrorType,
) -> APIResult<()> {
    let name = checked_new_name(sanitize_path(name))?;
    let dir = settings.template_dir(template_id).join(prefix);
    let lock = git::lock(template_id).await;

    if let Err(e) = tokio::fs::create_dir(dir.join(&name)).await {
        return Err(match e.kind() {
            io::ErrorKind::AlreadyExists => exists.into(),
            _ => {
                eprintln!("Error creating folder: {}", e);
                ApiErrorType::InternalServerError.into()
            }
        });
    }
    // Only a created folder is a change. git doesn't track empty folders, so it usually
    // results in no commit.
    record_change(
        pool,
        settings,
        template_id,
        lock,
        &format!("Add folder {prefix}/{name}"),
    )
    .await?;
    Ok(().into())
}

/// Replaces the content of the existing file `path` (user input) in the directory `prefix` of
/// the template.
async fn update_file(
    pool: &PgPool,
    settings: &Settings,
    template_id: Uuid,
    prefix: &str,
    path: &Path,
    content: String,
) -> APIResult<()> {
    let dir = settings.template_dir(template_id).join(prefix);
    let rel = sanitize_path(&path.to_string_lossy());
    let Ok(full_path) = safe_new_path_combine(&dir, &rel) else {
        eprintln!("Error updating asset, invalid path.");
        return Err(ApiErrorType::BadRequest("Invalid path".to_string()).into());
    };
    let lock = git::lock(template_id).await;

    if !full_path.exists() {
        return Err(ApiErrorType::ResourceNotFound("asset".to_string()).into());
    }
    match tokio::fs::write(&full_path, content).await {
        Ok(_) => {
            record_change(
                pool,
                settings,
                template_id,
                lock,
                &format!("Update asset {prefix}/{rel}"),
            )
            .await?;
            Ok(().into())
        }
        Err(e) => {
            eprintln!("Error updating asset: {}", e);
            Err(ApiErrorType::InternalServerError.into())
        }
    }
}

/// Moves a file or folder inside the directory `prefix` of the template.
async fn move_asset_in(
    pool: &PgPool,
    settings: &Settings,
    template_id: Uuid,
    prefix: &str,
    asset: &MoveAssetRequest,
) -> APIResult<()> {
    let base_path = template_dir(&settings.template_dir(template_id).join(prefix))?;

    let (Ok(old_path), Ok(new_path)) = (
        safe_path_combine(&base_path, &asset.old_path),
        safe_new_path_combine(&base_path, &asset.new_path),
    ) else {
        eprintln!("Error moving asset, invalid path.");
        return Err(ApiErrorType::BadRequest("Invalid path".to_string()).into());
    };
    let lock = git::lock(template_id).await;

    // If not overwriting, ensure target doesn't exist
    if !asset.overwrite && new_path.exists() {
        return Err(ApiErrorType::Conflict("Target path already exists".to_string()).into());
    }

    if let Err(e) = tokio::fs::rename(&old_path, &new_path).await {
        eprintln!("Error moving asset: {}", e);
        return Err(ApiErrorType::InternalServerError.into());
    }

    let message = format!(
        "Move asset {prefix}/{} to {prefix}/{}",
        sanitize_path(&asset.old_path),
        sanitize_path(&asset.new_path)
    );
    record_change(pool, settings, template_id, lock, &message).await?;
    Ok(().into())
}

/// Deletes the files and folders at the user-supplied relative `paths` below the directory
/// `prefix` of the template, stopping at the first failure. What was deleted until then is
/// recorded as a change, if anything was; a request that succeeds always is, even without paths.
/// `invalid_path` is the error for an unusable path.
async fn delete_and_record(
    pool: &PgPool,
    settings: &Settings,
    template_id: Uuid,
    prefix: &str,
    paths: &[String],
    invalid_path: ApiErrorType,
) -> Result<(), ApiError> {
    let base_path = template_dir(&settings.template_dir(template_id).join(prefix))?;
    let lock = git::lock(template_id).await;

    let mut deleted = Vec::new();
    let result = async {
        for path in paths {
            let Ok(full_path) = safe_path_combine(&base_path, path) else {
                eprintln!("Error deleting asset, invalid path.");
                return Err(invalid_path);
            };
            let removed = if full_path.is_dir() {
                tokio::fs::remove_dir_all(&full_path).await
            } else {
                tokio::fs::remove_file(&full_path).await
            };
            if let Err(e) = removed {
                eprintln!("Error deleting asset: {e}");
                return Err(ApiErrorType::InternalServerError);
            }
            deleted.push(format!("{prefix}/{}", sanitize_path(path)));
        }
        Ok(())
    }
    .await;

    if result.is_ok() || !deleted.is_empty() {
        let message = format!("Delete {}", deleted.join(", "));
        let recorded = record_change(pool, settings, template_id, lock, &message).await;
        match (&result, recorded) {
            (Ok(()), Err(e)) => return Err(e),
            // the deletion error is the one to report
            (Err(_), Err(e)) => eprintln!("Error recording a partial deletion: {e:?}"),
            _ => {}
        }
    }
    result.map_err(Into::into)
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
    fn test_new_paths_reject_git_dir_components() {
        let base = "/data/templates/t/assets";
        for reserved in [".git/config", "a/.GIT/b", ".git", "a/.git."] {
            assert_eq!(
                safe_new_path_combine(base, reserved),
                Err(()),
                "{reserved:?}"
            );
            assert!(
                checked_new_name(sanitize_path(reserved)).is_err(),
                "{reserved:?}"
            );
        }
        // other names git treats specially are ordinary template files
        for name in [".gitignore", "a/.gitattributes", ".github/x", "a/b.css"] {
            assert!(safe_new_path_combine(base, name).is_ok(), "{name:?}");
            assert!(checked_new_name(sanitize_path(name)).is_ok(), "{name:?}");
        }
        assert!(checked_new_name(String::new()).is_err());
        // existing files with such names can still be read, moved away and deleted
        assert!(safe_path_combine(base, "a/.git/x").is_ok());
        assert!(!is_valid_slug(".git"));
        assert!(is_valid_slug(".gitignore"));
    }

    #[test]
    fn test_numbered_name() {
        assert_eq!(numbered_name("up.css", 0), "up.css");
        assert_eq!(numbered_name("up.css", 1), "up._1.css");
        assert_eq!(numbered_name("a.b.css", 2), "a.b._2.css");
        assert_eq!(numbered_name("README", 3), "README_3");
    }

    #[test]
    fn test_upload_file_name() {
        assert_eq!(upload_file_name("style.css"), "style.css");
        assert_eq!(upload_file_name("sub/x.css"), "x.css");
        assert_eq!(upload_file_name("../../etc/passwd"), "passwd");
        assert_eq!(upload_file_name(r"C:\Users\x\font.woff2"), "font.woff2");
        assert_eq!(upload_file_name("we ird$.png"), "weird.png");
        // nothing usable left: rejected by `checked_new_name`
        assert_eq!(upload_file_name("dir/"), "");
        assert_eq!(upload_file_name(".."), "");
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
    settings: &State<Settings>,
) -> APIResult<()> {
    let template_id = auth.id;
    save_upload(
        pool.inner(),
        settings,
        template_id,
        "assets",
        asset.into_inner().file,
        ApiErrorType::Other("No file name provided".to_string()),
    )
    .await
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
    settings: &State<Settings>,
) -> APIResult<()> {
    let template_id = auth.id;

    delete_and_record(
        pool.inner(),
        settings,
        template_id,
        "assets",
        &paths.paths,
        ApiErrorType::Other("Invalid path".to_string()),
    )
    .await?;

    Ok(().into())
}

/// POST /api/templates/<_>/assets/folder
/// Creates a new asset in the global assets folder of the template
#[post("/api/templates/<_>/assets/folder", data = "<asset>")]
pub async fn create_folder_asset(
    auth: AuthorizedTemplate,
    asset: Json<NewAssetFolder>,
    pool: &State<PgPool>,
    settings: &State<Settings>,
) -> APIResult<()> {
    let template_id = auth.id;
    create_folder(
        pool.inner(),
        settings,
        template_id,
        "assets",
        &asset.name,
        ApiErrorType::Other("Folder already exists".to_string()),
    )
    .await
}

/// GET /api/templates/<_>/assets
/// List all global assets saved for the template
#[get("/api/templates/<_>/assets")]
pub async fn get_assets(
    auth: AuthorizedTemplate,
    settings: &State<Settings>,
) -> APIResult<AssetList> {
    let template_id = auth.id;

    let global_asset_path = assets_dir(settings, template_id);

    if !tokio::fs::try_exists(&global_asset_path).await? {
        tokio::fs::create_dir_all(&global_asset_path).await?;
    }

    // Get all entries in the global assets folder (via async fs) inside <data_path>/templates/<template_id>/assets
    let res =
        tokio::task::spawn_blocking(
            move || match get_assets_recursive(&global_asset_path, None) {
                Ok(assets) => Ok(AssetList { assets }.into()),
                Err(e) => {
                    eprintln!("Error getting assets: {}", e);
                    Err(ApiErrorType::InternalServerError.into())
                }
            },
        )
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
    current_path: &Path,
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
                assets: get_assets_recursive(&path, Some(&path_to_asset))?,
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
    settings: &State<Settings>,
) -> Result<NamedFile, Status> {
    let template_id = auth.map_err(file_route_status)?.id;

    // Get the path to the global assets folder
    let path = match safe_path_combine(assets_dir(settings, template_id), &path.to_string_lossy()) {
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
    settings: &State<Settings>,
) -> APIResult<()> {
    let template_id = auth.id;
    update_file(
        pool.inner(),
        settings,
        template_id,
        "assets",
        &path,
        content.into_inner().content,
    )
    .await
}

/// POST /api/templates/<_>/assets/move
/// Moves an asset in the global assets folder of the template
#[post("/api/templates/<_>/assets/move", data = "<asset>")]
pub async fn move_asset(
    auth: AuthorizedTemplate,
    asset: Json<MoveAssetRequest>,
    pool: &State<PgPool>,
    settings: &State<Settings>,
) -> APIResult<()> {
    let template_id = auth.id;
    move_asset_in(pool.inner(), settings, template_id, "assets", &asset).await
}

/// POST /api/templates/<_>/export_formats
/// Adds a new export format to the template and creates its asset directory on disk.
#[post("/api/templates/<_>/export_formats", data = "<data>")]
pub async fn add_export_format(
    auth: AuthorizedTemplate,
    pool: &State<PgPool>,
    data: Json<ExportFormat>,
    settings: &State<Settings>,
) -> APIResult<ExportFormat> {
    let template_id = auth.id;

    // Get the format to be added
    let format = data.into_inner();
    if !is_valid_slug(&format.slug) {
        return Err(ApiErrorType::BadRequest("Invalid Slug".to_string()).into());
    }

    // Add folder in file system
    let Ok(new_path) = safe_path_combine(formats_dir(settings, template_id), &format.slug) else {
        eprintln!("Error creating export Format, invalid slug.");
        return Err(ApiErrorType::BadRequest("Invalid Slug".to_string()).into());
    };

    let lock = git::lock(template_id).await;

    if new_path.exists() {
        return Err(ApiErrorType::Conflict(
            "An export format with this slug already exists.".to_string(),
        )
        .into());
    }

    templates::insert_export_format(pool.inner(), template_id, &format).await?;

    if let Err(e) = tokio::fs::create_dir_all(new_path).await {
        error!("Couldn't create folder for new export format: {e}");
        // Not recorded, so don't keep the format either: the template would list one that has
        // no folder, with nothing in its history about it, and adding it again would fail.
        if let Err(e) =
            templates::delete_export_format(pool.inner(), template_id, &format.slug).await
        {
            error!(
                "Couldn't undo adding export format {:?} to template {template_id}: {e:?}",
                format.slug
            );
        }
        return Err(ApiErrorType::InternalServerError.into());
    }
    record_change(
        pool.inner(),
        settings,
        template_id,
        lock,
        &format!("Add export format {}", format.slug),
    )
    .await?;
    Ok(format.into())
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
    settings: &State<Settings>,
) -> APIResult<()> {
    let template_id = auth.id;
    let old_slug = checked_slug(&slug)?;
    let new_slug = checked_slug(&data.slug)?;
    let lock = git::lock(template_id).await;

    // An export format without files has no directory to move. The files are moved first so a
    // failing DB update (unknown format, slug conflict) can undo it, leaving both unchanged.
    let mut moved = None;
    if old_slug != new_slug {
        // not canonicalized: the formats directory doesn't exist yet if no format has files
        let base_path = formats_dir(settings, template_id);
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

    record_change(
        pool.inner(),
        settings,
        template_id,
        lock,
        &format!("Update export format {new_slug}"),
    )
    .await?;

    Ok(().into())
}

/// DELETE /api/templates/<_>/export_formats/<slug>
/// Deletes export format with slug <slug> in the template
#[delete("/api/templates/<_>/export_formats/<slug>")]
pub async fn delete_export_format(
    auth: AuthorizedTemplate,
    pool: &State<PgPool>,
    slug: String,
    settings: &State<Settings>,
) -> APIResult<()> {
    let template_id = auth.id;
    let slug = checked_slug(&slug)?;
    let lock = git::lock(template_id).await;

    if !templates::export_format_exists(pool.inner(), template_id, &slug).await? {
        return Err(ApiErrorType::ResourceNotFound("export_format".to_string()).into());
    }

    // The folder goes first (a format without files has none): if that fails, the format is
    // still there and the request can be repeated. The other way around, the format would be
    // gone while its folder is left behind, and its slug couldn't be used again.
    let Ok(path) = safe_path_combine(formats_dir(settings, template_id), &slug) else {
        eprintln!("Couldn't delete physical folder for export format. Couldn't create safe_path");
        return Err(ApiErrorType::BadRequest("Invalid Slug".to_string()).into());
    };
    match tokio::fs::remove_dir_all(path).await {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => {
            error!("Couldn't delete physical folder for export format {slug:?}: {e}");
            return Err(ApiErrorType::InternalServerError.into());
        }
    }

    templates::delete_export_format(pool.inner(), template_id, &slug).await?;
    record_change(
        pool.inner(),
        settings,
        template_id,
        lock,
        &format!("Delete export format {slug}"),
    )
    .await?;
    Ok(().into())
}

/// GET /api/templates/<_>/export_formats/<slug>/assets
/// List all assets of the export_format
#[get("/api/templates/<_>/export_formats/<slug>/assets")]
pub async fn get_assets_for_export_format(
    auth: AuthorizedTemplate,
    slug: String,
    settings: &State<Settings>,
) -> APIResult<AssetList> {
    let template_id = auth.id;
    let slug = checked_slug(&slug)?;

    let dir = format_dir(settings, template_id, &slug);

    // Get all entries in the assets folder (via async fs) inside <data_path>/templates/<template_id>/assets
    let res = tokio::task::spawn_blocking(move || match get_assets_recursive(&dir, None) {
        Ok(assets) => Ok(AssetList { assets }.into()),
        Err(e) => {
            eprintln!("Error getting assets: {}", e);
            Err(ApiErrorType::InternalServerError.into())
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
    settings: &State<Settings>,
) -> Result<NamedFile, Status> {
    let template_id = auth.map_err(file_route_status)?.id;
    let slug = checked_slug(&slug).map_err(|_| Status::NotFound)?;

    // Get the path to the export format folder
    let path = match safe_path_combine(
        format_dir(settings, template_id, &slug),
        &path.to_string_lossy(),
    ) {
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
    settings: &State<Settings>,
) -> APIResult<()> {
    let template_id = auth.id;
    let slug = checked_slug(&slug)?;
    save_upload(
        pool.inner(),
        settings,
        template_id,
        &format!("formats/{slug}"),
        asset.into_inner().file,
        ApiErrorType::BadRequest("No file name provided".to_string()),
    )
    .await
}

/// DELETE /api/templates/<_>/export_formats/<slug>/assets
/// Deletes an asset in the export format folder of the template
#[delete("/api/templates/<_>/export_formats/<slug>/assets", data = "<paths>")]
pub async fn delete_assets_for_export_format(
    auth: AuthorizedTemplate,
    paths: Json<DeleteAssetRequest>,
    slug: String,
    pool: &State<PgPool>,
    settings: &State<Settings>,
) -> APIResult<()> {
    let template_id = auth.id;
    let slug = checked_slug(&slug)?;

    delete_and_record(
        pool.inner(),
        settings,
        template_id,
        &format!("formats/{slug}"),
        &paths.paths,
        ApiErrorType::BadRequest("Invalid path".to_string()),
    )
    .await?;

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
    settings: &State<Settings>,
) -> APIResult<()> {
    let template_id = auth.id;
    let slug = checked_slug(&slug)?;
    create_folder(
        pool.inner(),
        settings,
        template_id,
        &format!("formats/{slug}"),
        &asset.name,
        ApiErrorType::Conflict("Folder already exists".to_string()),
    )
    .await
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
    settings: &State<Settings>,
) -> APIResult<()> {
    let template_id = auth.id;
    let slug = checked_slug(&slug)?;
    update_file(
        pool.inner(),
        settings,
        template_id,
        &format!("formats/{slug}"),
        &path,
        content.into_inner().content,
    )
    .await
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
    settings: &State<Settings>,
) -> APIResult<()> {
    let template_id = auth.id;
    let slug = checked_slug(&slug)?;
    move_asset_in(
        pool.inner(),
        settings,
        template_id,
        &format!("formats/{slug}"),
        &asset,
    )
    .await
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
    settings: &State<Settings>,
) -> APIResult<ExportStep> {
    let template_id = auth.id;

    let slug = checked_slug(&slug)?;

    let lock = git::lock(template_id).await;

    let mut step = step.into_inner();
    step.id = Some(uuid::Uuid::new_v4());

    let mut steps = templates::get_export_steps(pool.inner(), template_id, &slug).await?;
    steps.push(step.clone());
    templates::update_export_steps(pool.inner(), template_id, &slug, &steps).await?;
    record_change(
        pool.inner(),
        settings,
        template_id,
        lock,
        &format!("Add export step to format {slug}"),
    )
    .await?;

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
    settings: &State<Settings>,
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
    let lock = git::lock(template_id).await;

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
    record_change(
        pool.inner(),
        settings,
        template_id,
        lock,
        &format!("Reorder export steps of format {slug}"),
    )
    .await?;
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
    settings: &State<Settings>,
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

    let lock = git::lock(template_id).await;
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
    record_change(
        pool.inner(),
        settings,
        template_id,
        lock,
        &format!("Update export step of format {slug}"),
    )
    .await?;
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
    settings: &State<Settings>,
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
    let lock = git::lock(template_id).await;

    let mut steps = templates::get_export_steps(pool.inner(), template_id, &slug).await?;
    steps.retain(|step| step.id != Some(step_id));
    templates::update_export_steps(pool.inner(), template_id, &slug, &steps).await?;
    record_change(
        pool.inner(),
        settings,
        template_id,
        lock,
        &format!("Delete export step from format {slug}"),
    )
    .await?;
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

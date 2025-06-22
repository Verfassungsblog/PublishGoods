use rocket::fs::NamedFile;
use rocket::http::Status;
use rocket::State;
use crate::session::session_guard::Session;
use crate::settings::Settings;

/// GET /download/renderings/<id>
///
/// Returns the rendered PDF file with the given `id`.
///
/// Looks for the file at `{data_path}/temp/{id}/output.pdf` as specified in the application settings.
/// A valid session is required to access this endpoint. Returns HTTP 404 Not Found if the file does not exist.
///
/// # Parameters
/// - `id`: The identifier for the rendering to be downloaded.
/// - `settings`: Shared application settings. Used to determine the data storage path.
/// - `_session`: Request guard ensuring a valid session.
#[get("/download/renderings/<id>")]
pub async fn download_rendering(id: String, settings: &State<Settings>, _session: Session) -> Result<NamedFile, Status> {
    let path = format!("{}/temp/{}/output.pdf", settings.data_path, id);
    let file = NamedFile::open(path).await.map_err(|_| Status::NotFound)?;
    Ok(file)
}
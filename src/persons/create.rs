use crate::session::session_guard::Session;
use rocket::http::Status;
use rocket_dyn_templates::Template;

/// Renders the create-person form page.
#[get("/persons/create")]
pub async fn show_create_person(_session: Session) -> Result<Template, Status> {
    Ok(Template::render("create_person", ()))
}

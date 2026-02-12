use crate::session::session_guard::Session;
use crate::storage::data_storage::DataStorage;
use rocket::http::Status;
use rocket::State;
use rocket_dyn_templates::Template;
use std::sync::Arc;

#[get("/persons/create")]
pub async fn show_create_person(
    _session: Session,
    _data_storage: &State<Arc<DataStorage>>,
) -> Result<Template, Status> {
    Ok(Template::render("create_person", ()))
}

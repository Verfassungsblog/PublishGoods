use std::sync::Arc;
use rocket::http::Status;
use rocket::State;
use rocket_dyn_templates::Template;
use crate::session::session_guard::Session;
use crate::settings::Settings;
use crate::storage::project_storage::ProjectStorage;

#[get("/projects/<project_id>")]
pub async fn show_editor(project_id: String, _session: Session, settings: &State<Settings>, project_storage: &State<Arc<ProjectStorage>>) -> Result<Template, Status> {
    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            eprintln!("Couldn't parse project id: {}", e);
            return Err(Status::NotFound);
        },
    };

    let project_storage = Arc::clone(project_storage);

    let _project_entry = match project_storage.get_project(&project_id, settings).await{
        Ok(project_entry) => project_entry.clone(),
        Err(_) => {
            eprintln!("Couldn't get project with id {}", project_id);
            return Err(Status::NotFound);
        },
    };

    Ok(Template::render("editor", project_id))
}
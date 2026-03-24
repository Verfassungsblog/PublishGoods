use crate::session::session_guard::Session;
use crate::settings::Settings;
use crate::storage::data_storage::DataStorage;
use crate::storage::data_storage::current::ProjectListEntry;
use crate::storage::project_storage::ProjectStorage;
use chrono::Utc;
use rocket::State;
use rocket::http::Status;
use rocket_dyn_templates::Template;
use std::sync::Arc;

#[get("/projects/<project_id>")]
pub async fn show_editor(
    project_id: String,
    _session: Session,
    settings: &State<Settings>,
    project_storage: &State<Arc<ProjectStorage>>,
    data_storage: &State<Arc<DataStorage>>,
) -> Result<Template, Status> {
    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            eprintln!("Couldn't parse project id: {}", e);
            return Err(Status::NotFound);
        }
    };

    let project_storage = Arc::clone(project_storage);
    if !project_storage.has_project(&project_id, settings).await {
        eprintln!("Couldn't get project with id {}", project_id);
        return Err(Status::NotFound);
    }

    // Update last interaction time in project list
    {
        let mut project_list = data_storage.data.projects.write().unwrap();
        if let Some(entry) = project_list
            .entries
            .iter_mut()
            .find(|entry| *entry.id() == project_id)
            && let ProjectListEntry::Project(project) = entry
        {
            project.last_interaction = Utc::now().naive_utc();
        }
    }

    Ok(Template::render("editor", project_id))
}

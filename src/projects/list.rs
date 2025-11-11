use crate::session::session_guard::Session;
use crate::settings::Settings;
use crate::storage::project_storage::ProjectStorage;
use crate::storage::ProjectListEntry;
use rocket::State;
use rocket_dyn_templates::Template;
use std::sync::Arc;

#[get("/")]
pub async fn list_projects(
    _session: Session,
    project_storage: &State<Arc<ProjectStorage>>,
    settings: &State<Settings>,
) -> Template {
    // Get all projects
    let projects = project_storage.get_projects_list().await;
    #[derive(serde::Serialize)]
    struct DashboardData<'a> {
        projects: Vec<ProjectListEntry>,
        version: &'a str,
    }

    Template::render(
        "dashboard",
        DashboardData {
            projects,
            version: &settings.version,
        },
    )
}

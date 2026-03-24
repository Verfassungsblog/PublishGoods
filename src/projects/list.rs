use crate::session::session_guard::Session;
use crate::settings::Settings;
use crate::storage::data_storage::current::ProjectListEntry;
use crate::storage::data_storage::DataStorage;
use crate::storage::project_storage::ProjectStorage;
use rocket::State;
use rocket_dyn_templates::Template;
use std::sync::Arc;

#[get("/")]
pub async fn list_projects(
    _session: Session,
    data_storage: &State<Arc<DataStorage>>,
    settings: &State<Settings>,
) -> Template {
    let mut projects = data_storage.data.projects.read().unwrap().entries.clone();
    projects.sort_by(|b, a| {
        return match a {
            ProjectListEntry::Folder(_) => std::cmp::Ordering::Greater,
            ProjectListEntry::Project(project) => match b {
                ProjectListEntry::Folder(_) => std::cmp::Ordering::Equal,
                ProjectListEntry::Project(project_b) => {
                    project.last_interaction.cmp(&project_b.last_interaction)
                }
            },
        };
    });

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

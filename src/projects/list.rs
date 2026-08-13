use crate::db::repositories::folders;
use crate::session::session_guard::Session;
use crate::settings::Settings;
use crate::storage::data_storage::current::ProjectListEntry;
use rocket::State;
use rocket_dyn_templates::Template;
use sqlx::PgPool;

/// Renders the dashboard page listing all projects and folders, sorted with folders
/// first and projects ordered by most recent interaction.
#[get("/")]
pub async fn list_projects(
    _session: Session,
    pool: &State<PgPool>,
    settings: &State<Settings>,
) -> Template {
    let mut projects = folders::get_project_list_tree(pool.inner())
        .await
        .map(|tree| tree.entries)
        .unwrap_or_default();
    projects.sort_by(|b, a| match a {
        ProjectListEntry::Folder(_) => std::cmp::Ordering::Greater,
        ProjectListEntry::Project(project) => match b {
            ProjectListEntry::Folder(_) => std::cmp::Ordering::Equal,
            ProjectListEntry::Project(project_b) => {
                project.last_interaction.cmp(&project_b.last_interaction)
            }
        },
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

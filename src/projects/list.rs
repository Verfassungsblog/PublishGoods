use crate::db::repositories::folders::{self, TeamProjects};
use crate::session::session_guard::Session;
use crate::settings::Settings;
use rocket::State;
use rocket_dyn_templates::Template;
use sqlx::PgPool;

/// Renders the dashboard page listing the user's projects grouped by team, with each team's
/// projects ordered by most recent interaction.
#[get("/")]
pub async fn list_projects(
    session: Session,
    pool: &State<PgPool>,
    settings: &State<Settings>,
) -> Template {
    let teams = folders::get_projects_by_team(pool.inner(), session.user_id)
        .await
        .unwrap_or_else(|e| {
            error!(
                "Couldn't load the project list of user {}: {}",
                session.user_id, e
            );
            Vec::new()
        });

    #[derive(serde::Serialize)]
    struct DashboardData<'a> {
        teams: Vec<TeamProjects>,
        version: &'a str,
    }

    Template::render(
        "dashboard",
        DashboardData {
            teams,
            version: &settings.version,
        },
    )
}

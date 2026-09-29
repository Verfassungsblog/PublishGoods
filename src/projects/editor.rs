use crate::db::repositories::{folders, projects};
use crate::session::session_guard::Session;
use rocket::State;
use rocket::http::Status;
use rocket_dyn_templates::Template;
use sqlx::PgPool;

/// Renders the project editor page for an existing project, returning 404 if the id
/// doesn't parse or no project with that id exists. Also touches the project's
/// last-interaction timestamp (used for sorting the project list) on each visit.
#[get("/projects/<project_id>")]
pub async fn show_editor(
    project_id: String,
    _session: Session,
    pool: &State<PgPool>,
) -> Result<Template, Status> {
    let project_id = match uuid::Uuid::parse_str(&project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            eprintln!("Couldn't parse project id: {}", e);
            return Err(Status::NotFound);
        }
    };

    if !projects::exists(pool.inner(), project_id)
        .await
        .unwrap_or(false)
    {
        eprintln!("Couldn't get project with id {}", project_id);
        return Err(Status::NotFound);
    }

    let _ = folders::touch_project_last_interaction(pool.inner(), project_id).await;

    Ok(Template::render("editor", project_id))
}

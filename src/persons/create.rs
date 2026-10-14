use crate::permissions::{ResourceType, has_permission};
use crate::session::session_guard::Session;
use rocket::State;
use rocket::http::Status;
use rocket_dyn_templates::Template;
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Serialize)]
struct CreateData {
    team_id: Uuid,
}

/// Renders the create-person form page for one team's library. The user must belong to the
/// team given by `team_id`.
#[get("/persons/create?<team_id>")]
pub async fn show_create_person(
    session: Session,
    pool: &State<PgPool>,
    team_id: String,
) -> Result<Template, Status> {
    let team_id = Uuid::parse_str(&team_id).map_err(|_| Status::BadRequest)?;
    has_permission(pool, ResourceType::Team, team_id, session.user_id, None)
        .await
        .map_err(|_| Status::NotFound)?;
    Ok(Template::render("create_person", CreateData { team_id }))
}

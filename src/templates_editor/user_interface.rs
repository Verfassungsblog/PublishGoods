use crate::db::repositories::{teams, templates};
use crate::permissions::{AuthorizedTemplate, ResourceType, has_permission};
use crate::session::session_guard::Session;
use rocket::State;
use rocket::http::Status;
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Serialize)]
struct ListData {
    teams: Vec<teams::TeamOption>,
    selected_team_id: Uuid,
    templates: Vec<templates::Template>,
}

/// Loads the user's teams and resolves the team a page is about: `team_id` if given (it must be
/// one of the user's teams, else 404), otherwise the user's preferred team. Returns the team
/// options for the page's team picker (with that team selected) and the team's id.
async fn selected_team(
    pool: &PgPool,
    user_id: Uuid,
    team_id: Option<String>,
) -> Result<(Vec<teams::TeamOption>, Uuid), Status> {
    let user_teams = teams::list_for_user(pool, user_id)
        .await
        .map_err(|_| Status::InternalServerError)?;
    let selected = match team_id {
        Some(team_id) => {
            let team_id = Uuid::parse_str(&team_id).map_err(|_| Status::BadRequest)?;
            user_teams
                .iter()
                .any(|t| t.id == team_id)
                .then_some(team_id)
                .ok_or(Status::NotFound)?
        }
        None => teams::preferred_team(&user_teams)
            .map(|t| t.id)
            .ok_or(Status::NotFound)?,
    };
    Ok((teams::team_options(&user_teams, selected), selected))
}

/// Lists the templates of one of the user's teams (`team_id`, defaulting to the user's
/// preferred team).
#[get("/templates?<team_id>")]
pub async fn list_templates(
    session: Session,
    pool: &State<PgPool>,
    team_id: Option<String>,
) -> Result<rocket_dyn_templates::Template, Status> {
    let (teams, selected_team_id) = selected_team(pool.inner(), session.user_id, team_id).await?;

    let templates = templates::list_for_team(pool.inner(), selected_team_id)
        .await
        .map_err(|_| Status::InternalServerError)?;
    Ok(rocket_dyn_templates::Template::render(
        "templates",
        ListData {
            teams,
            selected_team_id,
            templates,
        },
    ))
}

/// Get a specific template
#[get("/templates/<_>")]
pub async fn get_template(
    auth: AuthorizedTemplate,
    pool: &State<PgPool>,
) -> Result<rocket_dyn_templates::Template, Status> {
    let template = templates::get(pool.inner(), auth.id)
        .await
        .map_err(|_| Status::NotFound)?;
    Ok(rocket_dyn_templates::Template::render(
        "detailed_template",
        template,
    ))
}

/// Create new template
#[get("/templates/create?<team_id>")]
pub async fn create_template(
    session: Session,
    pool: &State<PgPool>,
    team_id: Option<String>,
) -> Result<rocket_dyn_templates::Template, Status> {
    let (teams, _) = selected_team(pool.inner(), session.user_id, team_id).await?;
    Ok(rocket_dyn_templates::Template::render(
        "create_template",
        serde_json::json!({ "teams": teams }),
    ))
}

#[derive(FromForm)]
/// Represents a template creation request.
pub struct CreateTemplate {
    /// The name of the template.
    pub name: String,
    /// The description of the template.
    pub description: String,
    /// uuid of the team that owns the template
    pub team_id: String,
}

/// POST /templates/create
///
/// Creates a new template owned by the submitted team (which the user must be a member of) and
/// redirects to that team's template list.
#[post("/templates/create", data = "<template>")]
pub async fn form_create_template(
    session: Session,
    template: rocket::form::Form<CreateTemplate>,
    pool: &State<PgPool>,
) -> Result<rocket::response::Redirect, Status> {
    let team_id = Uuid::parse_str(&template.team_id).map_err(|_| Status::BadRequest)?;
    has_permission(pool, ResourceType::Team, team_id, session.user_id, None)
        .await
        .map_err(|_| Status::NotFound)?;
    templates::insert(pool.inner(), team_id, &template.name, &template.description)
        .await
        .map_err(|_| Status::InternalServerError)?;
    Ok(rocket::response::Redirect::to(format!(
        "/templates?team_id={team_id}"
    )))
}

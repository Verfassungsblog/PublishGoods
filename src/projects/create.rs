use crate::db::repositories::{DbError, projects, teams, templates};
use crate::permissions::{ResourceType, has_permission};
use crate::session::session_guard::Session;
use rocket::State;
use rocket::http::Status;
use rocket::response::Redirect;
use rocket_dyn_templates::Template;
use serde::Serialize;
use sqlx::PgPool;

#[derive(Serialize)]
struct CreateProjectData {
    templates: Vec<templates::TemplateSummary>,
    teams: Vec<teams::TeamOption>,
}

/// Show create project form
#[get("/projects/create")]
pub async fn show_create_project(
    session: Session,
    pool: &State<PgPool>,
) -> Result<Template, Status> {
    // The templates of all the user's teams (the form only offers those of the selected team),
    // and the teams the project may belong to
    let (user_templates, user_teams) = tokio::try_join!(
        templates::list_summaries_for_user(pool.inner(), session.user_id),
        teams::list_for_user(pool.inner(), session.user_id),
    )
    .map_err(|_| Status::InternalServerError)?;
    let preselected = teams::preferred_team(&user_teams)
        .map(|t| t.id)
        .ok_or(Status::Forbidden)?;

    Ok(Template::render(
        "create_project",
        CreateProjectData {
            templates: user_templates,
            teams: teams::team_options(&user_teams, preselected),
        },
    ))
}

/// Struct used for creating a new project
#[derive(FromForm)]
pub struct CreateProjectForm {
    /// Project Name
    pub project_name: String,
    /// uuid of the template used
    pub template_id: String,
    /// optional project description
    pub project_description: Option<String>,
    /// uuid of the team that owns the project
    pub team_id: String,
}

/// Process create project form
#[post("/projects/create", data = "<data>")]
pub async fn process_create_project(
    session: Session,
    data: rocket::form::Form<CreateProjectForm>,
    pool: &State<PgPool>,
) -> Result<Redirect, Status> {
    let template_id = match uuid::Uuid::try_parse(&data.template_id) {
        Ok(template_id) => template_id,
        Err(e) => {
            eprintln!(
                "Couldn't parse template_id from create new project form: {}",
                e
            );
            return Err(Status::BadRequest);
        }
    };

    let team_id = uuid::Uuid::try_parse(&data.team_id).map_err(|_| Status::BadRequest)?;
    has_permission(pool, ResourceType::Team, team_id, session.user_id, None)
        .await
        .map_err(|_| Status::NotFound)?;

    // A project can only use templates of its own team, `projects::insert` refuses others
    let project_id = uuid::Uuid::new_v4();
    if let Err(e) = projects::insert(
        pool.inner(),
        project_id,
        &data.project_name,
        data.project_description.as_deref(),
        Some(template_id),
        None,
        team_id,
    )
    .await
    {
        return Err(match e {
            DbError::NotFound(_) => Status::BadRequest,
            e => {
                eprintln!("Couldn't insert project: {:?}", e);
                Status::InternalServerError
            }
        });
    }

    Ok(Redirect::to("/"))
}

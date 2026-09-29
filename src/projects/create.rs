use crate::db::repositories::{projects, templates, users};
use crate::session::session_guard::Session;
use rocket::State;
use rocket::http::Status;
use rocket::response::Redirect;
use rocket_dyn_templates::Template;
use sqlx::PgPool;
use std::collections::BTreeMap;

/// Show create project form
#[get("/projects/create")]
pub async fn show_create_project(
    _session: Session,
    pool: &State<PgPool>,
) -> Result<Template, Status> {
    // Get list of all templates
    let all_templates = templates::list_all(pool.inner())
        .await
        .map_err(|_| Status::InternalServerError)?;

    let mut data = BTreeMap::new();
    data.insert("templates", all_templates);
    Ok(Template::render("create_project", data))
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
}

/// Process create project form
#[post("/projects/create", data = "<data>")]
pub async fn process_create_project(
    _session: Session,
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

    //Check if template exists
    if !templates::exists(pool.inner(), template_id)
        .await
        .unwrap_or(false)
    {
        return Err(Status::BadRequest);
    }

    let default_team_id = users::ensure_default_team(pool.inner())
        .await
        .map_err(|_| Status::InternalServerError)?;

    let project_id = uuid::Uuid::new_v4();
    if let Err(e) = projects::insert(
        pool.inner(),
        project_id,
        &data.project_name,
        data.project_description.as_deref(),
        Some(template_id),
        None,
        default_team_id,
    )
    .await
    {
        eprintln!("Couldn't insert project: {:?}", e);
        return Err(Status::InternalServerError);
    }

    Ok(Redirect::to("/"))
}

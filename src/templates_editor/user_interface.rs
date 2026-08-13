use crate::db::repositories::templates;
use crate::session::session_guard::Session;
use rocket::State;
use rocket::http::Status;
use sqlx::PgPool;

/// Get a list of all templates
#[get("/templates")]
pub async fn list_templates(
    _session: Session,
    pool: &State<PgPool>,
) -> Result<rocket_dyn_templates::Template, Status> {
    let templates = templates::list_all(pool.inner())
        .await
        .map_err(|_| Status::InternalServerError)?;
    Ok(rocket_dyn_templates::Template::render(
        "templates",
        templates,
    ))
}

/// Get a specific template
#[get("/templates/<id>")]
pub async fn get_template(
    _session: Session,
    id: String,
    pool: &State<PgPool>,
) -> Result<rocket_dyn_templates::Template, Status> {
    let id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return Err(Status::BadRequest),
    };
    let template = templates::get(pool.inner(), id)
        .await
        .map_err(|_| Status::NotFound)?;
    Ok(rocket_dyn_templates::Template::render(
        "detailed_template",
        template,
    ))
}

/// Create new template
#[get("/templates/create")]
pub async fn create_template(_session: Session) -> Result<rocket_dyn_templates::Template, Status> {
    Ok(rocket_dyn_templates::Template::render(
        "create_template",
        (),
    ))
}

#[derive(FromForm)]
/// Represents a template creation request.
pub struct CreateTemplate {
    /// The name of the template.
    pub name: String,
    /// The description of the template.
    pub description: String,
}

/// POST /templates/create
///
/// Creates a new template from the submitted form data and redirects to the template list.
#[post("/templates/create", data = "<template>")]
pub async fn form_create_template(
    _session: Session,
    template: rocket::form::Form<CreateTemplate>,
    pool: &State<PgPool>,
) -> Result<rocket::response::Redirect, Status> {
    templates::insert(pool.inner(), &template.name, &template.description)
        .await
        .map_err(|_| Status::InternalServerError)?;
    Ok(rocket::response::Redirect::to("/templates"))
}

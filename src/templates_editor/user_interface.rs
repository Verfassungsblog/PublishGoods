use crate::session::session_guard::Session;
use crate::settings::Settings;
use crate::storage::data_storage::DataStorage;
use crate::storage::ProjectTemplateV2;
use rocket::http::Status;
use rocket::State;
use std::collections::HashMap;
use std::sync::Arc;

/// Get a list of all templates
#[get("/templates")]
pub async fn list_templates(
    _session: Session,
    data_storage: &State<Arc<DataStorage>>,
) -> Result<rocket_dyn_templates::Template, Status> {
    let data_storage = data_storage;
    let templates: Vec<ProjectTemplateV2> = data_storage
        .data
        .templates
        .iter()
        .map(|x| x.value().read().unwrap().clone())
        .collect();
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
    data_storage: &State<Arc<DataStorage>>,
) -> Result<rocket_dyn_templates::Template, Status> {
    let id = match uuid::Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return Err(Status::BadRequest),
    };
    let data_storage = Arc::clone(data_storage);
    let template = match data_storage.data.templates.get(&id) {
        Some(template) => template.read().unwrap().clone(),
        None => return Err(Status::NotFound),
    };
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

#[post("/templates/create", data = "<template>")]
pub async fn form_create_template(
    _session: Session,
    settings: &State<Settings>,
    template: rocket::form::Form<CreateTemplate>,
    data_storage: &State<Arc<DataStorage>>,
) -> Result<rocket::response::Redirect, Status> {
    let template = ProjectTemplateV2 {
        id: uuid::Uuid::new_v4(),
        version: Some(uuid::Uuid::new_v4()),
        name: template.name.clone(),
        description: template.description.clone(),
        export_formats: HashMap::new(),
    }; //TODO: create default export format for preview
    let data_storage = data_storage;
    data_storage
        .insert_template(template, settings)
        .await
        .unwrap();
    Ok(rocket::response::Redirect::to("/templates"))
}

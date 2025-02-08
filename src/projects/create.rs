use std::collections::HashMap;
use std::collections::BTreeMap;
use crate::data_storage::{ProjectDataV5, ProjectTemplateV2};
use std::sync::Arc;
use rocket::http::Status;
use rocket::response::Redirect;
use rocket::State;
use rocket_dyn_templates::Template;
use crate::data_storage::{DataStorage, ProjectStorage};
use crate::session::session_guard::Session;
use crate::settings::Settings;

/// Show create project form
#[get("/projects/create")]
pub async fn show_create_project(_session: Session, data_storage: &State<Arc<DataStorage>>) -> Result<Template, Status> {
    // Get list of all templates
    let templates : Vec<ProjectTemplateV2> = data_storage.data.read().unwrap().templates.iter().map(|(_id, entry) | entry.clone().read().unwrap().clone()).collect();

    let mut data = BTreeMap::new();
    data.insert("templates", templates);
    Ok(Template::render("create_project", data))
}
#[derive(FromForm)]
pub struct CreateProjectForm{
    pub project_name: String,
    pub template_id: String,
    pub project_description: Option<String>,
}

/// Process create project form
#[post("/projects/create", data = "<data>")]
pub async fn process_create_project(_session: Session, data: rocket::form::Form<CreateProjectForm>, data_storage: &State<Arc<DataStorage>>, project_storage: &State<Arc<ProjectStorage>>, settings: &State<Settings>) -> Result<Redirect, Status> {
    let template_id = match uuid::Uuid::try_parse(&data.template_id){
        Ok(template_id) => template_id,
        Err(e) => {
            eprintln!("Couldn't parse template_id from create new project form: {}", e);
            return Err(Status::BadRequest);
        }
    };

    //Check if template exists
    if !data_storage.data.read().unwrap().templates.contains_key(&template_id){
        return Err(Status::BadRequest)
    }

    let project_data = ProjectDataV5 {
        name: data.project_name.clone(),
        description: data.project_description.clone(),
        template_id,
        last_interaction: 0,
        metadata: None,
        settings: None,
        sections: vec![],
        bibliography: HashMap::new(),
    };

    match project_storage.insert_project(project_data, settings).await{
        Ok(id) => {
            println!("Successfully created new project with id {}", id);
            Ok(Redirect::to("/"))
        },
        Err(_) => {
            Err(Status::InternalServerError)
        }
    }
}
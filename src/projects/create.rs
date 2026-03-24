use crate::session::session_guard::Session;
use crate::settings::Settings;
use crate::storage::ProjectTemplateV2;
use crate::storage::data_storage::DataStorage;
use crate::storage::data_storage::current::{ProjectListEntry, ProjectListProject};
use crate::storage::project_storage::current::Bibliography;
use crate::storage::project_storage::{ProjectData, ProjectStorage};
use chrono::Utc;
use rocket::State;
use rocket::http::Status;
use rocket::response::Redirect;
use rocket_dyn_templates::Template;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Show create project form
#[get("/projects/create")]
pub async fn show_create_project(
    _session: Session,
    data_storage: &State<Arc<DataStorage>>,
) -> Result<Template, Status> {
    // Get list of all templates
    let templates: Vec<ProjectTemplateV2> = data_storage
        .data
        .templates
        .iter()
        .map(|x| x.value().read().unwrap().clone())
        .collect();

    let mut data = BTreeMap::new();
    data.insert("templates", templates);
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
    data_storage: &State<Arc<DataStorage>>,
    project_storage: &State<Arc<ProjectStorage>>,
    settings: &State<Settings>,
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
    if !data_storage.data.templates.contains_key(&template_id) {
        return Err(Status::BadRequest);
    }

    let project_data = ProjectData {
        name: data.project_name.clone(),
        description: data.project_description.clone(),
        template_id,
        last_interaction: 0,
        metadata: None,
        settings: None,
        sections: vec![],
        bibliography: Bibliography::new(),
    };

    let project_id = uuid::Uuid::new_v4();
    if let Err(e) = project_storage
        .insert_project(project_id, project_data.clone(), settings)
        .await
    {
        eprintln!("Couldn't insert project into project_storage: {:?}", e);
        return Err(Status::InternalServerError);
    }
    let project_list = &data_storage.data.projects;
    project_list
        .write()
        .unwrap()
        .entries
        .push(ProjectListEntry::Project(ProjectListProject {
            id: project_id,
            name: project_data.name,
            last_interaction: Utc::now().naive_utc(),
        }));
    if let Err(e) = data_storage.save_to_disk(settings).await {
        eprintln!("Couldn't save project list to disk: {:?}", e);
        return Err(Status::InternalServerError);
    }
    Ok(Redirect::to("/"))
}

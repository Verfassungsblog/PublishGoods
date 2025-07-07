use std::sync::Arc;
use rocket::serde::json::Json;
use rocket::State;
use crate::projects::ProjectMetadata;
use crate::session::session_guard::Session;
use crate::settings::Settings;
use crate::storage::data_storage::DataStorage;
use crate::storage::project_storage::{ProjectData, ProjectStorage};

pub struct APIProjectData{
    
}

/// GET /api/projects/<project_id>
#[get("/api/projects/<project_id>/")]
pub async fn get_project(
    project_id: String,
    _session: Session,
    settings: &State<Settings>,
    project_storage: &State<Arc<ProjectStorage>>,
    data_storage: &State<Arc<DataStorage>>,
) -> ApiResult<ProjectData> {
    
}
use std::sync::{Arc, RwLock};
use rocket::serde::json::Json;
use rocket::State;
use crate::data_storage::{ProjectData, ProjectStorage};
use crate::projects::api::{ApiError, ApiResult};
use crate::settings::Settings;

pub fn parse_uuid(uuid: &str) -> Result<uuid::Uuid, Json<ApiResult<ApiError>>>{
    match uuid::Uuid::parse_str(uuid){
        Ok(uuid) => Ok(uuid),
        Err(e) => {
            eprintln!("Couldn't parse UUID: {}", e);
            Err(ApiResult::new_error(ApiError::BadRequest("Invalid UUID".to_string())))
        }
    }
}
pub async fn get_project(project_id: &uuid::Uuid, settings: &State<Settings>, project_storage: Arc<ProjectStorage>) -> Result<Arc<RwLock<ProjectData>>, Json<ApiResult<ApiError>>>{
    match project_storage.get_project(&project_id, settings).await{
        Ok(project_entry) => Ok(project_entry.clone()),
        Err(_) => {
            eprintln!("Couldn't get project with id {}", project_id);
            Err(ApiResult::new_error(ApiError::NotFound))
        },
    }
}
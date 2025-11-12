use crate::projects::api::Patch;
use crate::projects::{PersonUuidOrString, ProjectMetadata, ProjectMetadataV5, SectionOrTocV5};
use crate::session::session_guard::Session;
use crate::settings::Settings;
use crate::storage::data_storage::DataStorage;
use crate::storage::project_storage::{ProjectData, ProjectStorage};
use crate::storage::{BibEntryV2, ProjectTemplateV2};
use crate::utils::api_helpers::{APIResponse, APIResult};
use bincode::{Decode, Encode};
use chrono::NaiveDate;
use language::Language;
use rocket::form::validate::Contains;
use rocket::serde::json::Json;
use rocket::State;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;
use vb_exchange::projects::{Identifier, Keyword, License, ProjectSettingsV5};

/// Return struct for ['get_project'].
/// Similar to ['crate::storage::project_storage::ProjectData'] but some fields are only Some if specified in extend
#[derive(Debug, Serialize, Deserialize)]
pub struct APIProjectData {
    /// Project Title
    pub name: String,
    /// Project Description
    pub description: Option<String>,
    /// Id for the ProjectTemplate
    pub template_id: uuid::Uuid,
    /// Optionally extended ProjectTemplate
    pub template_extended: Option<ProjectTemplateV2>,
    /// Optionally extended ProjectMetadata
    pub metadata: Option<ProjectMetadataV5>,
    /// Optionally extended ProjectSettings
    pub settings: Option<ProjectSettingsV5>,
    /// Optionally extended Sections
    pub sections: Option<Vec<SectionOrTocV5>>,
    /// Optionally extended Bibliography
    pub bibliography: Option<HashMap<String, BibEntryV2>>,
}

/// GET /api/projects/<project_id>
#[get("/api/projects/<project_id>?<extend>")]
pub async fn get_project(
    project_id: &str,
    extend: Option<String>,
    _session: Session,
    settings: &State<Settings>,
    project_storage: &State<Arc<ProjectStorage>>,
    data_storage: &State<Arc<DataStorage>>,
) -> APIResult<APIProjectData> {
    let loaded_project = project_storage
        .get_project(&Uuid::parse_str(project_id)?, settings)
        .await?
        .clone()
        .read()
        .unwrap()
        .clone();

    let mut api_response = APIProjectData {
        name: loaded_project.name,
        description: loaded_project.description,
        template_id: loaded_project.template_id,
        template_extended: None,
        metadata: None,
        settings: None,
        sections: None,
        bibliography: None,
    };

    if let Some(extend) = extend {
        let parts = extend.split(",").collect::<Vec<&str>>();
        if parts.contains("template") {
            api_response.template_extended = Some(
                data_storage
                    .get_template(&api_response.template_id)?
                    .clone()
                    .read()
                    .unwrap()
                    .clone(),
            );
        }
        if parts.contains("metadata") {
            api_response.metadata = loaded_project.metadata;
        }
        if parts.contains("settings") {
            api_response.settings = loaded_project.settings;
        }
        if parts.contains("sections") {
            api_response.sections = Some(loaded_project.sections);
        }
        if parts.contains("bibliography") {
            api_response.bibliography = Some(loaded_project.bibliography);
        }
    }

    Ok(APIResponse::from(api_response))
}

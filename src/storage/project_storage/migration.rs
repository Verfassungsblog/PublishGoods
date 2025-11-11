use crate::projects::{
    ProjectMetadataV1, ProjectMetadataV2, ProjectMetadataV3, SectionOrTocV1, SectionOrTocV2,
    SectionOrTocV3, SectionOrTocV4,
};
use crate::storage::project_storage::current::ProjectDataV8;
use crate::storage::project_storage::{ProjectData, ProjectStorageError, CURRENT_VERSION};
use crate::storage::{BibEntryV2, OldBibEntry};
use bincode::{Decode, Encode};
use rocket::http::hyper::body::HttpBody;
use rocket::serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use vb_exchange::deprecated::projects::project_settings::{
    ProjectSettingsV2, ProjectSettingsV3, ProjectSettingsV4,
};
use vb_exchange::projects::ProjectSettingsV5;

pub fn load_project_data(
    mut file: File,
    mut version: u64,
) -> Result<ProjectData, ProjectStorageError> {
    if version != CURRENT_VERSION {
        info!(
            "Migrating ProjectData from v{} to latest version (v{}).",
            version, CURRENT_VERSION
        );
    }

    let mut v1_data: Option<OldProjectData> = None;
    if version == 1 {
        v1_data = Some(bincode::decode_from_std_read::<OldProjectData, _, _>(
            &mut file,
            bincode::config::standard(),
        )?);
        version = 2;
    }
    let mut v2_data: Option<ProjectDataV2> = None;
    if version == 2 {
        v2_data = if let Some(v1_data) = v1_data {
            Some(v1_data.into())
        } else {
            Some(bincode::decode_from_std_read::<ProjectDataV2, _, _>(
                &mut file,
                bincode::config::standard(),
            )?)
        };
        version = 3;
    }
    let mut v3_data: Option<ProjectDataV3> = None;
    if version == 3 {
        v3_data = if let Some(v2_data) = v2_data {
            Some(v2_data.into())
        } else {
            Some(bincode::decode_from_std_read::<ProjectDataV3, _, _>(
                &mut file,
                bincode::config::standard(),
            )?)
        };
        version = 4;
    }
    let mut v4_data: Option<ProjectDataV4> = None;
    if version == 4 {
        v4_data = if let Some(v3_data) = v3_data {
            Some(v3_data.into())
        } else {
            Some(bincode::decode_from_std_read::<ProjectDataV4, _, _>(
                &mut file,
                bincode::config::standard(),
            )?)
        };
        version = 5;
    }
    let mut v5_data: Option<ProjectDataV5> = None;
    if version == 5 {
        v5_data = if let Some(v4_data) = v4_data {
            Some(v4_data.into())
        } else {
            Some(bincode::decode_from_std_read::<ProjectDataV5, _, _>(
                &mut file,
                bincode::config::standard(),
            )?)
        };
        version = 6;
    }
    let mut v6_data: Option<ProjectDataV6> = None;
    if version == 6 {
        v6_data = if let Some(v5_data) = v5_data {
            Some(v5_data.into())
        } else {
            Some(bincode::decode_from_std_read::<ProjectDataV6, _, _>(
                &mut file,
                bincode::config::standard(),
            )?)
        };
        version = 7;
    }
    let mut v7_data: Option<ProjectDataV7> = None;
    if version == 7 {
        v7_data = if let Some(v6_data) = v6_data {
            Some(v6_data.into())
        } else {
            Some(bincode::decode_from_std_read::<ProjectDataV7, _, _>(
                &mut file,
                bincode::config::standard(),
            )?)
        };
        version = 8;
    }
    let mut v8_data: Option<ProjectDataV8> = None;
    if version == 8 {
        v8_data = if let Some(v7_data) = v7_data {
            Some(v7_data.into())
        } else {
            Some(bincode::decode_from_std_read::<ProjectDataV8, _, _>(
                &mut file,
                bincode::config::standard(),
            )?)
        };
    }

    match v8_data {
        Some(data) => Ok(data),
        None => Err(ProjectStorageError::InvalidVersionNumber),
    }
}

#[derive(Debug, Serialize, Deserialize, Encode, Decode, Clone)]
pub struct OldProjectData {
    pub name: String,
    pub description: Option<String>,
    #[bincode(with_serde)]
    pub template_id: uuid::Uuid,
    pub last_interaction: u64,
    pub metadata: Option<ProjectMetadataV1>,
    pub settings: Option<ProjectSettingsV2>,
    pub sections: Vec<SectionOrTocV1>,
    #[bincode(with_serde)]
    pub bibliography: HashMap<String, OldBibEntry>,
}

#[derive(Debug, Serialize, Deserialize, Encode, Decode, Clone)]
pub struct ProjectDataV2 {
    pub name: String,
    pub description: Option<String>,
    #[bincode(with_serde)]
    pub template_id: uuid::Uuid,
    pub last_interaction: u64,
    pub metadata: Option<ProjectMetadataV1>,
    pub settings: Option<ProjectSettingsV2>,
    pub sections: Vec<SectionOrTocV1>,
    #[bincode(with_serde)]
    pub bibliography: HashMap<String, BibEntryV2>,
}

#[derive(Debug, Serialize, Deserialize, Encode, Decode, Clone)]
pub struct ProjectDataV3 {
    pub name: String,
    pub description: Option<String>,
    #[bincode(with_serde)]
    pub template_id: uuid::Uuid,
    pub last_interaction: u64,
    pub metadata: Option<ProjectMetadataV1>,
    pub settings: Option<ProjectSettingsV3>,
    pub sections: Vec<SectionOrTocV1>,
    #[bincode(with_serde)]
    pub bibliography: HashMap<String, BibEntryV2>,
}

#[derive(Debug, Serialize, Deserialize, Encode, Decode, Clone)]
pub struct ProjectDataV4 {
    pub name: String,
    pub description: Option<String>,
    #[bincode(with_serde)]
    pub template_id: uuid::Uuid,
    pub last_interaction: u64,
    pub metadata: Option<ProjectMetadataV1>,
    pub settings: Option<ProjectSettingsV4>,
    pub sections: Vec<SectionOrTocV2>,
    #[bincode(with_serde)]
    pub bibliography: HashMap<String, BibEntryV2>,
}

#[derive(Debug, Serialize, Deserialize, Encode, Decode, Clone)]
pub struct ProjectDataV5 {
    pub name: String,
    pub description: Option<String>,
    #[bincode(with_serde)]
    pub template_id: uuid::Uuid,
    pub last_interaction: u64,
    pub metadata: Option<ProjectMetadataV2>,
    pub settings: Option<ProjectSettingsV4>,
    pub sections: Vec<SectionOrTocV3>,
    #[bincode(with_serde)]
    pub bibliography: HashMap<String, BibEntryV2>,
}

#[derive(Debug, Serialize, Deserialize, Encode, Decode, Clone)]
pub struct ProjectDataV6 {
    pub name: String,
    pub description: Option<String>,
    #[bincode(with_serde)]
    pub template_id: uuid::Uuid,
    pub last_interaction: u64,
    pub metadata: Option<ProjectMetadataV2>,
    pub settings: Option<ProjectSettingsV5>,
    pub sections: Vec<SectionOrTocV3>,
    #[bincode(with_serde)]
    pub bibliography: HashMap<String, BibEntryV2>,
}

impl From<ProjectDataV6> for ProjectDataV7 {
    fn from(value: ProjectDataV6) -> Self {
        ProjectDataV7 {
            name: value.name,
            description: value.description,
            template_id: value.template_id,
            last_interaction: value.last_interaction,
            metadata: value.metadata.map(|v| v.into()),
            settings: value.settings,
            sections: value
                .sections
                .iter()
                .map(|section| section.clone().into())
                .collect(),
            bibliography: value.bibliography,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Encode, Decode, Clone)]
pub struct ProjectDataV7 {
    pub name: String,
    pub description: Option<String>,
    #[bincode(with_serde)]
    pub template_id: uuid::Uuid,
    pub last_interaction: u64,
    pub metadata: Option<ProjectMetadataV3>,
    pub settings: Option<ProjectSettingsV5>,
    pub sections: Vec<SectionOrTocV4>,
    #[bincode(with_serde)]
    pub bibliography: HashMap<String, BibEntryV2>, //TODO: add prefix & suffix support
}

impl From<ProjectDataV7> for ProjectDataV8 {
    fn from(value: ProjectDataV7) -> Self {
        ProjectDataV8 {
            name: value.name,
            description: value.description,
            template_id: value.template_id,
            last_interaction: value.last_interaction,
            metadata: value.metadata.map(|v| v.into()),
            settings: value.settings,
            sections: value
                .sections
                .into_iter()
                .map(|section| section.into())
                .collect(),
            bibliography: value.bibliography,
        }
    }
}

impl From<OldProjectData> for ProjectDataV2 {
    fn from(value: OldProjectData) -> Self {
        ProjectDataV2 {
            name: value.name,
            description: value.description,
            template_id: value.template_id,
            last_interaction: value.last_interaction,
            metadata: value.metadata,
            settings: value.settings,
            sections: value.sections,
            bibliography: value
                .bibliography
                .iter()
                .map(|(k, v)| (k.clone(), v.clone().into()))
                .collect(),
        }
    }
}

impl From<ProjectDataV2> for ProjectDataV3 {
    fn from(value: ProjectDataV2) -> Self {
        let settings: Option<ProjectSettingsV3> = match value.settings {
            Some(set) => Some(ProjectSettingsV3 {
                toc_enabled: set.toc_enabled,
                csl_style: set.csl_style,
                csl_language_code: None,
            }),
            None => None,
        };
        ProjectDataV3 {
            name: value.name,
            description: value.description,
            template_id: value.template_id,
            last_interaction: value.last_interaction,
            metadata: value.metadata,
            settings,
            sections: value.sections,
            bibliography: value
                .bibliography
                .iter()
                .map(|(k, v)| (k.clone(), v.clone().into()))
                .collect(),
        }
    }
}

impl From<ProjectDataV3> for ProjectDataV4 {
    fn from(value: ProjectDataV3) -> Self {
        let settings: Option<ProjectSettingsV4> = match value.settings {
            Some(set) => Some(ProjectSettingsV4 {
                toc_enabled: set.toc_enabled,
                csl_style: set.csl_style,
                csl_language_code: set.csl_language_code,
                metadata_page_additional_html: None,
                cover_image_path: None,
                backcover_image_path: None,
            }),
            None => None,
        };
        ProjectDataV4 {
            name: value.name,
            description: value.description,
            template_id: value.template_id,
            last_interaction: value.last_interaction,
            metadata: value.metadata,
            settings,
            sections: value
                .sections
                .iter()
                .map(|section| section.clone().into())
                .collect(),
            bibliography: value.bibliography,
        }
    }
}

impl From<ProjectDataV4> for ProjectDataV5 {
    fn from(value: ProjectDataV4) -> Self {
        let metadata: Option<ProjectMetadataV2> = match value.metadata {
            Some(metadata) => Some(metadata.into()),
            None => None,
        };

        ProjectDataV5 {
            name: value.name,
            description: value.description,
            template_id: value.template_id,
            last_interaction: value.last_interaction,
            metadata,
            settings: value.settings.map(|s| s.into()),
            sections: value
                .sections
                .iter()
                .map(|section| section.clone().into())
                .collect(),
            bibliography: value.bibliography,
        }
    }
}

impl From<ProjectDataV5> for ProjectDataV6 {
    fn from(value: ProjectDataV5) -> Self {
        let settings = match value.settings {
            Some(set) => Some(set.into()),
            None => None,
        };

        ProjectDataV6 {
            name: value.name,
            description: value.description,
            template_id: value.template_id,
            last_interaction: value.last_interaction,
            metadata: value.metadata,
            settings,
            sections: value
                .sections
                .iter()
                .map(|section| section.clone().into())
                .collect(),
            bibliography: value.bibliography,
        }
    }
}

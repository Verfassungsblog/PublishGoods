use std::collections::HashMap;
use std::fs;
use std::fs::File;
use std::path::Path;
use std::sync::{Arc, RwLock};
use bincode::{Decode, Encode};
use bincode::error::DecodeError;
use rocket::serde::{Deserialize, Serialize};
use vb_exchange::deprecated::projects::data_storage::PersonV1;
use vb_exchange::projects::PersonV2;
use crate::storage::{ProjectTemplateV1, ProjectTemplateV2, User};
use crate::storage::data_storage::current::InnerDataStorageV3;
use crate::storage::data_storage::{DataStorageLoadError, InnerDataStorage, CURRENT_VERSION};

/// Tries to load the InnerDataStorage and migrates it to the latest version if necessary
pub fn load_inner_data_storage(mut file: File, mut version: u64, path_to_data: &str) -> Result<InnerDataStorage, DataStorageLoadError> {
    if version != CURRENT_VERSION {
        info!("Migrating InnerDataStorage from v{} to latest version (v{}).", version, CURRENT_VERSION);
    }
    
    let mut v1_data: Option<InnerDataStorageV1> = None;
    if version == 1 {
        v1_data = Some(bincode::decode_from_std_read::<InnerDataStorageV1, _, _>(&mut file, bincode::config::standard())?);

        // Move old templates directory to templates-old
        if Path::exists(Path::new(&format!("{}/templates", path_to_data))){
            fs::rename(format!("{}/templates", path_to_data), format!("{}/templates-old", path_to_data))?;
            fs::create_dir_all(format!("{}/templates", path_to_data))?;
        }
        warn!("Migrated DataStorage from v1, you need to migrate your templates manually! Your templates were moved to data/templates-old!");

        version = 2;
    }
    let mut v2_data: Option<InnerDataStorageV2> = None;
    if version == 2 {
        v2_data = if let Some(v1_data) = v1_data {
            Some(v1_data.into())
        } else {
            Some(bincode::decode_from_std_read::<InnerDataStorageV2, _, _>(&mut file, bincode::config::standard())?)
        };
        version = 3;
    }
    let mut v3_data: Option<InnerDataStorageV3> = None;
    if version == 3 {
        v3_data = if let Some(v2_data) = v2_data {
            Some(v2_data.into())
        } else {
            Some(bincode::decode_from_std_read::<InnerDataStorageV3, _, _>(&mut file, bincode::config::standard())?)
        }
    }

    match v3_data{
        None => Err(DataStorageLoadError::InvalidVersionNumber),
        Some(data) => Ok(data)
    }
}

#[derive(Debug, Serialize, Deserialize, Encode, Decode, Clone)]
pub struct InnerDataStorageV1{
    /// HashMap with users, id as HashMap keys
    #[bincode(with_serde)]
    pub login_data: HashMap<uuid::Uuid, Arc<RwLock<User>>>,
    #[bincode(with_serde)]
    pub persons: HashMap<uuid::Uuid, Arc<RwLock<PersonV1>>>,
    #[bincode(with_serde)]
    pub templates: HashMap<uuid::Uuid, Arc<RwLock<ProjectTemplateV1>>>
}

#[derive(Debug, Serialize, Deserialize, Encode, Decode, Clone)]
pub struct InnerDataStorageV2{
    /// HashMap with users, id as HashMap keys
    #[bincode(with_serde)]
    pub login_data: HashMap<uuid::Uuid, Arc<RwLock<User>>>,
    #[bincode(with_serde)]
    pub persons: HashMap<uuid::Uuid, Arc<RwLock<PersonV1>>>,
    #[bincode(with_serde)]
    pub templates: HashMap<uuid::Uuid, Arc<RwLock<ProjectTemplateV2>>>
}

impl From<InnerDataStorageV2> for InnerDataStorageV3{
    fn from(value: InnerDataStorageV2) -> Self {
        InnerDataStorageV3{
            login_data: value.login_data,
            persons: value.persons.into_iter()
                .map(|(k, v)| (k, Arc::new(RwLock::new(PersonV2::from(v.read().unwrap().clone())))))
                .collect(),
            templates: value.templates,
        }
    }
}

impl From<InnerDataStorageV1> for InnerDataStorageV2{
    fn from(value: InnerDataStorageV1) -> Self {
        println!("Migrating data storage from V1 to V2. You have to migrate your templates manually. Your old templates where moved to data/templates-old"); // TODO: move
        InnerDataStorageV2{
            login_data: value.login_data,
            persons: value.persons,
            templates: HashMap::new(),
        }
    }
}
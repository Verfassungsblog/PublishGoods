use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};
use bincode::{Decode, Encode};
use bincode::error::DecodeError;
use rocket::serde::{Deserialize, Serialize};
use crate::storage::project_storage::current::ProjectDataV7;

pub mod migration;
pub mod current;

pub type ProjectData = ProjectDataV7;

pub const CURRENT_VERSION: u64 = 7;

/// Storage for all projects, gets build on startup based on project files in data_path
#[derive(Debug, Serialize, Deserialize)]
pub struct ProjectStorage {
    /// HashMap with project uuid and project data if project is already loaded into memory
    pub(crate) projects: RwLock<HashMap<uuid::Uuid, ProjectStorageEntry>>,
    file_locks: RwLock<HashMap<uuid::Uuid, Arc<AtomicBool>>>,
}

#[derive(Debug, Serialize, Deserialize, Encode, Decode, Clone)]
pub struct ProjectStorageEntry{
    pub name: String,
    pub data: Option<Arc<RwLock<ProjectData>>>,
}


#[derive(Debug)]
pub enum ProjectStorageError{
    BincodeDecodeError(DecodeError),
    IOError(std::io::Error),
    InvalidVersionNumber,
    ProjectNotFound,
    CouldntAcquireLock,
    TokioJoinError,
}

impl From<std::io::Error> for ProjectStorageError {
    fn from(value: std::io::Error) -> Self {
        ProjectStorageError::IOError(value)
    }
}

impl From<DecodeError> for ProjectStorageError {
    fn from(value: DecodeError) -> Self {
        ProjectStorageError::BincodeDecodeError(value)
    }
}
use crate::storage::project_storage::current::{ProjectDataV10, ProjectMetadataV5};
use bincode::error::DecodeError;
use bincode::{Decode, Encode};
use dashmap::DashMap;
use rocket::serde::{Deserialize, Serialize};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};
use tokio::task::JoinError;
use uuid::Uuid;

pub mod current;
pub mod migration;
pub mod sections;

pub type ProjectData = ProjectDataV10;
pub type ProjectMetadata = ProjectMetadataV5;

pub const CURRENT_VERSION: u64 = 10;

/// Storage for all projects, gets build on startup based on project files in data_path
#[derive(Debug)]
pub struct ProjectStorage {
    /// HashMap with project uuid and project data. Only contains projects that are loaded into memory
    pub projects: DashMap<Uuid, Arc<RwLock<ProjectData>>>,
    pub file_locks: DashMap<uuid::Uuid, Arc<AtomicBool>>,
}

#[derive(Debug)]
pub enum ProjectStorageError {
    BincodeDecodeError(DecodeError),
    BincodeEncodeError(bincode::error::EncodeError),
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

impl From<JoinError> for ProjectStorageError {
    fn from(value: JoinError) -> Self {
        ProjectStorageError::TokioJoinError
    }
}

impl From<bincode::error::EncodeError> for ProjectStorageError {
    fn from(value: bincode::error::EncodeError) -> Self {
        ProjectStorageError::BincodeEncodeError(value)
    }
}

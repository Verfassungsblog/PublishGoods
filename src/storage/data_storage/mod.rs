use bincode::error::DecodeError;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

pub mod current;
pub mod migration;

pub type InnerDataStorage = current::InnerDataStorageV4;
static CURRENT_VERSION: u64 = 4;

/// Storage for small data like users, passwords and login attempts
///
/// This data is stored in memory permanently and doesn't get unloaded
pub struct DataStorage {
    pub data: Arc<InnerDataStorage>,
    file_locked: AtomicBool,
}

#[derive(Debug)]
pub enum DataStorageLoadError {
    BincodeDecodeError(DecodeError),
    IoError(std::io::Error),
    InvalidVersionNumber,
    DataStorageMissing,
}

impl From<std::io::Error> for DataStorageLoadError {
    fn from(value: std::io::Error) -> Self {
        DataStorageLoadError::IoError(value)
    }
}

impl From<DecodeError> for DataStorageLoadError {
    fn from(value: DecodeError) -> Self {
        DataStorageLoadError::BincodeDecodeError(value)
    }
}

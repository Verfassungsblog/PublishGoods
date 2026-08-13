use crate::settings::Settings;
use crate::storage::data_storage::migration::load_inner_data_storage;
use crate::storage::data_storage::{DataStorage, DataStorageLoadError, InnerDataStorage};
pub(crate) use crate::storage::{ProjectTemplateV2, User};
use bincode::{Decode, Encode};
use dashmap::DashMap;
use rocket::serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock};
use uuid::Uuid;
use vb_exchange::projects::PersonV2;

#[derive(Debug, Serialize, Deserialize, Encode, Decode, Clone, Default)]
pub struct ProjectList {
    pub entries: Vec<ProjectListEntry>,
}

impl ProjectList {
    pub fn has(&self, id: &Uuid) -> bool {
        self.entries.iter().any(|entry| entry.id() == id)
    }
    pub fn get(&self, id: &Uuid) -> Option<&ProjectListEntry> {
        self.entries.iter().find(|entry| entry.id() == id)
    }
    pub fn get_mut(&mut self, id: &Uuid) -> Option<&mut ProjectListEntry> {
        self.entries.iter_mut().find(|entry| entry.id() == id)
    }
    pub fn get_folder(&self, id: &Uuid) -> Option<&ProjectListFolder> {
        self.entries
            .iter()
            .find(|entry| entry.id() == id)
            .and_then(|entry| match entry {
                ProjectListEntry::Folder(folder) => Some(folder),
                _ => None,
            })
    }
}

#[derive(Debug, Serialize, Deserialize, Encode, Decode, Clone)]
pub enum ProjectListEntry {
    Folder(ProjectListFolder),
    Project(ProjectListProject),
}

impl ProjectListEntry {
    pub fn id(&self) -> &Uuid {
        match self {
            ProjectListEntry::Folder(folder) => &folder.id,
            ProjectListEntry::Project(project) => &project.id,
        }
    }
    pub fn name(&self) -> &str {
        match self {
            ProjectListEntry::Folder(folder) => &folder.name,
            ProjectListEntry::Project(project) => &project.name,
        }
    }

    pub fn set_name(&mut self, name: String) {
        match self {
            ProjectListEntry::Folder(folder) => folder.name = name,
            ProjectListEntry::Project(project) => project.name = name,
        }
    }
    pub fn set_id(&mut self, id: Uuid) {
        match self {
            ProjectListEntry::Folder(folder) => folder.id = id,
            ProjectListEntry::Project(project) => project.id = id,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Encode, Decode, Clone)]
pub struct ProjectListProject {
    #[bincode(with_serde)]
    pub id: Uuid,
    pub name: String,
    #[bincode(with_serde)]
    pub last_interaction: chrono::NaiveDateTime,
}

#[derive(Debug, Serialize, Deserialize, Encode, Decode, Clone)]
pub struct ProjectListFolder {
    #[bincode(with_serde)]
    pub id: Uuid,
    pub name: String,
    pub children: Vec<ProjectListEntry>,
}

#[derive(Debug, Serialize, Deserialize, Encode, Decode, Clone, Default)]
pub struct InnerDataStorageV4 {
    #[bincode(with_serde)]
    pub login_data: DashMap<Uuid, Arc<RwLock<User>>>,
    #[bincode(with_serde)]
    pub persons: DashMap<Uuid, Arc<RwLock<PersonV2>>>,
    #[bincode(with_serde)]
    pub templates: DashMap<Uuid, Arc<RwLock<ProjectTemplateV2>>>,
    #[bincode(with_serde)]
    pub projects: Arc<RwLock<ProjectList>>,
}

impl Default for DataStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl DataStorage {
    /// Creates a new empty [DataStorage]
    pub fn new() -> Self {
        DataStorage {
            data: Arc::new(InnerDataStorageV4::default()),
        }
    }

    fn load_from_disk_blocking(
        settings: &Settings,
    ) -> Result<InnerDataStorage, DataStorageLoadError> {
        let path = settings.data_path.to_string();
        let files = std::fs::read_dir(&path)?;

        let mut file_versions: Vec<(u64, String)> = vec![];

        // Iterate through dir entries and find all data files with version number
        for file in files {
            match file {
                Ok(file) => {
                    if let Ok(file_type) = file.file_type()
                        && file_type.is_file()
                    {
                        let fname = file.file_name().clone();
                        let fname = fname.to_str().unwrap_or("");
                        let parts: Vec<&str> = fname.split(".").collect();

                        // First part = "data"
                        // Second part = version
                        // Third part = "bincode"

                        if parts.len() == 3 && parts[0] == "data" {
                            // parse version as usize
                            let version = match parts[1].parse::<u64>() {
                                Ok(version) => version,
                                Err(e) => {
                                    eprintln!(
                                        "error while loading data_storage file into memory: couldn't parse version number: {}. Skipping file.",
                                        e
                                    );
                                    continue;
                                }
                            };

                            file_versions.push((version, fname.to_string()));
                        }
                    }
                }
                Err(e) => {
                    eprintln!(
                        "io error while loading data_storage directory entry: {}. Skipping file.",
                        e
                    );
                    continue;
                }
            }
        }

        // Order file versions
        file_versions.sort_by(|a, b| a.0.cmp(&b.0));

        // Load the latest version of the data storage
        if file_versions.is_empty() {
            error!(
                "error while loading data storage into memory: no storage files found in data directory."
            );
            return Err(DataStorageLoadError::DataStorageMissing);
        }
        let (version, file_path) = file_versions.last().unwrap();

        let file = std::fs::File::open(format!("{}/{}", &path, file_path))?;

        load_inner_data_storage(file, *version, &settings.clone().data_path)
    }

    /// Loads the [DataStorage] from disk
    ///
    /// Path is defined as data_path from settings + /data.version.bincode
    pub async fn load_from_disk(settings: &Settings) -> Result<Self, DataStorageLoadError> {
        let mut data_storage = DataStorage::new();

        let settings_cpy = settings.clone();
        let res =
            tokio::task::spawn_blocking(move || Self::load_from_disk_blocking(&settings_cpy)).await;

        data_storage.data = Arc::new(res.unwrap()?);
        Ok(data_storage)
    }
}

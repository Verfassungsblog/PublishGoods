use crate::settings::Settings;
use crate::storage::data_storage::migration::load_inner_data_storage;
use crate::storage::data_storage::{
    CURRENT_VERSION, DataStorage, DataStorageLoadError, InnerDataStorage,
};
use crate::storage::project_storage::ProjectStorage;
pub(crate) use crate::storage::{ProjectTemplateV2, SingleFileLock, User};
use bincode::{Decode, Encode};
use chrono::Utc;
use dashmap::DashMap;
use rocket::serde::{Deserialize, Serialize};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};
use tokio::task::JoinError;
use uuid::Uuid;
use vb_exchange::projects::PersonV2;

#[derive(Debug)]
pub enum DataStorageError {
    /// Resource not found, contains resource type + id
    NotFound(String),
    TokioJoinError(JoinError),
    IOError(std::io::Error),
    CouldntAcquireLock,
    BincodeDecodeError(bincode::error::DecodeError),
    BincodeEncodeError(bincode::error::EncodeError),
}

impl From<JoinError> for DataStorageError {
    fn from(e: JoinError) -> Self {
        DataStorageError::TokioJoinError(e)
    }
}

impl From<std::io::Error> for DataStorageError {
    fn from(e: std::io::Error) -> Self {
        DataStorageError::IOError(e)
    }
}

impl From<bincode::error::DecodeError> for DataStorageError {
    fn from(e: bincode::error::DecodeError) -> Self {
        DataStorageError::BincodeDecodeError(e)
    }
}

impl From<bincode::error::EncodeError> for DataStorageError {
    fn from(e: bincode::error::EncodeError) -> Self {
        DataStorageError::BincodeEncodeError(e)
    }
}

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
            file_locked: Default::default(),
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

    /// Saves the [DataStorage] to disk
    pub async fn save_to_disk(&self, settings: &Settings) -> Result<(), DataStorageError> {
        self.wait_for_file_lock(settings)
            .await
            .map_err(|_| DataStorageError::CouldntAcquireLock)?;

        let cpy = self.data.clone();
        let path = format!("{}/data.{}.bincode", settings.data_path, CURRENT_VERSION);

        tokio::task::spawn_blocking(move || {
            let mut file = std::fs::File::create(path)?;

            bincode::encode_into_std_write(cpy, &mut file, bincode::config::standard())?;
            Ok::<(), DataStorageError>(())
        })
        .await??;

        self.remove_file_lock();
        Ok(())
    }

    /// Scans the project directory for projects that are not in the project list and adds them and
    /// for projects that are in the list but no longer exist
    pub async fn scan_for_missing_or_deleted_projects(
        &self,
        settings: &Settings,
    ) -> Result<(), DataStorageError> {
        let path = format!("{}/projects", settings.data_path);
        let project_list = self.data.projects.clone();
        let settings_clone = settings.clone();

        // 1. Get all project IDs from disk (blocking IO)
        let projects_on_disk: Vec<Uuid> = tokio::task::spawn_blocking(move || {
            let mut projects = Vec::new();
            if let Ok(dir) = std::fs::read_dir(path) {
                for entry in dir {
                    if let Ok(entry) = entry
                        && let Some(project_id) = entry.path().file_name()
                        && let Some(project_id) = project_id.to_str()
                        && let Ok(uuid) = uuid::Uuid::parse_str(project_id)
                    {
                        projects.push(uuid);
                    }
                }
            }
            projects
        })
        .await?;

        // 2. Find projects that are on disk but not in the list
        let mut new_projects = Vec::new();
        for uuid in &projects_on_disk {
            let exists = {
                let read_lock = project_list.read().unwrap();
                read_lock.has(uuid)
            };

            if !exists {
                // Get the project from disk to extract name
                let project_storage = ProjectStorage::new();
                if let Ok(project) = project_storage.get_project(uuid, &settings_clone).await {
                    let project_name = project.read().unwrap().name.clone();
                    new_projects.push(ProjectListProject {
                        id: *uuid,
                        name: project_name,
                        last_interaction: Utc::now().naive_utc(),
                    });
                }
            }
        }

        // 3. Update the project list
        {
            let mut write_lock = project_list.write().unwrap();

            // Add new projects
            for new_project in new_projects {
                if !write_lock.has(&new_project.id) {
                    write_lock
                        .entries
                        .push(ProjectListEntry::Project(new_project));
                }
            }

            // Remove deleted projects
            write_lock.entries.retain(|entry| {
                match entry {
                    // Only keep projects that also exist on disk
                    ProjectListEntry::Project(project) => projects_on_disk.contains(&project.id),
                    // Always keep folders
                    ProjectListEntry::Folder(_) => true,
                }
            });
        }

        Ok(())
    }

    pub async fn get_person_cloned(&self, id: &Uuid) -> Result<PersonV2, DataStorageError> {
        Ok(self
            .data
            .persons
            .get(id)
            .ok_or(DataStorageError::NotFound("person".to_string()))?
            .clone()
            .read()
            .unwrap()
            .clone())
    }

    pub async fn get_template_cloned(
        &self,
        id: &Uuid,
    ) -> Result<ProjectTemplateV2, DataStorageError> {
        Ok(self
            .data
            .templates
            .get(id)
            .ok_or(DataStorageError::NotFound("template".to_string()))?
            .clone()
            .read()
            .unwrap()
            .clone())
    }

    pub async fn insert_user(
        &self,
        user: User,
        settings: &Settings,
    ) -> Result<(), DataStorageError> {
        self.data
            .login_data
            .insert(user.id, Arc::new(RwLock::new(user)));
        self.save_to_disk(settings).await
    }

    pub async fn update_template_version_id(&self, template_id: Uuid) -> Result<(), ()> {
        if let Some(template) = self.data.templates.get(&template_id) {
            let mut template = template.write().unwrap();
            template.version = Some(Uuid::new_v4());
            Ok(())
        } else {
            Err(())
        }
    }

    pub fn person_exists(&self, id: &Uuid) -> bool {
        self.data.persons.contains_key(id)
    }

    pub async fn insert_template(
        &self,
        template: ProjectTemplateV2,
        settings: &Settings,
    ) -> Result<(), DataStorageError> {
        self.data
            .templates
            .insert(template.id, Arc::new(RwLock::new(template)));
        self.save_to_disk(settings).await
    }

    pub fn get_user(&self, email: &str) -> Result<Arc<RwLock<User>>, DataStorageError> {
        self.data
            .login_data
            .iter()
            .find(|x| x.value().read().unwrap().email == email)
            .map(|x| Arc::clone(x.value()))
            .ok_or(DataStorageError::NotFound("user".to_string()))
    }
}

impl SingleFileLock for DataStorage {
    fn get_file_lock(&self) -> &AtomicBool {
        &self.file_locked
    }
}

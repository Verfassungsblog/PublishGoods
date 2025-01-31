use hayagriva::types::{Date, Duration, DurationRange, FormatString, MaybeTyped, Numeric, NumericDelimiter, NumericValue, QualifiedUrl, SerialNumber};
use std::collections::{BTreeMap, HashMap};
use std::fs;

use std::path::Path;
use std::str::FromStr;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::sync::atomic::AtomicBool;
use std::time::SystemTime;
use argon2::{Argon2, PasswordHasher};
use argon2::password_hash::rand_core::OsRng;
use serde::{Deserialize, Serialize};
use bincode::{Encode, Decode};
use hayagriva::types::EntryType;
use vb_exchange::projects::*;
use crate::projects::{ProjectMetadata, Section, SectionOrToc};
use crate::projects::api::ApiError;
use crate::settings::Settings;
use reqwest::Url;

use unic_langid_impl::LanguageIdentifier;
use vb_exchange::export_formats::ExportFormat;

/// Storage for small data like users, passwords and login attempts
///
/// This data is stored in memory permanently and doesn't get unloaded
pub struct DataStorage{
    pub data: RwLock<InnerDataStorageV2>,
    file_locked: AtomicBool,
}

#[derive(Debug, Serialize, Deserialize, Encode, Decode, Clone)]
pub struct InnerDataStorageV1{
    /// HashMap with users, id as HashMap keys
    #[bincode(with_serde)]
    pub login_data: HashMap<uuid::Uuid, Arc<RwLock<User>>>,
    #[bincode(with_serde)]
    pub persons: HashMap<uuid::Uuid, Arc<RwLock<Person>>>,
    #[bincode(with_serde)]
    pub templates: HashMap<uuid::Uuid, Arc<RwLock<ProjectTemplateV1>>>
}

#[derive(Debug, Serialize, Deserialize, Encode, Decode, Clone)]
pub struct InnerDataStorageV2{
    /// HashMap with users, id as HashMap keys
    #[bincode(with_serde)]
    pub login_data: HashMap<uuid::Uuid, Arc<RwLock<User>>>,
    #[bincode(with_serde)]
    pub persons: HashMap<uuid::Uuid, Arc<RwLock<Person>>>,
    #[bincode(with_serde)]
    pub templates: HashMap<uuid::Uuid, Arc<RwLock<ProjectTemplateV2>>>
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

impl DataStorage{
    /// Creates a new empty [DataStorage]
    pub fn new() -> Self {
        DataStorage {
            data: RwLock::new(InnerDataStorageV2{
                login_data: Default::default(),
                persons: Default::default(),
                templates: Default::default(),
            }),
            file_locked: Default::default(),
        }
    }

    pub async fn update_template_version_id(&self, template_id: uuid::Uuid) -> Result<(), ()>{
        match self.data.read().unwrap().templates.get(&template_id){
            Some(template) => {
                let template = template.clone();
                template.write().unwrap().version = Some(uuid::Uuid::new_v4());
                Ok(())
            },
            None => {
                Err(())
            }
        }
    }



    pub async fn insert_template(&self, template: ProjectTemplateV2, settings: &Settings) -> Result<(), ()>{
        // Create template directory inside data if it doesn't exist
        if !Path::new(&format!("{}/templates/{}", settings.data_path, template.id)).exists(){
            if let Err(e) =  tokio::fs::create_dir_all(&format!("{}/templates/{}/assets", settings.data_path, template.id)).await{
                eprintln!("error while creating template directory: {}", e);
                return Err(())
            }
            if let Err(e) =  tokio::fs::create_dir_all(&format!("{}/templates/{}/formats", settings.data_path, template.id)).await{
                eprintln!("error while creating template directory: {}", e);
                return Err(())
            }
        }
        self.data.write().unwrap().templates.insert(template.id.clone(), Arc::new(RwLock::new(template)));
        self.save_to_disk(settings).await?;
        Ok(())
    }

    /// inserts a new user into the [DataStorage]
    pub async fn insert_user(&self, user: User, settings: &Settings) -> Result<(), ()>{
        self.data.write().unwrap().login_data.insert(user.id.clone(), Arc::new(RwLock::new(user)));
        self.save_to_disk(settings).await?;
        Ok(())
    }

    /// returns a user from the [DataStorage] as [Arc<RwLock<User>>]
    pub fn get_user(&self, email: &String) -> Result<Arc<RwLock<User>>, ()>{
        let data = self.data.read().unwrap();
        match data.login_data.values().find(|user| user.read().unwrap().email == *email){
            Some(user) => Ok(Arc::clone(user)),
            None => Err(()),
        }
    }

    /// Get person by id
    /// Returns a [Person] as [Arc<RwLock<Person>>] if the person exists
    pub fn get_person(&self, uuid: &uuid::Uuid) -> Option<Arc<RwLock<Person>>>{
        match self.data.read().unwrap().persons.get(uuid){
            None => None,
            Some(data) => Some(Arc::clone(data))
        }
    }

    /// Check if person exists
    pub fn person_exists(&self, uuid: &uuid::Uuid) -> bool{
        self.data.read().unwrap().persons.contains_key(uuid)
    }

    /// Loads the [DataStorage] from disk
    ///
    /// Path is defined as data_path from settings + /data.version.bincode
    pub async fn load_from_disk(settings: &Settings) -> Result<Self, ()>{
        let mut data_storage = DataStorage::new();

        let path = format!("{}", settings.data_path);

        let res = rocket::tokio::task::spawn_blocking(move || {
            let files = match std::fs::read_dir(&path){
                Ok(files) => files,
                Err(e) => {
                    eprintln!("io error while loading data_storage directory: {}. Check that your data_path is set correctly and we have sufficient file permissions.", e);
                    return Err(())
                }
            };

            let mut file_versions: Vec<(u64, String)> = vec![];

            // Iterate through dir entries and find all data files with version number
            for file in files{
                match file{
                    Ok(file) => {
                        if let Ok(file_type) = file.file_type(){
                            if file_type.is_file(){
                                let fname = file.file_name().clone();
                                let fname = fname.to_str().unwrap_or("");
                                let parts: Vec<&str> = fname.split(".").collect();

                                // First part = "data"
                                // Second part = version
                                // Third part = "bincode"

                                if parts.len() == 3 && parts[0] == "data"{
                                    // parse version as usize
                                    let version = match parts[1].parse::<u64>(){
                                        Ok(version) => version,
                                        Err(e) => {
                                            eprintln!("error while loading data_storage file into memory: couldn't parse version number: {}. Skipping file.", e);
                                            continue
                                        }
                                    };

                                    file_versions.push((version, fname.to_string()));
                                }
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("io error while loading data_storage directory entry: {}. Skipping file.", e);
                        continue
                    }
                }
            }

            // Order file versions
            file_versions.sort_by(|a, b| a.0.cmp(&b.0));

            // Load the latest version of the data storage
            if file_versions.is_empty(){
                eprintln!("error while loading data storage into memory: no storage files found in data directory.");
                return Err(())
            }
            let (version, file_path) = file_versions.last().unwrap();
            if *version == 1 {
                // Load old data format
                let mut file = match std::fs::File::open(format!("{}/{}", &path, file_path)) {
                    Ok(file) => file,
                    Err(e) => {
                        eprintln!("io error while loading data file into memory: {}", e);
                        return Err(())
                    },
                };

                println!("Found old data Format v1. Converting to v2. Moving templates/ to templates-old/!");

                // Move told templates directory to templates-old
                if Path::exists(Path::new(&format!("{}/templates", &path))){
                    if let Err(e) = fs::rename(format!("{}/templates", &path), format!("{}/templates-old", &path)){
                        eprintln!("error while moving templates directory to templates-old: {}", e);
                        return Err(())
                    }
                    if let Err(e) = std::fs::create_dir_all(format!("{}/templates", &path)){
                        eprintln!("error while creating new templates directory: {}", e);
                        return Err(())
                    }
                }

                match bincode::decode_from_std_read::<InnerDataStorageV1, _, _>(&mut file, bincode::config::standard()) {
                    Ok(data) => return Ok(InnerDataStorageV2::from(data)),
                    Err(e) => {
                        eprintln!("bincode decode error while loading data storage with version {} into memory: {}.", version, e);
                        return Err(())
                    },
                };
            }else if *version == 2 {
                // Load new project format
                let mut file = match std::fs::File::open(format!("{}/{}", &path, file_path)) {
                    Ok(file) => file,
                    Err(e) => {
                        eprintln!("io error while loading project file into memory: {}", e);
                        return Err(())
                    },
                };
                let project = match bincode::decode_from_std_read(&mut file, bincode::config::standard()) {
                    Ok(data) => data,
                    Err(e) => {
                        eprintln!("bincode decode error while loading data storage file with version {} into memory: {}.", version, e);
                        return Err(())
                    },
                };
                return Ok(project);
            }else{
                eprintln!("error while loading data storage into memory: unknown file version {}.", version);
                return Err(())
            }
        }).await;

        match res {
            Ok(data) => {
                match data{
                    Ok(data) => {
                        data_storage.data = RwLock::new(data);
                        Ok(data_storage)
                    },
                    Err(_) => {
                        return Err(())
                    }
                }
            },
            Err(_) => {
                Err(())
            }
        }
    }

    /// Saves the [DataStorage] to disk
    ///
    /// Creates a whole copy of the [DataStorage] and saves it to disk
    /// This may use a lot of memory, maybe change this in the future if it becomes a problem
    pub async fn save_to_disk(&self, settings: &Settings) -> Result<(), ()>{
        self.wait_for_file_lock(settings).await?;

        // Save login data
        let cpy = self.data.read().unwrap().clone();
        let path = format!("{}/data.2.bincode", settings.data_path);

        match rocket::tokio::task::spawn_blocking(move || {
            let mut file = match std::fs::File::create(path) {
                Ok(file) => file,
                Err(e) => {
                    eprintln!("io error while saving data to disk: {}", e);
                    return Err(())
                },
            };

            return match bincode::encode_into_std_write(cpy, &mut file, bincode::config::standard()) {
                Ok(_) => Ok(()),
                Err(e) => {
                    eprintln!("bincode encode error while saving data to disk: {}", e);
                    Err(())
                },
            }
        }).await{
            Ok(res) => res?,
            Err(e) => {
                eprintln!("error while saving data to disk: {}", e);
                return Err(())
            }
        };

        self.remove_file_lock();
        Ok(())
    }
}

impl SingleFileLock for DataStorage{
    fn get_file_lock(&self) -> &AtomicBool {
        &self.file_locked
    }
}

/// Trait for data structures that need to handle multiple file locks
///
/// Struct needs to implement [MultipleFileLocks::get_file_lock_entry]
#[async_trait]
pub trait MultipleFileLocks{
    /// Returns an [AtomicBool] that is used as file lock for the given uuid
    fn get_file_lock_entry(&self, uuid: &uuid::Uuid) -> Arc<AtomicBool>;
    /// Creates a file lock for the given uuid
    fn create_file_lock(&self, uuid: &uuid::Uuid) -> Result<(), ()> {
        if self.get_file_lock_entry(uuid).load(std::sync::atomic::Ordering::SeqCst) {
            // file already locked
            Err(())
        }else{
            // file not locked, lock it
            self.get_file_lock_entry(uuid).store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
    }
    fn remove_file_lock(&self, uuid: &uuid::Uuid){
        self.get_file_lock_entry(uuid).store(false, std::sync::atomic::Ordering::SeqCst);
    }

    async fn wait_for_file_lock(&self, uuid: &uuid::Uuid, settings: &Settings) -> Result<(), ()>{
        let mut time_waited = 0;
        while self.create_file_lock(uuid).is_err() {
            time_waited += 10;
            if time_waited > settings.file_lock_timeout {
                eprintln!("error while waiting for file lock: waiting for file lock timed out. Waited for {} ms, exceeding the configured limit.", time_waited);
                return Err(())
            }

            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        Ok(())
    }
}

#[async_trait]
pub trait SingleFileLock {
    fn get_file_lock(&self) -> &AtomicBool;

    fn create_file_lock(& self) -> Result<(), ()> {
        if self.get_file_lock().load(std::sync::atomic::Ordering::SeqCst) {
            // file already locked
            Err(())
        } else {
            // file not locked, lock it
            self.get_file_lock().store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
    }

    fn remove_file_lock(&self){
        self.get_file_lock().store(false, std::sync::atomic::Ordering::SeqCst);
    }

    async fn wait_for_file_lock(&self, settings: &Settings) -> Result<(), ()>{
        let mut time_waited = 0;
        while self.create_file_lock().is_err() {
            time_waited += 10;
            if time_waited > settings.file_lock_timeout {
                eprintln!("error while waiting for file lock: waiting for file lock timed out. Waited for {} ms, exceeding the configured limit.", time_waited);
                return Err(())
            }

            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        Ok(())
    }
}


/// Storage for all projects, gets build on startup based on project files in data_path
#[derive(Debug, Serialize, Deserialize)]
pub struct ProjectStorage {
    /// HashMap with project uuid and project data if project is already loaded into memory
    pub projects: RwLock<HashMap<uuid::Uuid, ProjectStorageEntry>>,
    pub file_locks: RwLock<HashMap<uuid::Uuid, Arc<AtomicBool>>>,
}

#[derive(Debug, Serialize, Deserialize, Encode, Decode, Clone)]
pub struct ProjectStorageEntry{
    pub name: String,
    pub data: Option<Arc<RwLock<ProjectDataV4>>>,
}

impl MultipleFileLocks for ProjectStorage{
    fn get_file_lock_entry(&self, uuid: &uuid::Uuid) -> Arc<AtomicBool> {
        if let Some(entry) =  self.file_locks.read().unwrap().get(uuid){
            return entry.clone()
        }
        // Create new entry
        self.file_locks.write().unwrap().insert(uuid.clone(), Arc::new(AtomicBool::new(false)));
        return self.file_locks.read().unwrap().get(uuid).unwrap().clone()
    }
}

impl ProjectStorage {
    /// Creates a new empty [ProjectStorage]
    pub fn new() -> Self {
        ProjectStorage {
            projects: RwLock::new(HashMap::new()),
            file_locks: Default::default(),
        }
    }

    /// Unloads all unused projects from memory
    ///
    /// Checks if projects last interaction time is older than project_cache_time defined in config
    /// Saves the project to disk before unloading it
    pub async fn unload_unused_projects(&mut self, settings: &Settings) -> Result<(), ()>{
        let mut projects_to_unload = vec![];

        for (uuid, project_data) in self.projects.read().unwrap().iter(){
            if let Some(project) = &project_data.data{
                let now =  SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs();
                if Arc::strong_count(&project) == 1 && project.read().unwrap().last_interaction + settings.project_cache_time < now{
                    projects_to_unload.push(uuid.clone());
                }
            }
        }

        for project in projects_to_unload{
            self.save_project_to_disk(&project, settings).await?;
            self.unload_project(&project)?;
        }

        Ok(())
    }

    /// Unloads a project from memory
    ///
    /// Does not save the project to disk, use [ProjectStorage::save_project_to_disk] for that
    fn unload_project(&self, uuid: &uuid::Uuid) -> Result<(), ()>{
        match self.projects.write().unwrap().get_mut(uuid){
            Some(project) => {
                project.data = None;
                println!("Unloaded project {} from memory.", uuid);
                Ok(())
            },
            None => {
                eprintln!("Requested to unload project {}, but no project exists.", uuid);
                Err(())
            },
        }
    }

    /// Loads a list of all projects from the projects directory inside the data_path
    /// Does not load the projects into memory
    ///
    /// # Returns
    /// * `ProjectStorage` - [ProjectStorage] with all projects uuids and None as project data
    pub async fn load_from_directory(&self, settings: &Settings) -> Result<(), ()> {
        // Get all project uuids
        let paths = match std::fs::read_dir(format!("{}/projects/", settings.data_path)) {
            Ok(paths) => paths,
            Err(e) => {
                eprintln!("io error while loading project directory: {}. Check that your data_path is set correctly and we have sufficient file permissions.", e);
                return Err(())
            }
        };

        for path in paths {
            match path {
                Ok(entry) => {
                    // Skip non directory entries
                    if !entry.path().is_dir(){
                        continue
                    }
                    match entry.file_name().to_str() {
                        Some(uuid) => {
                            match uuid.parse::<uuid::Uuid>() {
                                Ok(uuid) => {
                                    println!("Loading project {}.", uuid);
                                    match self.load_project_into_memory(&uuid, settings).await{
                                        Ok(_) => {
                                            println!("Successfully loaded project {} into memory.", uuid);
                                            if let Err(_) = self.unload_project(&uuid){
                                                eprintln!("error while unloading project {} after loading it into memory. Skipping project.", uuid);
                                                continue
                                            }
                                            println!("Project storage now contains: {:?}", self.projects.read().unwrap().keys());
                                        },
                                        Err(_) => {
                                            eprintln!("error while loading project {} into memory. Skipping project.", uuid);
                                            continue
                                        }
                                    }
                                },
                                Err(e) => {
                                    eprintln!("error while parsing project directory entry name into uuid: {}, Skipping project.", e);
                                    continue
                                }
                            }
                        },
                        None => {
                            eprintln!("error while parsing project directory entry: {:?}, Skipping project.", entry.file_name());
                            continue
                        }
                    };
                }
                Err(e) => {
                    eprintln!("io error while loading project directory entry: {}, Skipping project.", e);
                    continue
                }
            }
        }

        Ok(())
    }

    /// Insert new project into [ProjectStorage]
    ///
    /// Also calls [ProjectStorage::save_project_to_disk] to save the project to disk
    ///
    /// # Arguments
    /// * `project` - [OldProjectData] - Project to insert
    ///
    /// # Returns
    /// * `Ok(uuid::Uuid)` - Project inserted successfully - returns the generated [uuid::Uuid] of the project
    pub async fn insert_project(&self, mut project: ProjectDataV4, settings: &Settings) -> Result<uuid::Uuid, ()> {
        let uuid = uuid::Uuid::new_v4();

        // Update last edited to current time, so the project doesn't get unloaded immediately
        project.last_interaction = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs();
        let entry = ProjectStorageEntry{
            name: project.name.clone(),
            data: Some(Arc::new(RwLock::new(project))),
        };
        self.projects.write().unwrap().insert(uuid,entry);
        self.save_project_to_disk(&uuid, settings).await?;
        Ok(uuid)
    }

    async fn load_project_into_memory(&self, uuid: &uuid::Uuid, settings: &Settings) -> Result<(), ()> {
        // Try to load project file from disk
        //let path = format!("{}/projects/{}/project.bincode", settings.data_path, uuid);
        let npath = format!("{}/projects/{}", settings.data_path, uuid);

        println!("Aquiring file lock for project {}.", uuid);
        match self.wait_for_file_lock(uuid, settings).await{
            Ok(_) => {},
            Err(_) => {
                eprintln!("error while saving project to disk: couldn't get file lock");
                return Err(())
            }
        }

        println!("Loading project {} into memory.", uuid);

        let res = rocket::tokio::task::spawn_blocking(move || {
            // Load a list of all files in project directory
            match fs::read_dir(&npath){
                Ok(paths) => {
                    let mut project_versions : Vec<(u64, String)> = vec![];

                    for path in paths {
                        match path {
                            Ok(entry) => {
                                let fname = entry.file_name().clone();
                                let fname = fname.to_str().unwrap_or("");
                                let parts: Vec<&str> = fname.split(".").collect();
                                // First part = "project"
                                // Second part = version
                                // Third part = "bincode"

                                if parts.len() == 3 && parts[0] == "project"{
                                    // parse version as usize
                                    let version = match parts[1].parse::<u64>(){
                                        Ok(version) => version,
                                        Err(e) => {
                                            eprintln!("error while loading project into memory: couldn't parse version number: {}. Skipping file.", e);
                                            continue
                                        }
                                    };

                                    project_versions.push((version, fname.to_string()));
                                }
                            },
                            Err(e) => {
                                eprintln!("io error while loading project directory entry: {}, Skipping project.", e);
                                return Err(())
                            }
                        }
                    }

                    // Sort project versions by version number
                    project_versions.sort_by(|a, b| a.0.cmp(&b.0));

                    // Load the latest version of the project
                    if project_versions.is_empty(){
                        eprintln!("error while loading project into memory: no project files found in project directory. Skipping project.");
                        return Err(())
                    }
                    let (version, project_path) = project_versions.last().unwrap();
                    if *version == 1 {
                        // Load old project format
                        let mut file = match std::fs::File::open(format!("{}/{}", &npath, project_path)) {
                            Ok(file) => file,
                            Err(e) => {
                                eprintln!("io error while loading project file into memory: {}", e);
                                return Err(())
                            },
                        };
                        match bincode::decode_from_std_read::<OldProjectData, _, _>(&mut file, bincode::config::standard()) {
                            Ok(project) => return Ok(ProjectDataV4::from(ProjectDataV3::from(ProjectDataV2::from(project)))),
                            Err(e) => {
                                eprintln!("bincode decode error while loading project file with version {} into memory: {}.", version, e);
                                return Err(())
                            },
                        };
                    }else if *version == 2 {
                        // Load old project format
                        let mut file = match std::fs::File::open(format!("{}/{}", &npath, project_path)) {
                            Ok(file) => file,
                            Err(e) => {
                                eprintln!("io error while loading project file into memory: {}", e);
                                return Err(())
                            },
                        };
                        match bincode::decode_from_std_read::<ProjectDataV2, _, _>(&mut file, bincode::config::standard()) {
                            Ok(project) => return Ok(ProjectDataV4::from(ProjectDataV3::from(project))),
                            Err(e) => {
                                eprintln!("bincode decode error while loading project file with version {} into memory: {}.", version, e);
                                return Err(())
                            },
                        };
                    }else if *version == 3 {
                        // Load old project format
                        let mut file = match std::fs::File::open(format!("{}/{}", &npath, project_path)) {
                            Ok(file) => file,
                            Err(e) => {
                                eprintln!("io error while loading project file into memory: {}", e);
                                return Err(())
                            },
                        };
                        match bincode::decode_from_std_read::<ProjectDataV3, _, _>(&mut file, bincode::config::standard()) {
                            Ok(project) => return Ok(ProjectDataV4::from(project)),
                            Err(e) => {
                                eprintln!("bincode decode error while loading project file with version {} into memory: {}.", version, e);
                                return Err(())
                            },
                        };
                    }else if *version == 4 {
                        // Load new project format
                        let mut file = match std::fs::File::open(format!("{}/{}", &npath, project_path)) {
                            Ok(file) => file,
                            Err(e) => {
                                eprintln!("io error while loading project file into memory: {}", e);
                                return Err(())
                            },
                        };
                        let project: ProjectDataV4 = match bincode::decode_from_std_read(&mut file, bincode::config::standard()) {
                            Ok(project) => project,
                            Err(e) => {
                                eprintln!("bincode decode error while loading project file with version {} into memory: {}.", version, e);
                                return Err(())
                            },
                        };
                        return Ok(project);
                    }else{
                        eprintln!("error while loading project into memory: unknown project version {}. Skipping project.", version);
                        return Err(())
                    }
                },
                Err(e) => {
                    eprintln!("io error while loading project directory: {}. Check that your data_path is set correctly and we have sufficient file permissions.", e);
                    return Err(())
                }
            }
        }).await;

        println!("Read complete. Releasing file lock for project {}.", uuid);

        self.remove_file_lock(uuid);

        println!("File lock released for project {}.", uuid);

        match res{
            Ok(project) => {
                match project {
                    Ok(project) => {
                        println!("Loaded project, inserting into memory storage.");
                        if let Some(tproject) = self.projects.write().unwrap().get_mut(uuid){
                                // Update last edited to current time, so the project doesn't get unloaded immediately
                                let mut project: ProjectDataV4 = project;
                                project.last_interaction = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs();
                                tproject.name = project.name.clone();
                                println!("Replacing project");
                                tproject.data.replace(Arc::new(RwLock::new(project)));
                                println!("Inserted project into memory storage.");
                                return Ok(())
                        }

                        println!("Project not found in memory storage, creating new entry.");
                        let entry = ProjectStorageEntry{
                            name: project.name.clone(),
                            data: Some(Arc::new(RwLock::new(project))),
                        };
                        self.projects.write().unwrap().insert(uuid.clone(), entry);
                        println!("Created new entry in memory storage.");
                        Ok(())
                    },
                    Err(_) => Err(()),
                }
            },
            Err(e) => {
                eprintln!("error while loading project file into memory: {}", e);
                Err(())
            }
        }
    }

    pub async fn get_project(&self, uuid: &uuid::Uuid, settings: &Settings) -> Result<Arc<RwLock<ProjectDataV4>>, ()> {
        // Check if project exists
        match self.projects.read().unwrap().get(uuid) {
            Some(project) => {
                if let Some(project) = &project.data {
                    project.write().unwrap().last_interaction = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs();
                    return Ok(Arc::clone(project));
                }
            },
            None => return Err(()),
        }

        // Project doesn't exist in memory, try to load from disk
        self.load_project_into_memory(uuid, settings).await?;

        // Check if project exists
        match self.projects.read().unwrap().get(uuid) {
            Some(project) => {
                match &project.data {
                    None => {
                        //Still no project in memory, couldn't load from disk
                        Err(())
                    }
                    Some(project) => {
                        // Update last interaction time, so the project doesn't get unloaded immediately
                        project.write().unwrap().last_interaction = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs();
                        Ok(Arc::clone(project))
                    },
                }
            },
            None => return Err(()),
        }
    }

    async fn save_project_to_disk(&self, uuid: &uuid::Uuid, settings: &Settings) -> Result<(), ()> {
        // Get project
        let project = match self.projects.read().unwrap().get(&uuid) {
            Some(project) => {
                match &project.data {
                    Some(project) => project.clone(),
                    None => return Err(()),
                }
            },
            None => return Err(()),
        };
        match fs::create_dir(format!("{}/projects/{}", settings.data_path, uuid)){
            Ok(_) => {},
            Err(e) => {
                if e.kind() != std::io::ErrorKind::AlreadyExists {
                    eprintln!("io error while creating project directory: {}", e);
                    return Err(())
                }
            }
        }

        let version = "4"; //TODO: auto detect latest version

        // Encode project data with bincode and save to disk
        let path = format!("{}/projects/{}/project.{}.bincode", settings.data_path, uuid, version);

        match self.wait_for_file_lock(&uuid, settings).await{
            Ok(_) => {},
            Err(_) => {
                eprintln!("error while saving project to disk: couldn't get file lock");
                return Err(())
            }
        }

        //TODO: do not use spawn_blocking, but use tokio fs functions
        let res = rocket::tokio::task::spawn_blocking(move || {
            let mut file = match std::fs::File::create(path) {
                Ok(file) => file,
                Err(e) => {
                    eprintln!("io error while saving project to disk: {}", e);
                    return Err(())
                },
            };
            // Clone project data to avoid locking the project while saving
            let pcopy = project.read().unwrap().clone();
            match bincode::encode_into_std_write(&pcopy, &mut file, bincode::config::standard()) {
                Ok(_) => Ok(()),
                Err(e) => {
                    eprintln!("bincode encode error while saving project to disk: {}", e);
                    Err(())
                },
            }
        }).await;

        self.remove_file_lock(uuid);
        match res{
            Ok(_) => Ok(()),
            Err(e) => {
                eprintln!("error while saving project to disk: {}", e);
                Err(())
            }
        }
    }
}

#[derive(Serialize, Deserialize)]
pub enum ProjectData{
    V1(OldProjectData),
    V2(ProjectDataV2),
    V3(ProjectDataV3),
    V4(ProjectDataV4)
}

#[derive(Debug, Serialize, Deserialize, Encode, Decode, Clone)]
pub struct OldProjectData {
    pub name: String,
    pub description: Option<String>,
    #[bincode(with_serde)]
    pub template_id: uuid::Uuid,
    pub last_interaction: u64,
    pub metadata: Option<ProjectMetadata>,
    pub settings: Option<ProjectSettingsV2>,
    pub sections: Vec<SectionOrToc>,
    #[bincode(with_serde)]
    pub bibliography: HashMap<String, OldBibEntry>
}


#[derive(Debug, Serialize, Deserialize, Encode, Decode, Clone)]
pub struct ProjectDataV2 {
    pub name: String,
    pub description: Option<String>,
    #[bincode(with_serde)]
    pub template_id: uuid::Uuid,
    pub last_interaction: u64,
    pub metadata: Option<ProjectMetadata>,
    pub settings: Option<ProjectSettingsV2>,
    pub sections: Vec<SectionOrToc>,
    #[bincode(with_serde)]
    pub bibliography: HashMap<String, BibEntryV2>
}

#[derive(Debug, Serialize, Deserialize, Encode, Decode, Clone)]
pub struct ProjectDataV3 {
    pub name: String,
    pub description: Option<String>,
    #[bincode(with_serde)]
    pub template_id: uuid::Uuid,
    pub last_interaction: u64,
    pub metadata: Option<ProjectMetadata>,
    pub settings: Option<ProjectSettingsV3>,
    pub sections: Vec<SectionOrToc>,
    #[bincode(with_serde)]
    pub bibliography: HashMap<String, BibEntryV2>
}

#[derive(Debug, Serialize, Deserialize, Encode, Decode, Clone)]
pub struct ProjectDataV4 {
    pub name: String,
    pub description: Option<String>,
    #[bincode(with_serde)]
    pub template_id: uuid::Uuid,
    pub last_interaction: u64,
    pub metadata: Option<ProjectMetadata>,
    pub settings: Option<ProjectSettingsV4>,
    pub sections: Vec<SectionOrToc>,
    #[bincode(with_serde)]
    pub bibliography: HashMap<String, BibEntryV2> //TODO: add prefix & suffix support
}

impl From<OldProjectData> for ProjectDataV2{
    fn from(value: OldProjectData) -> Self {
        ProjectDataV2{
            name: value.name,
            description: value.description,
            template_id: value.template_id,
            last_interaction: value.last_interaction,
            metadata: value.metadata,
            settings: value.settings,
            sections: value.sections,
            bibliography: value.bibliography.iter().map(|(k, v)| (k.clone(), v.clone().into())).collect(),
        }
    }
}

impl From<ProjectDataV2> for ProjectDataV3{
    fn from(value: ProjectDataV2) -> Self {
        let settings: Option<ProjectSettingsV3> = match value.settings{
            Some(set) => {
                Some(ProjectSettingsV3{
                    toc_enabled: set.toc_enabled,
                    csl_style: set.csl_style,
                    csl_language_code: None,
                })
            },
            None => None,
        };
        ProjectDataV3{
            name: value.name,
            description: value.description,
            template_id: value.template_id,
            last_interaction: value.last_interaction,
            metadata: value.metadata,
            settings,
            sections: value.sections,
            bibliography: value.bibliography.iter().map(|(k, v)| (k.clone(), v.clone().into())).collect(),
        }
    }
}

impl From<ProjectDataV3> for ProjectDataV4{
    fn from(value: ProjectDataV3) -> Self {
        let settings: Option<ProjectSettingsV4> = match value.settings{
            Some(set) => {
                Some(ProjectSettingsV4{
                    toc_enabled: set.toc_enabled,
                    csl_style: set.csl_style,
                    csl_language_code: set.csl_language_code,
                    metadata_page_additional_html: None,
                    cover_image_path: None,
                    backcover_image_path: None,
                })
            },
            None => None,
        };
        ProjectDataV4{
            name: value.name,
            description: value.description,
            template_id: value.template_id,
            last_interaction: value.last_interaction,
            metadata: value.metadata,
            settings,
            sections: value.sections,
            bibliography: value.bibliography,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct MyPersonsWithRoles {
    /// The persons.
    pub names: Vec<MyPerson>,
    /// The role the persons had in the creation of the cited item.
    pub role: MyPersonRole,
}

impl From<hayagriva::types::PersonsWithRoles> for MyPersonsWithRoles{
    fn from(value: hayagriva::types::PersonsWithRoles) -> Self {
        MyPersonsWithRoles{
            names: value.names.iter().map(|p| <hayagriva::types::Person as Clone>::clone(&(*p)).into()).collect(),
            role: value.role.into(),
        }
    }
}

impl From<MyPersonsWithRoles> for hayagriva::types::PersonsWithRoles{
    fn from(value: MyPersonsWithRoles) -> Self {
        hayagriva::types::PersonsWithRoles{
            names: value.names.iter().map(|p| <MyPerson as Clone>::clone(&(*p)).into()).collect(),
            role: value.role.into(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MyPersonRole {
    /// Translated the work from a foreign language to the cited edition.
    Translator,
    /// Authored an afterword.
    Afterword,
    /// Authored an foreword.
    Foreword,
    /// Authored an introduction.
    Introduction,
    /// Provided value-adding annotations.
    Annotator,
    /// Commented the work.
    Commentator,
    /// Holds a patent or similar.
    Holder,
    /// Compiled the works in an [Anthology](super::EntryType::Anthology).
    Compiler,
    /// Founded the publication.
    Founder,
    /// Collaborated on the cited item.
    Collaborator,
    /// Organized the creation of the cited item.
    Organizer,
    /// Performed in the cited item.
    CastMember,
    /// Composed all or parts of the cited item's musical / audible components.
    Composer,
    /// Produced the cited item.
    Producer,
    /// Lead Producer for the cited item.
    ExecutiveProducer,
    /// Did the writing for the cited item.
    Writer,
    /// Shot film/video for the cited item.
    Cinematography,
    /// Directed the cited item.
    Director,
    /// Illustrated the cited item.
    Illustrator,
    /// Provided narration or voice-over for the cited item.
    Narrator,
    /// Various other roles described by the contained string.
    Unknown(String),
}

impl From<MyPersonRole> for hayagriva::types::PersonRole{
    fn from(value: MyPersonRole) -> Self {
        match value {
            MyPersonRole::Translator => hayagriva::types::PersonRole::Translator,
            MyPersonRole::Afterword => hayagriva::types::PersonRole::Afterword,
            MyPersonRole::Foreword => hayagriva::types::PersonRole::Foreword,
            MyPersonRole::Introduction => hayagriva::types::PersonRole::Introduction,
            MyPersonRole::Annotator => hayagriva::types::PersonRole::Annotator,
            MyPersonRole::Commentator => hayagriva::types::PersonRole::Commentator,
            MyPersonRole::Holder =>hayagriva::types::PersonRole::Holder,
            MyPersonRole::Compiler =>hayagriva::types::PersonRole::Compiler,
            MyPersonRole::Founder =>hayagriva::types::PersonRole::Founder,
            MyPersonRole::Collaborator =>hayagriva::types::PersonRole::Collaborator,
            MyPersonRole::Organizer =>hayagriva::types::PersonRole::Organizer,
            MyPersonRole::CastMember =>hayagriva::types::PersonRole::CastMember,
            MyPersonRole::Composer =>hayagriva::types::PersonRole::Composer,
            MyPersonRole::Producer =>hayagriva::types::PersonRole::Producer,
            MyPersonRole::ExecutiveProducer =>hayagriva::types::PersonRole::ExecutiveProducer,
            MyPersonRole::Writer =>hayagriva::types::PersonRole::Writer,
            MyPersonRole::Cinematography =>hayagriva::types::PersonRole::Cinematography,
            MyPersonRole::Director =>hayagriva::types::PersonRole::Director,
            MyPersonRole::Illustrator =>hayagriva::types::PersonRole::Illustrator,
            MyPersonRole::Narrator =>hayagriva::types::PersonRole::Narrator,
            MyPersonRole::Unknown(s) =>hayagriva::types::PersonRole::Unknown(s),
        }
    }
}

impl From<hayagriva::types::PersonRole> for MyPersonRole{
    fn from(value: hayagriva::types::PersonRole) -> Self {
        match value {
           hayagriva::types::PersonRole::Translator => MyPersonRole::Translator,
           hayagriva::types::PersonRole::Afterword => MyPersonRole::Afterword,
           hayagriva::types::PersonRole::Foreword => MyPersonRole::Foreword,
           hayagriva::types::PersonRole::Introduction => MyPersonRole::Introduction,
           hayagriva::types::PersonRole::Annotator => MyPersonRole::Annotator,
           hayagriva::types::PersonRole::Commentator => MyPersonRole::Commentator,
           hayagriva::types::PersonRole::Holder => MyPersonRole::Holder,
           hayagriva::types::PersonRole::Compiler => MyPersonRole::Compiler,
           hayagriva::types::PersonRole::Founder => MyPersonRole::Founder,
           hayagriva::types::PersonRole::Collaborator => MyPersonRole::Collaborator,
           hayagriva::types::PersonRole::Organizer => MyPersonRole::Organizer,
           hayagriva::types::PersonRole::CastMember => MyPersonRole::CastMember,
           hayagriva::types::PersonRole::Composer => MyPersonRole::Composer,
           hayagriva::types::PersonRole::Producer => MyPersonRole::Producer,
           hayagriva::types::PersonRole::ExecutiveProducer => MyPersonRole::ExecutiveProducer,
           hayagriva::types::PersonRole::Writer => MyPersonRole::Writer,
           hayagriva::types::PersonRole::Cinematography => MyPersonRole::Cinematography,
           hayagriva::types::PersonRole::Director => MyPersonRole::Director,
           hayagriva::types::PersonRole::Illustrator => MyPersonRole::Illustrator,
           hayagriva::types::PersonRole::Narrator => MyPersonRole::Narrator,
           hayagriva::types::PersonRole::Unknown(s) => MyPersonRole::Unknown(s),
            _ => MyPersonRole::Unknown("".to_string()),
        }
    }
}


/// Same as [MaybeTyped], but without serde untagged, because Bincode doesn't support this
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, Eq, Hash)]
pub enum MyMaybeTyped<T> {
    /// The typed variant.
    Typed(T),
    /// The fallback string variant.
    String(String),
}

impl<T> From<MyMaybeTyped<T>> for hayagriva::types::MaybeTyped<T> {
    fn from(value: MyMaybeTyped<T>) -> Self {
        match value {
            MyMaybeTyped::Typed(t) => hayagriva::types::MaybeTyped::Typed(t),
            MyMaybeTyped::String(s) => hayagriva::types::MaybeTyped::String(s),
        }
    }
}

impl<T> From<hayagriva::types::MaybeTyped<T>> for MyMaybeTyped<T> {
    fn from(value: hayagriva::types::MaybeTyped<T>) -> Self {
        match value {
            hayagriva::types::MaybeTyped::Typed(t) => MyMaybeTyped::Typed(t),
            hayagriva::types::MaybeTyped::String(s) => MyMaybeTyped::String(s),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, )]
pub struct MyPerson{
    pub name: String,
    /// The given name / forename.
    pub given_name: Option<String>,
    /// A prefix of the family name such as 'van' or 'de'.
    pub prefix: Option<String>,
    /// A suffix of the family name such as 'Jr.' or 'IV'.
    pub suffix: Option<String>,
    /// Another name (often user name) the person might be known under.
    pub alias: Option<String>,
}

impl From<hayagriva::types::Person> for MyPerson{
    fn from(value: hayagriva::types::Person) -> Self {
        MyPerson{
            name: value.name,
            given_name: value.given_name,
            prefix: value.prefix,
            suffix: value.suffix,
            alias: value.alias,
        }
    }
}

impl From<MyPerson> for hayagriva::types::Person{
    fn from(value: MyPerson) -> Self {
        hayagriva::types::Person{
            name: value.name,
            given_name: value.given_name,
            prefix: value.prefix,
            suffix: value.suffix,
            alias: value.alias,
        }
    }
}


/// Struct similar to [hayagriva::Entry], but without special serde annotations, since Bincode doesn't support these
/// For convenience, the struct implements [From] and [Into] for [hayagriva::Entry] and reverse
#[derive(Debug, Serialize, Deserialize, Encode, Decode, Clone)]
pub struct OldBibEntry {
    pub key: String,
    #[bincode(with_serde)]
    pub entry_type: EntryType,
    #[bincode(with_serde)]
    pub title: Option<MyFormatString>,
    #[bincode(with_serde)]
    pub authors: Vec<MyPerson>,
    #[bincode(with_serde)]
    pub date: Option<MyDate>,
    #[bincode(with_serde)]
    pub editors: Vec<MyPerson>,
    #[bincode(with_serde)]
    pub affiliated: Vec<MyPersonsWithRoles>,
    #[bincode(with_serde)]
    pub publisher: Option<MyFormatString>,
    #[bincode(with_serde)]
    pub location: Option<MyFormatString>,
    #[bincode(with_serde)]
    pub organization: Option<MyFormatString>,
    #[bincode(with_serde)]
    pub issue: Option<MyMaybeTyped<MyNumeric>>,
    #[bincode(with_serde)]
    pub volume: Option<MyMaybeTyped<MyNumeric>>,
    #[bincode(with_serde)]
    pub volume_total: Option<MyNumeric>,
    #[bincode(with_serde)]
    pub edition: Option<MyMaybeTyped<MyNumeric>>,
    #[bincode(with_serde)]
    pub page_range: Option<MyMaybeTyped<MyNumeric>>,
    #[bincode(with_serde)]
    pub page_total: Option<MyNumeric>,
    #[bincode(with_serde)]
    pub time_range: Option<MyMaybeTyped<MyDurationRange>>,
    #[bincode(with_serde)]
    pub runtime: Option<MyMaybeTyped<Duration>>,
    #[bincode(with_serde)]
    pub url: Option<MyQualifiedUrl>,
    #[bincode(with_serde)]
    pub serial_numbers: Option<BTreeMap<String, String>>,
    #[bincode(with_serde)]
    pub language: Option<String>,
    #[bincode(with_serde)]
    pub archive: Option<MyFormatString>,
    #[bincode(with_serde)]
    pub archive_location: Option<MyFormatString>,
    #[bincode(with_serde)]
    pub call_number: Option<MyFormatString>,
    #[bincode(with_serde)]
    pub note: Option<MyFormatString>,
    #[bincode(with_serde)]
    pub abstractt: Option<MyFormatString>,
    #[bincode(with_serde)]
    pub annote: Option<MyFormatString>,
    #[bincode(with_serde)]
    pub genre: Option<MyFormatString>,
    //#[bincode(with_serde)]
    //pub parents: Option<Vec<BibEntry>>,
}

/// Struct similar to [hayagriva::Entry], but without special serde annotations, since Bincode doesn't support these
/// For convenience, the struct implements [From] and [Into] for [hayagriva::Entry] and reverse
#[derive(Debug, Serialize, Deserialize, Encode, Decode, Clone)]
pub struct BibEntryV2 {
    pub key: String,
    #[bincode(with_serde)]
    pub entry_type: EntryType,
    #[bincode(with_serde)]
    pub title: Option<MyFormatString>,
    #[bincode(with_serde)]
    pub authors: Vec<MyPerson>,
    #[bincode(with_serde)]
    pub date: Option<MyDate>,
    #[bincode(with_serde)]
    pub editors: Vec<MyPerson>,
    #[bincode(with_serde)]
    pub affiliated: Vec<MyPersonsWithRoles>,
    #[bincode(with_serde)]
    pub publisher: Option<MyFormatString>,
    #[bincode(with_serde)]
    pub location: Option<MyFormatString>,
    #[bincode(with_serde)]
    pub organization: Option<MyFormatString>,
    #[bincode(with_serde)]
    pub issue: Option<MyMaybeTyped<MyNumeric>>,
    #[bincode(with_serde)]
    pub volume: Option<MyMaybeTyped<MyNumeric>>,
    #[bincode(with_serde)]
    pub volume_total: Option<MyNumeric>,
    #[bincode(with_serde)]
    pub edition: Option<MyMaybeTyped<MyNumeric>>,
    #[bincode(with_serde)]
    pub page_range: Option<MyMaybeTyped<MyNumeric>>,
    #[bincode(with_serde)]
    pub page_total: Option<MyNumeric>,
    #[bincode(with_serde)]
    pub time_range: Option<MyMaybeTyped<MyDurationRange>>,
    #[bincode(with_serde)]
    pub runtime: Option<MyMaybeTyped<Duration>>,
    #[bincode(with_serde)]
    pub url: Option<MyQualifiedUrl>,
    #[bincode(with_serde)]
    pub serial_numbers: Option<BTreeMap<String, String>>,
    #[bincode(with_serde)]
    pub language: Option<String>,
    #[bincode(with_serde)]
    pub archive: Option<MyFormatString>,
    #[bincode(with_serde)]
    pub archive_location: Option<MyFormatString>,
    #[bincode(with_serde)]
    pub call_number: Option<MyFormatString>,
    #[bincode(with_serde)]
    pub note: Option<MyFormatString>,
    #[bincode(with_serde)]
    pub abstractt: Option<MyFormatString>,
    #[bincode(with_serde)]
    pub annote: Option<MyFormatString>,
    #[bincode(with_serde)]
    pub genre: Option<MyFormatString>,
    #[bincode(with_serde)]
    pub parents: Vec<BibEntryV2>,
}

impl From<OldBibEntry> for BibEntryV2{
    fn from(value: OldBibEntry) -> Self{
        BibEntryV2{
            key: value.key,
            entry_type: value.entry_type,
            title: value.title,
            authors: value.authors,
            date: value.date,
            editors: value.editors,
            affiliated: value.affiliated,
            publisher: value.publisher,
            location: value.location,
            organization: value.organization,
            issue: value.issue,
            volume: value.volume,
            volume_total: value.volume_total,
            edition: value.edition,
            page_range: value.page_range,
            page_total: value.page_total,
            time_range: value.time_range,
            runtime: value.runtime,
            url: value.url,
            serial_numbers: value.serial_numbers,
            language: value.language,
            archive: value.archive,
            archive_location: value.archive_location,
            call_number: value.call_number,
            note: value.note,
            abstractt: value.abstractt,
            annote: value.annote,
            genre: value.genre,
            parents: vec![],
        }
    }
}

impl From<&hayagriva::Entry> for BibEntryV2 {
    fn from(value: &hayagriva::Entry) -> Self {
        let title = match value.title(){
            Some(title) => Some(title.clone().into()),
            None => None,
        };
        let publisher = match value.publisher(){
            Some(publisher) => Some(publisher.clone().into()),
            None => None,
        };
        let location = match value.location(){
            Some(location) => Some(location.clone().into()),
            None => None,
        };
        let organization = match value.organization(){
            Some(organization) => Some(organization.clone().into()),
            None => None,
        };
        let archive = match value.archive(){
            Some(archive) => Some(archive.clone().into()),
            None => None,
        };
        let archive_location = match value.archive_location(){
            Some(archive_location) => Some(archive_location.clone().into()),
            None => None,
        };
        let call_number = match value.call_number(){
            Some(call_number) => Some(call_number.clone().into()),
            None => None,
        };
        let note = match value.note(){
            Some(note) => Some(note.clone().into()),
            None => None,
        };
        let abstract_ = match value.abstract_(){
            Some(abstract_) => Some(abstract_.clone().into()),
            None => None,
        };
        let annote = match value.annote(){
            Some(annote) => Some(annote.clone().into()),
            None => None,
        };
        let genre = match value.genre(){
            Some(genre) => Some(genre.clone().into()),
            None => None,
        };
        let authors = match value.authors(){
            Some(authors) => authors.iter().map(|x| <hayagriva::types::Person as Clone>::clone(&(*x)).into()).collect(),
            None => vec![],
        };
        let editors = match value.editors(){
            Some(editors) => editors.iter().map(|x| <hayagriva::types::Person as Clone>::clone(&(*x)).into()).collect(),
            None => vec![],
        };

        let serial_numbers = match value.serial_number(){
            Some(serial_numbers) => Some(serial_numbers.0.clone()),
            None => None,
        };

        let issue= match value.issue(){
            Some(issue) => Some(issue.clone().into()),
            None => None,
        };
        let volume = match value.volume(){
            Some(volume) => Some(volume.clone().into()),
            None => None,
        };
        let edition = match value.edition(){
            Some(edition) => Some(edition.clone().into()),
            None => None,
        };
        let page_range = match value.page_range(){
            Some(page_range) => Some(page_range.clone().into()),
            None => None,
        };
        let volume_total = match value.volume_total(){
            Some(volume_total) => Some(volume_total.clone().into()),
            None => None,
        };
        let page_total = match value.page_total(){
            Some(page_total) => Some(page_total.clone().into()),
            None => None,
        };
        let url = match value.url(){
            Some(url) => Some(url.clone().into()),
            None => None,
        };
        let date = match value.date(){
            Some(date) => Some(date.clone().into()),
            None => None,
        };
        let language = match value.language(){
            Some(language) => Some(language.to_string()),
            None => None,
        };
        let affiliated = match value.affiliated(){
            Some(affiliated) => {
                affiliated.iter().map(|x| <hayagriva::types::PersonsWithRoles as Clone>::clone(&(*x)).into()).collect()
            },
            None => vec![],
        };
        let time_range = match value.time_range(){
            Some(time_range) => {
                match time_range {
                    hayagriva::types::MaybeTyped::Typed(t) => Some(MyMaybeTyped::Typed(t.clone().into())),
                    hayagriva::types::MaybeTyped::String(s) => Some(MyMaybeTyped::String(s.to_string())),
                }
            },
            None => None,
        };
        let runtime = match value.runtime(){
            Some(runtime) => Some(runtime.clone().into()),
            None => None,
        };

        let parents_arr = value.parents();
        let mut parents = vec![];
        if parents_arr.len() > 0{
            parents = parents_arr.iter().map(|x| (&<hayagriva::Entry as Clone>::clone(&(*x))).into()).collect();
        }
        BibEntryV2 {
            key: value.key().to_string(),
            entry_type: value.entry_type().clone(),
            title,
            authors,
            date,
            editors,
            affiliated,
            publisher,
            location,
            organization,
            issue,
            volume,
            volume_total,
            edition,
            page_range,
            page_total,
            time_range,
            runtime,
            url,
            serial_numbers,
            language,
            archive,
            archive_location,
            call_number,
            note,
            abstractt: abstract_,
            annote,
            genre,
            parents,
        }
    }
}

impl From<BibEntryV2> for hayagriva::Entry{
    fn from(value: BibEntryV2) -> Self {
        let mut entry = hayagriva::Entry::new(&value.key, value.entry_type);

        if let Some(title) = value.title{
            entry.set_title(title.into());
        }

        if value.authors.len() > 0 {
            entry.set_authors(value.authors.iter().map(|x| <MyPerson as Clone>::clone(&(*x)).into()).collect())
        }

        if let Some(date) = value.date {
            entry.set_date(date.into());
        }

        if value.editors.len() > 0 {
            entry.set_editors(value.editors.iter().map(|x| <MyPerson as Clone>::clone(&(*x)).into()).collect());
        }

        if value.affiliated.len() > 0 {
            entry.set_affiliated(value.affiliated.into_iter().map(|x| x.into()).collect());
        }

        if let Some(publisher) = value.publisher {
            entry.set_publisher(publisher.into());
        }

        if let Some(location) = value.location {
            entry.set_location(location.into());
        }

        if let Some(organization) = value.organization {
            entry.set_organization(organization.into());
        }

        if let Some(issue) = value.issue {
            let nissue : MaybeTyped<Numeric> = match issue {
                MyMaybeTyped::Typed(t) => MaybeTyped::Typed(t.into()),
                MyMaybeTyped::String(s) => MaybeTyped::String(s),
            };
            entry.set_issue(nissue);
        }

        if let Some(volume) = value.volume {
            let nvolume : MaybeTyped<Numeric> = match volume {
                MyMaybeTyped::Typed(t) => MaybeTyped::Typed(t.into()),
                MyMaybeTyped::String(s) => MaybeTyped::String(s),
            };
            entry.set_volume(nvolume);
        }

        if let Some(volume_total) = value.volume_total {
            entry.set_volume_total(volume_total.into());
        }

        if let Some(edition) = value.edition {
            let nedition : MaybeTyped<Numeric> = match edition {
                MyMaybeTyped::Typed(t) => MaybeTyped::Typed(t.into()),
                MyMaybeTyped::String(s) => MaybeTyped::String(s),
            };
            entry.set_edition(nedition);
        }

        if let Some(page_range) = value.page_range {
            let npage_range : MaybeTyped<Numeric> = match page_range {
                MyMaybeTyped::Typed(t) => MaybeTyped::Typed(t.into()),
                MyMaybeTyped::String(s) => MaybeTyped::String(s),
            };
            entry.set_page_range(npage_range);
        }

        if let Some(page_total) = value.page_total {
            entry.set_page_total(page_total.into());
        }

        if let Some(time_range) = value.time_range {
            let ntime_range : MaybeTyped<DurationRange> = match time_range {
                MyMaybeTyped::Typed(t) => MaybeTyped::Typed(t.into()),
                MyMaybeTyped::String(s) => MaybeTyped::String(s),
            };
            entry.set_time_range(ntime_range);
        }

        if let Some(runtime) = value.runtime {
            entry.set_runtime(runtime.into());
        }

        if let Some(url) = value.url {
            entry.set_url(url.into());
        }

        if let Some(serial_numbers) = value.serial_numbers {
            entry.set_serial_number(SerialNumber(serial_numbers));
        }

        if let Some(language) = value.language {
            entry.set_language(LanguageIdentifier::from_str(&language).unwrap_or(LanguageIdentifier::from_str("en-GB").unwrap()));
        }

        if let Some(archive) = value.archive {
            entry.set_archive(archive.into());
        }

        if let Some(archive_location) = value.archive_location {
            entry.set_archive_location(archive_location.into());
        }

        if let Some(call_number) = value.call_number {
            entry.set_call_number(call_number.into());
        }

        if let Some(note) = value.note {
            entry.set_note(note.into());
        }

        if let Some(abstract_) = value.abstractt {
            entry.set_abstract_(abstract_.into());
        }

        if let Some(annote) = value.annote {
            entry.set_annote(annote.into());
        }

        if let Some(genre) = value.genre {
            entry.set_genre(genre.into());
        }

        if value.parents.len() > 0 {
            entry.set_parents(value.parents.iter().map(|x| <BibEntryV2 as Clone>::clone(&(*(&<BibEntryV2 as Clone>::clone(&(*x))))).into()).collect());
        }

        entry
    }
}

impl BibEntryV2 {
    pub fn new(key: String, entry_type: EntryType) -> BibEntryV2 {
        BibEntryV2 {
            key,
            entry_type,
            title: None,
            authors: vec![],
            date: None,
            editors: vec![],
            affiliated: vec![],
            publisher: None,
            location: None,
            organization: None,
            issue: None,
            volume: None,
            volume_total: None,
            edition: None,
            page_range: None,
            page_total: None,
            time_range: None,
            runtime: None,
            url: None,
            serial_numbers: None,
            language: None,
            archive: None,
            archive_location: None,
            call_number: None,
            note: None,
            abstractt: None,
            annote: None,
            genre: None,
            parents: vec![],
        }
    }
}

impl ProjectDataV4 {
    // TODO migrate to using path instead of the id and searching for it
    pub fn remove_section(&mut self, section_to_remove_id: &uuid::Uuid) -> Option<Section> {
        let pos = self.sections.iter().position(|section| match section {
            SectionOrToc::Section(section) => section.id == Some(*section_to_remove_id),
            _ => false,
        });

        match pos {
            Some(pos) => self.sections.remove(pos).into_section(),
            None => {
                for section in &mut self.sections {
                    if let SectionOrToc::Section(section) = section {
                        if let Some(removed) = section.remove_child_section(section_to_remove_id) {
                            return Some(removed);
                        }
                    }
                }
                None
            }
        }
    }
    pub fn insert_section_as_first_child(&mut self, parent_section_id: &uuid::Uuid, section_to_insert: Section) -> Result<(), ()> {
        for section in &mut self.sections {
            if let SectionOrToc::Section(section) = section {
                // Check if this is the parent section
                if section.id == Some(*parent_section_id) {
                    section.sub_sections.insert(0, section_to_insert);
                    return Ok(());
                }else{
                    // Check if one of the children is the parent section
                    if let Some(_) = section.insert_child_section_as_child(parent_section_id, &section_to_insert) {
                        return Ok(());
                    }
                }
            }
        }
        Err(())
    }
    pub fn insert_section_after(&mut self, previous_element: &uuid::Uuid, section_to_insert: Section) -> Result<(), ()> {
        let pos = self.sections.iter().position(|section| match section {
            SectionOrToc::Section(section) => section.id == Some(*previous_element),
            _ => false,
        });

        match pos {
            Some(pos) => {
                self.sections.insert(pos + 1, SectionOrToc::Section(section_to_insert));
                Ok(())
            }
            None => {
                for section in &mut self.sections {
                    if let SectionOrToc::Section(section) = section {
                        if let Some(_) = section.insert_child_section_after(previous_element, &section_to_insert) {
                            return Ok(());
                        }
                    }
                }
                Err(())
            }
        }
    }
}

pub fn get_section_by_path_mut<'a>(
    project: &'a mut RwLockWriteGuard<ProjectDataV4>,
    path: &Vec<uuid::Uuid>
) -> Result<&'a mut Section, ApiError> {

    // Find first section
    let first_section_opt = project.sections.iter_mut()
        .find_map(|section| {
            if let SectionOrToc::Section(section) = section {
                if section.id.unwrap_or_default() == path[0] {
                    Some(section)
                } else {
                    None
                }
            } else {
                None
            }
        });

    // Return error if no first section found
    let mut current_section = first_section_opt
        .ok_or_else(|| {
            println!("Couldn't find section with id {}", path[0]);
            ApiError::NotFound
        })?;

    // Iterate through the path
    for &part in path.iter().skip(1) {
        let mut found_section = None;

        for section in &mut current_section.sub_sections {
                if section.id.unwrap_or_default() == part {
                    found_section = Some(section);
                    break;
                }
        }

        match found_section {
            Some(section) => {
                current_section = section;
            },
            None => {
                println!("Couldn't find section with id {}", part);
                return Err(ApiError::NotFound);
            }
        }
    }

    Ok(current_section)
}

pub fn get_section_by_path<'a>(project: &'a RwLockReadGuard<ProjectDataV4>, path: &Vec<uuid::Uuid>) -> Result<&'a Section, ApiError>{
    let mut first_section : Option<&Section> = None;

    // Find first section
    for section in project.sections.iter(){
        if let SectionOrToc::Section(section) = section{
            if section.id.unwrap_or_default() == path[0]{
                first_section = Some(section);
            }
        }
    }

    // Return error if no first section found
    let first_section: &Section = match first_section{
        Some(first_section) => first_section,
        None => {
            println!("Couldn't find section with id {}", path[0]);
            return Err(ApiError::NotFound);
        }
    };

    let mut current_section: &Section = first_section;

    // Skip first element, because we already found it
    for part in path.iter().skip(1){

        // Search for next section in the current sections children
        let mut found = false;
        for section in current_section.sub_sections.iter(){
                if section.id.unwrap_or_default() == *part{
                    current_section = section;
                    found = true;
                    break;
                }
        }
        if !found {
            println!("Couldn't find section with id {}", part);
            return Err(ApiError::NotFound);
        }
    }

    Ok(current_section)
}


#[derive(Debug, Serialize, Deserialize, Encode, Decode, Clone)]
/// Represents a user in the data storage.
pub struct User {
    #[bincode(with_serde)]
    /// The unique identifier for the user.
    pub id: uuid::Uuid,
    /// The email address of the user.
    pub email: String,
    /// The name of the user.
    pub name: String,
    /// The hashed password of the user.
    pub password_hash: String,
    /// The timestamp until which the user is locked out (if applicable).
    pub locked_until: Option<u64>,
    /// The list of login attempts made by the user.
    pub login_attempts: Vec<u64>,
}

impl User {
    /// Creates a new user with the specified email, name, and password.
    ///
    /// # Arguments
    ///
    /// * `email` - The email of the user.
    /// * `name` - The name of the user.
    /// * `password` - The password of the user.
    ///
    /// # Returns
    ///
    /// A new `User` instance with the specified email, name, and password.
    pub fn new(email: String, name: String, password: String) -> Self {
        let salt = argon2::password_hash::SaltString::generate(&mut OsRng);
        let password_hash = Argon2::default().hash_password(&password.as_bytes(), &salt).unwrap().to_string();

        User {
            id: uuid::Uuid::new_v4(),
            email,
            name,
            password_hash,
            locked_until: None,
            login_attempts: vec![],
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Encode, Decode, Clone)]
pub struct ProjectTemplateV1 {
    #[bincode(with_serde)]
    pub id: uuid::Uuid,
    pub name: String,
    pub description: String,
}

#[derive(Debug, Serialize, Deserialize, Encode, Decode, Clone)]
pub struct ProjectTemplateV2 {
    #[bincode(with_serde)]
    pub id: uuid::Uuid,
    /// Unique version id, changed if a Template is changed.
    /// Used to detect if a template on a rendering server needs to be updated
    #[bincode(with_serde)]
    pub version: Option<uuid::Uuid>,
    pub name: String,
    pub description: String,
    pub export_formats: HashMap<String, ExportFormat>,
}


pub async fn save_data_worker(data_storage: Arc<DataStorage>, project_storage: Arc<ProjectStorage>, settings: Settings){
    tokio::spawn(async move {
        let mut last_save = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs();
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(settings.backup_to_file_interval)).await;
            // Save DataStorage to disk
            println!("Saving DataStorage to disk");
            data_storage.save_to_disk(&settings).await.unwrap();

            // Save ProjectStorage to disk
            let mut projects_to_save = Vec::new();
            for project_id in project_storage.projects.read().unwrap().keys(){
                if let Some(project) = project_storage.projects.read().unwrap().get(project_id){
                    if let Some(project) = &project.data{
                        if project.read().unwrap().last_interaction > last_save{
                            projects_to_save.push(project_id.clone());
                        }
                    }

                }
            }
            for project_id in projects_to_save{
                println!("Saving changed project {} to disk", project_id);
                project_storage.save_project_to_disk(&project_id, &settings).await.unwrap(); //TODO: shutdown if this fails to avoid data loss
            }
            last_save = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs();
            println!("Finished saving projects to disk");
        }
    });
}

impl From<MyFormatString> for FormatString {
    fn from(my_format_string: MyFormatString) -> Self {
        match my_format_string.short{
            Some(short) => FormatString::with_short(my_format_string.value, short),
            None => FormatString::with_value(my_format_string.value),
        }
    }
}

impl From<FormatString> for MyFormatString {
    fn from(format_string: FormatString) -> Self {
        MyFormatString {
            value: format_string.value.to_string(),
            short: match format_string.short {
                Some(short) => Some(short.to_string()),
                None => None,
            },
        }
    }
}


#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MyQualifiedUrl{
    pub value: Url,
    pub visit_date: Option<MyDate>,
}

impl From<QualifiedUrl> for MyQualifiedUrl {
    fn from(value: QualifiedUrl) -> Self {
        MyQualifiedUrl {
            value: value.value,
            visit_date: value.visit_date.map(|d| d.into()),
        }
    }
}

impl From<MyQualifiedUrl> for QualifiedUrl {
    fn from(value: MyQualifiedUrl) -> Self {
        QualifiedUrl {
            value: value.value,
            visit_date: value.visit_date.map(|d| d.into()),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MyFormatString {
    /// The canonical version of the string.
    pub value: String,
    /// The short version of the string.
    pub short: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MyNumeric {
    /// The numeric value.
    pub value: MyNumericValue,
    /// A string that is prepended to the value.
    pub prefix: Option<Box<String>>,
    /// A string that is appended to the value.
    pub suffix: Option<Box<String>>,
}

impl From<MaybeTyped<Numeric>> for MyMaybeTyped<MyNumeric>{
    fn from(value: MaybeTyped<Numeric>) -> MyMaybeTyped<MyNumeric> {
        match value {
            MaybeTyped::Typed(n) => MyMaybeTyped::Typed(n.into()),
            MaybeTyped::String(s) => MyMaybeTyped::String(s),
        }
    }
}

impl From<Numeric> for MyNumeric {
    fn from(value: Numeric) -> Self {
        MyNumeric {
            value: value.value.into(),
            prefix: value.prefix,
            suffix: value.suffix,
        }
    }
}


impl From<MyNumeric> for Numeric {
    fn from(value: MyNumeric) -> Self {
        Numeric {
            value: value.value.into(),
            prefix: value.prefix,
            suffix: value.suffix,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MyNumericValue {
    /// A single number.
    Number(i32),
    /// A set of numbers.
    Set(Vec<(i32, Option<MyNumericDelimiter>)>),
}

impl From<NumericValue> for MyNumericValue {
    fn from(value: NumericValue) -> Self {
        match value {
            NumericValue::Number(n) => MyNumericValue::Number(n),
            NumericValue::Set(s) => MyNumericValue::Set(s.into_iter().map(|(n, d)| (n, d.map(|d| d.into()))).collect()),
        }
    }
}

impl From<MyNumericValue> for NumericValue {
    fn from(value: MyNumericValue) -> Self {
        match value {
            MyNumericValue::Number(n) => NumericValue::Number(n),
            MyNumericValue::Set(s) => NumericValue::Set(s.into_iter().map(|(n, d)| (n, d.map(|d| d.into()))).collect()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MyNumericDelimiter {
    /// A comma.
    Comma,
    /// An ampersand.
    Ampersand,
    /// A hyphen. Will be converted to an en dash for display.
    Hyphen,
}

impl From<MyNumericDelimiter> for NumericDelimiter {
    fn from(value: MyNumericDelimiter) -> Self {
        match value {
            MyNumericDelimiter::Comma => NumericDelimiter::Comma,
            MyNumericDelimiter::Ampersand => NumericDelimiter::Ampersand,
            MyNumericDelimiter::Hyphen => NumericDelimiter::Hyphen,
        }
    }
}

impl From<NumericDelimiter> for MyNumericDelimiter {
    fn from(value: NumericDelimiter) -> Self {
        match value {
            NumericDelimiter::Comma => MyNumericDelimiter::Comma,
            NumericDelimiter::Ampersand => MyNumericDelimiter::Ampersand,
            NumericDelimiter::Hyphen => MyNumericDelimiter::Hyphen,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MyDate {
    /// The year (1 B.C.E. is represented as 0 and so forth).
    pub year: i32,
    /// The optional month (1-12).
    pub month: Option<u8>,
    /// The optional day (1-31).
    pub day: Option<u8>,
    /// Whether the date is approximate.
    pub approximate: bool,
}

impl From<Date> for MyDate {
    fn from(value: Date) -> Self {
        // Convert 0-based to 1-based
        let month = match value.month {
            Some(month) => Some(month+1),
            None => None,
        };
        let day = match value.day {
            Some(day) => Some(day+1),
            None => None,
        };
        MyDate {
            year: value.year,
            month,
            day,
            approximate: value.approximate,
        }
    }
}

impl From<MyDate> for Date {
    fn from(value: MyDate) -> Self {
        // Convert 1-based to 0-based
        let month = match value.month {
            Some(month) => Some(month-1),
            None => None,
        };
        let day = match value.day {
            Some(day) => Some(day-1),
            None => None,
        };

        Date {
            year: value.year,
            month,
            day,
            approximate: value.approximate,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MyDurationRange{
    pub start: Duration,
    pub end: Duration,
}

impl From<DurationRange> for MyDurationRange {
    fn from(value: DurationRange) -> Self {
        MyDurationRange {
            start: value.start,
            end: value.end,
        }
    }
}

impl From<MyDurationRange> for DurationRange {
    fn from(value: MyDurationRange) -> Self {
        DurationRange {
            start: value.start,
            end: value.end,
        }
    }
}
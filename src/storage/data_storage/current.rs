use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, RwLock};
use std::sync::atomic::AtomicBool;
use bincode::{Decode, Encode};
use rocket::serde::{Deserialize, Serialize};
use vb_exchange::projects::{Person, PersonV2};
use crate::settings::Settings;
pub(crate) use crate::storage::{ProjectTemplateV2, SingleFileLock, User};
use crate::storage::data_storage::{DataStorage, DataStorageLoadError, InnerDataStorage};
use crate::storage::data_storage::migration::load_inner_data_storage;

#[derive(Debug, Serialize, Deserialize, Encode, Decode, Clone)]
pub struct InnerDataStorageV3{
    /// HashMap with users, id as HashMap keys
    #[bincode(with_serde)]
    pub login_data: HashMap<uuid::Uuid, Arc<RwLock<User>>>,
    #[bincode(with_serde)]
    pub persons: HashMap<uuid::Uuid, Arc<RwLock<PersonV2>>>,
    #[bincode(with_serde)]
    pub templates: HashMap<uuid::Uuid, Arc<RwLock<ProjectTemplateV2>>>
}

impl DataStorage{
    /// Creates a new empty [DataStorage]
    pub fn new() -> Self {
        DataStorage {
            data: RwLock::new(InnerDataStorageV3{
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
    
    fn load_from_disk_blocking(settings: &Settings) -> Result<InnerDataStorage, DataStorageLoadError>{
        let path = format!("{}", settings.data_path);
        let files = std::fs::read_dir(&path)?;

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
                error!("error while loading data storage into memory: no storage files found in data directory.");
                return Err(DataStorageLoadError::DataStorageMissing)
            }
            let (version, file_path) = file_versions.last().unwrap();

            let file = std::fs::File::open(format!("{}/{}", &path, file_path))?;

            load_inner_data_storage(file, *version, &settings.clone().data_path)
    }    
    
    /// Loads the [DataStorage] from disk
    ///
    /// Path is defined as data_path from settings + /data.version.bincode
    pub async fn load_from_disk(settings: &Settings) -> Result<Self, DataStorageLoadError>{
        let mut data_storage = DataStorage::new();

        let settings_cpy = settings.clone();
        let res = tokio::task::spawn_blocking(move || Self::load_from_disk_blocking(&settings_cpy)).await;

        data_storage.data = RwLock::new(res.unwrap()?);
        Ok(data_storage)
    }

    /// Saves the [DataStorage] to disk
    ///
    /// Creates a whole copy of the [DataStorage] and saves it to disk
    /// This may use a lot of memory, maybe change this in the future if it becomes a problem
    pub async fn save_to_disk(&self, settings: &Settings) -> Result<(), ()>{
        self.wait_for_file_lock(settings).await?;

        // Save login data
        let cpy = self.data.read().unwrap().clone();
        let path = format!("{}/data.3.bincode", settings.data_path);

        match tokio::task::spawn_blocking(move || {
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
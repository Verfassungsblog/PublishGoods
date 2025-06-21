use crate::projects::api::ApiError;
use crate::projects::{ProjectMetadataV3, ProjectMetadataV4, SectionOrTocV4, SectionOrTocV5, SectionV5};
use crate::settings::Settings;
use crate::storage::project_storage::migration::load_project_data;
use crate::storage::project_storage::{
    ProjectData, ProjectStorage, ProjectStorageEntry, ProjectStorageError,
};
use crate::storage::{BibEntryV2, MultipleFileLocks, ProjectListEntry};
use bincode::{Decode, Encode};
use rocket::outcome::IntoOutcome;
use rocket::serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::SystemTime;
use tokio::io::AsyncReadExt;
use vb_exchange::projects::ProjectSettingsV5;

impl MultipleFileLocks for ProjectStorage {
    fn get_file_lock_entry(&self, uuid: &uuid::Uuid) -> Arc<AtomicBool> {
        if let Some(entry) = self.file_locks.read().unwrap().get(uuid) {
            return entry.clone();
        }
        // Create new entry
        self.file_locks
            .write()
            .unwrap()
            .insert(uuid.clone(), Arc::new(AtomicBool::new(false)));
        return self.file_locks.read().unwrap().get(uuid).unwrap().clone();
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
    pub async fn unload_unused_projects(&mut self, settings: &Settings) -> Result<(), ()> {
        let mut projects_to_unload = vec![];

        for (uuid, project_data) in self.projects.read().unwrap().iter() {
            if let Some(project) = &project_data.data {
                let now = SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_secs();
                if Arc::strong_count(&project) == 1
                    && project.read().unwrap().last_interaction + settings.project_cache_time < now
                {
                    projects_to_unload.push(uuid.clone());
                }
            }
        }

        for project in projects_to_unload {
            self.save_project_to_disk(&project, settings).await?;
            self.unload_project(&project)?;
        }

        Ok(())
    }

    /// Unloads a project from memory
    ///
    /// Does not save the project to disk, use [ProjectStorage::save_project_to_disk] for that
    fn unload_project(&self, uuid: &uuid::Uuid) -> Result<(), ()> {
        match self.projects.write().unwrap().get_mut(uuid) {
            Some(project) => {
                project.data = None;
                debug!("Unloaded project {} from memory.", uuid);
                Ok(())
            }
            None => {
                error!(
                    "Requested to unload project {}, but project doesn't exists.",
                    uuid
                );
                Err(())
            }
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
                error!("io error while loading project directory: {}. Check that your data_path is set correctly and we have sufficient file permissions.", e);
                return Err(());
            }
        };

        for path in paths {
            match path {
                Ok(entry) => {
                    // Skip non directory entries
                    if !entry.path().is_dir() {
                        continue;
                    }
                    match entry.file_name().to_str() {
                        Some(uuid) => match uuid.parse::<uuid::Uuid>() {
                            Ok(uuid) => {
                                debug!("Loading project {}.", uuid);
                                match self.load_project_into_memory(&uuid, settings).await {
                                    Ok(_) => {
                                        debug!("Successfully loaded project {} into memory.", uuid);
                                        if let Err(_) = self.unload_project(&uuid) {
                                            error!("error while unloading project {} after loading it into memory. Skipping project.", uuid);
                                            continue;
                                        }
                                        debug!(
                                            "Project storage now contains: {:?}",
                                            self.projects.read().unwrap().keys()
                                        );
                                    }
                                    Err(_) => {
                                        error!("error while loading project {} into memory. Skipping project.", uuid);
                                        continue;
                                    }
                                }
                            }
                            Err(e) => {
                                error!("error while parsing project directory entry name into uuid: {}, Skipping project.", e);
                                continue;
                            }
                        },
                        None => {
                            error!("error while parsing project directory entry: {:?}, Skipping project.", entry.file_name());
                            continue;
                        }
                    };
                }
                Err(e) => {
                    error!(
                        "io error while loading project directory entry: {}, Skipping project.",
                        e
                    );
                    continue;
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
    pub async fn insert_project(
        &self,
        mut project: ProjectData,
        settings: &Settings,
    ) -> Result<uuid::Uuid, ()> {
        let uuid = uuid::Uuid::new_v4();

        // Update last edited to current time, so the project doesn't get unloaded immediately
        project.last_interaction = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let entry = ProjectStorageEntry {
            name: project.name.clone(),
            data: Some(Arc::new(RwLock::new(project))),
        };
        self.projects.write().unwrap().insert(uuid, entry);
        self.save_project_to_disk(&uuid, settings).await?;
        Ok(uuid)
    }

    async fn load_project_from_disk(
        &self,
        uuid: &uuid::Uuid,
        settings: &Settings,
    ) -> Result<ProjectData, ProjectStorageError> {
        let project_path = format!("{}/projects/{}", settings.data_path, uuid);

        debug!("Aquiring file lock for project {}.", uuid);
        if self.wait_for_file_lock(uuid, settings).await.is_err() {
            return Err(ProjectStorageError::CouldntAcquireLock);
        }

        let mut dir = tokio::fs::read_dir(&project_path).await?;

        let mut project_versions: Vec<(u64, PathBuf)> = vec![];

        while let Some(entry) = dir.next_entry().await? {
            if let Some(file_name) = entry.file_name().to_str() {
                let parts: Vec<&str> = file_name.split(".").collect();

                if parts.len() == 3 && parts[0] == "project" {
                    // parse version as usize
                    let version = match parts[1].parse::<u64>() {
                        Ok(version) => version,
                        Err(e) => {
                            error!("error while loading project into memory: couldn't parse version number: {}. Skipping file.", e);
                            continue;
                        }
                    };

                    project_versions.push((version, entry.path()));
                }
            }
        }

        // Sort project versions by version number
        project_versions.sort_by(|a, b| a.0.cmp(&b.0));

        let res = tokio::task::spawn_blocking(move || {
            // Load the latest version of the project
            let (file, version) = match project_versions.last(){
                Some((version, path)) => {
                    let file = std::fs::File::open(path)?;
                    (file, version)
                },
                None => {
                    eprintln!("error while loading project into memory: no project files found in project directory.");
                    return Err(ProjectStorageError::ProjectNotFound);
                }
            };

            load_project_data(file, *version)
        }).await;

        debug!("Read complete. Releasing file lock for project {}.", uuid);
        self.remove_file_lock(uuid);

        res.unwrap_or_else(|e| {
            error!("Join error: {}", e);
            Err(ProjectStorageError::TokioJoinError)
        })
    }

    async fn load_project_into_memory(
        &self,
        uuid: &uuid::Uuid,
        settings: &Settings,
    ) -> Result<(), ()> {
        let res = self.load_project_from_disk(uuid, settings).await;

        match res {
            Ok(project) => {
                println!("Loaded project, inserting into memory storage.");
                if let Some(tproject) = self.projects.write().unwrap().get_mut(uuid) {
                    // Update last edited to current time, so the project doesn't get unloaded immediately
                    let mut project: ProjectData = project;
                    project.last_interaction = SystemTime::now()
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .unwrap()
                        .as_secs();
                    tproject.name = project.name.clone();
                    println!("Replacing project");
                    tproject.data.replace(Arc::new(RwLock::new(project)));
                    println!("Inserted project into memory storage.");
                    return Ok(());
                }

                println!("Project not found in memory storage, creating new entry.");
                let entry = ProjectStorageEntry {
                    name: project.name.clone(),
                    data: Some(Arc::new(RwLock::new(project))),
                };
                self.projects.write().unwrap().insert(uuid.clone(), entry);
                println!("Created new entry in memory storage.");
                Ok(())
            }
            Err(e) => {
                error!("error while loading project file into memory: {:?}", e);
                Err(())
            }
        }
    }

    pub async fn get_project(
        &self,
        uuid: &uuid::Uuid,
        settings: &Settings,
    ) -> Result<Arc<RwLock<ProjectData>>, ()> {
        // Check if project exists
        match self.projects.read().unwrap().get(uuid) {
            Some(project) => {
                if let Some(project) = &project.data {
                    project.write().unwrap().last_interaction = SystemTime::now()
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .unwrap()
                        .as_secs();
                    return Ok(Arc::clone(project));
                }
            }
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
                        project.write().unwrap().last_interaction = SystemTime::now()
                            .duration_since(SystemTime::UNIX_EPOCH)
                            .unwrap()
                            .as_secs();
                        Ok(Arc::clone(project))
                    }
                }
            }
            None => return Err(()),
        }
    }

    pub async fn get_projects_list(&self) -> Vec<ProjectListEntry> {
        self.projects
            .read()
            .unwrap()
            .iter()
            .map(|(id, project)| ProjectListEntry {
                id: *id,
                name: project.name.clone(),
            })
            .collect()
    }

    /// Deletes a project from memory and disk permanently
    pub async fn delete_project(
        &self,
        project_id: &uuid::Uuid,
        settings: &Settings,
    ) -> Result<(), ProjectStorageError> {
        debug!("Deleting project {}", project_id);

        // Check if project exists:
        if !self.projects.read().unwrap().contains_key(&project_id) {
            warn!("Tried to delete non-existent project {}", project_id);
            return Err(ProjectStorageError::ProjectNotFound);
        }

        // Remove project from in-memory storage:
        self.projects.write().unwrap().remove(&project_id);

        // Remove project from disk:
        let path = format!("{}/projects/{}", settings.data_path, project_id);

        if let Err(e) = tokio::fs::remove_dir_all(path).await {
            error!("Failed to delete project {} from disk: {}", project_id, e);
            return Err(ProjectStorageError::IOError(e));
        };

        Ok(())
    }

    pub(crate) async fn save_project_to_disk(&self, uuid: &uuid::Uuid, settings: &Settings) -> Result<(), ()> {
        // Get project
        let project = match self.projects.read().unwrap().get(&uuid) {
            Some(project) => match &project.data {
                Some(project) => project.clone(),
                None => return Err(()),
            },
            None => return Err(()),
        };
        match fs::create_dir(format!("{}/projects/{}", settings.data_path, uuid)) {
            Ok(_) => {}
            Err(e) => {
                if e.kind() != std::io::ErrorKind::AlreadyExists {
                    eprintln!("io error while creating project directory: {}", e);
                    return Err(());
                }
            }
        }

        let version = "7"; //TODO: auto detect latest version

        // Encode project data with bincode and save to disk
        let path = format!(
            "{}/projects/{}/project.{}.bincode",
            settings.data_path, uuid, version
        );

        match self.wait_for_file_lock(&uuid, settings).await {
            Ok(_) => {}
            Err(_) => {
                eprintln!("error while saving project to disk: couldn't get file lock");
                return Err(());
            }
        }

        //TODO: do not use spawn_blocking, but use tokio fs functions
        let res = rocket::tokio::task::spawn_blocking(move || {
            let mut file = match std::fs::File::create(path) {
                Ok(file) => file,
                Err(e) => {
                    eprintln!("io error while saving project to disk: {}", e);
                    return Err(());
                }
            };
            // Clone project data to avoid locking the project while saving
            let pcopy = project.read().unwrap().clone();
            match bincode::encode_into_std_write(&pcopy, &mut file, bincode::config::standard()) {
                Ok(_) => Ok(()),
                Err(e) => {
                    eprintln!("bincode encode error while saving project to disk: {}", e);
                    Err(())
                }
            }
        })
        .await;

        self.remove_file_lock(uuid);
        match res {
            Ok(_) => Ok(()),
            Err(e) => {
                eprintln!("error while saving project to disk: {}", e);
                Err(())
            }
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Encode, Decode, Clone)]
pub struct ProjectDataV8 {
    pub name: String,
    pub description: Option<String>,
    #[bincode(with_serde)]
    pub template_id: uuid::Uuid,
    pub last_interaction: u64,
    pub metadata: Option<ProjectMetadataV4>,
    pub settings: Option<ProjectSettingsV5>,
    pub sections: Vec<SectionOrTocV5>,
    #[bincode(with_serde)]
    pub bibliography: HashMap<String, BibEntryV2>, //TODO: add prefix & suffix support
}

impl ProjectDataV8 {
    // TODO migrate to using path instead of the id and searching for it
    pub fn remove_section(&mut self, section_to_remove_id: &uuid::Uuid) -> Option<SectionV5> {
        let pos = self.sections.iter().position(|section| match section {
            SectionOrTocV5::Section(section) => section.id == Some(*section_to_remove_id),
            _ => false,
        });

        match pos {
            Some(pos) => self.sections.remove(pos).into_section(),
            None => {
                for section in &mut self.sections {
                    if let SectionOrTocV5::Section(section) = section {
                        if let Some(removed) = section.remove_child_section(section_to_remove_id) {
                            return Some(removed);
                        }
                    }
                }
                None
            }
        }
    }
    pub fn insert_section_as_first_child(
        &mut self,
        parent_section_id: &uuid::Uuid,
        section_to_insert: SectionV5,
    ) -> Result<(), ()> {
        for section in &mut self.sections {
            if let SectionOrTocV5::Section(section) = section {
                // Check if this is the parent section
                if section.id == Some(*parent_section_id) {
                    section.sub_sections.insert(0, section_to_insert);
                    return Ok(());
                } else {
                    // Check if one of the children is the parent section
                    if let Some(_) =
                        section.insert_child_section_as_child(parent_section_id, &section_to_insert)
                    {
                        return Ok(());
                    }
                }
            }
        }
        Err(())
    }
    pub fn insert_section_after(
        &mut self,
        previous_element: &uuid::Uuid,
        section_to_insert: SectionV5,
    ) -> Result<(), ()> {
        let pos = self.sections.iter().position(|section| match section {
            SectionOrTocV5::Section(section) => section.id == Some(*previous_element),
            _ => false,
        });

        match pos {
            Some(pos) => {
                self.sections
                    .insert(pos + 1, SectionOrTocV5::Section(section_to_insert));
                Ok(())
            }
            None => {
                for section in &mut self.sections {
                    if let SectionOrTocV5::Section(section) = section {
                        if let Some(_) =
                            section.insert_child_section_after(previous_element, &section_to_insert)
                        {
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
    project: &'a mut RwLockWriteGuard<ProjectData>,
    path: &Vec<uuid::Uuid>,
) -> Result<&'a mut SectionV5, ApiError> {
    // Find first section
    let first_section_opt = project.sections.iter_mut().find_map(|section| {
        if let SectionOrTocV5::Section(section) = section {
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
    let mut current_section = first_section_opt.ok_or_else(|| {
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
            }
            None => {
                println!("Couldn't find section with id {}", part);
                return Err(ApiError::NotFound);
            }
        }
    }

    Ok(current_section)
}

pub fn get_section_by_path<'a>(
    project: &'a RwLockReadGuard<ProjectData>,
    path: &Vec<uuid::Uuid>,
) -> Result<&'a SectionV5, ApiError> {
    let mut first_section: Option<&SectionV5> = None;

    // Find first section
    for section in project.sections.iter() {
        if let SectionOrTocV5::Section(section) = section {
            if section.id.unwrap_or_default() == path[0] {
                first_section = Some(section);
            }
        }
    }

    // Return error if no first section found
    let first_section: &SectionV5 = match first_section {
        Some(first_section) => first_section,
        None => {
            println!("Couldn't find section with id {}", path[0]);
            return Err(ApiError::NotFound);
        }
    };

    let mut current_section: &SectionV5 = first_section;

    // Skip first element, because we already found it
    for part in path.iter().skip(1) {
        // Search for next section in the current sections children
        let mut found = false;
        for section in current_section.sub_sections.iter() {
            if section.id.unwrap_or_default() == *part {
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

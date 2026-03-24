use crate::settings::Settings;
use crate::storage::project_storage::migration::load_project_data;
use crate::storage::project_storage::sections::current::SectionV6;
use crate::storage::project_storage::sections::Section;
use crate::storage::project_storage::{
    ProjectData, ProjectStorage, ProjectStorageError, CURRENT_VERSION,
};
use crate::storage::{BibEntryV3, MultipleFileLocks, MyMaybeTyped, MyPageRanges};
use crate::utils::api_helpers::{ApiError, ApiErrorType};
use bincode::{Decode, Encode};
use chrono::NaiveDate;
use dashmap::{DashMap, Entry};
use hayagriva::types::{MaybeTyped, SerialNumber};
use language::Language;
use rocket::serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::fs::File;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::SystemTime;
use unic_langid_impl::LanguageIdentifier;
use uuid::Uuid;
use vb_exchange::projects::{Identifier, Keyword, License, ProjectSettingsV5};

impl MultipleFileLocks for ProjectStorage {
    fn get_file_lock_entry(&self, uuid: &uuid::Uuid) -> Arc<AtomicBool> {
        match self.file_locks.entry(uuid.clone()) {
            Entry::Occupied(entry) => entry.get().clone(),
            Entry::Vacant(entry) => entry.insert(Arc::new(AtomicBool::new(false))).clone(),
        }
    }
}

impl ProjectStorage {
    /// Creates a new empty [ProjectStorage]
    pub fn new() -> Self {
        ProjectStorage {
            projects: DashMap::new(),
            file_locks: Default::default(),
        }
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

    pub async fn has_project(&self, uuid: &uuid::Uuid, settings: &Settings) -> bool {
        match self.get_project(uuid, settings).await {
            Ok(_) => true,
            Err(e) => match e {
                ProjectStorageError::ProjectNotFound => false,
                _ => {
                    error!("Error while checking if project exists: {:?}", e);
                    false
                }
            },
        }
    }

    pub async fn get_project(
        &self,
        uuid: &uuid::Uuid,
        settings: &Settings,
    ) -> Result<Arc<RwLock<ProjectData>>, ProjectStorageError> {
        // Check if project is already in memory
        match self.projects.entry(uuid.clone()) {
            Entry::Occupied(entry) => {
                let project = entry.get();
                project.write().unwrap().last_interaction = SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_secs();
                return Ok(Arc::clone(project));
            }
            Entry::Vacant(entry) => {
                // Try to load from disk
                match self.load_project_from_disk(uuid, settings).await {
                    Ok(project) => {
                        let new_entry = entry.insert_entry(Arc::new(RwLock::new(project)));
                        Ok(Arc::clone(new_entry.get()))
                    }
                    Err(e) => Err(e),
                }
            }
        }
    }

    /// Deletes a project from memory and disk permanently.
    /// Does not delete the project entry from the project list!
    pub async fn delete_project(
        &self,
        project_id: &uuid::Uuid,
        settings: &Settings,
    ) -> Result<(), ProjectStorageError> {
        debug!("Deleting project {}", project_id);

        // Remove project from memory:
        if let None = self.projects.remove(&project_id) {
            return Err(ProjectStorageError::ProjectNotFound);
        }

        // Remove project from disk:
        let path = format!("{}/projects/{}", settings.data_path, project_id);

        if let Err(e) = tokio::fs::remove_dir_all(path).await {
            error!("Failed to delete project {} from disk: {}", project_id, e);
            return Err(ProjectStorageError::IOError(e));
        };

        Ok(())
    }

    pub async fn save_project_to_disk(
        &self,
        uuid: &Uuid,
        settings: &Settings,
    ) -> Result<(), ProjectStorageError> {
        // Get project
        let project = self
            .projects
            .get(uuid)
            .ok_or(ProjectStorageError::ProjectNotFound)?
            .value()
            .clone();

        match fs::create_dir(format!("{}/projects/{}", settings.data_path, uuid)) {
            Ok(_) => {}
            Err(e) => {
                if e.kind() != std::io::ErrorKind::AlreadyExists {
                    eprintln!("io error while creating project directory: {}", e);
                    return Err(ProjectStorageError::IOError(e));
                }
            }
        }

        // Encode project data with bincode and save to disk
        let path = format!(
            "{}/projects/{}/project.{}.bincode",
            settings.data_path, uuid, CURRENT_VERSION
        );
        let path_temp = format!("{}.temp", &path);

        if let Err(_) = self.wait_for_file_lock(&uuid, settings).await {
            eprintln!("error while saving project to disk: couldn't get file lock");
            return Err(ProjectStorageError::CouldntAcquireLock);
        }

        // We can't use tokio::fs because bincode doesn't support async I/O
        rocket::tokio::task::spawn_blocking(move || {
            let mut file = File::create(&path_temp)?;
            let pcopy = project.read().unwrap().clone();
            bincode::encode_into_std_write(&pcopy, &mut file, bincode::config::standard())?;

            // Move temp_file to final location
            std::fs::rename(path_temp, path)?;
            Ok::<(), ProjectStorageError>(())
        })
        .await??;

        self.remove_file_lock(uuid);
        Ok(())
    }

    pub async fn insert_project(
        &self,
        uuid: Uuid,
        project_data: ProjectData,
        settings: &Settings,
    ) -> Result<(), ProjectStorageError> {
        self.projects
            .insert(uuid, Arc::new(RwLock::new(project_data)));
        self.save_project_to_disk(&uuid, settings).await?;
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize, Encode, Decode, Clone)]
pub struct ProjectDataV10 {
    pub name: String,
    pub description: Option<String>,
    #[bincode(with_serde)]
    pub template_id: Uuid,
    pub last_interaction: u64,
    pub metadata: Option<ProjectMetadataV5>,
    pub settings: Option<ProjectSettingsV5>,
    pub sections: Vec<SectionV6>,
    #[bincode(with_serde)]
    pub bibliography: Bibliography,
}

#[derive(Debug, Serialize, Deserialize, Encode, Decode, Clone)]
pub struct Bibliography {
    #[bincode(with_serde)]
    pub entries: HashMap<Uuid, BibEntryOrFolder>,
}

#[derive(Debug, Serialize, Deserialize, Encode, Decode, Clone)]
pub enum BibEntryOrFolder {
    BibEntry(BibEntryV3),
    BibFolder(BibFolder),
}

#[derive(Debug, Serialize, Deserialize, Encode, Decode, Clone)]
pub struct BibFolder {
    pub name: String,
    #[bincode(with_serde)]
    pub parent: Option<Uuid>,
}

impl Bibliography {
    pub fn new() -> Bibliography {
        Bibliography {
            entries: HashMap::new(),
        }
    }
    pub fn add_entry(&mut self, entry: BibEntryV3) {
        self.entries
            .insert(entry.key, BibEntryOrFolder::BibEntry(entry));
    }
    pub fn get_entry(&self, key: &Uuid) -> Option<&BibEntryOrFolder> {
        self.entries.get(key)
    }

    pub fn get_entry_as_hayagriva(&self, key: &Uuid) -> Option<hayagriva::Entry> {
        let value = match self.get_entry(key)?.clone() {
            BibEntryOrFolder::BibEntry(e) => e,
            BibEntryOrFolder::BibFolder(_) => {
                return None;
            }
        };

        let mut parents: Vec<hayagriva::Entry> = vec![];
        for parent in &value.parents {
            if let BibEntryOrFolder::BibEntry(_) = self.get_entry(parent)?.clone() {
                if let Some(parent) = self.get_entry_as_hayagriva(parent) {
                    // Caution: this could recurse infinitely if there are circular references which must be circumvented in creation
                    parents.push(parent);
                }
            }
        }

        let mut entry = hayagriva::Entry::new(&value.key.to_string(), value.entry_type);

        if let Some(title) = value.title {
            entry.set_title(title.into());
        }

        if value.authors.len() > 0 {
            entry.set_authors(value.authors.iter().map(|x| x.clone().into()).collect())
        }

        if let Some(date) = value.date {
            entry.set_date(date.into());
        }

        if value.editors.len() > 0 {
            entry.set_editors(value.editors.iter().map(|x| x.clone().into()).collect());
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
            entry.set_issue(issue.to_hayagriva());
        }

        if let Some(volume) = value.volume {
            entry.set_volume(volume.to_hayagriva())
        }

        if let Some(volume_total) = value.volume_total {
            entry.set_volume_total(volume_total.into());
        }

        if let Some(edition) = value.edition {
            entry.set_edition(edition.to_hayagriva())
        }

        if let Some(page_range) = value.page_range {
            let npage_range: MaybeTyped<hayagriva::types::PageRanges> = match page_range {
                MyMaybeTyped::Typed(t) => {
                    let my_page_ranges: MyPageRanges = t.into();
                    MaybeTyped::Typed(my_page_ranges.into())
                }
                MyMaybeTyped::String(s) => MaybeTyped::String(s),
            };
            entry.set_page_range(npage_range);
        }

        if let Some(page_total) = value.page_total {
            entry.set_page_total(page_total.into());
        }

        if let Some(time_range) = value.time_range {
            entry.set_time_range(time_range.to_hayagriva())
        }

        if let Some(runtime) = value.runtime {
            entry.set_runtime(runtime.to_hayagriva());
        }

        if let Some(url) = value.url {
            entry.set_url(url.into());
        }

        if let Some(serial_numbers) = value.serial_numbers {
            entry.set_serial_number(SerialNumber(serial_numbers));
        }

        if let Some(language) = value.language {
            entry.set_language(
                LanguageIdentifier::from_str(&language)
                    .unwrap_or(LanguageIdentifier::from_str("en-GB").unwrap()),
            );
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

        if let Some(genre) = value.genre {
            entry.set_genre(genre.into());
        }

        entry.set_parents(parents);

        Some(entry)
    }
}

/// New default metadata version
#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq, Default)]
pub struct ProjectMetadataV5 {
    /// Book Title
    pub title: String,
    /// Subtitle of the book
    pub subtitle: Option<String>,
    /// List of authors (uuid reference or free-form string)
    #[bincode(with_serde)]
    pub authors: Option<Vec<PersonUuidOrString>>,
    /// List of editors (uuid reference or free-form string)
    #[bincode(with_serde)]
    pub editors: Option<Vec<PersonUuidOrString>>,
    /// URL to a web version of the book or reference
    pub web_url: Option<String>,
    /// List of identifiers of the book (e.g. ISBNs)
    pub identifiers: Option<Vec<Identifier>>,
    /// Date of publication
    #[bincode(with_serde)]
    pub published: Option<NaiveDate>,
    /// Languages of the book
    #[bincode(with_serde)]
    pub languages: Option<Vec<Language>>,
    /// Number of pages of the book (should be automatically calculated)
    pub number_of_pages: Option<u32>,
    /// Short abstract of the book
    pub short_abstract: Option<String>,
    /// Long abstract of the book
    pub long_abstract: Option<String>,
    /// Keywords of the book
    pub keywords: Option<Vec<Keyword>>,
    /// Dewey Decimal Classification (DDC) classes (subject groups)
    pub ddc: Option<String>,
    /// License of the book
    pub license: Option<License>,
    /// Series the book belongs to
    pub series: Option<String>,
    /// Volume of the book in the series
    pub volume: Option<String>,
    /// Edition of the book
    pub edition: Option<String>,
    /// Publisher of the book
    pub publisher: Option<String>,
    /// additional fields
    pub custom_fields: HashMap<String, String>,
}

impl ProjectDataV10 {
    // TODO migrate to using path instead of the id and searching for it
    pub fn remove_section(&mut self, section_to_remove_id: &uuid::Uuid) -> Option<Section> {
        let pos = self
            .sections
            .iter()
            .position(|section| section.id == Some(*section_to_remove_id));

        match pos {
            Some(pos) => Some(self.sections.remove(pos)),
            None => {
                for section in &mut self.sections {
                    if let Some(removed) = section.remove_child_section(section_to_remove_id) {
                        return Some(removed);
                    }
                }
                None
            }
        }
    }
    pub fn insert_section_as_first_child(
        &mut self,
        parent_section_id: &uuid::Uuid,
        section_to_insert: Section,
    ) -> Result<(), ()> {
        for section in &mut self.sections {
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
        Err(())
    }
    pub fn insert_section_after(
        &mut self,
        previous_element: &uuid::Uuid,
        section_to_insert: Section,
    ) -> Result<(), ()> {
        let pos = self
            .sections
            .iter()
            .position(|section| section.id == Some(*previous_element));

        match pos {
            Some(pos) => {
                self.sections.insert(pos + 1, section_to_insert);
                Ok(())
            }
            None => {
                for section in &mut self.sections {
                    if let Some(_) =
                        section.insert_child_section_after(previous_element, &section_to_insert)
                    {
                        return Ok(());
                    }
                }
                Err(())
            }
        }
    }

    pub fn get_section(&self, section_id: &uuid::Uuid) -> Option<&Section> {
        for section in &self.sections {
            if let Some(found) = section.get_section(section_id) {
                return Some(found);
            }
        }
        None
    }

    pub fn get_section_mut(&mut self, section_id: &uuid::Uuid) -> Option<&mut Section> {
        for section in &mut self.sections {
            if let Some(found) = section.get_section_mut(section_id) {
                return Some(found);
            }
        }
        None
    }
}

pub fn get_section_by_path_mut<'a>(
    project: &'a mut RwLockWriteGuard<ProjectData>,
    path: &Vec<uuid::Uuid>,
) -> Result<&'a mut Section, ApiError> {
    // Find first section
    let first_section_opt = project.sections.iter_mut().find_map(|section| {
        if section.id.unwrap_or_default() == path[0] {
            Some(section)
        } else {
            None
        }
    });

    // Return error if no first section found
    let mut current_section = first_section_opt.ok_or_else(|| {
        println!("Couldn't find section with id {}", path[0]);
        ApiError::from(ApiErrorType::ResourceNotFound(String::from("section")))
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
                return Err(ApiErrorType::ResourceNotFound(String::from("section")).into());
            }
        }
    }

    Ok(current_section)
}

pub fn get_section_by_path<'a>(
    project: &'a RwLockReadGuard<ProjectData>,
    path: &Vec<uuid::Uuid>,
) -> Result<&'a Section, ApiError> {
    let mut first_section: Option<&Section> = None;

    // Find first section
    for section in project.sections.iter() {
        if section.id.unwrap_or_default() == path[0] {
            first_section = Some(section);
        }
    }

    // Return error if no first section found
    let first_section: &Section = match first_section {
        Some(first_section) => first_section,
        None => {
            println!("Couldn't find section with id {}", path[0]);
            return Err(ApiErrorType::ResourceNotFound(String::from("section")).into());
        }
    };

    let mut current_section: &Section = first_section;

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
            return Err(ApiErrorType::ResourceNotFound(String::from("section")).into());
        }
    }

    Ok(current_section)
}

/// is either the uuid to a person or just a string with a name
#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq, Eq, Hash)]
pub enum PersonUuidOrString {
    PersonUuid(#[bincode(with_serde)] Uuid),
    NameString(String),
}

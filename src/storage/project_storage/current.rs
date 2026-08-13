use crate::settings::Settings;
use crate::storage::project_storage::migration::load_project_data;
use crate::storage::project_storage::sections::current::SectionV6;
use crate::storage::project_storage::{ProjectData, ProjectStorage, ProjectStorageError};
use crate::storage::{BibEntryV3, MultipleFileLocks, MyMaybeTyped, MyPageRanges};
use bincode::{Decode, Encode};
use chrono::NaiveDate;
use dashmap::{DashMap, Entry};
use hayagriva::types::{MaybeTyped, SerialNumber};
use language::Language;
use rocket::serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};
use std::time::SystemTime;
use unic_langid_impl::LanguageIdentifier;
use uuid::Uuid;
use vb_exchange::projects::{Identifier, Keyword, License, ProjectSettingsV5};

impl MultipleFileLocks for ProjectStorage {
    fn get_file_lock_entry(&self, uuid: &uuid::Uuid) -> Arc<AtomicBool> {
        match self.file_locks.entry(*uuid) {
            Entry::Occupied(entry) => entry.get().clone(),
            Entry::Vacant(entry) => entry.insert(Arc::new(AtomicBool::new(false))).clone(),
        }
    }
}

impl Default for ProjectStorage {
    fn default() -> Self {
        Self::new()
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
                            error!(
                                "error while loading project into memory: couldn't parse version number: {}. Skipping file.",
                                e
                            );
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

    pub async fn get_project(
        &self,
        uuid: &uuid::Uuid,
        settings: &Settings,
    ) -> Result<Arc<RwLock<ProjectData>>, ProjectStorageError> {
        // Check if project is already in memory
        match self.projects.entry(*uuid) {
            Entry::Occupied(entry) => {
                let project = entry.get();
                project.write().unwrap().last_interaction = SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_secs();
                Ok(Arc::clone(project))
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

impl Default for Bibliography {
    fn default() -> Self {
        Self::new()
    }
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
            if let BibEntryOrFolder::BibEntry(_) = self.get_entry(parent)?.clone()
                && let Some(parent) = self.get_entry_as_hayagriva(parent)
            {
                // Caution: this could recurse infinitely if there are circular references which must be circumvented in creation
                parents.push(parent);
            }
        }

        let mut entry = hayagriva::Entry::new(&value.key.to_string(), value.entry_type);

        if let Some(title) = value.title {
            entry.set_title(title.into());
        }

        if !value.authors.is_empty() {
            entry.set_authors(value.authors.iter().map(|x| x.clone().into()).collect())
        }

        if let Some(date) = value.date {
            entry.set_date(date.into());
        }

        if !value.editors.is_empty() {
            entry.set_editors(value.editors.iter().map(|x| x.clone().into()).collect());
        }

        if !value.affiliated.is_empty() {
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
                    let my_page_ranges: MyPageRanges = t;
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

/// Serializes a [License] to its stored DB string form (for the `license` column).
pub(crate) fn license_to_string(license: &License) -> String {
    match license {
        License::CC0 => "CC0".to_string(),
        License::CC_BY_4 => "CC_BY_4".to_string(),
        License::CC_BY_SA_4 => "CC_BY_SA_4".to_string(),
        License::CC_BY_ND_4 => "CC_BY_ND_4".to_string(),
        License::CC_BY_NC_4 => "CC_BY_NC_4".to_string(),
        License::CC_BY_NC_SA_4 => "CC_BY_NC_SA_4".to_string(),
        License::CC_BY_NC_ND_4 => "CC_BY_NC_ND_4".to_string(),
        License::Other(other) => other.clone(),
    }
}

/// Parses a stored license string back into a [License], treating unknown values as
/// `License::Other`.
pub(crate) fn license_from_string(value: &str) -> License {
    match value {
        "CC0" => License::CC0,
        "CC_BY_4" => License::CC_BY_4,
        "CC_BY_SA_4" => License::CC_BY_SA_4,
        "CC_BY_ND_4" => License::CC_BY_ND_4,
        "CC_BY_NC_4" => License::CC_BY_NC_4,
        "CC_BY_NC_SA_4" => License::CC_BY_NC_SA_4,
        "CC_BY_NC_ND_4" => License::CC_BY_NC_ND_4,
        other => License::Other(other.to_string()),
    }
}

/// `authors`/`editors` come from the `persons_projects` join, not a plain column — always
/// `None` here; [`crate::db::repositories::projects::get_metadata_in_tx`] fetches them
/// separately and fills the fields in afterward (same pattern as `PersonV2::bios`).
impl<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow> for ProjectMetadataV5 {
    fn from_row(row: &'r sqlx::postgres::PgRow) -> sqlx::Result<Self> {
        use sqlx::Row;
        let keywords: Option<sqlx::types::Json<Vec<Keyword>>> = row.try_get("keywords")?;
        let custom_fields: Option<sqlx::types::Json<HashMap<String, String>>> =
            row.try_get("custom_fields")?;
        let identifiers: Option<sqlx::types::Json<Vec<Identifier>>> = row.try_get("identifiers")?;
        let languages: Option<Vec<String>> = row.try_get("languages")?;
        let license: Option<String> = row.try_get("license")?;

        Ok(ProjectMetadataV5 {
            title: row.try_get("title")?,
            subtitle: row.try_get("subtitle")?,
            authors: None,
            editors: None,
            web_url: row.try_get("web_url")?,
            identifiers: identifiers.map(|j| j.0),
            published: row.try_get("publish_date")?,
            languages: languages
                .map(|langs| langs.iter().filter_map(|l| Language::from_tag(l)).collect()),
            number_of_pages: row
                .try_get::<Option<i32>, _>("number_of_pages")?
                .map(|n| n as u32),
            short_abstract: row.try_get("short_abstract")?,
            long_abstract: row.try_get("long_abstract")?,
            keywords: keywords.map(|j| j.0),
            ddc: row.try_get("ddc")?,
            license: license.as_deref().map(license_from_string),
            series: row.try_get("series")?,
            volume: row.try_get("volume")?,
            edition: row.try_get("edition")?,
            publisher: row.try_get("publisher")?,
            custom_fields: custom_fields.map(|j| j.0).unwrap_or_default(),
        })
    }
}

/// is either the uuid to a person or just a string with a name
#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq, Eq, Hash)]
pub enum PersonUuidOrString {
    PersonUuid(#[bincode(with_serde)] Uuid),
    NameString(String),
}

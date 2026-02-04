pub mod data_storage;
pub mod project_storage;

use crate::settings::Settings;
use argon2::password_hash::rand_core::OsRng;
use argon2::{Argon2, PasswordHasher};
use bincode::{Decode, Encode};
use hayagriva::types::{
    Date, Duration, DurationRange, FormatString, MaybeTyped, Numeric, NumericDelimiter,
    NumericValue, QualifiedUrl, SerialNumber,
};
use hayagriva::types::{EntryType, PageRanges, PageRangesPart};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::str::FromStr;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::SystemTime;

use crate::storage::data_storage::DataStorage;
use crate::storage::project_storage::ProjectStorage;
use unic_langid_impl::LanguageIdentifier;
use vb_exchange::export_formats::ExportFormat;

/// Trait for data structures that need to handle multiple file locks
///
/// Struct needs to implement [MultipleFileLocks::get_file_lock_entry]
#[async_trait]
pub trait MultipleFileLocks {
    /// Returns an [AtomicBool] that is used as file lock for the given uuid
    fn get_file_lock_entry(&self, uuid: &uuid::Uuid) -> Arc<AtomicBool>;
    /// Creates a file lock for the given uuid
    fn create_file_lock(&self, uuid: &uuid::Uuid) -> Result<(), ()> {
        if self
            .get_file_lock_entry(uuid)
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            // file already locked
            Err(())
        } else {
            // file not locked, lock it
            self.get_file_lock_entry(uuid)
                .store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
    }
    fn remove_file_lock(&self, uuid: &uuid::Uuid) {
        self.get_file_lock_entry(uuid)
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }

    async fn wait_for_file_lock(&self, uuid: &uuid::Uuid, settings: &Settings) -> Result<(), ()> {
        let mut time_waited = 0;
        while self.create_file_lock(uuid).is_err() {
            time_waited += 10;
            if time_waited > settings.file_lock_timeout {
                error!("error while waiting for file lock: waiting for file lock timed out. Waited for {} ms, exceeding the configured limit.", time_waited);
                return Err(());
            }

            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        Ok(())
    }
}

#[async_trait]
pub trait SingleFileLock {
    fn get_file_lock(&self) -> &AtomicBool;

    fn create_file_lock(&self) -> Result<(), ()> {
        if self
            .get_file_lock()
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            // file already locked
            Err(())
        } else {
            // file not locked, lock it
            self.get_file_lock()
                .store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
    }

    fn remove_file_lock(&self) {
        self.get_file_lock()
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }

    async fn wait_for_file_lock(&self, settings: &Settings) -> Result<(), ()> {
        let mut time_waited = 0;
        while self.create_file_lock().is_err() {
            time_waited += 10;
            if time_waited > settings.file_lock_timeout {
                eprintln!("error while waiting for file lock: waiting for file lock timed out. Waited for {} ms, exceeding the configured limit.", time_waited);
                return Err(());
            }

            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        Ok(())
    }
}

#[derive(serde::Serialize)]
pub struct ProjectListEntry {
    id: uuid::Uuid,
    name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct MyPersonsWithRoles {
    /// The persons.
    pub names: Vec<MyPerson>,
    /// The role the persons had in the creation of the cited item.
    pub role: MyPersonRole,
}

impl From<hayagriva::types::PersonsWithRoles> for MyPersonsWithRoles {
    fn from(value: hayagriva::types::PersonsWithRoles) -> Self {
        MyPersonsWithRoles {
            names: value.names.iter().map(|p| p.clone().into()).collect(),
            role: value.role.into(),
        }
    }
}

impl From<MyPersonsWithRoles> for hayagriva::types::PersonsWithRoles {
    fn from(value: MyPersonsWithRoles) -> Self {
        hayagriva::types::PersonsWithRoles {
            names: value.names.iter().map(|p| p.clone().into()).collect(),
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

impl From<MyPersonRole> for hayagriva::types::PersonRole {
    fn from(value: MyPersonRole) -> Self {
        match value {
            MyPersonRole::Translator => hayagriva::types::PersonRole::Translator,
            MyPersonRole::Afterword => hayagriva::types::PersonRole::Afterword,
            MyPersonRole::Foreword => hayagriva::types::PersonRole::Foreword,
            MyPersonRole::Introduction => hayagriva::types::PersonRole::Introduction,
            MyPersonRole::Annotator => hayagriva::types::PersonRole::Annotator,
            MyPersonRole::Commentator => hayagriva::types::PersonRole::Commentator,
            MyPersonRole::Holder => hayagriva::types::PersonRole::Holder,
            MyPersonRole::Compiler => hayagriva::types::PersonRole::Compiler,
            MyPersonRole::Founder => hayagriva::types::PersonRole::Founder,
            MyPersonRole::Collaborator => hayagriva::types::PersonRole::Collaborator,
            MyPersonRole::Organizer => hayagriva::types::PersonRole::Organizer,
            MyPersonRole::CastMember => hayagriva::types::PersonRole::CastMember,
            MyPersonRole::Composer => hayagriva::types::PersonRole::Composer,
            MyPersonRole::Producer => hayagriva::types::PersonRole::Producer,
            MyPersonRole::ExecutiveProducer => hayagriva::types::PersonRole::ExecutiveProducer,
            MyPersonRole::Writer => hayagriva::types::PersonRole::Writer,
            MyPersonRole::Cinematography => hayagriva::types::PersonRole::Cinematography,
            MyPersonRole::Director => hayagriva::types::PersonRole::Director,
            MyPersonRole::Illustrator => hayagriva::types::PersonRole::Illustrator,
            MyPersonRole::Narrator => hayagriva::types::PersonRole::Narrator,
            MyPersonRole::Unknown(s) => hayagriva::types::PersonRole::Unknown(s),
        }
    }
}

impl From<hayagriva::types::PersonRole> for MyPersonRole {
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

impl<T> MyMaybeTyped<T> {
    pub fn to_hayagriva<U>(self) -> hayagriva::types::MaybeTyped<U>
    where
        T: Into<U>,
    {
        match self {
            MyMaybeTyped::Typed(t) => hayagriva::types::MaybeTyped::Typed(t.into()),
            MyMaybeTyped::String(s) => hayagriva::types::MaybeTyped::String(s),
        }
    }

    pub fn from_hayagriva<U>(value: hayagriva::types::MaybeTyped<U>) -> Self
    where
        U: Into<T>,
    {
        match value {
            hayagriva::types::MaybeTyped::Typed(t) => MyMaybeTyped::Typed(t.into()),
            hayagriva::types::MaybeTyped::String(s) => MyMaybeTyped::String(s),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MyPerson {
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

impl From<hayagriva::types::Person> for MyPerson {
    fn from(value: hayagriva::types::Person) -> Self {
        MyPerson {
            name: value.name,
            given_name: value.given_name,
            prefix: value.prefix,
            suffix: value.suffix,
            alias: value.alias,
        }
    }
}

impl From<MyPerson> for hayagriva::types::Person {
    fn from(value: MyPerson) -> Self {
        hayagriva::types::Person {
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

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Encode, Decode)]
pub struct MyPublisher {
    #[bincode(with_serde)]
    pub name: MyFormatString,
    #[bincode(with_serde)]
    pub location: Option<MyFormatString>,
}

impl From<hayagriva::types::Publisher> for MyPublisher {
    fn from(value: hayagriva::types::Publisher) -> Self {
        let default_fs = FormatString::from_str("").unwrap();
        let name = value.name().unwrap_or(&default_fs);
        MyPublisher {
            name: name.clone().into(),
            location: value.location().map(|l| l.clone().into()),
        }
    }
}

impl From<MyPublisher> for hayagriva::types::Publisher {
    fn from(value: MyPublisher) -> Self {
        let name: FormatString = value.name.into();
        hayagriva::types::Publisher::new(Some(name), value.location.map(|l| l.into()))
    }
}

/// Struct similar to [hayagriva::Entry], but without special serde annotations, since Bincode doesn't support these
/// For convenience, the struct implements [From] and [Into] for [hayagriva::Entry] and reverse
#[derive(Debug, Serialize, Deserialize, Encode, Decode, Clone)]
pub struct BibEntryV3 {
    #[bincode(with_serde)]
    pub key: uuid::Uuid,
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
    pub publisher: Option<MyPublisher>,
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
    pub page_range: Option<MyMaybeTyped<MyPageRanges>>,
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
    pub genre: Option<MyFormatString>,
    #[bincode(with_serde)]
    pub parents: Vec<uuid::Uuid>,
}

impl From<OldBibEntry> for BibEntryV2 {
    fn from(value: OldBibEntry) -> Self {
        BibEntryV2 {
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
        let title = match value.title() {
            Some(title) => Some(title.clone().into()),
            None => None,
        };
        let publisher = match value.publisher() {
            Some(publisher) => {
                let default_fs = FormatString::from_str("").unwrap();
                let name = publisher.name().unwrap_or(&default_fs);
                Some(MyFormatString {
                    value: name.value.to_string(),
                    short: name.short.as_ref().map(|s| s.to_string()),
                })
            }
            None => None,
        };
        let location = match value.location() {
            Some(location) => Some(location.clone().into()),
            None => None,
        };
        let organization = match value.organization() {
            Some(organization) => Some(organization.clone().into()),
            None => None,
        };
        let archive = match value.archive() {
            Some(archive) => Some(archive.clone().into()),
            None => None,
        };
        let archive_location = match value.archive_location() {
            Some(archive_location) => Some(archive_location.clone().into()),
            None => None,
        };
        let call_number = match value.call_number() {
            Some(call_number) => Some(call_number.clone().into()),
            None => None,
        };
        let note = match value.note() {
            Some(note) => Some(note.clone().into()),
            None => None,
        };
        let abstract_ = match value.abstract_() {
            Some(abstract_) => Some(abstract_.clone().into()),
            None => None,
        };
        let annote = None;
        let genre = match value.genre() {
            Some(genre) => Some(genre.clone().into()),
            None => None,
        };
        let authors = match value.authors() {
            Some(authors) => authors.iter().map(|x| x.clone().into()).collect(),
            None => vec![],
        };
        let editors = match value.editors() {
            Some(editors) => editors.iter().map(|x| x.clone().into()).collect(),
            None => vec![],
        };

        let serial_numbers = match value.serial_number() {
            Some(serial_numbers) => Some(serial_numbers.0.clone()),
            None => None,
        };

        let issue = match value.issue() {
            Some(issue) => Some(MyMaybeTyped::from_hayagriva(issue.clone())),
            None => None,
        };
        let volume = match value.volume() {
            Some(volume) => Some(MyMaybeTyped::from_hayagriva(volume.clone())),
            None => None,
        };
        let edition = match value.edition() {
            Some(edition) => Some(MyMaybeTyped::from_hayagriva(edition.clone())),
            None => None,
        };
        let page_range = match value.page_range() {
            Some(page_range) => match page_range {
                MaybeTyped::Typed(t) => {
                    let my_page_ranges: MyPageRanges = t.clone().into();
                    let my_numeric: MyNumeric = my_page_ranges.into();
                    Some(MyMaybeTyped::Typed(my_numeric))
                }
                MaybeTyped::String(s) => Some(MyMaybeTyped::String(s.to_string())),
            },
            None => None,
        };
        let volume_total = match value.volume_total() {
            Some(volume_total) => Some(volume_total.clone().into()),
            None => None,
        };
        let page_total = match value.page_total() {
            Some(page_total) => Some(page_total.clone().into()),
            None => None,
        };
        let url = match value.url() {
            Some(url) => Some(url.clone().into()),
            None => None,
        };
        let date = match value.date() {
            Some(date) => Some(date.clone().into()),
            None => None,
        };
        let language = match value.language() {
            Some(language) => Some(language.to_string()),
            None => None,
        };
        let affiliated = match value.affiliated() {
            Some(affiliated) => affiliated.iter().map(|x| x.clone().into()).collect(),
            None => vec![],
        };
        let time_range = match value.time_range() {
            Some(time_range) => Some(MyMaybeTyped::from_hayagriva(time_range.clone())),
            None => None,
        };
        let runtime = match value.runtime() {
            Some(runtime) => Some(MyMaybeTyped::from_hayagriva(runtime.clone())),
            None => None,
        };

        let parents_arr = value.parents();
        let mut parents = vec![];
        if parents_arr.len() > 0 {
            parents = parents_arr.iter().map(|x| (&x.clone()).into()).collect();
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

impl From<BibEntryV2> for hayagriva::Entry {
    fn from(value: BibEntryV2) -> Self {
        let mut entry = hayagriva::Entry::new(&value.key, value.entry_type);

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
            entry.set_publisher(hayagriva::types::Publisher::new(
                Some(publisher.into()),
                None,
            ));
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
            entry.set_volume(volume.to_hayagriva());
        }

        if let Some(volume_total) = value.volume_total {
            entry.set_volume_total(volume_total.into());
        }

        if let Some(edition) = value.edition {
            entry.set_edition(edition.to_hayagriva());
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
            entry.set_time_range(time_range.to_hayagriva());
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

        if value.parents.len() > 0 {
            entry.set_parents(
                value
                    .parents
                    .iter()
                    .map(|x| {
                        <BibEntryV2 as Clone>::clone(&(*(&<BibEntryV2 as Clone>::clone(&(*x)))))
                            .into()
                    })
                    .collect(),
            );
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
        let password_hash = Argon2::default()
            .hash_password(&password.as_bytes(), &salt)
            .unwrap()
            .to_string();

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

pub async fn save_data_worker(
    data_storage: Arc<DataStorage>,
    project_storage: Arc<ProjectStorage>,
    settings: Settings,
) {
    tokio::spawn(async move {
        let mut last_save = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(
                settings.backup_to_file_interval,
            ))
            .await;
            // Save DataStorage to disk
            println!("Saving DataStorage to disk");
            data_storage.save_to_disk(&settings).await.unwrap();

            // Save ProjectStorage to disk
            let mut projects_to_save = Vec::new();
            for project_id in project_storage.projects.read().unwrap().keys() {
                if let Some(project) = project_storage.projects.read().unwrap().get(project_id) {
                    if let Some(project) = &project.data {
                        if project.read().unwrap().last_interaction > last_save {
                            projects_to_save.push(project_id.clone());
                        }
                    }
                }
            }
            for project_id in projects_to_save {
                println!("Saving changed project {} to disk", project_id);
                project_storage
                    .save_project_to_disk(&project_id, &settings)
                    .await
                    .unwrap(); //TODO: shutdown if this fails to avoid data loss
            }
            last_save = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs();
            println!("Finished saving projects to disk");
        }
    });
}

impl From<MyFormatString> for FormatString {
    fn from(my_format_string: MyFormatString) -> Self {
        match my_format_string.short {
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
pub struct MyQualifiedUrl {
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
pub struct MyPageRanges {
    /// The given ranges.
    pub ranges: Vec<MyPageRangesPart>,
}

impl From<hayagriva::types::PageRanges> for MyPageRanges {
    fn from(value: hayagriva::types::PageRanges) -> Self {
        MyPageRanges {
            ranges: value.ranges.into_iter().map(|r| r.into()).collect(),
        }
    }
}

impl From<MyPageRanges> for hayagriva::types::PageRanges {
    fn from(value: MyPageRanges) -> Self {
        hayagriva::types::PageRanges {
            ranges: value.ranges.into_iter().map(|r| r.into()).collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MyPageRangesPart {
    /// An and, i.e, `&`.
    Ampersand,
    /// A comma, i.e., `,`.
    Comma,
    /// An escaped range with start and end, e.g., `1\-4`.
    EscapedRange(MyNumeric, MyNumeric),
    /// A single page, e.g., `5`.
    SinglePage(MyNumeric),
    /// A full range, e.g., `1n8--1n14`.
    Range(MyNumeric, MyNumeric),
}

impl From<hayagriva::types::PageRangesPart> for MyPageRangesPart {
    fn from(value: PageRangesPart) -> Self {
        match value {
            PageRangesPart::Ampersand => MyPageRangesPart::Ampersand,
            PageRangesPart::Comma => MyPageRangesPart::Comma,
            PageRangesPart::EscapedRange(start, end) => {
                MyPageRangesPart::EscapedRange(start.into(), end.into())
            }
            PageRangesPart::SinglePage(page) => MyPageRangesPart::SinglePage(page.into()),
            PageRangesPart::Range(start, end) => MyPageRangesPart::Range(start.into(), end.into()),
        }
    }
}

impl From<MyPageRangesPart> for PageRangesPart {
    fn from(value: MyPageRangesPart) -> Self {
        match value {
            MyPageRangesPart::Ampersand => PageRangesPart::Ampersand,
            MyPageRangesPart::Comma => PageRangesPart::Comma,
            MyPageRangesPart::EscapedRange(start, end) => {
                PageRangesPart::EscapedRange(start.into(), end.into())
            }
            MyPageRangesPart::SinglePage(page) => PageRangesPart::SinglePage(page.into()),
            MyPageRangesPart::Range(start, end) => PageRangesPart::Range(start.into(), end.into()),
        }
    }
}

impl From<MyPageRanges> for MyNumeric {
    fn from(value: MyPageRanges) -> Self {
        if value.ranges.is_empty() {
            return MyNumeric {
                value: MyNumericValue::Number(0),
                prefix: None,
                suffix: None,
            };
        }

        let mut set = Vec::new();
        let mut last_prefix = None;
        let mut last_suffix = None;

        for (i, range_part) in value.ranges.iter().enumerate() {
            match range_part {
                MyPageRangesPart::SinglePage(n) => {
                    if let MyNumericValue::Number(val) = n.value {
                        let delimiter = if i + 1 < value.ranges.iter().len() {
                            match &value.ranges[i + 1] {
                                MyPageRangesPart::Comma => Some(MyNumericDelimiter::Comma),
                                MyPageRangesPart::Ampersand => Some(MyNumericDelimiter::Ampersand),
                                _ => None,
                            }
                        } else {
                            None
                        };
                        set.push((val, delimiter));
                        if last_prefix.is_none() {
                            last_prefix = n.prefix.clone();
                        }
                        if last_suffix.is_none() {
                            last_suffix = n.suffix.clone();
                        }
                    }
                }
                MyPageRangesPart::Range(start, end)
                | MyPageRangesPart::EscapedRange(start, end) => {
                    if let MyNumericValue::Number(s_val) = start.value {
                        set.push((s_val, Some(MyNumericDelimiter::Hyphen)));
                        if last_prefix.is_none() {
                            last_prefix = start.prefix.clone();
                        }
                        if last_suffix.is_none() {
                            last_suffix = start.suffix.clone();
                        }
                    }
                    if let MyNumericValue::Number(e_val) = end.value {
                        let delimiter = if i + 1 < value.ranges.iter().len() {
                            match &value.ranges[i + 1] {
                                MyPageRangesPart::Comma => Some(MyNumericDelimiter::Comma),
                                MyPageRangesPart::Ampersand => Some(MyNumericDelimiter::Ampersand),
                                _ => None,
                            }
                        } else {
                            None
                        };
                        set.push((e_val, delimiter));
                    }
                }
                _ => {}
            }
        }

        if set.len() == 1 && set[0].1.is_none() {
            MyNumeric {
                value: MyNumericValue::Number(set[0].0),
                prefix: last_prefix,
                suffix: last_suffix,
            }
        } else {
            MyNumeric {
                value: MyNumericValue::Set(set),
                prefix: last_prefix,
                suffix: last_suffix,
            }
        }
    }
}

impl From<MyNumeric> for MyPageRanges {
    fn from(value: MyNumeric) -> Self {
        match value.value {
            MyNumericValue::Number(_) => MyPageRanges {
                ranges: vec![MyPageRangesPart::SinglePage(value)],
            },
            MyNumericValue::Set(set) => {
                let mut ranges = Vec::new();
                let mut i = 0;
                while i < set.len() {
                    let (val, delim) = &set[i];
                    let current_numeric = MyNumeric {
                        value: MyNumericValue::Number(*val),
                        prefix: value.prefix.clone(),
                        suffix: value.suffix.clone(),
                    };

                    match delim {
                        Some(MyNumericDelimiter::Hyphen) => {
                            if i + 1 < set.len() {
                                let (next_val, next_delim) = &set[i + 1];
                                let next_numeric = MyNumeric {
                                    value: MyNumericValue::Number(*next_val),
                                    prefix: value.prefix.clone(),
                                    suffix: value.suffix.clone(),
                                };
                                ranges.push(MyPageRangesPart::Range(current_numeric, next_numeric));
                                if let Some(d) = next_delim {
                                    match d {
                                        MyNumericDelimiter::Comma => {
                                            ranges.push(MyPageRangesPart::Comma)
                                        }
                                        MyNumericDelimiter::Ampersand => {
                                            ranges.push(MyPageRangesPart::Ampersand)
                                        }
                                        _ => {}
                                    }
                                }
                                i += 2;
                                continue;
                            } else {
                                ranges.push(MyPageRangesPart::SinglePage(current_numeric));
                            }
                        }
                        Some(MyNumericDelimiter::Comma) => {
                            ranges.push(MyPageRangesPart::SinglePage(current_numeric));
                            ranges.push(MyPageRangesPart::Comma);
                        }
                        Some(MyNumericDelimiter::Ampersand) => {
                            ranges.push(MyPageRangesPart::SinglePage(current_numeric));
                            ranges.push(MyPageRangesPart::Ampersand);
                        }
                        None => {
                            ranges.push(MyPageRangesPart::SinglePage(current_numeric));
                        }
                    }
                    i += 1;
                }
                MyPageRanges { ranges }
            }
        }
    }
}

impl From<MyMaybeTyped<MyPageRanges>> for MyMaybeTyped<MyNumeric> {
    fn from(value: MyMaybeTyped<MyPageRanges>) -> Self {
        match value {
            MyMaybeTyped::Typed(t) => MyMaybeTyped::Typed(t.into()),
            MyMaybeTyped::String(s) => MyMaybeTyped::String(s),
        }
    }
}

impl From<MyMaybeTyped<MyNumeric>> for MyMaybeTyped<MyPageRanges> {
    fn from(value: MyMaybeTyped<MyNumeric>) -> Self {
        match value {
            MyMaybeTyped::Typed(t) => MyMaybeTyped::Typed(t.into()),
            MyMaybeTyped::String(s) => MyMaybeTyped::String(s),
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
            NumericValue::Set(s) => MyNumericValue::Set(
                s.into_iter()
                    .map(|(n, d)| (n, d.map(|d| d.into())))
                    .collect(),
            ),
        }
    }
}

impl From<MyNumericValue> for NumericValue {
    fn from(value: MyNumericValue) -> Self {
        match value {
            MyNumericValue::Number(n) => NumericValue::Number(n),
            MyNumericValue::Set(s) => NumericValue::Set(
                s.into_iter()
                    .map(|(n, d)| (n, d.map(|d| d.into())))
                    .collect(),
            ),
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
            Some(month) => Some(month + 1),
            None => None,
        };
        let day = match value.day {
            Some(day) => Some(day + 1),
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
            Some(month) => {
                if month == 0 {
                    Some(month)
                } else {
                    Some(month - 1)
                }
            }
            None => None,
        };
        let day = match value.day {
            Some(day) => {
                if day == 0 {
                    Some(day)
                } else {
                    Some(day - 1)
                }
            }
            None => None,
        };

        Date {
            year: value.year,
            month,
            day,
            approximate: value.approximate,
            season: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MyDurationRange {
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

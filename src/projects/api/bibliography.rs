use std::collections::BTreeMap;
use hayagriva::types::{Duration, EntryType};
use rocket::serde::{Deserialize, Serialize};
use crate::data_storage::{BibEntryOrFolder, ProjectBibliography, MyDate, MyDurationRange, MyFormatString, MyMaybeTyped, MyNumeric, MyPerson, MyPersonsWithRoles, MyQualifiedUrl};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ApiBibEntryShort{
    pub uuid: uuid::Uuid,
    pub entry_type: EntryType,
    pub title: Option<MyFormatString>,
    pub authors: Vec<MyPerson>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ApiBibEntry{
    pub uuid: uuid::Uuid,
    pub entry_type: EntryType,
    pub title: Option<MyFormatString>,
    pub authors: Vec<MyPerson>,
    pub date: Option<MyDate>,
    pub editors: Vec<MyPerson>,
    pub affiliated: Vec<MyPersonsWithRoles>,
    pub publisher: Option<MyFormatString>,
    pub location: Option<MyFormatString>,
    pub organization: Option<MyFormatString>,
    pub issue: Option<MyMaybeTyped<MyNumeric>>,
    pub volume: Option<MyMaybeTyped<MyNumeric>>,
    pub volume_total: Option<MyNumeric>,
    pub edition: Option<MyMaybeTyped<MyNumeric>>,
    pub page_range: Option<MyMaybeTyped<MyNumeric>>,
    pub page_total: Option<MyNumeric>,
    pub time_range: Option<MyMaybeTyped<MyDurationRange>>,
    pub runtime: Option<MyMaybeTyped<Duration>>,
    pub url: Option<MyQualifiedUrl>,
    pub serial_numbers: Option<BTreeMap<String, String>>,
    pub language: Option<String>,
    pub archive: Option<MyFormatString>,
    pub archive_location: Option<MyFormatString>,
    pub call_number: Option<MyFormatString>,
    pub note: Option<MyFormatString>,
    pub abstractt: Option<MyFormatString>,
    pub annote: Option<MyFormatString>,
    pub genre: Option<MyFormatString>,
    pub children: Vec<uuid::Uuid>,
    pub children_expanded: Vec<BibEntryOrFolder>
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ApiNewBibEntry{
    pub entry_type: EntryType
}



/// GET /api/projects/<project_id>/bibliography/<entry_id>
/// Retrieve a specific bib entry
///
/// Returns: [ApiBibEntry]


/// GET /api/projects/<project_id>/bibliography?<expand>
/// Retrieve all bib entries
///
/// Parameters:
/// * expand - true if children entries should be expanded
///
/// Returns: [ProjectBibliography]


/// DELETE /api/projects/<project_id>/bibliography/<entry_id>
/// Deletes bib entry and all sub entries
///
/// Parameters:
/// * entry_id - uuid of the bib entry


/// POST /api/projects/<project_id>/bibliography
/// Creates a new bib entry


/// PATCH /api/projects/<project_id>/bibliography/<entry_id>
pub fn test(){}
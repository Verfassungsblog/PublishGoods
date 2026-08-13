use crate::storage::project_storage::current::PersonUuidOrString;
use crate::storage::project_storage::sections::Section;
use bincode::{Decode, Encode};
use chrono::{NaiveDate, NaiveDateTime};
use language::Language;
use rocket::serde::{Deserialize, Serialize};
use std::collections::HashMap;
use vb_exchange::projects::Identifier;

/// Struct holds all metadata for a section
#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq)]
pub struct SectionV6 {
    /// Unique id of the section
    /// Only None if the section is not yet saved in the database
    #[bincode(with_serde)]
    pub id: Option<uuid::Uuid>,
    /// Additional classes to style the Section
    pub css_classes: Vec<String>,
    /// Holds all subsections
    pub sub_sections: Vec<SectionV6>,
    /// Holds a copy of the yrs document
    pub content: Vec<u8>,
    /// If true, the section is visible in the table of contents
    pub visible_in_toc: bool,
    /// Metadata of the section
    pub metadata: SectionMetadataV6,
}

/// `sub_sections` is the recursive tree shape (assembled in-Rust from multiple flat rows, not
/// a column) and `content` is the CRDT body, which never lives in Postgres at all (stays on
/// the filesystem, see `db::section_content`) — both always come back empty/default here;
/// [`crate::db::repositories::sections::get_tree_for_project`] fills them in afterward.
impl<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow> for SectionV6 {
    fn from_row(row: &'r sqlx::postgres::PgRow) -> sqlx::Result<Self> {
        use sqlx::Row;
        let css_classes: Option<Vec<String>> = row.try_get("css_classes")?;
        Ok(SectionV6 {
            id: Some(row.try_get("id")?),
            css_classes: css_classes.unwrap_or_default(),
            sub_sections: vec![],
            content: vec![],
            visible_in_toc: row.try_get("visible_in_toc")?,
            metadata: SectionMetadataV6::from_row(row)?,
        })
    }
}

/// Struct holds all metadata of a section
#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq)]
pub struct SectionMetadataV6 {
    pub title: String,
    pub toc_title_subtitle_override: Option<String>,
    pub subtitle: Option<String>,
    #[bincode(with_serde)]
    pub authors: Vec<PersonUuidOrString>,
    #[bincode(with_serde)]
    pub editors: Vec<PersonUuidOrString>,
    pub web_url: Option<String>,
    pub identifiers: Vec<Identifier>,
    #[bincode(with_serde)]
    pub published: Option<NaiveDate>,
    #[bincode(with_serde)]
    pub last_changed: Option<NaiveDateTime>,
    #[bincode(with_serde)]
    pub lang: Option<Language>,
    /// additional fields
    pub custom_fields: HashMap<String, String>,
}

/// `authors`/`editors` come from the `persons_sections` join, not a plain column — always
/// empty here; [`crate::db::repositories::sections::get_tree_for_project`] fetches them
/// separately and fills the fields in afterward (same pattern as `PersonV2::bios`).
/// `last_changed` has no backing column at all (see the schema-gap note in
/// `db::repositories::sections`) — always round-trips as `None`.
impl<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow> for SectionMetadataV6 {
    fn from_row(row: &'r sqlx::postgres::PgRow) -> sqlx::Result<Self> {
        use sqlx::Row;
        let identifiers: Option<sqlx::types::Json<Vec<Identifier>>> = row.try_get("identifiers")?;
        let custom_fields: Option<sqlx::types::Json<HashMap<String, String>>> =
            row.try_get("custom_fields")?;
        let language: Option<String> = row.try_get("language")?;
        Ok(SectionMetadataV6 {
            title: row.try_get("title")?,
            toc_title_subtitle_override: row.try_get("toc_title_subtitle_override")?,
            subtitle: row.try_get("subtitle")?,
            authors: vec![],
            editors: vec![],
            web_url: row.try_get("web_url")?,
            identifiers: identifiers.map(|j| j.0).unwrap_or_default(),
            published: row.try_get("publish_date")?,
            last_changed: None,
            lang: language.as_deref().and_then(Language::from_tag),
            custom_fields: custom_fields.map(|j| j.0).unwrap_or_default(),
        })
    }
}

impl Section {
    pub fn clone_without_subsections(&self) -> Section {
        Section {
            id: self.id,
            css_classes: self.css_classes.clone(),
            sub_sections: Vec::new(),
            content: self.content.clone(),
            visible_in_toc: self.visible_in_toc,
            metadata: self.metadata.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_v6_deserializes_from_project_contents_payload() {
        // This matches the payload sent by the frontend when creating a new section via
        // `POST /api/projects/<project_id>/contents`.
        let payload = r#"{
            "css_classes":[],
            "sub_sections":[],
            "content":[],
            "visible_in_toc":true,
            "metadata":{
                "title":"New Section",
                "toc_title_subtitle_override":null,
                "subtitle":null,
                "authors":[],
                "editors":[],
                "web_url":null,
                "identifiers":[],
                "published":null,
                "last_changed":null,
                "lang":null,
                "custom_fields":{}
            }
        }"#;

        let section: Section =
            serde_json::from_str(payload).expect("Section JSON should deserialize");
        assert_eq!(section.metadata.title, "New Section");
        assert!(section.content.is_empty());
        assert!(section.metadata.custom_fields.is_empty());
        assert!(section.visible_in_toc);
    }
}

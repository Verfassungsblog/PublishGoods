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

impl Section {
    pub fn clone_without_content(&self) -> Section {
        let mut new_section = Section {
            id: self.id.clone(),
            css_classes: self.css_classes.clone(),
            sub_sections: self.sub_sections.clone(),
            content: vec![],
            visible_in_toc: self.visible_in_toc.clone(),
            metadata: self.metadata.clone(),
        };
        new_section
    }

    pub fn clone_without_subsections(&self) -> Section {
        let mut new_section = Section {
            id: self.id.clone(),
            css_classes: self.css_classes.clone(),
            sub_sections: Vec::new(),
            content: self.content.clone(),
            visible_in_toc: self.visible_in_toc.clone(),
            metadata: self.metadata.clone(),
        };
        new_section
    }

    pub fn truncate_content_recursive(&mut self) {
        self.content = vec![];
        for sub_section in self.sub_sections.iter_mut() {
            sub_section.truncate_content_recursive();
        }
    }

    pub fn insert_child_section_as_child(
        &mut self,
        parent_section_id: &uuid::Uuid,
        new_section: &Section,
    ) -> Option<()> {
        for section in self.sub_sections.iter_mut() {
            if section.id == Some(*parent_section_id) {
                section.sub_sections.push(new_section.clone());
                return Some(());
            } else {
                match section.insert_child_section_as_child(parent_section_id, new_section) {
                    Some(_) => return Some(()),
                    None => {}
                }
            }
        }
        None
    }

    pub fn insert_child_section_after(
        &mut self,
        section_id: &uuid::Uuid,
        new_section: &Section,
    ) -> Option<()> {
        for (i, section) in self.sub_sections.iter_mut().enumerate() {
            if section.id == Some(*section_id) {
                self.sub_sections.insert(i + 1, new_section.clone());
                return Some(());
            } else {
                match section.insert_child_section_after(section_id, new_section) {
                    Some(_) => return Some(()),
                    None => {}
                }
            }
        }
        None
    }

    pub fn remove_child_section(&mut self, section_id: &uuid::Uuid) -> Option<Section> {
        let mut index = None;
        for (i, section) in self.sub_sections.iter_mut().enumerate() {
            if section.id == Some(*section_id) {
                index = Some(i);
            } else {
                match section.remove_child_section(section_id) {
                    Some(section) => return Some(section),
                    None => {}
                }
            }
        }
        match index {
            Some(index) => {
                let section = self.sub_sections.remove(index);
                Some(section)
            }
            None => None,
        }
    }

    pub fn get_section(&self, section_id: &uuid::Uuid) -> Option<&Section> {
        if self.id == Some(*section_id) {
            return Some(self);
        }
        for section in &self.sub_sections {
            if let Some(found) = section.get_section(section_id) {
                return Some(found);
            }
        }
        None
    }

    pub fn get_section_mut(&mut self, section_id: &uuid::Uuid) -> Option<&mut Section> {
        if self.id == Some(*section_id) {
            return Some(self);
        }
        for section in &mut self.sub_sections {
            if let Some(found) = section.get_section_mut(section_id) {
                return Some(found);
            }
        }
        None
    }
}

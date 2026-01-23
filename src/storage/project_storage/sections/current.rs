use crate::storage::project_storage::current::PersonUuidOrString;
use crate::storage::project_storage::sections::content::current::NewContentBlock;
use crate::storage::project_storage::sections::Section;
use bincode::{Decode, Encode};
use chrono::{NaiveDate, NaiveDateTime};
use language::Language;
use rocket::serde::{Deserialize, Serialize};
use vb_exchange::projects::Identifier;

#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq)]
pub enum SectionOrTocV5 {
    Section(SectionV5),
    Toc,
}

/// Struct holds all data for a section (e.g. chapter, part, ...)
#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq)]
pub struct SectionV5 {
    /// Unique id of the section
    /// Only None if the section is not yet saved in the database
    #[bincode(with_serde)]
    pub id: Option<uuid::Uuid>,
    /// Additional classes to style the Section
    pub css_classes: Vec<String>,
    /// Holds all subsections
    pub sub_sections: Vec<SectionV5>,
    // Holds all content blocks
    pub children: Vec<NewContentBlock>,
    /// If true, the section is visible in the table of contents
    pub visible_in_toc: bool,
    /// Metadata of the section
    pub metadata: SectionMetadataV5,
}

/// Struct holds all metadata of a section
#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq)]
pub struct SectionMetadataV5 {
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
}

impl Section {
    pub fn clone_without_contentblocks(&self) -> Section {
        let mut new_section = self.clone();
        new_section.children = vec![];
        new_section
    }

    pub fn clone_without_subsections(&self) -> Section {
        let mut new_section = self.clone();
        new_section.sub_sections = vec![];
        new_section
    }

    pub fn truncate_children_recursive(&mut self) {
        self.children = vec![];
        for sub_section in self.sub_sections.iter_mut() {
            sub_section.truncate_children_recursive();
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
}

impl SectionOrTocV5 {
    pub fn into_section(self) -> Option<SectionV5> {
        match self {
            SectionOrTocV5::Section(section) => Some(section),
            SectionOrTocV5::Toc => None,
        }
    }
}

use crate::storage::project_storage::current::PersonUuidOrString;
use crate::storage::project_storage::sections::content::current::NewContentBlock;
use crate::storage::project_storage::sections::current::{SectionMetadataV6, SectionV6};
use bincode::{Decode, Encode};
use chrono::{NaiveDate, NaiveDateTime};
use language::Language;
use rocket::serde::{Deserialize, Serialize};
use std::collections::HashMap;
use vb_exchange::deprecated::projects::data_storage::OldLanguage;
use vb_exchange::projects::Identifier;
use yrs::{Array, Doc, MapPrelim, ReadTxn, StateVector, Transact};

/// Enum to differentiate between real sections and the position of the table of contents
//TODO: remove
#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq)]
pub enum SectionOrTocV1 {
    Section(SectionV1),
    Toc,
}

impl SectionOrTocV1 {
    pub fn into_section(self) -> Option<SectionV1> {
        match self {
            SectionOrTocV1::Section(section) => Some(section),
            SectionOrTocV1::Toc => None,
        }
    }
}

impl From<SectionOrTocV1> for SectionOrTocV2 {
    fn from(value: SectionOrTocV1) -> Self {
        match value {
            SectionOrTocV1::Section(section) => SectionOrTocV2::Section(section.into()),
            SectionOrTocV1::Toc => SectionOrTocV2::Toc,
        }
    }
}

/// Enum to differentiate between real sections and the position of the table of contents
//TODO: remove
#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq)]
pub enum SectionOrTocV2 {
    Section(SectionV2),
    Toc,
}

impl SectionOrTocV2 {
    pub fn into_section(self) -> Option<SectionV2> {
        match self {
            SectionOrTocV2::Section(section) => Some(section),
            SectionOrTocV2::Toc => None,
        }
    }
}

impl From<SectionOrTocV2> for SectionOrTocV3 {
    fn from(value: SectionOrTocV2) -> Self {
        match value {
            SectionOrTocV2::Section(section) => SectionOrTocV3::Section(section.into()),
            SectionOrTocV2::Toc => SectionOrTocV3::Toc,
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq)]
pub enum SectionOrTocV3 {
    Section(SectionV3),
    Toc,
}

impl From<SectionOrTocV3> for SectionOrTocV4 {
    fn from(value: SectionOrTocV3) -> Self {
        match value {
            SectionOrTocV3::Section(section) => SectionOrTocV4::Section(section.into()),
            SectionOrTocV3::Toc => SectionOrTocV4::Toc,
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq)]
pub enum SectionOrTocV4 {
    Section(SectionV4),
    Toc,
}

impl From<SectionOrTocV4> for SectionOrTocV5 {
    fn from(value: SectionOrTocV4) -> Self {
        match value {
            SectionOrTocV4::Section(section) => SectionOrTocV5::Section(section.into()),
            SectionOrTocV4::Toc => SectionOrTocV5::Toc,
        }
    }
}

/// Struct holds all data for a section (e.g. chapter, part, ...)
#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq)]
pub struct SectionV1 {
    /// Unique id of the section
    /// Only None if the section is not yet saved in the database
    #[bincode(with_serde)]
    pub id: Option<uuid::Uuid>,
    /// Additional classes to style the Section
    pub css_classes: Vec<String>,
    /// Holds all subsections
    pub sub_sections: Vec<SectionV1>,
    // Holds all content blocks
    pub children: Vec<NewContentBlock>,
    /// If true, the section is visible in the table of contents
    pub visible_in_toc: bool,
    /// Metadata of the section
    pub metadata: SectionMetadataV1,
}

impl From<SectionV1> for SectionV2 {
    fn from(value: SectionV1) -> Self {
        SectionV2 {
            id: value.id,
            css_classes: value.css_classes,
            sub_sections: value.sub_sections.into_iter().map(|s| s.into()).collect(),
            children: value.children,
            visible_in_toc: value.visible_in_toc,
            metadata: value.metadata.into(),
        }
    }
}

/// Struct holds all data for a section (e.g. chapter, part, ...)
#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq)]
pub struct SectionV2 {
    /// Unique id of the section
    /// Only None if the section is not yet saved in the database
    #[bincode(with_serde)]
    pub id: Option<uuid::Uuid>,
    /// Additional classes to style the Section
    pub css_classes: Vec<String>,
    /// Holds all subsections
    pub sub_sections: Vec<SectionV2>,
    // Holds all content blocks
    pub children: Vec<NewContentBlock>,
    /// If true, the section is visible in the table of contents
    pub visible_in_toc: bool,
    /// Metadata of the section
    pub metadata: SectionMetadataV2,
}

impl From<SectionV2> for SectionV3 {
    fn from(value: SectionV2) -> Self {
        SectionV3 {
            id: value.id,
            css_classes: value.css_classes,
            sub_sections: value.sub_sections.into_iter().map(|s| s.into()).collect(),
            children: value.children,
            visible_in_toc: value.visible_in_toc,
            metadata: value.metadata.into(),
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq)]
pub struct SectionV4 {
    /// Unique id of the section
    /// Only None if the section is not yet saved in the database
    #[bincode(with_serde)]
    pub id: Option<uuid::Uuid>,
    /// Additional classes to style the Section
    pub css_classes: Vec<String>,
    /// Holds all subsections
    pub sub_sections: Vec<SectionV4>,
    // Holds all content blocks
    pub children: Vec<NewContentBlock>,
    /// If true, the section is visible in the table of contents
    pub visible_in_toc: bool,
    /// Metadata of the section
    pub metadata: SectionMetadataV4,
}
impl From<SectionV3> for SectionV4 {
    fn from(value: SectionV3) -> Self {
        SectionV4 {
            id: value.id,
            css_classes: value.css_classes,
            sub_sections: value
                .sub_sections
                .into_iter()
                .map(SectionV4::from)
                .collect(),
            children: value.children,
            visible_in_toc: value.visible_in_toc,
            metadata: value.metadata.into(),
        }
    }
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

impl From<SectionV4> for SectionV5 {
    fn from(value: SectionV4) -> Self {
        SectionV5 {
            id: value.id,
            css_classes: value.css_classes,
            sub_sections: value
                .sub_sections
                .into_iter()
                .map(SectionV5::from)
                .collect(),
            children: value.children,
            visible_in_toc: value.visible_in_toc,
            metadata: value.metadata.into(),
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq)]
pub enum SectionOrTocV5 {
    Section(SectionV5),
    Toc,
}

impl SectionOrTocV5 {
    pub fn into_section(self) -> Option<SectionV5> {
        match self {
            SectionOrTocV5::Section(section) => Some(section),
            SectionOrTocV5::Toc => None,
        }
    }
}

impl From<SectionMetadataV5> for SectionMetadataV6 {
    fn from(value: SectionMetadataV5) -> Self {
        SectionMetadataV6 {
            title: value.title,
            toc_title_subtitle_override: value.toc_title_subtitle_override,
            subtitle: value.subtitle,
            authors: value.authors,
            editors: value.editors,
            web_url: value.web_url,
            identifiers: value.identifiers,
            published: value.published,
            last_changed: value.last_changed,
            lang: value.lang,
            custom_fields: HashMap::new(),
        }
    }
}

pub fn convert_contentblocks_to_yrs(blocks: Vec<NewContentBlock>) -> Doc {
    let doc = Doc::new();
    {
        let blocks_array = doc.get_or_insert_array("blocks");
        let mut txn = doc.transact_mut();

        for block in blocks {
            let prelim: MapPrelim = block.into();
            blocks_array.push_back(&mut txn, prelim);
        }
    }
    doc
}

impl From<SectionV5> for SectionV6 {
    fn from(value: SectionV5) -> Self {
        let doc = convert_contentblocks_to_yrs(value.children);
        let content = doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default());

        SectionV6 {
            id: value.id,
            css_classes: value.css_classes,
            sub_sections: value
                .sub_sections
                .into_iter()
                .map(SectionV6::from)
                .collect(),
            content,
            visible_in_toc: value.visible_in_toc,
            metadata: value.metadata.into(),
        }
    }
}

impl From<SectionOrTocV5> for SectionV6 {
    fn from(value: SectionOrTocV5) -> Self {
        match value {
            SectionOrTocV5::Section(section) => section.into(),
            SectionOrTocV5::Toc => SectionV6 {
                id: Some(uuid::Uuid::new_v4()),
                css_classes: Vec::new(),
                sub_sections: Vec::new(),
                content: Vec::new(),
                visible_in_toc: true,
                metadata: SectionMetadataV6 {
                    title: "Table of Contents".to_string(),
                    toc_title_subtitle_override: None,
                    subtitle: None,
                    authors: Vec::new(),
                    editors: Vec::new(),
                    web_url: None,
                    identifiers: Vec::new(),
                    published: None,
                    last_changed: None,
                    lang: None,
                    custom_fields: HashMap::new(),
                },
            },
        }
    }
}

/// Struct holds all data for a section (e.g. chapter, part, ...)
#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq)]
pub struct SectionV3 {
    /// Unique id of the section
    /// Only None if the section is not yet saved in the database
    #[bincode(with_serde)]
    pub id: Option<uuid::Uuid>,
    /// Additional classes to style the Section
    pub css_classes: Vec<String>,
    /// Holds all subsections
    pub sub_sections: Vec<SectionV3>,
    // Holds all content blocks
    pub children: Vec<NewContentBlock>,
    /// If true, the section is visible in the table of contents
    pub visible_in_toc: bool,
    /// Metadata of the section
    pub metadata: SectionMetadataV3,
}

/// Struct holds all metadata of a section
#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq)]
pub struct SectionMetadataV1 {
    pub title: String,
    pub subtitle: Option<String>,
    #[bincode(with_serde)]
    pub authors: Vec<uuid::Uuid>,
    #[bincode(with_serde)]
    pub editors: Vec<uuid::Uuid>,
    pub web_url: Option<String>,
    pub identifiers: Vec<Identifier>,
    #[bincode(with_serde)]
    pub published: Option<NaiveDateTime>,
    #[bincode(with_serde)]
    pub last_changed: Option<NaiveDateTime>,
    pub lang: Option<OldLanguage>,
}

impl From<SectionMetadataV1> for SectionMetadataV2 {
    fn from(value: SectionMetadataV1) -> Self {
        SectionMetadataV2 {
            title: value.title,
            toc_title_override: None,
            subtitle: value.subtitle,
            toc_subtitle_override: None,
            authors: value.authors,
            editors: value.editors,
            web_url: value.web_url,
            identifiers: value.identifiers,
            published: value.published,
            last_changed: value.last_changed,
            lang: value.lang,
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq)]
pub struct SectionMetadataV2 {
    pub title: String,
    pub toc_title_override: Option<String>,
    pub subtitle: Option<String>,
    pub toc_subtitle_override: Option<String>,
    #[bincode(with_serde)]
    pub authors: Vec<uuid::Uuid>,
    #[bincode(with_serde)]
    pub editors: Vec<uuid::Uuid>,
    pub web_url: Option<String>,
    pub identifiers: Vec<Identifier>,
    #[bincode(with_serde)]
    pub published: Option<NaiveDateTime>,
    #[bincode(with_serde)]
    pub last_changed: Option<NaiveDateTime>,
    pub lang: Option<OldLanguage>,
}

impl From<SectionMetadataV2> for SectionMetadataV3 {
    fn from(value: SectionMetadataV2) -> Self {
        SectionMetadataV3 {
            title: value.title,
            toc_title_override: value.toc_title_override,
            subtitle: value.subtitle,
            toc_subtitle_override: value.toc_subtitle_override,
            authors: value.authors,
            editors: value.editors,
            web_url: value.web_url,
            identifiers: value.identifiers,
            published: value.published.map(|d| d.date()),
            last_changed: value.last_changed,
            lang: value.lang,
        }
    }
}

/// Struct holds all metadata of a section
#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq)]
pub struct SectionMetadataV4 {
    pub title: String,
    pub toc_title_override: Option<String>,
    pub subtitle: Option<String>,
    pub toc_subtitle_override: Option<String>,
    #[bincode(with_serde)]
    pub authors: Vec<uuid::Uuid>,
    #[bincode(with_serde)]
    pub editors: Vec<uuid::Uuid>,
    pub web_url: Option<String>,
    pub identifiers: Vec<Identifier>,
    #[bincode(with_serde)]
    pub published: Option<NaiveDate>,
    #[bincode(with_serde)]
    pub last_changed: Option<NaiveDateTime>,
    #[bincode(with_serde)]
    pub lang: Option<Language>,
}

impl From<SectionMetadataV4> for SectionMetadataV5 {
    fn from(value: SectionMetadataV4) -> Self {
        let toc_title_subtitle_override =
            if value.toc_title_override.is_none() && value.toc_subtitle_override.is_none() {
                None
            } else {
                let mut combined_override = String::new();

                // Add title or toc_title_override as first part
                if let Some(title_override) = value.toc_title_override {
                    combined_override.push_str(&title_override);
                } else {
                    combined_override.push_str(&value.title)
                }

                if let Some(subtitle_override) = value.toc_subtitle_override {
                    combined_override.push_str(": ");
                    combined_override.push_str(&subtitle_override);
                } else if let Some(subtitle) = &value.subtitle {
                    combined_override.push_str(": ");
                    combined_override.push_str(subtitle);
                }
                Some(combined_override)
            };

        let authors = value
            .authors
            .into_iter()
            .map(PersonUuidOrString::PersonUuid)
            .collect();
        let editors = value
            .editors
            .into_iter()
            .map(PersonUuidOrString::PersonUuid)
            .collect();

        SectionMetadataV5 {
            title: value.title,
            toc_title_subtitle_override,
            subtitle: value.subtitle,
            authors,
            editors,
            web_url: value.web_url,
            identifiers: value.identifiers,
            published: value.published,
            last_changed: value.last_changed,
            lang: value.lang,
        }
    }
}

/// Struct holds all metadata of a section
#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq)]
pub struct SectionMetadataV3 {
    pub title: String,
    pub toc_title_override: Option<String>,
    pub subtitle: Option<String>,
    pub toc_subtitle_override: Option<String>,
    #[bincode(with_serde)]
    pub authors: Vec<uuid::Uuid>,
    #[bincode(with_serde)]
    pub editors: Vec<uuid::Uuid>,
    pub web_url: Option<String>,
    pub identifiers: Vec<Identifier>,
    #[bincode(with_serde)]
    pub published: Option<NaiveDate>,
    #[bincode(with_serde)]
    pub last_changed: Option<NaiveDateTime>,
    pub lang: Option<OldLanguage>,
}

impl From<SectionMetadataV3> for SectionMetadataV4 {
    fn from(val: SectionMetadataV3) -> Self {
        SectionMetadataV4 {
            title: val.title,
            toc_title_override: val.toc_title_override,
            subtitle: val.subtitle,
            toc_subtitle_override: val.toc_subtitle_override,
            authors: val.authors,
            editors: val.editors,
            web_url: val.web_url,
            identifiers: val.identifiers,
            published: val.published,
            last_changed: val.last_changed,
            lang: val.lang.map(|l| l.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::project_storage::sections::content::current::BlockData;
    use vb_exchange::projects::BlockType;
    use yrs::Map;
    use yrs::updates::decoder::Decode;

    #[test]
    fn test_section_v5_to_v6_migration() {
        let block = NewContentBlock {
            id: "1".to_string(),
            block_type: BlockType::Paragraph,
            data: BlockData::Paragraph {
                text: "Hello migration".to_string(),
            },
            css_classes: vec![],
            revision_id: None,
        };

        let section_v5 = SectionV5 {
            id: Some(uuid::Uuid::new_v4()),
            css_classes: vec!["class1".to_string()],
            sub_sections: vec![SectionV5 {
                id: Some(uuid::Uuid::new_v4()),
                css_classes: vec![],
                sub_sections: vec![],
                children: vec![],
                visible_in_toc: true,
                metadata: SectionMetadataV5 {
                    title: "Sub Section".to_string(),
                    toc_title_subtitle_override: None,
                    subtitle: None,
                    authors: vec![],
                    editors: vec![],
                    web_url: None,
                    identifiers: vec![],
                    published: None,
                    last_changed: None,
                    lang: None,
                },
            }],
            children: vec![block],
            visible_in_toc: true,
            metadata: SectionMetadataV5 {
                title: "Main Section".to_string(),
                toc_title_subtitle_override: Some("Override".to_string()),
                subtitle: Some("Subtitle".to_string()),
                authors: vec![],
                editors: vec![],
                web_url: None,
                identifiers: vec![],
                published: None,
                last_changed: None,
                lang: None,
            },
        };

        let section_v6: SectionV6 = section_v5.clone().into();

        assert_eq!(section_v6.id, section_v5.id);
        assert_eq!(section_v6.css_classes, section_v5.css_classes);
        assert_eq!(section_v6.visible_in_toc, section_v5.visible_in_toc);
        assert_eq!(section_v6.metadata.title, section_v5.metadata.title);
        assert_eq!(section_v6.metadata.subtitle, section_v5.metadata.subtitle);
        assert_eq!(section_v6.sub_sections.len(), 1);
        assert_eq!(section_v6.sub_sections[0].metadata.title, "Sub Section");

        // Verify content document
        let doc = Doc::new();
        {
            let mut txn = doc.transact_mut();
            txn.apply_update(yrs::Update::decode_v1(&section_v6.content).unwrap())
                .unwrap();
        }

        let blocks_array = doc.get_or_insert_array("blocks");
        let txn = doc.transact();
        assert_eq!(blocks_array.len(&txn), 1);

        let block_map = blocks_array
            .get(&txn, 0)
            .unwrap()
            .cast::<yrs::types::map::MapRef>()
            .unwrap();
        assert_eq!(
            block_map.get(&txn, "type").unwrap().to_string(&txn),
            "paragraph"
        );
        let data_map = block_map
            .get(&txn, "data")
            .unwrap()
            .cast::<yrs::types::map::MapRef>()
            .unwrap();
        assert_eq!(
            data_map.get(&txn, "text").unwrap().to_string(&txn),
            "Hello migration"
        );
    }
}

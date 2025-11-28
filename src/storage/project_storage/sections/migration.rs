use crate::storage::project_storage::current::PersonUuidOrString;
use crate::storage::project_storage::sections::content::current::NewContentBlock;
use crate::storage::project_storage::sections::current::{
    SectionMetadataV5, SectionOrTocV5, SectionV5,
};
use bincode::{Decode, Encode};
use chrono::{NaiveDate, NaiveDateTime};
use language::Language;
use rocket::serde::{Deserialize, Serialize};
use vb_exchange::deprecated::projects::data_storage::OldLanguage;
use vb_exchange::projects::Identifier;

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
                .map(|s| SectionV4::from(s))
                .collect(),
            children: value.children,
            visible_in_toc: value.visible_in_toc,
            metadata: value.metadata.into(),
        }
    }
}

impl From<SectionV4> for SectionV5 {
    fn from(value: SectionV4) -> Self {
        SectionV5 {
            id: value.id,
            css_classes: value.css_classes,
            sub_sections: value
                .sub_sections
                .into_iter()
                .map(|s| SectionV5::from(s))
                .collect(),
            children: value.children,
            visible_in_toc: value.visible_in_toc,
            metadata: value.metadata.into(),
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
            lang: value.lang.map(|l| l.into()),
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
                    combined_override.push_str(&subtitle);
                }
                Some(combined_override)
            };

        let authors = value
            .authors
            .into_iter()
            .map(|x| PersonUuidOrString::PersonUuid(x))
            .collect();
        let editors = value
            .editors
            .into_iter()
            .map(|x| PersonUuidOrString::PersonUuid(x))
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

impl Into<SectionMetadataV4> for SectionMetadataV3 {
    fn into(self) -> SectionMetadataV4 {
        SectionMetadataV4 {
            title: self.title,
            toc_title_override: self.toc_title_override,
            subtitle: self.subtitle,
            toc_subtitle_override: self.toc_subtitle_override,
            authors: self.authors,
            editors: self.editors,
            web_url: self.web_url,
            identifiers: self.identifiers,
            published: self.published,
            last_changed: self.last_changed,
            lang: self.lang.map(|l| l.into()),
        }
    }
}

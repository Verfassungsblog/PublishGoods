use crate::storage::project_storage::current::{
    Bibliography, PersonUuidOrString, ProjectDataV10, ProjectMetadataV5,
};
use crate::storage::project_storage::sections::current::SectionV6;
use crate::storage::project_storage::sections::migration::{
    SectionOrTocV1, SectionOrTocV2, SectionOrTocV3, SectionOrTocV4, SectionOrTocV5, SectionV5,
};
use crate::storage::project_storage::{ProjectData, ProjectStorageError, CURRENT_VERSION};
use crate::storage::{BibEntryV2, BibEntryV3, MyPublisher, OldBibEntry};
use bincode::{Decode, Encode};
use chrono::{NaiveDate, NaiveDateTime};
use language::Language;
use rocket::serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use vb_exchange::deprecated::projects::data_storage::OldLanguage;
use vb_exchange::deprecated::projects::project_settings::{
    ProjectSettingsV2, ProjectSettingsV3, ProjectSettingsV4,
};
use vb_exchange::projects::{Identifier, Keyword, License, ProjectSettingsV5};

pub fn load_project_data(
    mut file: File,
    mut version: u64,
) -> Result<ProjectData, ProjectStorageError> {
    if version != CURRENT_VERSION {
        info!(
            "Migrating ProjectData from v{} to latest version (v{}).",
            version, CURRENT_VERSION
        );
    }

    let mut v1_data: Option<OldProjectData> = None;
    if version == 1 {
        v1_data = Some(bincode::decode_from_std_read::<OldProjectData, _, _>(
            &mut file,
            bincode::config::standard(),
        )?);
        version = 2;
    }
    let mut v2_data: Option<ProjectDataV2> = None;
    if version == 2 {
        v2_data = if let Some(v1_data) = v1_data {
            Some(v1_data.into())
        } else {
            Some(bincode::decode_from_std_read::<ProjectDataV2, _, _>(
                &mut file,
                bincode::config::standard(),
            )?)
        };
        version = 3;
    }
    let mut v3_data: Option<ProjectDataV3> = None;
    if version == 3 {
        v3_data = if let Some(v2_data) = v2_data {
            Some(v2_data.into())
        } else {
            Some(bincode::decode_from_std_read::<ProjectDataV3, _, _>(
                &mut file,
                bincode::config::standard(),
            )?)
        };
        version = 4;
    }
    let mut v4_data: Option<ProjectDataV4> = None;
    if version == 4 {
        v4_data = if let Some(v3_data) = v3_data {
            Some(v3_data.into())
        } else {
            Some(bincode::decode_from_std_read::<ProjectDataV4, _, _>(
                &mut file,
                bincode::config::standard(),
            )?)
        };
        version = 5;
    }
    let mut v5_data: Option<ProjectDataV5> = None;
    if version == 5 {
        v5_data = if let Some(v4_data) = v4_data {
            Some(v4_data.into())
        } else {
            Some(bincode::decode_from_std_read::<ProjectDataV5, _, _>(
                &mut file,
                bincode::config::standard(),
            )?)
        };
        version = 6;
    }
    let mut v6_data: Option<ProjectDataV6> = None;
    if version == 6 {
        v6_data = if let Some(v5_data) = v5_data {
            Some(v5_data.into())
        } else {
            Some(bincode::decode_from_std_read::<ProjectDataV6, _, _>(
                &mut file,
                bincode::config::standard(),
            )?)
        };
        version = 7;
    }
    let mut v7_data: Option<ProjectDataV7> = None;
    if version == 7 {
        v7_data = if let Some(v6_data) = v6_data {
            Some(v6_data.into())
        } else {
            Some(bincode::decode_from_std_read::<ProjectDataV7, _, _>(
                &mut file,
                bincode::config::standard(),
            )?)
        };
        version = 8;
    }
    let mut v8_data: Option<ProjectDataV8> = None;
    if version == 8 {
        v8_data = if let Some(v7_data) = v7_data {
            Some(v7_data.into())
        } else {
            Some(bincode::decode_from_std_read::<ProjectDataV8, _, _>(
                &mut file,
                bincode::config::standard(),
            )?)
        };
        version = 9;
    }

    let mut v9_data: Option<ProjectDataV9> = None;
    if version == 9 {
        v9_data = if let Some(v8_data) = v8_data {
            Some(v8_data.into())
        } else {
            Some(bincode::decode_from_std_read::<ProjectDataV9, _, _>(
                &mut file,
                bincode::config::standard(),
            )?)
        };
        version = 10;
    }

    let mut v10_data: Option<ProjectDataV10> = None;
    if version == 10 {
        v10_data = if let Some(v9_data) = v9_data {
            Some(v9_data.into())
        } else {
            Some(bincode::decode_from_std_read::<ProjectDataV10, _, _>(
                &mut file,
                bincode::config::standard(),
            )?)
        };
    }

    match v10_data {
        Some(data) => Ok(data),
        None => Err(ProjectStorageError::InvalidVersionNumber),
    }
}

#[derive(Debug, Serialize, Deserialize, Encode, Decode, Clone)]
pub struct OldProjectData {
    pub name: String,
    pub description: Option<String>,
    #[bincode(with_serde)]
    pub template_id: uuid::Uuid,
    pub last_interaction: u64,
    pub metadata: Option<ProjectMetadataV1>,
    pub settings: Option<ProjectSettingsV2>,
    pub sections: Vec<SectionOrTocV1>,
    #[bincode(with_serde)]
    pub bibliography: HashMap<String, OldBibEntry>,
}

#[derive(Debug, Serialize, Deserialize, Encode, Decode, Clone)]
pub struct ProjectDataV2 {
    pub name: String,
    pub description: Option<String>,
    #[bincode(with_serde)]
    pub template_id: uuid::Uuid,
    pub last_interaction: u64,
    pub metadata: Option<ProjectMetadataV1>,
    pub settings: Option<ProjectSettingsV2>,
    pub sections: Vec<SectionOrTocV1>,
    #[bincode(with_serde)]
    pub bibliography: HashMap<String, BibEntryV2>,
}

#[derive(Debug, Serialize, Deserialize, Encode, Decode, Clone)]
pub struct ProjectDataV3 {
    pub name: String,
    pub description: Option<String>,
    #[bincode(with_serde)]
    pub template_id: uuid::Uuid,
    pub last_interaction: u64,
    pub metadata: Option<ProjectMetadataV1>,
    pub settings: Option<ProjectSettingsV3>,
    pub sections: Vec<SectionOrTocV1>,
    #[bincode(with_serde)]
    pub bibliography: HashMap<String, BibEntryV2>,
}

#[derive(Debug, Serialize, Deserialize, Encode, Decode, Clone)]
pub struct ProjectDataV4 {
    pub name: String,
    pub description: Option<String>,
    #[bincode(with_serde)]
    pub template_id: uuid::Uuid,
    pub last_interaction: u64,
    pub metadata: Option<ProjectMetadataV1>,
    pub settings: Option<ProjectSettingsV4>,
    pub sections: Vec<SectionOrTocV2>,
    #[bincode(with_serde)]
    pub bibliography: HashMap<String, BibEntryV2>,
}

#[derive(Debug, Serialize, Deserialize, Encode, Decode, Clone)]
pub struct ProjectDataV5 {
    pub name: String,
    pub description: Option<String>,
    #[bincode(with_serde)]
    pub template_id: uuid::Uuid,
    pub last_interaction: u64,
    pub metadata: Option<ProjectMetadataV2>,
    pub settings: Option<ProjectSettingsV4>,
    pub sections: Vec<SectionOrTocV3>,
    #[bincode(with_serde)]
    pub bibliography: HashMap<String, BibEntryV2>,
}

#[derive(Debug, Serialize, Deserialize, Encode, Decode, Clone)]
pub struct ProjectDataV6 {
    pub name: String,
    pub description: Option<String>,
    #[bincode(with_serde)]
    pub template_id: uuid::Uuid,
    pub last_interaction: u64,
    pub metadata: Option<ProjectMetadataV2>,
    pub settings: Option<ProjectSettingsV5>,
    pub sections: Vec<SectionOrTocV3>,
    #[bincode(with_serde)]
    pub bibliography: HashMap<String, BibEntryV2>,
}

impl From<ProjectDataV6> for ProjectDataV7 {
    fn from(value: ProjectDataV6) -> Self {
        ProjectDataV7 {
            name: value.name,
            description: value.description,
            template_id: value.template_id,
            last_interaction: value.last_interaction,
            metadata: value.metadata.map(|v| v.into()),
            settings: value.settings,
            sections: value
                .sections
                .iter()
                .map(|section| section.clone().into())
                .collect(),
            bibliography: value.bibliography,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Encode, Decode, Clone)]
pub struct ProjectDataV7 {
    pub name: String,
    pub description: Option<String>,
    #[bincode(with_serde)]
    pub template_id: uuid::Uuid,
    pub last_interaction: u64,
    pub metadata: Option<ProjectMetadataV3>,
    pub settings: Option<ProjectSettingsV5>,
    pub sections: Vec<SectionOrTocV4>,
    #[bincode(with_serde)]
    pub bibliography: HashMap<String, BibEntryV2>, //TODO: add prefix & suffix support
}

impl From<ProjectDataV7> for ProjectDataV8 {
    fn from(value: ProjectDataV7) -> Self {
        ProjectDataV8 {
            name: value.name,
            description: value.description,
            template_id: value.template_id,
            last_interaction: value.last_interaction,
            metadata: value.metadata.map(|v| v.into()),
            settings: value.settings,
            sections: value
                .sections
                .into_iter()
                .map(|section| section.into())
                .collect(),
            bibliography: value.bibliography,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Encode, Decode, Clone)]
pub struct ProjectDataV9 {
    pub name: String,
    pub description: Option<String>,
    #[bincode(with_serde)]
    pub template_id: uuid::Uuid,
    pub last_interaction: u64,
    pub metadata: Option<ProjectMetadataV5>,
    pub settings: Option<ProjectSettingsV5>,
    pub sections: Vec<SectionOrTocV5>,
    #[bincode(with_serde)]
    pub bibliography: HashMap<String, BibEntryV2>, //TODO: add prefix & suffix support
}

impl From<ProjectDataV8> for ProjectDataV9 {
    fn from(value: ProjectDataV8) -> Self {
        ProjectDataV9 {
            name: value.name,
            description: value.description,
            template_id: value.template_id,
            last_interaction: value.last_interaction,
            metadata: value.metadata.map(|m| m.into()),
            settings: value.settings,
            sections: value.sections.into_iter().collect(),
            bibliography: value.bibliography,
        }
    }
}

impl From<OldProjectData> for ProjectDataV2 {
    fn from(value: OldProjectData) -> Self {
        ProjectDataV2 {
            name: value.name,
            description: value.description,
            template_id: value.template_id,
            last_interaction: value.last_interaction,
            metadata: value.metadata,
            settings: value.settings,
            sections: value.sections,
            bibliography: value
                .bibliography
                .iter()
                .map(|(k, v)| (k.clone(), v.clone().into()))
                .collect(),
        }
    }
}

impl From<ProjectDataV2> for ProjectDataV3 {
    fn from(value: ProjectDataV2) -> Self {
        let settings: Option<ProjectSettingsV3> = match value.settings {
            Some(set) => Some(ProjectSettingsV3 {
                toc_enabled: set.toc_enabled,
                csl_style: set.csl_style,
                csl_language_code: None,
            }),
            None => None,
        };
        ProjectDataV3 {
            name: value.name,
            description: value.description,
            template_id: value.template_id,
            last_interaction: value.last_interaction,
            metadata: value.metadata,
            settings,
            sections: value.sections,
            bibliography: value
                .bibliography
                .iter()
                .map(|(k, v)| (k.clone(), v.clone().into()))
                .collect(),
        }
    }
}

impl From<ProjectDataV3> for ProjectDataV4 {
    fn from(value: ProjectDataV3) -> Self {
        let settings: Option<ProjectSettingsV4> = match value.settings {
            Some(set) => Some(ProjectSettingsV4 {
                toc_enabled: set.toc_enabled,
                csl_style: set.csl_style,
                csl_language_code: set.csl_language_code,
                metadata_page_additional_html: None,
                cover_image_path: None,
                backcover_image_path: None,
            }),
            None => None,
        };
        ProjectDataV4 {
            name: value.name,
            description: value.description,
            template_id: value.template_id,
            last_interaction: value.last_interaction,
            metadata: value.metadata,
            settings,
            sections: value
                .sections
                .iter()
                .map(|section| section.clone().into())
                .collect(),
            bibliography: value.bibliography,
        }
    }
}

impl From<ProjectDataV4> for ProjectDataV5 {
    fn from(value: ProjectDataV4) -> Self {
        let metadata: Option<ProjectMetadataV2> = match value.metadata {
            Some(metadata) => Some(metadata.into()),
            None => None,
        };

        ProjectDataV5 {
            name: value.name,
            description: value.description,
            template_id: value.template_id,
            last_interaction: value.last_interaction,
            metadata,
            settings: value.settings.map(|s| s.into()),
            sections: value
                .sections
                .iter()
                .map(|section| section.clone().into())
                .collect(),
            bibliography: value.bibliography,
        }
    }
}

impl From<ProjectDataV5> for ProjectDataV6 {
    fn from(value: ProjectDataV5) -> Self {
        let settings = match value.settings {
            Some(set) => Some(set.into()),
            None => None,
        };

        ProjectDataV6 {
            name: value.name,
            description: value.description,
            template_id: value.template_id,
            last_interaction: value.last_interaction,
            metadata: value.metadata,
            settings,
            sections: value
                .sections
                .iter()
                .map(|section| section.clone().into())
                .collect(),
            bibliography: value.bibliography,
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

/// Struct holds all project-level metadata
#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq, Default)]
pub struct ProjectMetadataV1 {
    /// Book Title
    pub title: String,
    /// Subtitle of the book
    pub subtitle: Option<String>,
    /// List of ids of authors of the book
    #[bincode(with_serde)]
    pub authors: Option<Vec<uuid::Uuid>>,
    /// List of ids of editors of the book
    #[bincode(with_serde)]
    pub editors: Option<Vec<uuid::Uuid>>,
    /// URL to a web version of the book or reference
    pub web_url: Option<String>,
    /// List of identifiers of the book (e.g. ISBNs)
    // TODO: build identifier validator
    pub identifiers: Option<Vec<Identifier>>,
    /// Date of publication
    #[bincode(with_serde)]
    pub published: Option<NaiveDateTime>,
    /// Languages of the book
    pub languages: Option<Vec<OldLanguage>>,
    /// Number of pages of the book (should be automatically calculated)
    pub number_of_pages: Option<u32>,
    /// Short abstract of the book
    pub short_abstract: Option<String>,
    /// Long abstract of the book
    pub long_abstract: Option<String>,
    /// Keywords of the book
    pub keywords: Option<Vec<Keyword>>,
    /// Dewey Decimal Classification (DDC) classes (subject groups)
    pub ddc: Option<String>, //TODO: validate DDC on api set
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
}

impl From<ProjectMetadataV1> for ProjectMetadataV2 {
    fn from(value: ProjectMetadataV1) -> Self {
        ProjectMetadataV2 {
            title: value.title,
            subtitle: value.subtitle,
            authors: value.authors,
            editors: value.editors,
            web_url: value.web_url,
            identifiers: value.identifiers,
            published: value.published.map(|d| d.date()),
            languages: value.languages,
            number_of_pages: value.number_of_pages,
            short_abstract: value.short_abstract,
            long_abstract: value.long_abstract,
            keywords: value.keywords,
            ddc: value.ddc,
            license: value.license,
            series: value.series,
            volume: value.volume,
            edition: value.edition,
            publisher: value.publisher,
        }
    }
}

/// Struct holds all project-level metadata
#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq, Default)]
pub struct ProjectMetadataV2 {
    /// Book Title
    pub title: String,
    /// Subtitle of the book
    pub subtitle: Option<String>,
    /// List of ids of authors of the book
    #[bincode(with_serde)]
    pub authors: Option<Vec<uuid::Uuid>>,
    /// List of ids of editors of the book
    #[bincode(with_serde)]
    pub editors: Option<Vec<uuid::Uuid>>,
    /// URL to a web version of the book or reference
    pub web_url: Option<String>,
    /// List of identifiers of the book (e.g. ISBNs)
    // TODO: build identifier validator
    pub identifiers: Option<Vec<Identifier>>,
    /// Date of publication
    #[bincode(with_serde)]
    pub published: Option<NaiveDate>,
    /// Languages of the book
    pub languages: Option<Vec<OldLanguage>>,
    /// Number of pages of the book (should be automatically calculated)
    pub number_of_pages: Option<u32>,
    /// Short abstract of the book
    pub short_abstract: Option<String>,
    /// Long abstract of the book
    pub long_abstract: Option<String>,
    /// Keywords of the book
    pub keywords: Option<Vec<Keyword>>,
    /// Dewey Decimal Classification (DDC) classes (subject groups)
    pub ddc: Option<String>, //TODO: validate DDC on api set
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
}

/// Struct holds all project-level metadata
#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq, Default)]
pub struct ProjectMetadataV3 {
    /// Book Title
    pub title: String,
    /// Subtitle of the book
    pub subtitle: Option<String>,
    /// List of ids of authors of the book
    #[bincode(with_serde)]
    pub authors: Option<Vec<uuid::Uuid>>,
    /// List of ids of editors of the book
    #[bincode(with_serde)]
    pub editors: Option<Vec<uuid::Uuid>>,
    /// URL to a web version of the book or reference
    pub web_url: Option<String>,
    /// List of identifiers of the book (e.g. ISBNs)
    // TODO: build identifier validator
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
    pub ddc: Option<String>, //TODO: validate DDC on api set
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
}

impl From<ProjectMetadataV2> for ProjectMetadataV3 {
    fn from(value: ProjectMetadataV2) -> Self {
        ProjectMetadataV3 {
            title: value.title,
            subtitle: value.subtitle,
            authors: value.authors,
            editors: value.editors,
            web_url: value.web_url,
            identifiers: value.identifiers,
            published: value.published,
            languages: value
                .languages
                .map(|langs| langs.into_iter().map(|l| l.into()).collect()),
            number_of_pages: value.number_of_pages,
            short_abstract: value.short_abstract,
            long_abstract: value.long_abstract,
            keywords: value.keywords,
            ddc: value.ddc,
            license: value.license,
            series: value.series,
            volume: value.volume,
            edition: value.edition,
            publisher: value.publisher,
        }
    }
}

/// Struct holds all project-level metadata
#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq, Default)]
pub struct ProjectMetadataV4 {
    /// Book Title
    pub title: String,
    /// Subtitle of the book
    pub subtitle: Option<String>,
    /// List of ids of authors of the book
    #[bincode(with_serde)]
    pub authors: Option<Vec<PersonUuidOrString>>,
    /// List of ids of editors of the book
    #[bincode(with_serde)]
    pub editors: Option<Vec<PersonUuidOrString>>,
    /// URL to a web version of the book or reference
    pub web_url: Option<String>,
    /// List of identifiers of the book (e.g. ISBNs)
    // TODO: build identifier validator
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
    pub ddc: Option<String>, //TODO: validate DDC on api set
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
}

impl From<ProjectMetadataV4> for ProjectMetadataV5 {
    fn from(value: ProjectMetadataV4) -> Self {
        ProjectMetadataV5 {
            title: value.title,
            subtitle: value.subtitle,
            authors: value.authors,
            editors: value.editors,
            web_url: value.web_url,
            identifiers: value.identifiers,
            published: value.published,
            languages: value.languages,
            number_of_pages: value.number_of_pages,
            short_abstract: value.short_abstract,
            long_abstract: value.long_abstract,
            keywords: value.keywords,
            ddc: value.ddc,
            license: value.license,
            series: value.series,
            volume: value.volume,
            edition: value.edition,
            publisher: value.publisher,
            custom_fields: Default::default(),
        }
    }
}

impl From<ProjectMetadataV3> for ProjectMetadataV4 {
    fn from(value: ProjectMetadataV3) -> Self {
        let authors = value.authors.map(|authors| {
            authors
                .into_iter()
                .map(|a| PersonUuidOrString::PersonUuid(a))
                .collect()
        });
        let editors = value.editors.map(|editors| {
            editors
                .into_iter()
                .map(|e| PersonUuidOrString::PersonUuid(e))
                .collect()
        });

        ProjectMetadataV4 {
            title: value.title,
            subtitle: value.subtitle,
            authors,
            editors,
            web_url: value.web_url,
            identifiers: value.identifiers,
            published: value.published,
            languages: value.languages,
            number_of_pages: value.number_of_pages,
            short_abstract: value.short_abstract,
            long_abstract: value.long_abstract,
            keywords: value.keywords,
            ddc: value.ddc,
            license: value.license,
            series: value.series,
            volume: value.volume,
            edition: value.edition,
            publisher: value.publisher,
        }
    }
}

impl From<ProjectDataV9> for ProjectDataV10 {
    fn from(mut value: ProjectDataV9) -> Self {
        let (bibliography, mapping) =
            extract_bibliography(value.bibliography.into_iter().collect());

        for section in &mut value.sections {
            migrate_citations_in_section(section, &mapping);
        }

        ProjectDataV10 {
            name: value.name,
            description: value.description,
            template_id: value.template_id,
            last_interaction: value.last_interaction,
            metadata: value.metadata,
            settings: value.settings,
            sections: value.sections.into_iter().map(|s| s.into()).collect(),
            bibliography,
        }
    }
}

fn migrate_citations_in_section(section: &mut SectionOrTocV5, mapping: &HashMap<String, String>) {
    if let SectionOrTocV5::Section(section) = section {
        migrate_citations_in_section_v5(section, mapping);
    }
}

fn migrate_citations_in_section_v5(section: &mut SectionV5, mapping: &HashMap<String, String>) {
    for block in &mut section.children {
        migrate_citations_in_block(block, mapping);
    }
    for sub_section in &mut section.sub_sections {
        migrate_citations_in_section_v5(sub_section, mapping);
    }
}

fn migrate_citations_in_block(
    block: &mut crate::storage::project_storage::sections::content::current::NewContentBlock,
    mapping: &HashMap<String, String>,
) {
    use crate::storage::project_storage::sections::content::current::BlockData;
    match &mut block.data {
        BlockData::Paragraph { text } => {
            *text = replace_citations(text, mapping);
        }
        BlockData::Heading { text, .. } => {
            *text = replace_citations(text, mapping);
        }
        BlockData::Raw { html } => {
            *html = replace_citations(html, mapping);
        }
        BlockData::List { items, .. } => {
            for item in items {
                *item = replace_citations(item, mapping);
            }
        }
        BlockData::Quote { text, caption, .. } => {
            *text = replace_citations(text, mapping);
            *caption = replace_citations(caption, mapping);
        }
        BlockData::Image { caption, .. } => {
            if let Some(c) = caption {
                *c = replace_citations(c, mapping);
            }
        }
    }
}

fn replace_citations(text: &str, mapping: &HashMap<String, String>) -> String {
    let mut result = text.to_string();
    for (old_key, new_uuid) in mapping {
        // We look for <citation data-key="old_key">
        let from = format!(r#"data-key="{}""#, old_key);
        let to = format!(r#"data-key="{}""#, new_uuid);
        result = result.replace(&from, &to);
    }
    result
}

fn extract_bibliography(
    old_entries: Vec<(String, BibEntryV2)>,
) -> (Bibliography, HashMap<String, String>) {
    let mut res = Bibliography::new();
    let mut mapping = HashMap::new();
    for (old_key, entry) in old_entries {
        let (parents, parent_mapping) = extract_bibliography(
            entry
                .parents
                .into_iter()
                .map(|e| (String::new(), e))
                .collect(),
        );
        mapping.extend(parent_mapping);

        let parent_ids: Vec<uuid::Uuid> = parents.entries.keys().cloned().collect();
        for parent in parents.entries {
            res.entries.insert(parent.0, parent.1);
        }

        let publisher: Option<MyPublisher> = entry.publisher.map(|publ| MyPublisher {
            name: publ,
            location: None,
        });

        let new_uuid = uuid::Uuid::new_v4();
        mapping.insert(old_key, new_uuid.to_string());

        let entry_v3 = BibEntryV3 {
            key: new_uuid,
            entry_type: entry.entry_type,
            title: entry.title,
            authors: entry.authors,
            date: entry.date,
            editors: entry.editors,
            affiliated: entry.affiliated,
            publisher,
            location: entry.location,
            organization: entry.organization,
            issue: entry.issue,
            volume: entry.volume,
            volume_total: entry.volume_total,
            edition: entry.edition,
            page_range: entry.page_range.map(|pr| pr.into()),
            page_total: entry.page_total,
            time_range: entry.time_range,
            runtime: entry.runtime,
            url: entry.url,
            serial_numbers: entry.serial_numbers,
            language: entry.language,
            archive: entry.archive,
            archive_location: entry.archive_location,
            call_number: entry.call_number,
            note: entry.note,
            abstractt: entry.abstractt,
            genre: entry.genre,
            parents: parent_ids,
        };
        res.entries.insert(entry_v3.key, entry_v3);
    }
    (res, mapping)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::project_storage::sections::content::current::{BlockData, NewContentBlock};
    use crate::storage::project_storage::sections::migration::{
        SectionMetadataV5, SectionOrTocV5, SectionV5,
    };
    use std::collections::HashMap;
    use vb_exchange::projects::BlockType;

    #[test]
    fn test_v9_to_v10_migration() {
        let old_key = "citation_key_1".to_string();
        let mut bibliography = HashMap::new();
        let entry = BibEntryV2::new(old_key.clone(), hayagriva::types::EntryType::Article);
        bibliography.insert(old_key.clone(), entry);

        let section_v5 = SectionV5 {
            id: Some(uuid::Uuid::new_v4()),
            css_classes: vec![],
            sub_sections: vec![],
            children: vec![
                NewContentBlock {
                    id: "block1".to_string(),
                    block_type: BlockType::Paragraph,
                    data: BlockData::Paragraph {
                        text: format!(
                            r#"Some text with <citation data-key="{}">C</citation>."#,
                            old_key
                        ),
                    },
                    css_classes: vec![],
                    revision_id: None,
                },
                NewContentBlock {
                    id: "block2".to_string(),
                    block_type: BlockType::Heading,
                    data: BlockData::Heading {
                        text: format!(
                            r#"Heading with <citation data-key="{}">C</citation>"#,
                            old_key
                        ),
                        level: 1,
                    },
                    css_classes: vec![],
                    revision_id: None,
                },
            ],
            visible_in_toc: true,
            metadata: SectionMetadataV5 {
                title: "Test Section".to_string(),
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
        };

        let project_v9 = ProjectDataV9 {
            name: "Test Project".to_string(),
            description: None,
            template_id: uuid::Uuid::new_v4(),
            last_interaction: 0,
            metadata: None,
            settings: None,
            sections: vec![SectionOrTocV5::Section(section_v5)],
            bibliography,
        };

        let project_v10 = ProjectDataV10::from(project_v9);

        // Check bibliography
        assert_eq!(project_v10.bibliography.entries.len(), 1);
        let (new_uuid, entry_v3) = project_v10.bibliography.entries.iter().next().unwrap();
        let new_uuid_str = new_uuid.to_string();

        // Check citations in blocks
        // Note: ProjectDataV10 contains SectionV6 which encodes content as Yrs updates.
        // We test replace_citations directly below to verify the logic.

        // Test replace_citations directly to be sure
        let mut mapping = HashMap::new();
        mapping.insert(old_key.clone(), new_uuid_str.clone());
        let input = format!(r#"<citation data-key="{}">C</citation>"#, old_key);
        let expected = format!(r#"<citation data-key="{}">C</citation>"#, new_uuid_str);
        assert_eq!(replace_citations(&input, &mapping), expected);

        // Test with multiple citations and surrounding text
        let input2 = format!(
            r#"Text <citation data-key="{}">C</citation> and more <citation data-key="{}">C</citation>."#,
            old_key, old_key
        );
        let expected2 = format!(
            r#"Text <citation data-key="{}">C</citation> and more <citation data-key="{}">C</citation>."#,
            new_uuid_str, new_uuid_str
        );
        assert_eq!(replace_citations(&input2, &mapping), expected2);
    }
}

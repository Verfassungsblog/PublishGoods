use crate::projects::api::Patch;
use crate::session::session_guard::Session;
use crate::settings::Settings;
use crate::storage::data_storage::DataStorage;
use crate::storage::project_storage::current::{Bibliography, PersonUuidOrString};
use crate::storage::project_storage::sections::Section;
use crate::storage::project_storage::{ProjectData, ProjectMetadata, ProjectStorage};
use crate::utils::api_helpers::APIResult;
use bincode::{Decode, Encode};
use chrono::NaiveDate;
use language::Language;
use rocket::State;
use rocket::serde::json::Json;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use vb_exchange::projects::{Identifier, Keyword, License, ProjectSettingsV5};

#[derive(Debug, Serialize, Deserialize)]
pub struct PatchProjectData {
    /// Optionally patched Project Title
    pub name: Option<String>,
    /// Optionally patched Project Description
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "::serde_with::rust::double_option"
    )]
    pub description: Option<Option<String>>,
    /// Optionally patched template_id
    pub template_id: Option<uuid::Uuid>,
    /// Optionally patched metadata
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "::serde_with::rust::double_option"
    )]
    pub metadata: Option<Option<PatchProjectMetadata>>,
    /// Optionally patched settings
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "::serde_with::rust::double_option"
    )]
    pub settings: Option<Option<PatchProjectSettings>>,
    /// Optionally patched sections
    pub sections: Option<Vec<Section>>,
    /// Optionally patched bibliography
    pub bibliography: Option<Bibliography>,
}

impl Patch<PatchProjectData, ProjectData> for ProjectData {
    fn patch(&mut self, patch: PatchProjectData) -> ProjectData {
        let mut new = self.clone();

        if let Some(name) = patch.name {
            new.name = name;
        } else if let Some(metadata) = &patch.metadata
            && let Some(metadata) = &metadata
            && let Some(title) = &metadata.title
        {
            new.name = title.clone();
        }

        if let Some(description) = patch.description {
            new.description = description;
        }

        if let Some(template_id) = patch.template_id {
            new.template_id = template_id;
        }

        if let Some(patch_metadata) = patch.metadata {
            new.metadata = new.metadata.patch(patch_metadata);
        }

        if let Some(patch_settings) = patch.settings {
            new.settings = new.settings.patch(patch_settings);
        }

        if let Some(sections) = patch.sections {
            new.sections = sections;
        }

        if let Some(bibliography) = patch.bibliography {
            new.bibliography = bibliography;
        }

        new
    }
}

impl Patch<PatchProjectMetadata, ProjectMetadata> for ProjectMetadata {
    fn patch(&mut self, patch: PatchProjectMetadata) -> ProjectMetadata {
        let mut new_metadata = self.clone();

        if let Some(title) = patch.title {
            new_metadata.title = title;
        }

        if let Some(subtitle) = patch.subtitle {
            new_metadata.subtitle = subtitle;
        }

        if let Some(authors) = patch.authors {
            new_metadata.authors = authors;
        }

        if let Some(editors) = patch.editors {
            new_metadata.editors = editors;
        }

        if let Some(web_url) = patch.web_url {
            new_metadata.web_url = web_url;
        }

        if let Some(identifiers) = patch.identifiers {
            new_metadata.identifiers = identifiers;
        }

        if let Some(published) = patch.published {
            match published {
                Some(published) => {
                    match NaiveDate::parse_from_str(&published, "%Y-%m-%d") {
                        Ok(parsed_date) => new_metadata.published = Some(parsed_date),
                        Err(e) => {
                            warn!("Couldn't parse date: {}", e);
                            new_metadata.published = None;
                        }
                    };
                }
                None => {
                    new_metadata.published = None;
                }
            }
        }

        if let Some(languages) = patch.languages {
            new_metadata.languages = languages;
        }

        if let Some(number_of_pages) = patch.number_of_pages {
            new_metadata.number_of_pages = number_of_pages;
        }

        if let Some(short_abstract) = patch.short_abstract {
            new_metadata.short_abstract = short_abstract;
        }

        if let Some(long_abstract) = patch.long_abstract {
            new_metadata.long_abstract = long_abstract;
        }

        if let Some(keywords) = patch.keywords {
            new_metadata.keywords = keywords;
        }

        if let Some(ddc) = patch.ddc {
            new_metadata.ddc = ddc;
        }

        if let Some(license) = patch.license {
            new_metadata.license = license;
        }

        if let Some(series) = patch.series {
            new_metadata.series = series;
        }

        if let Some(volume) = patch.volume {
            new_metadata.volume = volume;
        }

        if let Some(edition) = patch.edition {
            new_metadata.edition = edition;
        }

        if let Some(publisher) = patch.publisher {
            new_metadata.publisher = publisher;
        }

        if let Some(custom_fields) = patch.custom_fields {
            new_metadata.custom_fields = custom_fields;
        }

        new_metadata
    }
}

/// Struct for HTTP PATCH routes to update the project metadata
#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq, Default)]
pub struct PatchProjectMetadata {
    /// Book Title
    pub title: Option<String>,
    /// Subtitle of the book
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "::serde_with::rust::double_option"
    )]
    pub subtitle: Option<Option<String>>,
    /// List of ids of authors of the book
    #[bincode(with_serde)]
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "::serde_with::rust::double_option"
    )]
    pub authors: Option<Option<Vec<PersonUuidOrString>>>,
    /// List of ids of editors of the book
    #[bincode(with_serde)]
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "::serde_with::rust::double_option"
    )]
    pub editors: Option<Option<Vec<PersonUuidOrString>>>,
    /// URL to a web version of the book or reference
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "::serde_with::rust::double_option"
    )]
    pub web_url: Option<Option<String>>,
    /// List of identifiers of the book (e.g. ISBNs)
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "::serde_with::rust::double_option"
    )]
    pub identifiers: Option<Option<Vec<Identifier>>>,
    /// Date of publication
    #[bincode(with_serde)]
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "::serde_with::rust::double_option"
    )]
    pub published: Option<Option<String>>,
    /// Languages of the book
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "::serde_with::rust::double_option"
    )]
    #[bincode(with_serde)]
    pub languages: Option<Option<Vec<Language>>>,
    /// Number of pages of the book (should be automatically calculated)
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "::serde_with::rust::double_option"
    )]
    pub number_of_pages: Option<Option<u32>>,
    /// Short abstract of the book
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "::serde_with::rust::double_option"
    )]
    pub short_abstract: Option<Option<String>>,
    /// Long abstract of the book
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "::serde_with::rust::double_option"
    )]
    pub long_abstract: Option<Option<String>>,
    /// Keywords of the book
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "::serde_with::rust::double_option"
    )]
    pub keywords: Option<Option<Vec<Keyword>>>,
    /// Dewey Decimal Classification (DDC) classes (subject groups)
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "::serde_with::rust::double_option"
    )]
    pub ddc: Option<Option<String>>,
    /// License of the book
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "::serde_with::rust::double_option"
    )]
    pub license: Option<Option<License>>,
    /// Series the book belongs to
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "::serde_with::rust::double_option"
    )]
    pub series: Option<Option<String>>,
    /// Volume of the book in the series
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "::serde_with::rust::double_option"
    )]
    pub volume: Option<Option<String>>,
    /// Edition of the book
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "::serde_with::rust::double_option"
    )]
    pub edition: Option<Option<String>>,
    /// Publisher of the book
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "::serde_with::rust::double_option"
    )]
    pub publisher: Option<Option<String>>,
    /// Custom Fields
    #[serde(default)]
    pub custom_fields: Option<HashMap<String, String>>,
}

impl Patch<PatchProjectSettings, ProjectSettingsV5> for ProjectSettingsV5 {
    fn patch(&mut self, patch: PatchProjectSettings) -> ProjectSettingsV5 {
        let mut new = self.clone();

        if let Some(toc_enabled) = patch.toc_enabled {
            new.toc_enabled = toc_enabled;
        }

        if let Some(csl_style) = patch.csl_style {
            new.csl_style = csl_style;
        }

        if let Some(csl_language_code) = patch.csl_language_code {
            new.csl_language_code = csl_language_code;
        }

        if let Some(metadata_page_additional_html) = patch.metadata_page_additional_html {
            new.metadata_page_additional_html = metadata_page_additional_html;
        }

        if let Some(cover_image_path) = patch.cover_image_path {
            new.cover_image_path = cover_image_path;
        }

        if let Some(backcover_image_path) = patch.backcover_image_path {
            new.backcover_image_path = backcover_image_path;
        }

        if let Some(add_soft_hyphens) = patch.add_soft_hyphens {
            new.add_soft_hyphens = add_soft_hyphens;
        }

        new
    }
}
#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq, Default)]
pub struct PatchProjectSettings {
    pub toc_enabled: Option<bool>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "::serde_with::rust::double_option"
    )]
    pub csl_style: Option<Option<String>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "::serde_with::rust::double_option"
    )]
    pub csl_language_code: Option<Option<String>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "::serde_with::rust::double_option"
    )]
    pub metadata_page_additional_html: Option<Option<String>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "::serde_with::rust::double_option"
    )]
    pub cover_image_path: Option<Option<String>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "::serde_with::rust::double_option"
    )]
    pub backcover_image_path: Option<Option<String>>,
    pub add_soft_hyphens: Option<bool>,
}

/// PATCH /api/projects/<project_id>
#[patch("/api/projects/<project_id>", data = "<patch>")]
pub async fn patch_project(
    project_id: &str,
    patch: Json<PatchProjectData>,
    _session: Session,
    settings: &State<Settings>,
    project_storage: &State<Arc<ProjectStorage>>,
    data_storage: &State<Arc<DataStorage>>,
) -> APIResult<()> {
    let id = uuid::Uuid::parse_str(project_id)?;
    let project = project_storage.get_project(&id, settings).await?.clone();

    let mut project_cpy = project.read().unwrap().clone();
    project_cpy = project_cpy.patch(patch.into_inner());

    // Update the project in the data storage
    let project_list = data_storage.data.projects.clone();
    let read_lock = project_list.read().unwrap();

    if let Some(project) = read_lock.get(&id)
        && project.name() != project_cpy.name
    {
        drop(read_lock);
        if let Some(project) = project_list.write().unwrap().get_mut(&id) {
            project.set_name(project_cpy.name.clone());
        }
    }

    let mut project_state = project.write().unwrap();
    *project_state = project_cpy;
    Ok(().into())
}

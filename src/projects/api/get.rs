use crate::session::session_guard::Session;
use crate::settings::Settings;
use crate::storage::data_storage::DataStorage;
use crate::storage::project_storage::current::PersonUuidOrString;
use crate::storage::project_storage::migration::load_project_data;
use crate::storage::project_storage::sections::current::SectionOrTocV5;
use crate::storage::project_storage::{ProjectMetadata, ProjectStorage};
use crate::storage::{BibEntryV2, ProjectTemplateV2};
use crate::utils::api_helpers::{APIResponse, APIResult};
use crate::utils::csl::{list_available_locales, list_available_styles};
use chrono::NaiveDate;
use language::Language;
use rocket::form::validate::Contains;
use rocket::State;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;
use vb_exchange::projects::{Identifier, Keyword, License, PersonOrString, ProjectSettingsV5};

/// Return struct for ['get_project'].
/// Similar to ['crate::storage::project_storage::ProjectData'] but some fields are only Some if specified in extend
#[derive(Debug, Serialize, Deserialize)]
pub struct APIProjectData {
    /// Project uuid
    pub project_id: uuid::Uuid,
    /// Project Title
    pub name: String,
    /// Project Description
    pub description: Option<String>,
    /// Id for the ProjectTemplate
    pub template_id: uuid::Uuid,
    /// Optionally extended ProjectTemplate
    pub template_extended: Option<ProjectTemplateV2>,
    /// Optionally extended ProjectMetadata
    pub metadata: Option<APIProjectMetadata>,
    /// Optionally extended ProjectSettings
    pub settings: Option<ProjectSettingsV5>,
    /// Optionally extended Sections
    pub sections: Option<Vec<SectionOrTocV5>>,
    /// Optionally extended Bibliography
    pub bibliography: Option<HashMap<String, BibEntryV2>>,
    /// Optionally extended available CSL styles
    pub available_csl_styles: Option<Vec<String>>,
    /// Optionally extended available CSL locales
    pub available_csl_locales: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct APIProjectMetadata {
    /// Book Title
    pub title: String,
    /// Subtitle of the book
    pub subtitle: Option<String>,
    /// List of authors (uuid reference or free-form string)
    pub authors: Option<Vec<PersonUuidOrString>>,
    /// List of authors extended
    pub authors_expanded: Option<Vec<PersonOrString>>,
    /// List of editors (uuid reference or free-form string)
    pub editors: Option<Vec<PersonUuidOrString>>,
    /// List of editors extended
    pub editors_expanded: Option<Vec<PersonOrString>>,
    /// URL to a web version of the book or reference
    pub web_url: Option<String>,
    /// List of identifiers of the book (e.g. ISBNs)
    pub identifiers: Option<Vec<Identifier>>,
    /// Date of publication
    pub published: Option<NaiveDate>,
    /// Languages of the book
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

impl From<ProjectMetadata> for APIProjectMetadata {
    fn from(value: ProjectMetadata) -> Self {
        Self {
            title: value.title,
            subtitle: value.subtitle,
            authors: value.authors,
            authors_expanded: None,
            editors: value.editors,
            editors_expanded: None,
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
            custom_fields: value.custom_fields,
        }
    }
}

/// GET /api/projects/<project_id>
#[get("/api/projects/<project_id>?<extend>")]
pub async fn get_project(
    project_id: &str,
    extend: Option<String>,
    _session: Session,
    settings: &State<Settings>,
    project_storage: &State<Arc<ProjectStorage>>,
    data_storage: &State<Arc<DataStorage>>,
) -> APIResult<APIProjectData> {
    let project_id = Uuid::parse_str(project_id)?;
    let loaded_project = project_storage
        .get_project(&project_id, settings)
        .await?
        .clone()
        .read()
        .unwrap()
        .clone();

    let mut api_response = APIProjectData {
        project_id,
        name: loaded_project.name,
        description: loaded_project.description,
        template_id: loaded_project.template_id,
        template_extended: None,
        metadata: None,
        settings: None,
        sections: None,
        bibliography: None,
        available_csl_styles: None,
        available_csl_locales: None,
    };

    if let Some(extend) = extend {
        let parts = extend.split(",").collect::<Vec<&str>>();
        if parts.contains("template") {
            api_response.template_extended = Some(
                data_storage
                    .get_template(&api_response.template_id)?
                    .clone()
                    .read()
                    .unwrap()
                    .clone(),
            );
        }
        if parts.contains("metadata") {
            api_response.metadata = loaded_project.metadata.map(|md| md.into());
            if let Some(metadata) = &mut api_response.metadata {
                if let Some(authors) = &metadata.authors {
                    if parts.contains("metadata.authors") {
                        let mut authors_extended: Vec<PersonOrString> = Vec::new();

                        for author in authors {
                            match author {
                                PersonUuidOrString::NameString(name) => {
                                    authors_extended.push(PersonOrString::NameString(name.clone()))
                                }
                                PersonUuidOrString::PersonUuid(uuid) => {
                                    match data_storage.get_person_cloned(&uuid) {
                                        Some(person) => {
                                            authors_extended.push(PersonOrString::Person(person))
                                        }
                                        None => {
                                            warn!("Person with uuid used in project metadata, but no longer exists. Skipping.");
                                        }
                                    }
                                }
                            }
                        }
                        metadata.authors_expanded = Some(authors_extended);
                    }
                }

                if let Some(editors) = &metadata.editors {
                    if parts.contains("metadata.editors") {
                        let mut editors_expanded: Vec<PersonOrString> = Vec::new();

                        for editor in editors {
                            match editor {
                                PersonUuidOrString::NameString(name) => {
                                    editors_expanded.push(PersonOrString::NameString(name.clone()))
                                }
                                PersonUuidOrString::PersonUuid(uuid) => {
                                    match data_storage.get_person_cloned(&uuid) {
                                        Some(person) => {
                                            editors_expanded.push(PersonOrString::Person(person))
                                        }
                                        None => {
                                            warn!("Person with uuid used in project metadata, but no longer exists. Skipping.");
                                        }
                                    }
                                }
                            }
                        }
                        metadata.editors_expanded = Some(editors_expanded);
                    }
                }
            }
        }
        if parts.contains("settings") {
            api_response.settings = loaded_project.settings;
        }
        if parts.contains("sections") {
            let mut sections = loaded_project.sections;
            for section in sections.iter_mut() {
                if let SectionOrTocV5::Section(section) = section {
                    section.truncate_children_recursive();
                }
            }
            api_response.sections = Some(sections);
        }
        if parts.contains("bibliography") {
            api_response.bibliography = Some(loaded_project.bibliography);
        }
        if parts.contains("available_csl_styles") {
            api_response.available_csl_styles = Some(list_available_styles(settings).await?);
        }
        if parts.contains("available_csl_locales") {
            api_response.available_csl_locales = Some(list_available_locales(settings).await?);
        }
    }

    Ok(APIResponse::from(api_response))
}

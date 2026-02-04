use crate::projects::api::Patch;
use crate::session::session_guard::Session;
use crate::settings::Settings;
use crate::storage::data_storage::DataStorage;
use crate::storage::project_storage::current::{
    get_section_by_path, get_section_by_path_mut, PersonUuidOrString,
};
use crate::storage::project_storage::sections::content::current::NewContentBlock;
use crate::storage::project_storage::sections::{Section, SectionMetadata};
use crate::storage::project_storage::ProjectStorage;
use crate::utils::api_helpers::{APIResult, ApiErrorType};
use crate::utils::dedup::dedup_vec;
use bincode::{Decode, Encode};
use chrono::{NaiveDate, NaiveDateTime};
use language::Language;
use rocket::form::validate::Contains;
use rocket::serde::json::Json;
use rocket::serde::{Deserialize, Serialize};
use rocket::State;
use std::collections::HashMap;
/// Contains API routes to view and modify sections inside a project
use std::sync::Arc;
use vb_exchange::projects::Identifier;
use vb_exchange::projects::PersonOrString;

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
/// API struct variant for [`Section`] with optional expansion of sub_sections and some metadata fields
pub struct APISectionResult {
    pub id: uuid::Uuid,
    /// Additional classes to style the Section
    pub css_classes: Vec<String>,
    /// Holds all subsections
    pub sub_sections: Option<Vec<Section>>,
    /// If true, the section is visible in the table of contents
    pub visible_in_toc: bool,
    /// Metadata of the section
    pub metadata: APISectionMetadataResult,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
/// API version for [`SectionMetadataV6`] with optional expansion of authors and editors
pub struct APISectionMetadataResult {
    pub title: String,
    pub toc_title_subtitle_override: Option<String>,
    pub subtitle: Option<String>,
    pub authors: Vec<PersonUuidOrString>,
    pub authors_expanded: Option<Vec<PersonOrString>>,
    pub editors: Vec<PersonUuidOrString>,
    pub editors_expanded: Option<Vec<PersonOrString>>,
    pub web_url: Option<String>,
    pub identifiers: Vec<Identifier>,
    pub published: Option<NaiveDate>,
    pub last_changed: Option<NaiveDateTime>,
    pub lang: Option<Language>,
    pub custom_fields: HashMap<String, String>,
}

/// GET /api/projects/<project_id>/sections/<content_path>?<expand>
///
/// Parameters:
/// * project_id (string) - the projects uuid
/// * content_path (string) - path to a specific section, split by ':'
/// * expand (string, optional) - optionally expand one of these fields: authors, editors, subsections
///
/// By default strips out subsections & only returns id's for authors and editors.
/// Use the optional expand query parameter to expand these fields
/// E.g. ?expand=authors,editors,subsections will show the full data
///
#[get("/api/projects/<project_id>/sections/<content_path>?<expand>")]
pub async fn get_section(
    project_id: &str,
    content_path: &str,
    expand: Option<&str>,
    _session: Session,
    settings: &State<Settings>,
    project_storage: &State<Arc<ProjectStorage>>,
    data_storage: &State<Arc<DataStorage>>,
) -> APIResult<APISectionResult> {
    debug!(
        "get_section API request: project_id={:?}, content_path={:?}, expand={:?}",
        project_id, content_path, expand
    );
    let project_id = uuid::Uuid::parse_str(project_id)?;

    // Parse expand:
    let expand_parts = if let Some(expand) = expand {
        expand
            .split(",")
            .map(|part| part.to_string())
            .collect::<Vec<String>>()
    } else {
        Vec::new()
    };

    let expand_authors = expand_parts.contains(&String::from("authors"));
    let expand_editors = expand_parts.contains(&String::from("editors"));
    let expand_subsections = expand_parts.contains(&String::from("subsections"));

    debug!(
        "Parsed expand options: expand_authors={:?}, expand_editors={:?}, expand_subsections={:?}",
        expand_authors, expand_editors, expand_subsections
    );

    let mut path = vec![];

    for part in content_path.split(":") {
        let part = uuid::Uuid::parse_str(part)?;
        path.push(part)
    }

    debug!("Parsed content_path: {:?}", path);

    if path.is_empty() {
        warn!("Couldn't parse content path: path is empty");
        return Err(ApiErrorType::UnparsableParameter(String::from("content_path")).into());
    }

    let project_storage = Arc::clone(project_storage);
    let data_storage = Arc::clone(data_storage);

    let project_entry = project_storage.get_project(&project_id, settings).await?;

    let project_read_guard = project_entry.read().unwrap();

    let section = get_section_by_path(&project_read_guard, &path)?;

    let mut section = if expand_subsections {
        section.clone()
    } else {
        section.clone_without_subsections()
    };
    drop(project_read_guard);

    // Check if all persons in section metadata are still valid
    let old_metadata = section.metadata.clone();
    let valid_persons: Vec<uuid::Uuid> = {
        let data_read_guard = data_storage.data.read().unwrap();
        data_read_guard.persons.keys().cloned().collect()
    };

    let mut metadata = section.metadata.clone();
    metadata.authors.retain_mut(|x| match x {
        PersonUuidOrString::PersonUuid(id) => valid_persons.contains(id.clone()),
        PersonUuidOrString::NameString(_) => true,
    });
    metadata.editors.retain_mut(|x| match x {
        PersonUuidOrString::PersonUuid(id) => valid_persons.contains(id.clone()),
        PersonUuidOrString::NameString(_) => true,
    });

    if metadata != old_metadata {
        // Save edited metadata
        let mut project_write_guard = project_entry.write().unwrap();
        let mut_section = get_section_by_path_mut(&mut project_write_guard, &path)?;
        mut_section.metadata = metadata.clone();
        section.metadata = metadata;
    }

    let authors_expanded = if expand_authors {
        let mut authors_detailed: Vec<PersonOrString> = Vec::new();
        for person_or_string in section.metadata.authors.iter_mut() {
            match person_or_string {
                PersonUuidOrString::PersonUuid(id) => match data_storage.get_person(&id) {
                    Some(person) => authors_detailed
                        .push(PersonOrString::Person(person.read().unwrap().clone())),
                    None => {
                        error!(
                            "Couldn't extend author details, author_id {} not found.",
                            id
                        );
                        return Err(ApiErrorType::ResourceNotFound(format!(
                            "author with id {}",
                            id
                        ))
                        .into());
                    }
                },
                PersonUuidOrString::NameString(namestr) => {
                    authors_detailed.push(PersonOrString::NameString(namestr.clone()))
                }
            }
        }

        Some(authors_detailed)
    } else {
        None
    };
    let editors_expanded = if expand_editors {
        let mut editors_detailed: Vec<PersonOrString> = Vec::new();
        for person_or_string in section.metadata.editors.iter_mut() {
            match person_or_string {
                PersonUuidOrString::PersonUuid(id) => match data_storage.get_person(&id) {
                    Some(person) => editors_detailed
                        .push(PersonOrString::Person(person.read().unwrap().clone())),
                    None => {
                        error!(
                            "Couldn't extend author details, author_id {} not found.",
                            id
                        );
                        return Err(ApiErrorType::ResourceNotFound(format!(
                            "editor with id {}",
                            id
                        ))
                        .into());
                    }
                },
                PersonUuidOrString::NameString(namestr) => {
                    editors_detailed.push(PersonOrString::NameString(namestr.clone()))
                }
            }
        }

        Some(editors_detailed)
    } else {
        None
    };

    let metadata_res = APISectionMetadataResult {
        title: section.metadata.title,
        subtitle: section.metadata.subtitle,
        authors: section.metadata.authors,
        authors_expanded,
        editors: section.metadata.editors,
        editors_expanded,
        web_url: section.metadata.web_url,
        identifiers: section.metadata.identifiers,
        published: section.metadata.published,
        last_changed: section.metadata.last_changed,
        lang: section.metadata.lang,
        toc_title_subtitle_override: section.metadata.toc_title_subtitle_override,
        custom_fields: section.metadata.custom_fields,
    };
    let section_id = match section.id {
        Some(id) => id,
        None => {
            error!("Section is missing id!");
            return Err(ApiErrorType::InternalServerError.into());
        }
    };
    let sub_sections = if section.sub_sections.is_empty() {
        None
    } else {
        Some(section.sub_sections)
    };
    let section_res = APISectionResult {
        id: section_id,
        css_classes: section.css_classes,
        sub_sections,
        visible_in_toc: section.visible_in_toc,
        metadata: metadata_res,
    };

    Ok(section_res.into())
}

/// PATCH /api/projects/<project_id>/sections/<content_path>
/// Patch a section, but without content (subsections / content blocks)
/// Check [PatchSection] for more information
#[patch(
    "/api/projects/<project_id>/sections/<content_path>",
    data = "<section_patch>"
)]
pub async fn update_section(
    project_id: String,
    content_path: String,
    section_patch: Json<PatchSection>,
    _session: Session,
    settings: &State<Settings>,
    project_storage: &State<Arc<ProjectStorage>>,
    data_storage: &State<Arc<DataStorage>>,
) -> APIResult<()> {
    let project_id = uuid::Uuid::parse_str(&project_id)?;

    let mut path = vec![];

    for part in content_path.split(":") {
        path.push(uuid::Uuid::parse_str(part)?);
    }

    if path.len() == 0 {
        println!("Couldn't parse content path: path is empty");
        return Err(ApiErrorType::UnparsableParameter("content_path".to_string()).into());
    }

    let project_storage = Arc::clone(project_storage);

    let project = project_storage.get_project(&project_id, settings).await?;

    let mut project = project.write().unwrap();

    let section = get_section_by_path_mut(&mut project, &path)?;

    let mut new_section_data = section.patch(section_patch.into_inner());
    // Check if new section data is valid
    // Check authors
    for author in new_section_data.metadata.authors.iter() {
        if let PersonUuidOrString::PersonUuid(id) = author {
            if !data_storage.person_exists(id) {
                return Err(
                    ApiErrorType::ResourceNotFound(format!("author with id {}", id)).into(),
                );
            }
        }
    }

    // Check editors
    for editor in new_section_data.metadata.editors.iter() {
        if let PersonUuidOrString::PersonUuid(id) = editor {
            if !data_storage.person_exists(id) {
                return Err(
                    ApiErrorType::ResourceNotFound(format!("editor with id {}", id)).into(),
                );
            }
        }
    }

    // Remove duplicants
    new_section_data.metadata.authors = dedup_vec(new_section_data.metadata.authors);
    new_section_data.metadata.editors = dedup_vec(new_section_data.metadata.editors);

    // Add ids for identifiers
    for identifier in new_section_data.metadata.identifiers.iter_mut() {
        if identifier.id.is_none() {
            identifier.id = Some(uuid::Uuid::new_v4());
        }
    }

    // Set last changed to now
    new_section_data.metadata.last_changed = Some(chrono::Utc::now().naive_utc());

    *section = new_section_data.clone();

    Ok(().into())
}

/// Struct for patching a section
/// Does NOT allow to patch the content of a section, use the content_block endpoints or move endpoints for that
#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq, Default)]
pub struct PatchSection {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "::serde_with::rust::double_option"
    )]
    #[bincode(with_serde)]
    pub id: Option<Option<uuid::Uuid>>,
    pub css_classes: Option<Vec<String>>,
    pub visible_in_toc: Option<bool>,
    pub metadata: Option<PatchSectionMetadata>,
}

#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq, Default)]
pub struct PatchSectionMetadata {
    pub title: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "::serde_with::rust::double_option"
    )]
    pub toc_title_subtitle_override: Option<Option<String>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "::serde_with::rust::double_option"
    )]
    pub subtitle: Option<Option<String>>,
    #[bincode(with_serde)]
    pub authors: Option<Vec<PersonUuidOrString>>,
    #[bincode(with_serde)]
    pub editors: Option<Vec<PersonUuidOrString>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "::serde_with::rust::double_option"
    )]
    pub web_url: Option<Option<String>>,
    pub identifiers: Option<Vec<Identifier>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "::serde_with::rust::double_option"
    )]
    #[bincode(with_serde)]
    pub published: Option<Option<String>>,
    #[bincode(with_serde)]
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "::serde_with::rust::double_option"
    )]
    pub last_changed: Option<Option<NaiveDateTime>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "::serde_with::rust::double_option"
    )]
    #[bincode(with_serde)]
    pub lang: Option<Option<Language>>,
}

impl Patch<PatchSectionMetadata, SectionMetadata> for SectionMetadata {
    fn patch(&mut self, patch: PatchSectionMetadata) -> SectionMetadata {
        let mut new_metadata = self.clone();

        if let Some(title) = patch.title {
            new_metadata.title = title;
        }

        if let Some(toc_title_subtitle_override) = patch.toc_title_subtitle_override {
            new_metadata.toc_title_subtitle_override = toc_title_subtitle_override;
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
                Some(published) => match NaiveDate::parse_from_str(&published, "%Y-%m-%d") {
                    Ok(published) => new_metadata.published = Some(published),
                    Err(e) => {
                        println!("Couldn't parse published date: {}", e);
                    }
                },
                None => new_metadata.published = None,
            }
        }

        if let Some(last_changed) = patch.last_changed {
            new_metadata.last_changed = last_changed;
        }

        if let Some(lang) = patch.lang {
            new_metadata.lang = lang;
        }

        new_metadata
    }
}

// Implement patch for PatchSection
impl Patch<PatchSection, Section> for Section {
    fn patch(&mut self, patch: PatchSection) -> Section {
        let mut new_section = self.clone();

        if let Some(id) = patch.id {
            new_section.id = id;
        }

        if let Some(css_classes) = patch.css_classes {
            new_section.css_classes = css_classes;
        }

        if let Some(visible_in_toc) = patch.visible_in_toc {
            new_section.visible_in_toc = visible_in_toc;
        }

        if let Some(metadata) = patch.metadata {
            new_section.metadata = self.metadata.patch(metadata);
        }

        new_section
    }
}

/// DELETE /api/projects/<project_id>/sections/<content_path>
/// Delete a section including all subsections and content blocks
#[delete("/api/projects/<project_id>/sections/<content_path>")]
pub async fn delete_section(
    project_id: String,
    content_path: String,
    _session: Session,
    settings: &State<Settings>,
    project_storage: &State<Arc<ProjectStorage>>,
) -> APIResult<()> {
    let project_id = uuid::Uuid::parse_str(&project_id)?;

    let mut path = vec![];
    for part in content_path.split(":") {
        let part = uuid::Uuid::parse_str(part)?;
        path.push(part);
    }

    if path.is_empty() {
        warn!("Couldn't parse content path: path is empty");
        return Err(ApiErrorType::UnparsableParameter(String::from("content_path")).into());
    }

    let project_entry = project_storage.get_project(&project_id, settings).await?;

    debug!("Deleting section with path {:?}", path);

    let mut project = project_entry.write().unwrap();

    match project.remove_section(path.last().unwrap()) {
        Some(_) => Ok(().into()),
        None => Err(ApiErrorType::ResourceNotFound(String::from("section")).into()),
    }
}

/// PUT /api/projects/<project_id>/sections/<section_id>/move/after/<after_id>
/// Move a section (and its subtree) to be a sibling placed right after another section
#[put("/api/projects/<project_id>/sections/<section_id>/move/after/<after_id>")]
pub async fn move_section_after(
    project_id: String,
    section_id: String,
    after_id: String,
    _session: Session,
    settings: &State<Settings>,
    project_storage: &State<Arc<ProjectStorage>>,
) -> APIResult<()> {
    let section_id = uuid::Uuid::parse_str(&section_id)?;
    let after_id = uuid::Uuid::parse_str(&after_id)?;
    let project_id = uuid::Uuid::parse_str(&project_id)?;

    let project = project_storage.get_project(&project_id, settings).await?;

    let mut project = project.write().unwrap();

    // Remove section from current location
    let section = match project.remove_section(&section_id) {
        Some(section) => section,
        None => return Err(ApiErrorType::ResourceNotFound(String::from("section")).into()),
    };

    // Insert after the target
    match project.insert_section_after(&after_id, section.clone()) {
        Ok(_) => Ok(().into()),
        Err(_) => {
            // rollback: append to root to avoid data loss
            project.sections.push(section);
            Err(ApiErrorType::ResourceNotFound(String::from("section")).into())
        }
    }
}

/// PUT /api/projects/<project_id>/sections/<section_id>/move/child_of/<parent_id>
/// Move a section to become the first child of another section
#[put("/api/projects/<project_id>/sections/<section_id>/move/child_of/<parent_id>")]
pub async fn move_section_child_of(
    project_id: String,
    section_id: String,
    parent_id: String,
    _session: Session,
    settings: &State<Settings>,
    project_storage: &State<Arc<ProjectStorage>>,
) -> APIResult<()> {
    let section_id = uuid::Uuid::parse_str(&section_id)?;
    let parent_id = uuid::Uuid::parse_str(&parent_id)?;
    let project_id = uuid::Uuid::parse_str(&project_id)?;

    let project = project_storage.get_project(&project_id, settings).await?;

    let mut project = project.write().unwrap();

    // Remove section from current location
    let section = match project.remove_section(&section_id) {
        Some(section) => section,
        None => return Err(ApiErrorType::ResourceNotFound(String::from("section")).into()),
    };

    // Insert as first child of parent
    match project.insert_section_as_first_child(&parent_id, section.clone()) {
        Ok(_) => Ok(().into()),
        Err(_) => {
            // rollback: append to root to avoid data loss
            project.sections.push(section);
            Err(ApiErrorType::ResourceNotFound(String::from("section")).into())
        }
    }
}

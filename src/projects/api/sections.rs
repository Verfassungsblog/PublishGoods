use crate::db::repositories::{persons, sections};
use crate::db::section_content;
use crate::projects::api::Patch;
use crate::session::session_guard::Session;
use crate::settings::Settings;
use crate::storage::project_storage::current::PersonUuidOrString;
use crate::storage::project_storage::sections::{Section, SectionMetadata};
use crate::utils::api_helpers::{APIResult, ApiErrorType};
use crate::utils::dedup::dedup_vec;
use bincode::{Decode, Encode};
use chrono::{NaiveDate, NaiveDateTime};
use language::Language;
use rocket::State;
use rocket::serde::json::Json;
use rocket::serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::collections::HashMap;
/// Contains API routes to view and modify sections inside a project
use vb_exchange::projects::Identifier;
use vb_exchange::projects::PersonOrString;

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
/// API struct variant for [`Section`] with optional expansion of some metadata fields. Never
/// carries subsections — use the project contents tree endpoint for navigation/display.
pub struct APISectionResult {
    pub id: uuid::Uuid,
    /// Additional classes to style the Section
    pub css_classes: Vec<String>,
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

/// GET /api/projects/<project_id>/sections/<section_id>?<expand>
///
/// Parameters:
/// * project_id (string) - the projects uuid
/// * section_id (string) - the section's uuid
/// * expand (string, optional) - optionally expand one of these fields: authors, editors
///
/// By default only returns id's for authors and editors.
/// Use the optional expand query parameter to expand these fields
/// E.g. ?expand=authors,editors will show the full data
///
#[get("/api/projects/<project_id>/sections/<section_id>?<expand>")]
pub async fn get_section(
    project_id: &str,
    section_id: &str,
    expand: Option<&str>,
    _session: Session,
    pool: &State<PgPool>,
) -> APIResult<APISectionResult> {
    debug!(
        "get_section API request: project_id={:?}, section_id={:?}, expand={:?}",
        project_id, section_id, expand
    );
    let project_id = uuid::Uuid::parse_str(project_id)?;
    let section_id = uuid::Uuid::parse_str(section_id)?;

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

    debug!(
        "Parsed expand options: expand_authors={:?}, expand_editors={:?}",
        expand_authors, expand_editors
    );

    let pool = pool.inner();

    let section = sections::get_by_id(pool, project_id, section_id).await?;

    // Check if all persons in section metadata are still valid
    let old_metadata = section.metadata.clone();
    let mut metadata = section.metadata.clone();

    let mut valid_authors = Vec::new();
    for author in section.metadata.authors.iter() {
        match author {
            PersonUuidOrString::PersonUuid(id) => {
                if persons::exists(pool, *id).await.unwrap_or(false) {
                    valid_authors.push(author.clone());
                }
            }
            PersonUuidOrString::NameString(_) => valid_authors.push(author.clone()),
        }
    }
    let mut valid_editors = Vec::new();
    for editor in section.metadata.editors.iter() {
        match editor {
            PersonUuidOrString::PersonUuid(id) => {
                if persons::exists(pool, *id).await.unwrap_or(false) {
                    valid_editors.push(editor.clone());
                }
            }
            PersonUuidOrString::NameString(_) => valid_editors.push(editor.clone()),
        }
    }
    metadata.authors = valid_authors;
    metadata.editors = valid_editors;

    if metadata != old_metadata {
        // Save edited metadata
        let mut updated_section = section.clone();
        updated_section.metadata = metadata.clone();
        if let Some(section_id) = section.id {
            sections::update_metadata(pool, section_id, &updated_section).await?;
        }
    }

    let mut authors_detailed: Vec<PersonOrString> = Vec::new();
    if expand_authors {
        for person_or_string in metadata.authors.iter() {
            match person_or_string {
                PersonUuidOrString::PersonUuid(id) => match persons::get(pool, *id).await {
                    Ok(person) => authors_detailed.push(PersonOrString::Person(person)),
                    Err(_) => {
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
    }
    let authors_expanded = if expand_authors {
        Some(authors_detailed)
    } else {
        None
    };

    let mut editors_detailed: Vec<PersonOrString> = Vec::new();
    if expand_editors {
        for person_or_string in metadata.editors.iter() {
            match person_or_string {
                PersonUuidOrString::PersonUuid(id) => match persons::get(pool, *id).await {
                    Ok(person) => editors_detailed.push(PersonOrString::Person(person)),
                    Err(_) => {
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
    }
    let editors_expanded = if expand_editors {
        Some(editors_detailed)
    } else {
        None
    };

    let metadata_res = APISectionMetadataResult {
        title: metadata.title,
        subtitle: metadata.subtitle,
        authors: metadata.authors,
        authors_expanded,
        editors: metadata.editors,
        editors_expanded,
        web_url: metadata.web_url,
        identifiers: metadata.identifiers,
        published: metadata.published,
        last_changed: metadata.last_changed,
        lang: metadata.lang,
        toc_title_subtitle_override: metadata.toc_title_subtitle_override,
        custom_fields: metadata.custom_fields,
    };
    let section_id = match section.id {
        Some(id) => id,
        None => {
            error!("Section is missing id!");
            return Err(ApiErrorType::InternalServerError.into());
        }
    };
    let section_res = APISectionResult {
        id: section_id,
        css_classes: section.css_classes,
        visible_in_toc: section.visible_in_toc,
        metadata: metadata_res,
    };

    Ok(section_res.into())
}

/// PATCH /api/projects/<project_id>/sections/<section_id>
/// Patch a section, but without content (subsections / content blocks)
/// Check [PatchSection] for more information
#[patch(
    "/api/projects/<project_id>/sections/<section_id>",
    data = "<section_patch>"
)]
pub async fn update_section(
    project_id: String,
    section_id: String,
    section_patch: Json<PatchSection>,
    _session: Session,
    pool: &State<PgPool>,
) -> APIResult<()> {
    let project_id = uuid::Uuid::parse_str(&project_id)?;
    let section_id = uuid::Uuid::parse_str(&section_id)?;
    let pool = pool.inner();

    let section = sections::get_by_id(pool, project_id, section_id).await?;

    let mut new_section_data = section.clone().patch(section_patch.into_inner());
    // Check if new section data is valid
    // Check authors
    for author in new_section_data.metadata.authors.iter() {
        if let PersonUuidOrString::PersonUuid(id) = author
            && !persons::exists(pool, *id).await.unwrap_or(false)
        {
            return Err(ApiErrorType::ResourceNotFound(format!("author with id {}", id)).into());
        }
    }

    // Check editors
    for editor in new_section_data.metadata.editors.iter() {
        if let PersonUuidOrString::PersonUuid(id) = editor
            && !persons::exists(pool, *id).await.unwrap_or(false)
        {
            return Err(ApiErrorType::ResourceNotFound(format!("editor with id {}", id)).into());
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

    sections::update_metadata(pool, section_id, &new_section_data).await?;

    Ok(().into())
}

/// Struct for patching a section
/// Does NOT allow to patch the content of a section, use websockets or move endpoints for that
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

/// DELETE /api/projects/<project_id>/sections/<section_id>
/// Delete a section including all subsections and content blocks
#[delete("/api/projects/<project_id>/sections/<section_id>")]
pub async fn delete_section(
    project_id: String,
    section_id: String,
    _session: Session,
    pool: &State<PgPool>,
    settings: &State<Settings>,
) -> APIResult<()> {
    let project_id = uuid::Uuid::parse_str(&project_id)?;
    let section_id = uuid::Uuid::parse_str(&section_id)?;

    debug!("Deleting section {:?}", section_id);

    let deleted_ids = sections::delete_subtree(pool.inner(), project_id, section_id).await?;
    for id in deleted_ids {
        if let Err(e) = section_content::delete(settings.inner(), id).await {
            warn!(
                "Couldn't delete CRDT content file for section {}: {}",
                id, e
            );
        }
    }
    Ok(().into())
}

/// PUT /api/projects/<project_id>/sections/<section_id>/move/after/<after_id>
/// Move a section (and its subtree) to be a sibling placed right after another section
#[put("/api/projects/<project_id>/sections/<section_id>/move/after/<after_id>")]
pub async fn move_section_after(
    project_id: String,
    section_id: String,
    after_id: String,
    _session: Session,
    pool: &State<PgPool>,
) -> APIResult<()> {
    let section_id = uuid::Uuid::parse_str(&section_id)?;
    let after_id = uuid::Uuid::parse_str(&after_id)?;
    let project_id = uuid::Uuid::parse_str(&project_id)?;

    sections::move_after(pool.inner(), project_id, section_id, after_id).await?;
    Ok(().into())
}

/// PUT /api/projects/<project_id>/sections/<section_id>/move/child_of/<parent_id>
/// Move a section to become the first child of another section
#[put("/api/projects/<project_id>/sections/<section_id>/move/child_of/<parent_id>")]
pub async fn move_section_child_of(
    project_id: String,
    section_id: String,
    parent_id: String,
    _session: Session,
    pool: &State<PgPool>,
) -> APIResult<()> {
    let section_id = uuid::Uuid::parse_str(&section_id)?;
    let parent_id = uuid::Uuid::parse_str(&parent_id)?;
    let project_id = uuid::Uuid::parse_str(&project_id)?;

    sections::move_child_of(pool.inner(), project_id, section_id, parent_id).await?;
    Ok(().into())
}

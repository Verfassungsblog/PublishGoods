use crate::db::repositories::DbError;
use crate::db::repositories::projects;
use crate::projects::api::Patch;
use crate::session::session_guard::Session;
use crate::storage::project_storage::current::PersonUuidOrString;
use crate::utils::api_helpers::APIResult;
use bincode::{Decode, Encode};
use chrono::NaiveDate;
use language::Language;
use rocket::State;
use rocket::serde::json::Json;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::collections::HashMap;
use vb_exchange::projects::{Identifier, Keyword, License, ProjectSettingsV5};

/// Request body for [`patch_project`]: any field left as `None` is left untouched.
/// Sections and bibliography are no longer patchable through this endpoint — they have
/// their own dedicated routes now that they live in separate DB tables.
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
}

/// Applies a [`PatchProjectMetadata`] onto a `ProjectMetadataV5`, leaving any field not
/// present in the patch at its current value.
impl Patch<PatchProjectMetadata, crate::storage::project_storage::current::ProjectMetadataV5>
    for crate::storage::project_storage::current::ProjectMetadataV5
{
    fn patch(
        &mut self,
        patch: PatchProjectMetadata,
    ) -> crate::storage::project_storage::current::ProjectMetadataV5 {
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

/// Applies a partial update to a project's title, description, template id, metadata
/// and/or settings. All writes for this request run inside a single DB transaction, so
/// if any individual update fails the whole patch is rolled back rather than leaving the
/// project partially updated. An explicit `name` takes precedence over a patched
/// `metadata.title`, but either one renames the project.
#[patch("/api/projects/<project_id>", data = "<patch>")]
pub async fn patch_project(
    project_id: &str,
    patch: Json<PatchProjectData>,
    _session: Session,
    pool: &State<PgPool>,
) -> APIResult<()> {
    let id = uuid::Uuid::parse_str(project_id)?;
    let pool = pool.inner();
    let patch = patch.into_inner();

    // Applied as one transaction so a failure partway through (e.g. an invalid template_id)
    // rolls back every field already written in this request instead of leaving the project
    // in a partially-patched state.
    let mut tx = pool.begin().await.map_err(DbError::from)?;

    // Resolve the (now-unified) title: an explicit `name` wins, otherwise a patched
    // `metadata.title` also renames the project — matches the old behavior where
    // `ProjectData.name` and `ProjectData.metadata.title` could set each other.
    let title_from_metadata = match &patch.metadata {
        Some(Some(m)) => m.title.clone(),
        _ => None,
    };
    if let Some(name) = patch.name.or(title_from_metadata) {
        projects::update_title(&mut *tx, id, &name).await?;
    }

    if let Some(description) = patch.description {
        projects::update_description(&mut *tx, id, description.as_deref()).await?;
    }

    if let Some(template_id) = patch.template_id {
        projects::update_template(&mut *tx, id, Some(template_id)).await?;
    }

    if let Some(Some(metadata_patch)) = patch.metadata {
        let mut metadata = projects::get_metadata_in_tx(&mut tx, id).await?;
        metadata = metadata.patch(metadata_patch);
        projects::update_metadata_in_tx(&mut tx, id, &metadata).await?;
    }

    if let Some(Some(settings_patch)) = patch.settings {
        let mut settings = projects::get_settings_in_tx(&mut tx, id).await?;
        settings = settings.patch(settings_patch);
        projects::update_settings(&mut *tx, id, &settings).await?;
    }

    tx.commit().await.map_err(DbError::from)?;

    Ok(().into())
}

use crate::db::repositories::{bibliography, persons, projects, sections, templates};
use crate::session::session_guard::Session;
use crate::settings::Settings;
use crate::storage::project_storage::current::{Bibliography, PersonUuidOrString};
use crate::storage::project_storage::sections::Section;
use crate::utils::api_helpers::{APIResponse, APIResult};
use crate::utils::csl::{list_available_locales, list_available_styles};
use chrono::NaiveDate;
use language::Language;
use rocket::State;
use rocket::form::validate::Contains;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::collections::HashMap;
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
    pub template_id: Option<uuid::Uuid>,
    /// Optionally extended ProjectTemplate
    pub template_extended: Option<templates::Template>,
    /// Optionally extended ProjectMetadata
    pub metadata: Option<APIProjectMetadata>,
    /// Optionally extended ProjectSettings
    pub settings: Option<ProjectSettingsV5>,
    /// Optionally extended Sections
    pub sections: Option<Vec<Section>>,
    /// Optionally extended Bibliography
    pub bibliography: Option<Bibliography>,
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

/// Converts the DB-native project metadata into the API-facing shape, leaving the
/// `*_expanded` person fields unset (they are only filled in by [`get_project`] when
/// explicitly requested via the `extend` query parameter).
impl From<crate::storage::project_storage::current::ProjectMetadataV5> for APIProjectMetadata {
    fn from(value: crate::storage::project_storage::current::ProjectMetadataV5) -> Self {
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

/// Fetches core project fields (title, description, template id) and, based on the
/// comma-separated `extend` query parameter, optionally assembles and attaches
/// template, metadata (with author/editor expansion), settings, section tree,
/// bibliography, and available CSL styles/locales data by querying the relevant
/// repositories individually.
#[get("/api/projects/<project_id>?<extend>")]
pub async fn get_project(
    project_id: &str,
    extend: Option<String>,
    _session: Session,
    settings: &State<Settings>,
    pool: &State<PgPool>,
) -> APIResult<APIProjectData> {
    let project_id = Uuid::parse_str(project_id)?;
    let pool = pool.inner();

    let title = projects::get_title(pool, project_id).await?;
    let description = projects::get_description(pool, project_id).await?;
    let template_id = projects::get_template_id(pool, project_id).await?;

    let mut api_response = APIProjectData {
        project_id,
        name: title,
        description,
        template_id,
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
        if parts.contains("template")
            && let Some(template_id) = api_response.template_id
        {
            api_response.template_extended = Some(templates::get(pool, template_id).await?);
        }
        if parts.contains("metadata") {
            let metadata = projects::get_metadata(pool, project_id).await?;
            let mut metadata: APIProjectMetadata = metadata.into();

            if let Some(authors) = &metadata.authors
                && parts.contains("metadata.authors")
            {
                let mut authors_extended: Vec<PersonOrString> = Vec::new();
                for author in authors {
                    match author {
                        PersonUuidOrString::NameString(name) => {
                            authors_extended.push(PersonOrString::NameString(name.clone()))
                        }
                        PersonUuidOrString::PersonUuid(uuid) => {
                            match persons::get(pool, *uuid).await {
                                Ok(person) => authors_extended.push(PersonOrString::Person(person)),
                                Err(e) => {
                                    warn!(
                                        "Person with uuid used in project metadata, but no longer exists. Skipping. {:?}",
                                        e
                                    );
                                }
                            }
                        }
                    }
                }
                metadata.authors_expanded = Some(authors_extended);
            }

            if let Some(editors) = &metadata.editors
                && parts.contains("metadata.editors")
            {
                let mut editors_expanded: Vec<PersonOrString> = Vec::new();
                for editor in editors {
                    match editor {
                        PersonUuidOrString::NameString(name) => {
                            editors_expanded.push(PersonOrString::NameString(name.clone()))
                        }
                        PersonUuidOrString::PersonUuid(uuid) => {
                            match persons::get(pool, *uuid).await {
                                Ok(person) => editors_expanded.push(PersonOrString::Person(person)),
                                Err(e) => {
                                    warn!(
                                        "Person with uuid used in project metadata, but no longer exists. Skipping. {:?}",
                                        e
                                    );
                                }
                            }
                        }
                    }
                }
                metadata.editors_expanded = Some(editors_expanded);
            }

            api_response.metadata = Some(metadata);
        }
        if parts.contains("settings") {
            api_response.settings = Some(projects::get_settings(pool, project_id).await?);
        }
        if parts.contains("sections") {
            // Sections fetched from Postgres already never carry CRDT content (it lives on
            // the filesystem, see `db::section_content`), so no truncation step is needed.
            api_response.sections = Some(sections::get_tree_for_project(pool, project_id).await?);
        }
        if parts.contains("bibliography") {
            api_response.bibliography =
                Some(bibliography::get_all_for_project(pool, project_id).await?);
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

/// Genuine runtime coverage (not just compile-checking) of the sqlx cutover's most
/// cross-cutting read/write handlers: `Session` guard + `State<PgPool>` wiring +
/// `get.rs`'s multi-repo assembly + `patch.rs`'s targeted-update logic, all exercised over
/// real HTTP through a `rocket::local::asynchronous::Client` (mirrors `session::login`'s
/// integration tests).
#[cfg(test)]
mod integration_tests {
    use super::*;
    use crate::db::repositories::{bibliography, users};
    use crate::projects::api::patch::{PatchProjectData, patch_project};
    use crate::session::session_storage::SessionStorage;
    use crate::settings::{ExportServer, Settings};
    use crate::storage::BibEntryV3;
    use crate::storage::project_storage::current::BibEntryOrFolder;
    use crate::storage::project_storage::sections::SectionMetadata;
    use argon2::Argon2;
    use argon2::password_hash::{PasswordHasher, SaltString, rand_core::OsRng};
    use hayagriva::types::EntryType;
    use rocket::http::{ContentType, Status};
    use rocket::local::asynchronous::Client;
    use rocket_dyn_templates::Template;

    fn dummy_settings() -> Settings {
        Settings {
            app_title: "test".to_string(),
            project_cache_time: 0,
            data_path: "/tmp".to_string(),
            database_url: "".to_string(),
            database_max_connections: 1,
            file_lock_timeout: 0,
            backup_to_file_interval: 0,
            max_connections_to_rendering_server: 0,
            max_import_threads: 0,
            zotero_translation_server: "".to_string(),
            export_servers: vec![ExportServer {
                hostname: "".to_string(),
                port: 0,
                domain_name: "".to_string(),
            }],
            ca_cert_path: "".to_string(),
            client_cert_path: "".to_string(),
            client_key_path: "".to_string(),
            revocation_list_path: "".to_string(),
            version: "test".to_string(),
            max_login_attempts: 5,
            lockout_window_minutes: 15,
        }
    }

    async fn test_client(pool: PgPool) -> Client {
        // Force the debug profile so Rocket doesn't demand a configured secret_key, which it
        // otherwise requires outside the debug profile (e.g. when tests are built with
        // `cargo test --release`).
        let figment = rocket::Config::figment().select(rocket::Config::DEBUG_PROFILE);
        let rocket = rocket::custom(figment)
            .manage(pool)
            .manage(dummy_settings())
            .manage(SessionStorage::new())
            .attach(Template::fairing())
            .mount(
                "/",
                routes![
                    crate::session::login::login_page,
                    crate::session::login::process_login_form,
                    get_project,
                    patch_project
                ],
            );
        Client::tracked(rocket).await.unwrap()
    }

    /// Logs in as a freshly-created user and returns an authenticated client, mirroring how a
    /// real browser session would carry the private session cookie across requests.
    async fn authenticated_client(pool: &PgPool) -> Client {
        let team_id = users::ensure_default_team(pool).await.unwrap();
        let salt = SaltString::generate(&mut OsRng);
        let hash = Argon2::default()
            .hash_password(b"correct horse", &salt)
            .unwrap()
            .to_string();
        users::insert(pool, "reader@example.com", "Reader", &hash, team_id)
            .await
            .unwrap();

        let client = test_client(pool.clone()).await;
        {
            let response = client
                .post("/login")
                .header(ContentType::Form)
                .body("email=reader%40example.com&password=correct+horse")
                .dispatch()
                .await;
            assert_eq!(response.status(), Status::SeeOther);
        }
        client
    }

    async fn seed_project(pool: &PgPool) -> Uuid {
        let team_id = users::ensure_default_team(pool).await.unwrap();
        let project_id = Uuid::new_v4();
        projects::insert(
            pool,
            project_id,
            "Original Title",
            None,
            None,
            None,
            team_id,
        )
        .await
        .unwrap();

        let section = Section {
            id: Some(Uuid::new_v4()),
            css_classes: vec![],
            sub_sections: vec![],
            content: vec![],
            visible_in_toc: true,
            metadata: SectionMetadata {
                title: "Chapter One".to_string(),
                toc_title_subtitle_override: None,
                subtitle: None,
                authors: vec![],
                editors: vec![],
                web_url: None,
                identifiers: vec![],
                published: None,
                last_changed: None,
                lang: None,
                custom_fields: HashMap::new(),
            },
        };
        sections::insert_at_end(pool, &dummy_settings(), project_id, None, &section)
            .await
            .unwrap();

        let entry = BibEntryV3 {
            key: Uuid::new_v4(),
            entry_type: EntryType::Article,
            title: None,
            authors: vec![],
            date: None,
            editors: vec![],
            affiliated: vec![],
            publisher: None,
            location: None,
            organization: None,
            issue: None,
            volume: None,
            volume_total: None,
            edition: None,
            page_range: None,
            page_total: None,
            time_range: None,
            runtime: None,
            url: None,
            serial_numbers: None,
            language: None,
            archive: None,
            archive_location: None,
            call_number: None,
            note: None,
            abstractt: None,
            genre: None,
            parents: vec![],
        };
        bibliography::insert(pool, project_id, &BibEntryOrFolder::BibEntry(entry))
            .await
            .unwrap();

        project_id
    }

    #[sqlx::test]
    async fn get_project_assembles_metadata_sections_and_bibliography_over_http(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        let project_id = seed_project(&pool).await;
        let client = authenticated_client(&pool).await;

        let response = client
            .get(format!(
                "/api/projects/{}?extend=metadata,sections,bibliography",
                project_id
            ))
            .dispatch()
            .await;

        assert_eq!(response.status(), Status::Ok);
        let body: serde_json::Value =
            serde_json::from_str(&response.into_string().await.unwrap()).unwrap();
        let data = &body["data"];

        assert_eq!(data["name"], "Original Title");
        assert_eq!(data["metadata"]["title"], "Original Title");
        assert_eq!(data["sections"].as_array().unwrap().len(), 1);
        assert_eq!(data["sections"][0]["metadata"]["title"], "Chapter One");
        assert_eq!(
            data["bibliography"]["entries"].as_object().unwrap().len(),
            1
        );
        Ok(())
    }

    #[sqlx::test]
    async fn patch_then_get_round_trips_the_new_title(pool: PgPool) -> sqlx::Result<()> {
        let project_id = seed_project(&pool).await;
        let client = authenticated_client(&pool).await;

        let patch = PatchProjectData {
            name: Some("Patched Title".to_string()),
            description: None,
            template_id: None,
            metadata: None,
            settings: None,
        };
        let patch_response = client
            .patch(format!("/api/projects/{}", project_id))
            .header(ContentType::JSON)
            .body(serde_json::to_string(&patch).unwrap())
            .dispatch()
            .await;
        assert_eq!(patch_response.status(), Status::Ok);

        let get_response = client
            .get(format!("/api/projects/{}", project_id))
            .dispatch()
            .await;
        let body: serde_json::Value =
            serde_json::from_str(&get_response.into_string().await.unwrap()).unwrap();
        assert_eq!(body["data"]["name"], "Patched Title");
        Ok(())
    }
}

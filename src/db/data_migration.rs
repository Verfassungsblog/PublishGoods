//! One-shot migration of the legacy bincode-based storage
//! ([`DataStorage`](crate::storage::data_storage::DataStorage) /
//! [`ProjectStorage`](crate::storage::project_storage::ProjectStorage)) into PostgreSQL.
//!
//! Runs automatically at startup (see `main.rs`) and is idempotent: if the `users`
//! table is already populated, the migration is skipped.

use crate::settings::Settings;
use crate::storage::data_storage::DataStorage;
use crate::storage::data_storage::current::{ProjectListEntry, ProjectListFolder};
use crate::storage::project_storage::ProjectStorage;
use crate::storage::project_storage::current::{BibEntryOrFolder, PersonUuidOrString};
use crate::storage::project_storage::sections::Section;
use crate::storage::{ProjectTemplateV2, User};
use chrono::{DateTime, Utc};
use language::Language;
use sqlx::types::Json;
use sqlx::{PgPool, Postgres, Transaction};
use std::collections::HashSet;
use uuid::Uuid;
use vb_exchange::projects::{Identifier, License, PersonV2};

/// Reads the currently-loaded bincode [`DataStorage`]/[`ProjectStorage`] and writes it
/// into `pool`. No-op if `users` already has rows (already migrated).
pub async fn migrate_from_bincode(
    pool: &PgPool,
    data_storage: &DataStorage,
    project_storage: &ProjectStorage,
    settings: &Settings,
) -> Result<(), sqlx::Error> {
    let already_migrated: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users)")
        .fetch_one(pool)
        .await?;
    if already_migrated {
        info!("Database already contains data, skipping bincode -> PostgreSQL migration.");
        return Ok(());
    }

    info!("Migrating legacy bincode data storage into PostgreSQL...");
    let mut tx = pool.begin().await?;

    let default_team_id = Uuid::new_v4();
    sqlx::query("INSERT INTO teams (id, name) VALUES ($1, $2)")
        .bind(default_team_id)
        .bind("Default")
        .execute(&mut *tx)
        .await?;

    migrate_users(&mut tx, data_storage, default_team_id).await?;
    let person_ids = migrate_persons(&mut tx, data_storage).await?;
    let template_ids = migrate_templates(&mut tx, data_storage).await?;

    let project_list = data_storage.data.projects.read().unwrap().clone();
    let mut project_to_folder: std::collections::HashMap<Uuid, Option<Uuid>> =
        std::collections::HashMap::new();
    migrate_folders(
        &mut tx,
        &project_list.entries,
        None,
        default_team_id,
        &mut project_to_folder,
    )
    .await?;

    for (project_id, folder_id) in &project_to_folder {
        migrate_project(
            &mut tx,
            project_storage,
            settings,
            *project_id,
            *folder_id,
            default_team_id,
            &template_ids,
            &person_ids,
        )
        .await?;
    }

    tx.commit().await?;
    info!("Finished migrating bincode data storage into PostgreSQL.");
    Ok(())
}

async fn migrate_users(
    tx: &mut Transaction<'_, Postgres>,
    data_storage: &DataStorage,
    default_team_id: Uuid,
) -> Result<(), sqlx::Error> {
    let users: Vec<User> = data_storage
        .data
        .login_data
        .iter()
        .map(|e| e.value().read().unwrap().clone())
        .collect();

    for user in users {
        let locked_until = user
            .locked_until
            .and_then(|secs| DateTime::from_timestamp(secs as i64, 0));

        sqlx::query(
            "INSERT INTO users (id, email, name, password_hash, locked_until) VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(user.id)
        .bind(&user.email)
        .bind(&user.name)
        .bind(&user.password_hash)
        .bind(locked_until)
        .execute(&mut **tx)
        .await?;

        // Every migrated user is added to the default team as owner.
        sqlx::query(
            "INSERT INTO users_teams (user_id, team_id, role) VALUES ($1, $2, $3::team_role)",
        )
        .bind(user.id)
        .bind(default_team_id)
        .bind("owner")
        .execute(&mut **tx)
        .await?;
    }

    Ok(())
}

/// Migrates persons + biographies. Returns the set of person ids now present in `persons`,
/// used to validate person references from projects/sections before inserting them.
async fn migrate_persons(
    tx: &mut Transaction<'_, Postgres>,
    data_storage: &DataStorage,
) -> Result<HashSet<Uuid>, sqlx::Error> {
    let persons: Vec<PersonV2> = data_storage
        .data
        .persons
        .iter()
        .map(|e| e.value().read().unwrap().clone())
        .collect();

    let mut person_ids = HashSet::new();

    for person in persons {
        let person_id = person.id.unwrap_or_else(Uuid::new_v4);
        person_ids.insert(person_id);

        sqlx::query(
            "INSERT INTO persons (id, first_names, last_names, orcid, gnd, ror) VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(person_id)
        .bind(&person.first_names)
        .bind(&person.last_names)
        .bind(identifier_value(&person.orcid))
        .bind(identifier_value(&person.gnd))
        .bind(identifier_value(&person.ror))
        .execute(&mut **tx)
        .await?;

        for bio in person.bios.unwrap_or_default() {
            let language = bio
                .lang
                .map(|l| l.as_tag().to_string())
                .unwrap_or_else(|| "en-US".to_string());

            sqlx::query(
                "INSERT INTO biographies (person_id, content, language) VALUES ($1, $2, $3)",
            )
            .bind(person_id)
            .bind(&bio.content)
            .bind(&language)
            .execute(&mut **tx)
            .await?;
        }
    }

    Ok(person_ids)
}

fn identifier_value(identifier: &Option<Identifier>) -> Option<String> {
    identifier.as_ref().map(|i| i.value.clone())
}

/// Migrates project templates + their export formats. Returns the set of migrated
/// template ids, used to validate a project's `template_id` reference.
async fn migrate_templates(
    tx: &mut Transaction<'_, Postgres>,
    data_storage: &DataStorage,
) -> Result<HashSet<Uuid>, sqlx::Error> {
    let templates: Vec<ProjectTemplateV2> = data_storage
        .data
        .templates
        .iter()
        .map(|e| e.value().read().unwrap().clone())
        .collect();

    let mut template_ids = HashSet::new();

    for template in templates {
        template_ids.insert(template.id);
        let version = template.version.unwrap_or_else(Uuid::new_v4);

        sqlx::query(
            "INSERT INTO project_templates (id, version, name, description) VALUES ($1, $2, $3, $4)",
        )
        .bind(template.id)
        .bind(version)
        .bind(&template.name)
        .bind(&template.description)
        .execute(&mut **tx)
        .await?;

        for format in template.export_formats.values() {
            sqlx::query(
                "INSERT INTO export_formats (id, project_template_id, slug, name, preview_pdf_path, output_files, export_steps) VALUES ($1, $2, $3, $4, $5, $6, $7)",
            )
            .bind(Uuid::new_v4())
            .bind(template.id)
            .bind(&format.slug)
            .bind(&format.name)
            .bind(&format.preview_pdf_path)
            .bind(&format.output_files)
            .bind(Json(&format.export_steps))
            .execute(&mut **tx)
            .await?;
        }
    }

    Ok(template_ids)
}

/// Recursively flattens the [`ProjectList`](crate::storage::data_storage::current::ProjectList)
/// tree into `project_folders` rows (parent rows are inserted before their children, so FK
/// ordering is always satisfied), and records each project's containing folder (if any) in
/// `project_to_folder`.
fn migrate_folders<'a>(
    tx: &'a mut Transaction<'_, Postgres>,
    entries: &'a [ProjectListEntry],
    parent: Option<Uuid>,
    default_team_id: Uuid,
    project_to_folder: &'a mut std::collections::HashMap<Uuid, Option<Uuid>>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), sqlx::Error>> + Send + 'a>> {
    Box::pin(async move {
        for entry in entries {
            match entry {
                ProjectListEntry::Folder(folder) => {
                    insert_folder(tx, folder, parent, default_team_id).await?;
                    migrate_folders(
                        tx,
                        &folder.children,
                        Some(folder.id),
                        default_team_id,
                        project_to_folder,
                    )
                    .await?;
                }
                ProjectListEntry::Project(project) => {
                    project_to_folder.insert(project.id, parent);
                }
            }
        }
        Ok(())
    })
}

async fn insert_folder(
    tx: &mut Transaction<'_, Postgres>,
    folder: &ProjectListFolder,
    parent: Option<Uuid>,
    default_team_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO project_folders (id, name, owner_team_id, parent) VALUES ($1, $2, $3, $4)",
    )
    .bind(folder.id)
    .bind(&folder.name)
    .bind(default_team_id)
    .bind(parent)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn migrate_project(
    tx: &mut Transaction<'_, Postgres>,
    project_storage: &ProjectStorage,
    settings: &Settings,
    project_id: Uuid,
    folder_id: Option<Uuid>,
    default_team_id: Uuid,
    template_ids: &HashSet<Uuid>,
    person_ids: &HashSet<Uuid>,
) -> Result<(), sqlx::Error> {
    let project_data = match project_storage.get_project(&project_id, settings).await {
        Ok(data) => data,
        Err(e) => {
            error!(
                "Skipping project {} during migration: couldn't load from disk: {:?}",
                project_id, e
            );
            return Ok(());
        }
    };
    let project_data = project_data.read().unwrap().clone();

    let metadata = project_data.metadata.unwrap_or_default();
    let settings_data = project_data.settings.unwrap_or_default();

    let title = if metadata.title.trim().is_empty() {
        project_data.name.clone()
    } else {
        metadata.title.clone()
    };

    let template_id = template_ids
        .contains(&project_data.template_id)
        .then_some(project_data.template_id);
    if template_id.is_none() {
        warn!(
            "Project {} references unknown template {}, leaving template_id NULL.",
            project_id, project_data.template_id
        );
    }

    let last_interaction =
        DateTime::from_timestamp(project_data.last_interaction as i64, 0).unwrap_or_else(Utc::now);

    let languages: Option<Vec<String>> = metadata
        .languages
        .as_ref()
        .map(|langs| langs.iter().map(|l| l.as_tag().to_string()).collect());

    let license = metadata.license.as_ref().map(license_to_string);

    sqlx::query(
        "INSERT INTO projects (
            id, description, template_id, last_interaction, title, subtitle, web_url,
            publish_date, languages, number_of_pages, short_abstract, long_abstract,
            keywords, ddc, license, series, volume, edition, publisher, custom_fields,
            toc_enabled, csl_style, csl_language_code, cover_image_path, backcover_image_path,
            add_soft_hyphens, identifiers, folder, owner_team_id
        ) VALUES (
            $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18,
            $19, $20, $21, $22, $23, $24, $25, $26, $27, $28, $29
        )",
    )
    .bind(project_id)
    .bind(&project_data.description)
    .bind(template_id)
    .bind(last_interaction)
    .bind(&title)
    .bind(&metadata.subtitle)
    .bind(&metadata.web_url)
    .bind(metadata.published)
    .bind(&languages)
    .bind(metadata.number_of_pages.map(|n| n as i32))
    .bind(&metadata.short_abstract)
    .bind(&metadata.long_abstract)
    .bind(metadata.keywords.as_ref().map(Json))
    .bind(&metadata.ddc)
    .bind(&license)
    .bind(&metadata.series)
    .bind(&metadata.volume)
    .bind(&metadata.edition)
    .bind(&metadata.publisher)
    .bind(Json(&metadata.custom_fields))
    .bind(settings_data.toc_enabled)
    .bind(&settings_data.csl_style)
    .bind(&settings_data.csl_language_code)
    .bind(&settings_data.cover_image_path)
    .bind(&settings_data.backcover_image_path)
    .bind(settings_data.add_soft_hyphens)
    .bind(metadata.identifiers.as_ref().map(Json))
    .bind(folder_id)
    .bind(default_team_id)
    .execute(&mut **tx)
    .await?;

    migrate_persons_with_roles(
        tx,
        PersonLinkTable::Projects,
        project_id,
        metadata.authors.as_deref().unwrap_or(&[]),
        "author",
        person_ids,
    )
    .await?;
    migrate_persons_with_roles(
        tx,
        PersonLinkTable::Projects,
        project_id,
        metadata.editors.as_deref().unwrap_or(&[]),
        "editor",
        person_ids,
    )
    .await?;

    let section_dir = format!("{}/sections", settings.data_path);
    tokio::fs::create_dir_all(&section_dir).await.ok();

    let mut position = 1000.0f64;
    for section in &project_data.sections {
        migrate_section(
            tx,
            &section_dir,
            section,
            project_id,
            None,
            &mut position,
            person_ids,
        )
        .await?;
        position += 1000.0;
    }

    migrate_bibliography(tx, &project_data.bibliography, project_id).await?;

    Ok(())
}

fn license_to_string(license: &License) -> String {
    match license {
        License::CC0 => "CC0".to_string(),
        License::CC_BY_4 => "CC_BY_4".to_string(),
        License::CC_BY_SA_4 => "CC_BY_SA_4".to_string(),
        License::CC_BY_ND_4 => "CC_BY_ND_4".to_string(),
        License::CC_BY_NC_4 => "CC_BY_NC_4".to_string(),
        License::CC_BY_NC_SA_4 => "CC_BY_NC_SA_4".to_string(),
        License::CC_BY_NC_ND_4 => "CC_BY_NC_ND_4".to_string(),
        License::Other(other) => other.clone(),
    }
}

/// Which link table [`migrate_persons_with_roles`] writes to.
enum PersonLinkTable {
    Projects,
    Sections,
}

/// Inserts one row per [`PersonUuidOrString`] into `persons_projects`/`persons_sections`
/// with the given `role`, skipping (with a warning) any dangling person references.
async fn migrate_persons_with_roles(
    tx: &mut Transaction<'_, Postgres>,
    link_table: PersonLinkTable,
    owner_id: Uuid,
    people: &[PersonUuidOrString],
    role: &str,
    person_ids: &HashSet<Uuid>,
) -> Result<(), sqlx::Error> {
    for (index, person) in people.iter().enumerate() {
        let (person_id, name) = match person {
            PersonUuidOrString::PersonUuid(id) => {
                if !person_ids.contains(id) {
                    warn!("Skipping dangling person reference {} for {}", id, owner_id);
                    continue;
                }
                (Some(*id), None)
            }
            PersonUuidOrString::NameString(name) => (None, Some(name.clone())),
        };

        let query = match link_table {
            PersonLinkTable::Projects => {
                "INSERT INTO persons_projects (person_id, name, project_id, role, position) VALUES ($1, $2, $3, $4, $5)"
            }
            PersonLinkTable::Sections => {
                "INSERT INTO persons_sections (person_id, name, section_id, role, position) VALUES ($1, $2, $3, $4, $5)"
            }
        };
        sqlx::query(query)
            .bind(person_id)
            .bind(&name)
            .bind(owner_id)
            .bind(role)
            .bind(index as f64)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

fn migrate_section<'a>(
    tx: &'a mut Transaction<'_, Postgres>,
    section_dir: &'a str,
    section: &'a Section,
    project_id: Uuid,
    parent_section: Option<Uuid>,
    position: &'a mut f64,
    person_ids: &'a HashSet<Uuid>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), sqlx::Error>> + Send + 'a>> {
    Box::pin(async move {
        let section_id = section.id.unwrap_or_else(Uuid::new_v4);
        let metadata = &section.metadata;
        let language = metadata.lang.map(|l: Language| l.as_tag().to_string());

        sqlx::query(
            "INSERT INTO sections (
                id, project_id, parent_section, position, visible_in_toc, css_classes,
                title, toc_title_subtitle_override, subtitle, web_url, publish_date,
                language, custom_fields, identifiers
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
        )
        .bind(section_id)
        .bind(project_id)
        .bind(parent_section)
        .bind(*position)
        .bind(section.visible_in_toc)
        .bind(&section.css_classes)
        .bind(&metadata.title)
        .bind(&metadata.toc_title_subtitle_override)
        .bind(&metadata.subtitle)
        .bind(&metadata.web_url)
        .bind(metadata.published)
        .bind(&language)
        .bind(Json(&metadata.custom_fields))
        .bind(Json(&metadata.identifiers))
        .execute(&mut **tx)
        .await?;

        migrate_persons_with_roles(
            tx,
            PersonLinkTable::Sections,
            section_id,
            &metadata.authors,
            "author",
            person_ids,
        )
        .await?;
        migrate_persons_with_roles(
            tx,
            PersonLinkTable::Sections,
            section_id,
            &metadata.editors,
            "editor",
            person_ids,
        )
        .await?;

        // The section's yrs/Yjs CRDT content stays on the filesystem, keyed by section id.
        let content_path = format!("{}/{}", section_dir, section_id);
        if let Err(e) = tokio::fs::write(&content_path, &section.content).await {
            error!(
                "Failed to write CRDT content for section {} to {}: {}",
                section_id, content_path, e
            );
        }

        let mut child_position = 1000f64;
        for sub_section in &section.sub_sections {
            migrate_section(
                tx,
                section_dir,
                sub_section,
                project_id,
                Some(section_id),
                &mut child_position,
                person_ids,
            )
            .await?;
        }

        *position += 1000.0;
        Ok(())
    })
}

async fn migrate_bibliography(
    tx: &mut Transaction<'_, Postgres>,
    bibliography: &crate::storage::project_storage::current::Bibliography,
    project_id: Uuid,
) -> Result<(), sqlx::Error> {
    // Pass 1: insert all folders with parent = NULL to avoid ordering issues around the
    // self-referencing FK (the flat HashMap doesn't guarantee parents come before children).
    for (id, entry) in &bibliography.entries {
        if let BibEntryOrFolder::BibFolder(folder) = entry {
            sqlx::query(
                "INSERT INTO bibliography_folders (id, name, parent, project_id) VALUES ($1, $2, NULL, $3)",
            )
            .bind(*id)
            .bind(&folder.name)
            .bind(project_id)
            .execute(&mut **tx)
            .await?;
        }
    }

    // Pass 2: entries, whose `folder` is the first parent that resolves to a BibFolder
    // (BibEntryV3.parents also references non-folder hayagriva "parent" entries, e.g. a
    // containing book).
    for (id, entry) in &bibliography.entries {
        if let BibEntryOrFolder::BibEntry(bib_entry) = entry {
            let folder_id = bib_entry.parents.iter().find(|parent_id| {
                matches!(
                    bibliography.entries.get(*parent_id),
                    Some(BibEntryOrFolder::BibFolder(_))
                )
            });

            sqlx::query(
                "INSERT INTO bibliography_entries (id, data, folder, project_id) VALUES ($1, $2, $3, $4)",
            )
            .bind(*id)
            .bind(Json(bib_entry))
            .bind(folder_id)
            .bind(project_id)
            .execute(&mut **tx)
            .await?;
        }
    }

    // Pass 3: fix up folder parents now that every folder row exists.
    for (id, entry) in &bibliography.entries {
        if let BibEntryOrFolder::BibFolder(folder) = entry
            && let Some(parent) = folder.parent
        {
            sqlx::query("UPDATE bibliography_folders SET parent = $1 WHERE id = $2")
                .bind(parent)
                .bind(*id)
                .execute(&mut **tx)
                .await?;
        }
    }

    Ok(())
}

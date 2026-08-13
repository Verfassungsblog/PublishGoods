//! `sections` / `persons_sections`.
//!
//! `Section`/`SectionMetadata` (= `SectionV6`/`SectionMetadataV6`) implement `sqlx::FromRow`
//! directly (see their impls in `storage::project_storage::sections::current`) — a fetched
//! `Section` always has `sub_sections: vec![]`, `content: vec![]`, and empty
//! `metadata.authors`/`metadata.editors`, since a section's tree shape comes from multiple
//! rows, its author/editor list comes from a join, and its CRDT `content` never lives in
//! Postgres at all (stays on the filesystem, see `src/db/section_content.rs`). [`fetch_rows`]
//! fills in the tree shape via [`build_tree`] and [`fetch_links`] fills in authors/editors;
//! `content` is filled in separately by [`get_tree_for_project_with_content`] for the few
//! callers that need it. Because `FromRow` (not the compile-time-checked `query_as!` macro) is
//! what consumes these impls, the section-tree read query is runtime-checked.
//!
//! Known schema gap (inherited, not introduced here): `SectionMetadata.last_changed` has no
//! backing column (the migration that defined this schema already drops it, see
//! `db::data_migration::migrate_section`) — it always round-trips as `None`.

use super::DbError;
use crate::settings::Settings;
use crate::storage::project_storage::current::PersonUuidOrString;
use crate::storage::project_storage::sections::Section;
use sqlx::postgres::PgExecutor;
use sqlx::{FromRow, PgPool, Postgres, Row, Transaction};
use std::collections::HashMap;
use uuid::Uuid;
use vb_exchange::projects::Identifier;

/// Recursively assembles flat `(parent_id, Section)` rows plus per-section author/editor
/// `links` into a nested `Vec<Section>`, filling in `sub_sections` and
/// `metadata.authors`/`metadata.editors` along the way.
fn build_tree(
    parent: Option<Uuid>,
    rows: &[(Option<Uuid>, Section)],
    links: &HashMap<Uuid, (Vec<PersonUuidOrString>, Vec<PersonUuidOrString>)>,
) -> Vec<Section> {
    rows.iter()
        .filter(|(row_parent, _)| *row_parent == parent)
        .map(|(_, section)| {
            let id = section
                .id
                .expect("a section fetched from the DB always has an id");
            let (authors, editors) = links.get(&id).cloned().unwrap_or_default();
            let mut section = section.clone();
            section.metadata.authors = authors;
            section.metadata.editors = editors;
            section.sub_sections = build_tree(Some(id), rows, links);
            section
        })
        .collect()
}

/// Fetches every section row for a project, paired with its `parent_section` id, ordered
/// by `position`, ready for [`build_tree`] to assemble.
async fn fetch_rows(
    pool: &PgPool,
    project_id: Uuid,
) -> Result<Vec<(Option<Uuid>, Section)>, DbError> {
    let rows = sqlx::query(
        r#"SELECT id, parent_section, visible_in_toc, css_classes, title,
                  toc_title_subtitle_override, subtitle, web_url, publish_date, language,
                  custom_fields, identifiers
           FROM sections WHERE project_id = $1
           ORDER BY position"#,
    )
    .bind(project_id)
    .fetch_all(pool)
    .await?;

    let mut result = Vec::with_capacity(rows.len());
    for row in &rows {
        let parent_section: Option<Uuid> = row.try_get("parent_section")?;
        result.push((parent_section, Section::from_row(row)?));
    }
    Ok(result)
}

/// Loads every section's author/editor links for a project from `persons_sections`, keyed
/// by section id.
async fn fetch_links(
    pool: &PgPool,
    project_id: Uuid,
) -> Result<HashMap<Uuid, (Vec<PersonUuidOrString>, Vec<PersonUuidOrString>)>, DbError> {
    let rows = sqlx::query!(
        "SELECT ps.section_id, ps.role, ps.person_id, ps.name
         FROM persons_sections ps JOIN sections s ON ps.section_id = s.id
         WHERE s.project_id = $1
         ORDER BY ps.position",
        project_id
    )
    .fetch_all(pool)
    .await?;

    let mut links: HashMap<Uuid, (Vec<PersonUuidOrString>, Vec<PersonUuidOrString>)> =
        HashMap::new();
    for row in rows {
        let person = match row.person_id {
            Some(id) => PersonUuidOrString::PersonUuid(id),
            None => PersonUuidOrString::NameString(row.name.unwrap_or_default()),
        };
        let entry = links.entry(row.section_id).or_default();
        match row.role.as_str() {
            "author" => entry.0.push(person),
            "editor" => entry.1.push(person),
            _ => {}
        }
    }
    Ok(links)
}

/// Fetches every section for `project_id` and assembles them into the root-level `Vec<Section>`
/// tree shape, same ordering semantics as the old in-memory `Vec`-of-`Section` (children sorted
/// by `position`, recursively). `content` is always empty (see module docs).
pub async fn get_tree_for_project(
    pool: &PgPool,
    project_id: Uuid,
) -> Result<Vec<Section>, DbError> {
    let rows = fetch_rows(pool, project_id).await?;
    let links = fetch_links(pool, project_id).await?;
    Ok(build_tree(None, &rows, &links))
}

/// Like [`get_tree_for_project`], but also reads each section's CRDT body bytes off the
/// filesystem (see `db::section_content`) into `content`. Needed by consumers that actually
/// render/export the section body (`export::preprocessing`) — everyday tree consumers (the
/// contents API, move/delete) don't need the bytes and should keep using the cheaper
/// metadata-only `get_tree_for_project`.
pub async fn get_tree_for_project_with_content(
    pool: &PgPool,
    settings: &Settings,
    project_id: Uuid,
) -> Result<Vec<Section>, DbError> {
    let mut tree = get_tree_for_project(pool, project_id).await?;
    fill_content(&mut tree, settings).await?;
    Ok(tree)
}

/// Recursively reads each section's CRDT body bytes off the filesystem into `content`.
#[async_recursion::async_recursion]
async fn fill_content(sections: &mut [Section], settings: &Settings) -> Result<(), DbError> {
    for section in sections.iter_mut() {
        if let Some(id) = section.id {
            section.content = crate::db::section_content::read(settings, id).await?;
        }
        fill_content(&mut section.sub_sections, settings).await?;
    }
    Ok(())
}

/// Replaces [`crate::storage::project_storage::current::get_section_by_path`]: fetches the
/// full project tree once and walks `path`, validating that each hop is truly a child of the
/// previous one (matching the old behavior).
pub async fn resolve_path(
    pool: &PgPool,
    project_id: Uuid,
    path: &[Uuid],
) -> Result<Section, DbError> {
    let tree = get_tree_for_project(pool, project_id).await?;
    let mut current = tree
        .iter()
        .find(|s| s.id == Some(path[0]))
        .ok_or(DbError::NotFound("section"))?;
    for part in path.iter().skip(1) {
        current = current
            .sub_sections
            .iter()
            .find(|s| s.id == Some(*part))
            .ok_or(DbError::NotFound("section"))?;
    }
    Ok(current.clone())
}

/// Fetches the project id a section belongs to.
pub async fn get_project_id<'e>(
    exec: impl PgExecutor<'e>,
    section_id: Uuid,
) -> Result<Uuid, DbError> {
    sqlx::query_scalar!("SELECT project_id FROM sections WHERE id = $1", section_id)
        .fetch_optional(exec)
        .await?
        .ok_or(DbError::NotFound("section"))
}

/// All section ids for a project, for callers that need to clean up CRDT files before/around
/// a cascading SQL delete (the flat `<data_path>/sections/` directory isn't touched by any
/// DB cascade).
pub async fn get_all_ids_for_project<'e>(
    exec: impl PgExecutor<'e>,
    project_id: Uuid,
) -> Result<Vec<Uuid>, DbError> {
    let ids = sqlx::query_scalar!("SELECT id FROM sections WHERE project_id = $1", project_id)
        .fetch_all(exec)
        .await?;
    Ok(ids)
}

/// Replaces a section's author/editor links (delete-then-reinsert).
async fn replace_persons(
    tx: &mut Transaction<'_, Postgres>,
    section_id: Uuid,
    authors: &[PersonUuidOrString],
    editors: &[PersonUuidOrString],
) -> Result<(), DbError> {
    sqlx::query!(
        "DELETE FROM persons_sections WHERE section_id = $1",
        section_id
    )
    .execute(&mut **tx)
    .await?;
    insert_persons(tx, section_id, authors, "author").await?;
    insert_persons(tx, section_id, editors, "editor").await?;
    Ok(())
}

/// Inserts `people` as `persons_sections` rows under `role`, preserving list order via
/// the `position` column.
async fn insert_persons(
    tx: &mut Transaction<'_, Postgres>,
    section_id: Uuid,
    people: &[PersonUuidOrString],
    role: &str,
) -> Result<(), DbError> {
    for (index, person) in people.iter().enumerate() {
        let (person_id, name) = match person {
            PersonUuidOrString::PersonUuid(id) => (Some(*id), None),
            PersonUuidOrString::NameString(name) => (None, Some(name.clone())),
        };
        sqlx::query!(
            "INSERT INTO persons_sections (person_id, name, section_id, role, position) VALUES ($1, $2, $3, $4, $5)",
            person_id,
            name,
            section_id,
            role,
            index as f64
        )
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

/// Inserts `section` (its `id` must already be assigned by the caller, matching today's API
/// convention) as a new row appended after the last existing sibling under `parent_section`
/// (`None` = root level). If `section.content` is non-empty (e.g. sections produced by import,
/// which carry real CRDT bytes — unlike editor-created sections, which start empty), it's
/// written to the filesystem via [`crate::db::section_content`] while the DB transaction is
/// still open, so a filesystem failure rolls the row back instead of leaving an orphaned,
/// content-less section. Any `section.sub_sections` are inserted recursively afterward, under
/// the newly created row, assigning ids to children that don't already have one.
#[async_recursion::async_recursion]
pub async fn insert_at_end(
    pool: &PgPool,
    settings: &Settings,
    project_id: Uuid,
    parent_section: Option<Uuid>,
    section: &Section,
) -> Result<(), DbError> {
    let id = section
        .id
        .ok_or(DbError::Conflict("section id must be assigned".to_string()))?;
    let mut tx = pool.begin().await?;

    // Serializes concurrent inserts under the same (project, parent) pair so the
    // MAX(position) read below can't race with another transaction computing the same
    // "next" position — the lock is released automatically at commit/rollback.
    let lock_key = format!(
        "insert_at_end:{}:{}",
        project_id,
        parent_section.map(|p| p.to_string()).unwrap_or_default()
    );
    sqlx::query!(
        "SELECT pg_advisory_xact_lock(hashtext($1)::bigint)",
        lock_key
    )
    .execute(&mut *tx)
    .await?;

    let position: f64 = sqlx::query_scalar!(
        r#"SELECT COALESCE(MAX(position), 0) + 1000.0 as "position!" FROM sections
           WHERE project_id = $1 AND parent_section IS NOT DISTINCT FROM $2"#,
        project_id,
        parent_section
    )
    .fetch_one(&mut *tx)
    .await?;

    let m = &section.metadata;
    sqlx::query!(
        r#"INSERT INTO sections (
            id, project_id, parent_section, position, visible_in_toc, css_classes,
            title, toc_title_subtitle_override, subtitle, web_url, publish_date, language,
            custom_fields, identifiers
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)"#,
        id,
        project_id,
        parent_section,
        position,
        section.visible_in_toc,
        &section.css_classes,
        m.title,
        m.toc_title_subtitle_override,
        m.subtitle,
        m.web_url,
        m.published,
        m.lang.map(|l| l.as_tag().to_string()),
        sqlx::types::Json(&m.custom_fields) as sqlx::types::Json<&HashMap<String, String>>,
        sqlx::types::Json(&m.identifiers) as sqlx::types::Json<&Vec<Identifier>>,
    )
    .execute(&mut *tx)
    .await?;

    replace_persons(&mut tx, id, &m.authors, &m.editors).await?;

    if !section.content.is_empty() {
        crate::db::section_content::write(settings, id, &section.content).await?;
    }

    tx.commit().await?;

    for child in &section.sub_sections {
        let mut child = child.clone();
        if child.id.is_none() {
            child.id = Some(Uuid::new_v4());
        }
        insert_at_end(pool, settings, project_id, Some(id), &child).await?;
    }

    Ok(())
}

/// Overwrites a section's metadata/css_classes/visible_in_toc in place (matches the old
/// `*section = new_section_data` whole-struct writeback). Does not touch the tree position or
/// the CRDT content file.
pub async fn update_metadata(
    pool: &PgPool,
    section_id: Uuid,
    section: &Section,
) -> Result<(), DbError> {
    let mut tx = pool.begin().await?;
    let m = &section.metadata;

    let result = sqlx::query!(
        r#"UPDATE sections SET
            visible_in_toc = $2, css_classes = $3, title = $4, toc_title_subtitle_override = $5,
            subtitle = $6, web_url = $7, publish_date = $8, language = $9,
            custom_fields = $10, identifiers = $11
           WHERE id = $1"#,
        section_id,
        section.visible_in_toc,
        &section.css_classes,
        m.title,
        m.toc_title_subtitle_override,
        m.subtitle,
        m.web_url,
        m.published,
        m.lang.map(|l| l.as_tag().to_string()),
        sqlx::types::Json(&m.custom_fields) as sqlx::types::Json<&HashMap<String, String>>,
        sqlx::types::Json(&m.identifiers) as sqlx::types::Json<&Vec<Identifier>>,
    )
    .execute(&mut *tx)
    .await?;
    if result.rows_affected() == 0 {
        return Err(DbError::NotFound("section"));
    }

    replace_persons(&mut tx, section_id, &m.authors, &m.editors).await?;

    tx.commit().await?;
    Ok(())
}

/// Deletes `section_id` and its entire subtree in one statement (collected via a recursive
/// CTE first), returning every deleted id so the caller can clean up their CRDT files.
///
/// `section_id` must belong to `project_id` — the anchor row of the recursive CTE is scoped
/// to it, so a section id from a different project resolves to an empty set (`NotFound`)
/// instead of deleting another project's data.
///
/// `parent_section` is `ON DELETE SET NULL`, so deleting just the root row would orphan its
/// children to the top level instead of removing them — the recursive collect-then-bulk-delete
/// avoids that (every row in the subtree is removed in the same statement, so no
/// re-parenting-to-null step ever runs for rows inside the subtree).
pub async fn delete_subtree(
    pool: &PgPool,
    project_id: Uuid,
    section_id: Uuid,
) -> Result<Vec<Uuid>, DbError> {
    let ids: Vec<Uuid> = sqlx::query_scalar!(
        r#"WITH RECURSIVE descendants AS (
             SELECT id FROM sections WHERE id = $1 AND project_id = $2
             UNION ALL
             SELECT s.id FROM sections s JOIN descendants d ON s.parent_section = d.id
           )
           SELECT id as "id!" FROM descendants"#,
        section_id,
        project_id
    )
    .fetch_all(pool)
    .await?;

    if ids.is_empty() {
        return Err(DbError::NotFound("section"));
    }

    sqlx::query!("DELETE FROM sections WHERE id = ANY($1)", &ids)
        .execute(pool)
        .await?;
    Ok(ids)
}

/// Re-spaces all siblings under `parent` to evenly-spaced integer-multiple positions
/// (`1000.0`, `2000.0`, ...), used as a fallback by [`move_after`] when repeated moves have
/// collapsed the float gap between two adjacent positions below usable precision.
async fn renormalize_siblings(
    tx: &mut Transaction<'_, Postgres>,
    project_id: Uuid,
    parent: Option<Uuid>,
) -> Result<(), DbError> {
    sqlx::query!(
        r#"UPDATE sections s SET position = t.rn * 1000.0
           FROM (SELECT id, row_number() OVER (ORDER BY position) AS rn FROM sections
                 WHERE project_id = $1 AND parent_section IS NOT DISTINCT FROM $2) t
           WHERE s.id = t.id"#,
        project_id,
        parent
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Moves `section_id` to become the sibling directly after `after_id` (same parent as
/// `after_id`), using float-gap positioning with a renormalization fallback when repeated
/// moves have collapsed the gap between two positions below float precision.
///
/// Both `section_id` and `after_id` are required to belong to `project_id` — every query
/// below is scoped by it, so a section id from a different project resolves to `NotFound`
/// instead of silently re-parenting/moving data across projects.
pub async fn move_after(
    pool: &PgPool,
    project_id: Uuid,
    section_id: Uuid,
    after_id: Uuid,
) -> Result<(), DbError> {
    let mut tx = pool.begin().await?;

    let after = sqlx::query!(
        "SELECT parent_section, position FROM sections WHERE id = $1 AND project_id = $2",
        after_id,
        project_id
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(DbError::NotFound("section"))?;

    let next_position: Option<f64> = sqlx::query_scalar!(
        r#"SELECT position FROM sections
           WHERE project_id = $1 AND parent_section IS NOT DISTINCT FROM $2 AND position > $3 AND id != $4
           ORDER BY position LIMIT 1"#,
        project_id,
        after.parent_section,
        after.position,
        section_id
    )
    .fetch_optional(&mut *tx)
    .await?;

    let new_position = match next_position {
        Some(next) if next - after.position < 1e-6 => {
            renormalize_siblings(&mut tx, project_id, after.parent_section).await?;
            let after_position: f64 = sqlx::query_scalar!(
                "SELECT position FROM sections WHERE id = $1 AND project_id = $2",
                after_id,
                project_id
            )
            .fetch_one(&mut *tx)
            .await?;
            let next_position: Option<f64> = sqlx::query_scalar!(
                r#"SELECT position FROM sections
                   WHERE project_id = $1 AND parent_section IS NOT DISTINCT FROM $2 AND position > $3 AND id != $4
                   ORDER BY position LIMIT 1"#,
                project_id,
                after.parent_section,
                after_position,
                section_id
            )
            .fetch_optional(&mut *tx)
            .await?;
            match next_position {
                Some(next) => (after_position + next) / 2.0,
                None => after_position + 1000.0,
            }
        }
        Some(next) => (after.position + next) / 2.0,
        None => after.position + 1000.0,
    };

    let result = sqlx::query!(
        "UPDATE sections SET parent_section = $1, position = $2 WHERE id = $3 AND project_id = $4",
        after.parent_section,
        new_position,
        section_id,
        project_id
    )
    .execute(&mut *tx)
    .await?;
    if result.rows_affected() == 0 {
        return Err(DbError::NotFound("section"));
    }

    tx.commit().await?;
    Ok(())
}

/// Moves `section_id` to become the first child of `parent_id`.
///
/// Both ids are required to belong to `project_id` — see [`move_after`]'s doc comment for why.
pub async fn move_child_of(
    pool: &PgPool,
    project_id: Uuid,
    section_id: Uuid,
    parent_id: Uuid,
) -> Result<(), DbError> {
    let mut tx = pool.begin().await?;

    let parent_exists: bool = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM sections WHERE id = $1 AND project_id = $2)",
        parent_id,
        project_id
    )
    .fetch_one(&mut *tx)
    .await?
    .unwrap_or(false);
    if !parent_exists {
        return Err(DbError::NotFound("section"));
    }

    let min_position: Option<f64> = sqlx::query_scalar!(
        "SELECT MIN(position) FROM sections WHERE project_id = $1 AND parent_section = $2 AND id != $3",
        project_id,
        parent_id,
        section_id
    )
    .fetch_one(&mut *tx)
    .await?;
    let new_position = min_position.map(|p| p - 1000.0).unwrap_or(1000.0);

    let result = sqlx::query!(
        "UPDATE sections SET parent_section = $1, position = $2 WHERE id = $3 AND project_id = $4",
        parent_id,
        new_position,
        section_id,
        project_id
    )
    .execute(&mut *tx)
    .await?;
    if result.rows_affected() == 0 {
        return Err(DbError::NotFound("section"));
    }

    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::repositories::users;
    use crate::settings::ExportServer;
    use crate::storage::project_storage::sections::SectionMetadata;

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

    fn sample_section(title: &str) -> Section {
        Section {
            id: Some(Uuid::new_v4()),
            css_classes: vec![],
            sub_sections: vec![],
            content: vec![],
            visible_in_toc: true,
            metadata: SectionMetadata {
                title: title.to_string(),
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
        }
    }

    async fn seed_project(pool: &PgPool) -> Uuid {
        let team_id = users::ensure_default_team(pool).await.unwrap();
        sqlx::query_scalar!(
            "INSERT INTO projects (title, owner_team_id) VALUES ('Test', $1) RETURNING id",
            team_id
        )
        .fetch_one(pool)
        .await
        .unwrap()
    }

    #[sqlx::test]
    async fn tree_reconstruction_orders_by_position(pool: PgPool) -> sqlx::Result<()> {
        let project_id = seed_project(&pool).await;

        let root1 = sample_section("Root 1");
        insert_at_end(&pool, &dummy_settings(), project_id, None, &root1)
            .await
            .unwrap();
        let root2 = sample_section("Root 2");
        insert_at_end(&pool, &dummy_settings(), project_id, None, &root2)
            .await
            .unwrap();
        let child = sample_section("Child of Root 1");
        insert_at_end(&pool, &dummy_settings(), project_id, root1.id, &child)
            .await
            .unwrap();

        let tree = get_tree_for_project(&pool, project_id).await.unwrap();
        assert_eq!(tree.len(), 2);
        assert_eq!(tree[0].metadata.title, "Root 1");
        assert_eq!(tree[1].metadata.title, "Root 2");
        assert_eq!(tree[0].sub_sections.len(), 1);
        assert_eq!(tree[0].sub_sections[0].metadata.title, "Child of Root 1");
        Ok(())
    }

    #[sqlx::test]
    async fn delete_subtree_removes_descendants_but_not_siblings(pool: PgPool) -> sqlx::Result<()> {
        let project_id = seed_project(&pool).await;

        let target = sample_section("Target");
        insert_at_end(&pool, &dummy_settings(), project_id, None, &target)
            .await
            .unwrap();
        let sibling = sample_section("Sibling");
        insert_at_end(&pool, &dummy_settings(), project_id, None, &sibling)
            .await
            .unwrap();
        let child = sample_section("Child");
        insert_at_end(&pool, &dummy_settings(), project_id, target.id, &child)
            .await
            .unwrap();
        let grandchild = sample_section("Grandchild");
        insert_at_end(&pool, &dummy_settings(), project_id, child.id, &grandchild)
            .await
            .unwrap();

        let deleted = delete_subtree(&pool, project_id, target.id.unwrap())
            .await
            .unwrap();
        assert_eq!(deleted.len(), 3);
        assert!(deleted.contains(&target.id.unwrap()));
        assert!(deleted.contains(&child.id.unwrap()));
        assert!(deleted.contains(&grandchild.id.unwrap()));

        let tree = get_tree_for_project(&pool, project_id).await.unwrap();
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].metadata.title, "Sibling");
        Ok(())
    }

    #[sqlx::test]
    async fn move_after_renormalizes_when_gap_collapses(pool: PgPool) -> sqlx::Result<()> {
        let project_id = seed_project(&pool).await;

        let a = sample_section("A");
        insert_at_end(&pool, &dummy_settings(), project_id, None, &a)
            .await
            .unwrap();
        let b = sample_section("B");
        insert_at_end(&pool, &dummy_settings(), project_id, None, &b)
            .await
            .unwrap();
        let c = sample_section("C");
        insert_at_end(&pool, &dummy_settings(), project_id, None, &c)
            .await
            .unwrap();

        // Force an artificially tiny gap between A and B so the next move must renormalize.
        sqlx::query!("UPDATE sections SET position = 1.0 WHERE id = $1", a.id)
            .execute(&pool)
            .await?;
        sqlx::query!(
            "UPDATE sections SET position = 1.0000000001 WHERE id = $1",
            b.id
        )
        .execute(&pool)
        .await?;

        move_after(&pool, project_id, c.id.unwrap(), a.id.unwrap())
            .await
            .unwrap();

        let tree = get_tree_for_project(&pool, project_id).await.unwrap();
        assert_eq!(
            tree.iter()
                .map(|s| s.metadata.title.as_str())
                .collect::<Vec<_>>(),
            vec!["A", "C", "B"]
        );
        Ok(())
    }

    #[sqlx::test]
    async fn move_child_of_becomes_first_child(pool: PgPool) -> sqlx::Result<()> {
        let project_id = seed_project(&pool).await;

        let parent = sample_section("Parent");
        insert_at_end(&pool, &dummy_settings(), project_id, None, &parent)
            .await
            .unwrap();
        let existing_child = sample_section("Existing child");
        insert_at_end(
            &pool,
            &dummy_settings(),
            project_id,
            parent.id,
            &existing_child,
        )
        .await
        .unwrap();
        let mover = sample_section("Mover");
        insert_at_end(&pool, &dummy_settings(), project_id, None, &mover)
            .await
            .unwrap();

        move_child_of(&pool, project_id, mover.id.unwrap(), parent.id.unwrap())
            .await
            .unwrap();

        let tree = get_tree_for_project(&pool, project_id).await.unwrap();
        let parent_node = tree.iter().find(|s| s.metadata.title == "Parent").unwrap();
        assert_eq!(
            parent_node
                .sub_sections
                .iter()
                .map(|s| s.metadata.title.as_str())
                .collect::<Vec<_>>(),
            vec!["Mover", "Existing child"]
        );
        Ok(())
    }

    #[sqlx::test]
    async fn resolve_path_validates_each_hop(pool: PgPool) -> sqlx::Result<()> {
        let project_id = seed_project(&pool).await;
        let root = sample_section("Root");
        insert_at_end(&pool, &dummy_settings(), project_id, None, &root)
            .await
            .unwrap();
        let child = sample_section("Child");
        insert_at_end(&pool, &dummy_settings(), project_id, root.id, &child)
            .await
            .unwrap();
        let unrelated = sample_section("Unrelated");
        insert_at_end(&pool, &dummy_settings(), project_id, None, &unrelated)
            .await
            .unwrap();

        let found = resolve_path(&pool, project_id, &[root.id.unwrap(), child.id.unwrap()])
            .await
            .unwrap();
        assert_eq!(found.metadata.title, "Child");

        let not_found = resolve_path(
            &pool,
            project_id,
            &[unrelated.id.unwrap(), child.id.unwrap()],
        )
        .await;
        assert!(matches!(not_found, Err(DbError::NotFound("section"))));
        Ok(())
    }

    #[sqlx::test]
    async fn move_and_delete_reject_sections_from_a_different_project(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        let project_a = seed_project(&pool).await;
        let project_b = seed_project(&pool).await;

        let a_root = sample_section("A root");
        insert_at_end(&pool, &dummy_settings(), project_a, None, &a_root)
            .await
            .unwrap();
        let b_root = sample_section("B root");
        insert_at_end(&pool, &dummy_settings(), project_b, None, &b_root)
            .await
            .unwrap();

        // Moving/deleting an id from project B while scoped to project A must fail, not
        // silently operate on the other project's data.
        assert!(matches!(
            move_after(&pool, project_a, b_root.id.unwrap(), a_root.id.unwrap()).await,
            Err(DbError::NotFound("section"))
        ));
        assert!(matches!(
            move_child_of(&pool, project_a, b_root.id.unwrap(), a_root.id.unwrap()).await,
            Err(DbError::NotFound("section"))
        ));
        assert!(matches!(
            delete_subtree(&pool, project_a, b_root.id.unwrap()).await,
            Err(DbError::NotFound("section"))
        ));

        // B's section must be untouched.
        let tree_b = get_tree_for_project(&pool, project_b).await.unwrap();
        assert_eq!(tree_b.len(), 1);
        assert_eq!(tree_b[0].metadata.title, "B root");
        Ok(())
    }

    #[sqlx::test]
    async fn person_link_requires_exactly_one_of_id_or_name(pool: PgPool) -> sqlx::Result<()> {
        let project_id = seed_project(&pool).await;
        let section_id = sqlx::query_scalar!(
            "INSERT INTO sections (project_id, position, title) VALUES ($1, 1000, 'S') RETURNING id",
            project_id
        )
        .fetch_one(&pool)
        .await?;

        let result = sqlx::query!(
            "INSERT INTO persons_sections (section_id, role, position) VALUES ($1, 'author', 0)",
            section_id
        )
        .execute(&pool)
        .await;
        assert!(result.is_err());
        Ok(())
    }

    /// `insert_at_end` must persist non-empty `content` to the filesystem — otherwise CRDT
    /// bytes produced by e.g. import are silently dropped (the section row exists, but its
    /// body never lands on disk, so it opens blank).
    #[sqlx::test]
    async fn insert_at_end_persists_content_to_filesystem(pool: PgPool) -> sqlx::Result<()> {
        let project_id = seed_project(&pool).await;
        let settings = dummy_settings();

        let mut section = sample_section("With content");
        section.content = vec![1, 2, 3, 4];
        let id = section.id.unwrap();

        insert_at_end(&pool, &settings, project_id, None, &section)
            .await
            .unwrap();

        let on_disk = crate::db::section_content::read(&settings, id)
            .await
            .unwrap();
        assert_eq!(on_disk, vec![1, 2, 3, 4]);

        crate::db::section_content::delete(&settings, id)
            .await
            .unwrap();
        Ok(())
    }

    /// `insert_at_end` must recursively insert `sub_sections` too — otherwise a caller-supplied
    /// nested tree is silently truncated to just its top-level section.
    #[sqlx::test]
    async fn insert_at_end_persists_nested_sub_sections(pool: PgPool) -> sqlx::Result<()> {
        let project_id = seed_project(&pool).await;
        let settings = dummy_settings();

        let mut root = sample_section("Root");
        let mut child = sample_section("Child");
        let grandchild = sample_section("Grandchild");
        child.sub_sections = vec![grandchild.clone()];
        root.sub_sections = vec![child.clone()];

        insert_at_end(&pool, &settings, project_id, None, &root)
            .await
            .unwrap();

        let tree = get_tree_for_project(&pool, project_id).await.unwrap();
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].sub_sections.len(), 1);
        assert_eq!(tree[0].sub_sections[0].metadata.title, "Child");
        assert_eq!(tree[0].sub_sections[0].sub_sections.len(), 1);
        assert_eq!(
            tree[0].sub_sections[0].sub_sections[0].metadata.title,
            "Grandchild"
        );
        Ok(())
    }
}

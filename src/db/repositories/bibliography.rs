//! `bibliography_folders` / `bibliography_entries`.
//!
//! [`Bibliography`]/[`BibEntryOrFolder`]/[`BibFolder`]/[`BibEntryV3`] are local crate types
//! (`storage::project_storage::current` / `storage::mod`), reused as-is — only their
//! load/store moves from an in-memory `HashMap` to two flat tables. A `BibEntryV3`'s
//! `parents: Vec<Uuid>` (hayagriva "contained in" relation) already includes its containing
//! folder's id when one exists (see `db::data_migration::migrate_bibliography`), so the
//! `bibliography_entries.folder` FK column is a derived index, not a separate source of
//! truth — reads never need to reconcile it back into `parents`.

use super::DbError;
use crate::storage::BibEntryV3;
use crate::storage::project_storage::current::{BibEntryOrFolder, BibFolder, Bibliography};
use sqlx::PgPool;
use sqlx::types::Json;
use std::collections::HashMap;
use uuid::Uuid;

/// Loads a project's entire bibliography (folders and entries) and reconstructs it into
/// the flat `entries` map that [`Bibliography`] expects.
pub async fn get_all_for_project(pool: &PgPool, project_id: Uuid) -> Result<Bibliography, DbError> {
    let mut entries = HashMap::new();

    let folder_rows = sqlx::query!(
        "SELECT id, name, parent FROM bibliography_folders WHERE project_id = $1",
        project_id
    )
    .fetch_all(pool)
    .await?;
    for row in folder_rows {
        entries.insert(
            row.id,
            BibEntryOrFolder::BibFolder(BibFolder {
                name: row.name,
                parent: row.parent,
            }),
        );
    }

    let entry_rows = sqlx::query!(
        r#"SELECT id, data as "data: Json<BibEntryV3>" FROM bibliography_entries WHERE project_id = $1"#,
        project_id
    )
    .fetch_all(pool)
    .await?;
    for row in entry_rows {
        if let Some(Json(entry)) = row.data {
            entries.insert(row.id, BibEntryOrFolder::BibEntry(entry));
        }
    }

    Ok(Bibliography { entries })
}

/// Derives the `bibliography_entries.folder` FK value from an entry's `parents` list by
/// returning the first `parents` id that is itself a folder in this project (if any).
async fn find_folder_parent(
    pool: &PgPool,
    project_id: Uuid,
    parents: &[Uuid],
) -> Result<Option<Uuid>, DbError> {
    for parent in parents {
        let exists = sqlx::query_scalar!(
            "SELECT EXISTS(SELECT 1 FROM bibliography_folders WHERE id = $1 AND project_id = $2)",
            parent,
            project_id
        )
        .fetch_one(pool)
        .await?
        .unwrap_or(false);
        if exists {
            return Ok(Some(*parent));
        }
    }
    Ok(None)
}

/// Inserts a new entry or folder (whichever variant `item` is), returning its new id.
/// Mirrors `POST /api/project/<id>/bibliography`, which accepts either shape.
pub async fn insert(
    pool: &PgPool,
    project_id: Uuid,
    item: &BibEntryOrFolder,
) -> Result<Uuid, DbError> {
    match item {
        BibEntryOrFolder::BibEntry(entry) => {
            let mut entry = entry.clone();
            if entry.key == Uuid::nil() {
                entry.key = Uuid::new_v4();
            }
            let folder_id = find_folder_parent(pool, project_id, &entry.parents).await?;
            sqlx::query!(
                "INSERT INTO bibliography_entries (id, data, folder, project_id) VALUES ($1, $2, $3, $4)",
                entry.key,
                Json(&entry) as Json<&BibEntryV3>,
                folder_id,
                project_id
            )
            .execute(pool)
            .await?;
            Ok(entry.key)
        }
        BibEntryOrFolder::BibFolder(folder) => {
            let id = sqlx::query_scalar!(
                "INSERT INTO bibliography_folders (name, parent, project_id) VALUES ($1, $2, $3) RETURNING id",
                folder.name,
                folder.parent,
                project_id
            )
            .fetch_one(pool)
            .await?;
            Ok(id)
        }
    }
}

/// Overwrites the entry/folder at `id` with `item`, requiring `id` to already exist as the
/// *same* variant (an entry row can't be turned into a folder row in place, and vice versa —
/// nothing in the app does this today; the old whole-`HashMap`-value replace technically
/// allowed it, but treating it as a type-stable update is the sound relational equivalent).
pub async fn update(
    pool: &PgPool,
    project_id: Uuid,
    id: Uuid,
    item: &BibEntryOrFolder,
) -> Result<(), DbError> {
    match item {
        BibEntryOrFolder::BibEntry(entry) => {
            let folder_id = find_folder_parent(pool, project_id, &entry.parents).await?;
            let result = sqlx::query!(
                "UPDATE bibliography_entries SET data = $2, folder = $3 WHERE id = $1 AND project_id = $4",
                id,
                Json(entry) as Json<&BibEntryV3>,
                folder_id,
                project_id
            )
            .execute(pool)
            .await?;
            if result.rows_affected() == 0 {
                return Err(DbError::NotFound("bibliography_entry"));
            }
        }
        BibEntryOrFolder::BibFolder(folder) => {
            let result = sqlx::query!(
                "UPDATE bibliography_folders SET name = $2, parent = $3 WHERE id = $1 AND project_id = $4",
                id,
                folder.name,
                folder.parent,
                project_id
            )
            .execute(pool)
            .await?;
            if result.rows_affected() == 0 {
                return Err(DbError::NotFound("bibliography_entry"));
            }
        }
    }
    Ok(())
}

/// Deletes an entry or folder by id (tries the entries table, then the folders table).
/// Folder deletion cascades child folders'/entries' `parent`/`folder` columns to NULL via FK.
/// Either way, `parents` jsonb references to the deleted id are swept from every entry in
/// this project — that relation isn't a FK (an entry's `parents` can name a deleted folder
/// just as easily as a deleted entry), so it needs a manual pass regardless of which table
/// the deleted row came from.
pub async fn delete(pool: &PgPool, project_id: Uuid, id: Uuid) -> Result<(), DbError> {
    let entry_deleted = sqlx::query!(
        "DELETE FROM bibliography_entries WHERE id = $1 AND project_id = $2",
        id,
        project_id
    )
    .execute(pool)
    .await?
    .rows_affected()
        > 0;

    if entry_deleted {
        sweep_parent_references(pool, project_id, id).await?;
        return Ok(());
    }

    let folder_deleted = sqlx::query!(
        "DELETE FROM bibliography_folders WHERE id = $1 AND project_id = $2",
        id,
        project_id
    )
    .execute(pool)
    .await?
    .rows_affected()
        > 0;

    if folder_deleted {
        sweep_parent_references(pool, project_id, id).await?;
        return Ok(());
    }

    Err(DbError::NotFound("bibliography_entry"))
}

/// Removes `removed_id` from every entry's `parents` array in the project (the `parents`
/// jsonb field isn't FK-backed, so deletions elsewhere don't clean it up automatically).
async fn sweep_parent_references(
    pool: &PgPool,
    project_id: Uuid,
    removed_id: Uuid,
) -> Result<(), DbError> {
    let rows = sqlx::query!(
        r#"SELECT id, data as "data: Json<BibEntryV3>" FROM bibliography_entries WHERE project_id = $1"#,
        project_id
    )
    .fetch_all(pool)
    .await?;

    for row in rows {
        if let Some(Json(mut entry)) = row.data
            && entry.parents.contains(&removed_id)
        {
            entry.parents.retain(|id| *id != removed_id);
            sqlx::query!(
                "UPDATE bibliography_entries SET data = $2 WHERE id = $1",
                row.id,
                Json(&entry) as Json<&BibEntryV3>
            )
            .execute(pool)
            .await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::repositories::users;
    use hayagriva::types::EntryType;

    fn sample_entry(key: Uuid, parents: Vec<Uuid>) -> BibEntryV3 {
        BibEntryV3 {
            key,
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
            parents,
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
    async fn folder_and_entry_tree_reconstruction(pool: PgPool) -> sqlx::Result<()> {
        let project_id = seed_project(&pool).await;

        let folder_id = insert(
            &pool,
            project_id,
            &BibEntryOrFolder::BibFolder(BibFolder {
                name: "Folder".to_string(),
                parent: None,
            }),
        )
        .await
        .unwrap();

        let entry = sample_entry(Uuid::new_v4(), vec![folder_id]);
        let entry_id = insert(&pool, project_id, &BibEntryOrFolder::BibEntry(entry))
            .await
            .unwrap();

        let bib = get_all_for_project(&pool, project_id).await.unwrap();
        assert_eq!(bib.entries.len(), 2);
        assert!(matches!(
            bib.entries.get(&folder_id),
            Some(BibEntryOrFolder::BibFolder(_))
        ));
        match bib.entries.get(&entry_id) {
            Some(BibEntryOrFolder::BibEntry(e)) => assert_eq!(e.parents, vec![folder_id]),
            other => panic!("expected entry, got {:?}", other),
        }

        // folder FK derivation: the entry's row-level `folder` column should match too
        let folder_column: Option<Uuid> = sqlx::query_scalar!(
            "SELECT folder FROM bibliography_entries WHERE id = $1",
            entry_id
        )
        .fetch_one(&pool)
        .await?;
        assert_eq!(folder_column, Some(folder_id));
        Ok(())
    }

    #[sqlx::test]
    async fn delete_entry_sweeps_parents_references(pool: PgPool) -> sqlx::Result<()> {
        let project_id = seed_project(&pool).await;

        let parent_entry = sample_entry(Uuid::new_v4(), vec![]);
        let parent_id = insert(&pool, project_id, &BibEntryOrFolder::BibEntry(parent_entry))
            .await
            .unwrap();

        let child_entry = sample_entry(Uuid::new_v4(), vec![parent_id]);
        let child_id = insert(&pool, project_id, &BibEntryOrFolder::BibEntry(child_entry))
            .await
            .unwrap();

        delete(&pool, project_id, parent_id).await.unwrap();

        let bib = get_all_for_project(&pool, project_id).await.unwrap();
        match bib.entries.get(&child_id) {
            Some(BibEntryOrFolder::BibEntry(e)) => assert!(e.parents.is_empty()),
            other => panic!("expected entry, got {:?}", other),
        }
        Ok(())
    }

    #[sqlx::test]
    async fn delete_folder_cascades_set_null_to_children(pool: PgPool) -> sqlx::Result<()> {
        let project_id = seed_project(&pool).await;

        let parent_folder_id = insert(
            &pool,
            project_id,
            &BibEntryOrFolder::BibFolder(BibFolder {
                name: "Parent".to_string(),
                parent: None,
            }),
        )
        .await
        .unwrap();
        let child_folder_id = insert(
            &pool,
            project_id,
            &BibEntryOrFolder::BibFolder(BibFolder {
                name: "Child".to_string(),
                parent: Some(parent_folder_id),
            }),
        )
        .await
        .unwrap();

        delete(&pool, project_id, parent_folder_id).await.unwrap();

        let bib = get_all_for_project(&pool, project_id).await.unwrap();
        match bib.entries.get(&child_folder_id) {
            Some(BibEntryOrFolder::BibFolder(f)) => assert_eq!(f.parent, None),
            other => panic!("expected folder, got {:?}", other),
        }
        Ok(())
    }

    /// Deleting a folder must sweep the folder's id out of any entry's `parents` array too —
    /// otherwise the entry gets bucketed under a dead folder id and vanishes from the tree.
    #[sqlx::test]
    async fn delete_folder_sweeps_parents_references(pool: PgPool) -> sqlx::Result<()> {
        let project_id = seed_project(&pool).await;

        let folder_id = insert(
            &pool,
            project_id,
            &BibEntryOrFolder::BibFolder(BibFolder {
                name: "Folder".to_string(),
                parent: None,
            }),
        )
        .await
        .unwrap();

        let entry = sample_entry(Uuid::new_v4(), vec![folder_id]);
        let entry_id = insert(&pool, project_id, &BibEntryOrFolder::BibEntry(entry))
            .await
            .unwrap();

        delete(&pool, project_id, folder_id).await.unwrap();

        let bib = get_all_for_project(&pool, project_id).await.unwrap();
        match bib.entries.get(&entry_id) {
            Some(BibEntryOrFolder::BibEntry(e)) => assert!(e.parents.is_empty()),
            other => panic!("expected entry, got {:?}", other),
        }
        Ok(())
    }
}

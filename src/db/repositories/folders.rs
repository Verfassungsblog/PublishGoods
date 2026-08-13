//! `project_folders` + the project-list-tree read model.
//!
//! Reuses [`ProjectList`]/[`ProjectListEntry`]/[`ProjectListFolder`]/[`ProjectListProject`]
//! from the old bincode storage module — they're plain data structs with no DashMap/RwLock
//! coupling, so there's no need for a parallel type here; only how they're built changes
//! (fetch-all-then-assemble-in-Rust from two flat tables instead of one in-memory tree).

use super::DbError;
use crate::storage::data_storage::current::{
    ProjectList, ProjectListEntry, ProjectListFolder, ProjectListProject,
};
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use sqlx::postgres::PgExecutor;
use uuid::Uuid;

/// `(id, name, parent)` from `project_folders`.
type FolderRow = (Uuid, String, Option<Uuid>);
/// `(id, title, last_interaction, folder)` from `projects`.
type ProjectRow = (Uuid, String, DateTime<Utc>, Option<Uuid>);

/// Recursively assembles the folders/projects belonging to `parent` into the
/// [`ProjectListEntry`] tree shape, descending into sub-folders.
fn build_entries(
    parent: Option<Uuid>,
    folders: &[FolderRow],
    projects: &[ProjectRow],
) -> Vec<ProjectListEntry> {
    let mut entries = Vec::new();
    for (id, name, _) in folders.iter().filter(|(_, _, p)| *p == parent) {
        entries.push(ProjectListEntry::Folder(ProjectListFolder {
            id: *id,
            name: name.clone(),
            children: build_entries(Some(*id), folders, projects),
        }));
    }
    for (id, name, last_interaction, _) in projects.iter().filter(|(_, _, _, f)| *f == parent) {
        entries.push(ProjectListEntry::Project(ProjectListProject {
            id: *id,
            name: name.clone(),
            last_interaction: last_interaction.naive_utc(),
        }));
    }
    entries
}

/// Replaces `data_storage.data.projects.read().unwrap().entries.clone()`: fetches every
/// folder and every project's summary in two queries and assembles the same tree shape.
pub async fn get_project_list_tree(pool: &PgPool) -> Result<ProjectList, DbError> {
    let folders: Vec<FolderRow> = sqlx::query!("SELECT id, name, parent FROM project_folders")
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|r| (r.id, r.name, r.parent))
        .collect();

    let projects: Vec<ProjectRow> =
        sqlx::query!("SELECT id, title, last_interaction, folder FROM projects")
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(|r| (r.id, r.title, r.last_interaction, r.folder))
            .collect();

    Ok(ProjectList {
        entries: build_entries(None, &folders, &projects),
    })
}

/// Updates a project's `last_interaction` timestamp to now. Fails with
/// [`DbError::NotFound`] if no project with that id exists.
pub async fn touch_project_last_interaction<'e>(
    exec: impl PgExecutor<'e>,
    project_id: Uuid,
) -> Result<(), DbError> {
    let result = sqlx::query!(
        "UPDATE projects SET last_interaction = now() WHERE id = $1",
        project_id
    )
    .execute(exec)
    .await?;
    if result.rows_affected() == 0 {
        return Err(DbError::NotFound("project"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::repositories::users;

    async fn seed_project(
        pool: &PgPool,
        title: &str,
        folder: Option<Uuid>,
        owner_team_id: Uuid,
    ) -> Uuid {
        sqlx::query_scalar!(
            "INSERT INTO projects (title, folder, owner_team_id) VALUES ($1, $2, $3) RETURNING id",
            title,
            folder,
            owner_team_id
        )
        .fetch_one(pool)
        .await
        .unwrap()
    }

    #[sqlx::test]
    async fn tree_reconstruction_nests_folders_and_projects(pool: PgPool) -> sqlx::Result<()> {
        let team_id = users::ensure_default_team(&pool).await.unwrap();

        let root_folder: Uuid = sqlx::query_scalar!(
            "INSERT INTO project_folders (name, owner_team_id) VALUES ('Root', $1) RETURNING id",
            team_id
        )
        .fetch_one(&pool)
        .await?;

        seed_project(&pool, "Top-level project", None, team_id).await;
        seed_project(&pool, "Nested project", Some(root_folder), team_id).await;

        let tree = get_project_list_tree(&pool).await.unwrap();
        assert_eq!(tree.entries.len(), 2);

        let folder = tree
            .entries
            .iter()
            .find_map(|e| match e {
                ProjectListEntry::Folder(f) if f.id == root_folder => Some(f),
                _ => None,
            })
            .unwrap();
        assert_eq!(folder.children.len(), 1);
        Ok(())
    }

    #[sqlx::test]
    async fn deleting_parent_folder_orphans_child_to_root(pool: PgPool) -> sqlx::Result<()> {
        let team_id = users::ensure_default_team(&pool).await.unwrap();
        let parent: Uuid = sqlx::query_scalar!(
            "INSERT INTO project_folders (name, owner_team_id) VALUES ('Parent', $1) RETURNING id",
            team_id
        )
        .fetch_one(&pool)
        .await?;
        let child: Uuid = sqlx::query_scalar!(
            "INSERT INTO project_folders (name, owner_team_id, parent) VALUES ('Child', $1, $2) RETURNING id",
            team_id,
            parent
        )
        .fetch_one(&pool)
        .await?;

        sqlx::query!("DELETE FROM project_folders WHERE id = $1", parent)
            .execute(&pool)
            .await?;

        // Unlike sections (full-subtree delete), folders use ON DELETE SET NULL: the
        // child folder survives, re-parented to root, instead of being deleted too.
        let remaining_parent: Option<Uuid> =
            sqlx::query_scalar!("SELECT parent FROM project_folders WHERE id = $1", child)
                .fetch_one(&pool)
                .await?;
        assert_eq!(remaining_parent, None);
        Ok(())
    }
}

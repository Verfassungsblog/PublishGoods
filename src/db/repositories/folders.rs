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

/// Replaces `data_storage.data.projects.read().unwrap().entries.clone()`: fetches the folders
/// and project summaries of every team `user_id` belongs to in two queries and assembles the
/// same tree shape.
pub async fn get_project_list_tree(pool: &PgPool, user_id: Uuid) -> Result<ProjectList, DbError> {
    let folders: Vec<FolderRow> = sqlx::query!(
        "SELECT id, name, parent FROM project_folders
         WHERE owner_team_id IN (SELECT team_id FROM users_teams WHERE user_id = $1)",
        user_id
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|r| (r.id, r.name, r.parent))
    .collect();

    let projects: Vec<ProjectRow> = sqlx::query!(
        "SELECT id, title, last_interaction, folder FROM projects
             WHERE owner_team_id IN (SELECT team_id FROM users_teams WHERE user_id = $1)",
        user_id
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|r| (r.id, r.title, r.last_interaction, r.folder))
    .collect();

    Ok(ProjectList {
        entries: build_entries(None, &folders, &projects),
    })
}

/// The projects of one team, as shown on the dashboard.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TeamProjects {
    pub team_id: Uuid,
    pub team_name: String,
    /// Most recently used first.
    pub projects: Vec<ProjectListProject>,
}

/// Returns every team `user_id` belongs to (ordered by name, empty teams included) together
/// with that team's projects, most recently used first. Folders are not part of this view.
pub async fn get_projects_by_team(
    pool: &PgPool,
    user_id: Uuid,
) -> Result<Vec<TeamProjects>, DbError> {
    let rows = sqlx::query!(
        r#"SELECT t.id AS team_id, t.name AS team_name,
                  p.id AS "project_id?", p.title AS "title?",
                  p.last_interaction AS "last_interaction?"
           FROM teams t
           JOIN users_teams ut ON ut.team_id = t.id
           LEFT JOIN projects p ON p.owner_team_id = t.id
           WHERE ut.user_id = $1
           ORDER BY t.name, t.id, p.last_interaction DESC, p.id"#,
        user_id
    )
    .fetch_all(pool)
    .await?;

    let mut teams: Vec<TeamProjects> = Vec::new();
    for row in rows {
        if teams.last().is_none_or(|t| t.team_id != row.team_id) {
            teams.push(TeamProjects {
                team_id: row.team_id,
                team_name: row.team_name,
                projects: Vec::new(),
            });
        }
        if let (Some(id), Some(name), Some(last_interaction)) =
            (row.project_id, row.title, row.last_interaction)
        {
            teams
                .last_mut()
                .expect("a team was just pushed")
                .projects
                .push(ProjectListProject {
                    id,
                    name,
                    last_interaction: last_interaction.naive_utc(),
                });
        }
    }
    Ok(teams)
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

        let alice = users::insert(&pool, "alice@example.com", "Alice", "hash", team_id)
            .await
            .unwrap();
        let tree = get_project_list_tree(&pool, alice.id).await.unwrap();
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
    async fn projects_are_grouped_per_team_including_empty_teams(pool: PgPool) -> sqlx::Result<()> {
        let default_team = users::ensure_default_team(&pool).await.unwrap();
        let alice = users::insert(&pool, "alice@example.com", "Alice", "hash", default_team)
            .await
            .unwrap();
        let personal_team: Uuid = sqlx::query_scalar!(
            "SELECT team_id FROM users_teams WHERE user_id = $1 AND team_id <> $2",
            alice.id,
            default_team
        )
        .fetch_one(&pool)
        .await?;
        let other_team: Uuid =
            sqlx::query_scalar!("INSERT INTO teams (name) VALUES ('Other') RETURNING id")
                .fetch_one(&pool)
                .await?;

        // "Old" is inserted last but is the older one, so insertion order can't satisfy the assertion
        let new = seed_project(&pool, "New", None, default_team).await;
        let old = seed_project(&pool, "Old", None, default_team).await;
        sqlx::query!(
            "UPDATE projects SET last_interaction = now() - interval '2 days' WHERE id = $1",
            old
        )
        .execute(&pool)
        .await?;
        seed_project(&pool, "Not mine", None, other_team).await;

        let teams = get_projects_by_team(&pool, alice.id).await.unwrap();
        assert_eq!(teams.len(), 2);
        let personal = teams.iter().find(|t| t.team_id == personal_team).unwrap();
        assert!(personal.projects.is_empty());
        let shared = teams.iter().find(|t| t.team_id == default_team).unwrap();
        assert_eq!(shared.projects.len(), 2);
        let order: Vec<Uuid> = shared.projects.iter().map(|p| p.id).collect();
        assert_eq!(order, [new, old]);
        Ok(())
    }

    #[sqlx::test]
    async fn tree_only_contains_projects_of_the_users_teams(pool: PgPool) -> sqlx::Result<()> {
        let default_team = users::ensure_default_team(&pool).await.unwrap();
        let alice = users::insert(&pool, "alice@example.com", "Alice", "hash", default_team)
            .await
            .unwrap();
        // bob is only in his personal team, not in the shared Default team
        let bob = users::insert(&pool, "bob@example.com", "Bob", "hash", default_team)
            .await
            .unwrap();
        sqlx::query!(
            "DELETE FROM users_teams WHERE user_id = $1 AND team_id = $2",
            bob.id,
            default_team
        )
        .execute(&pool)
        .await?;
        let bobs_team: Uuid =
            sqlx::query_scalar!("SELECT team_id FROM users_teams WHERE user_id = $1", bob.id)
                .fetch_one(&pool)
                .await?;

        seed_project(&pool, "Shared", None, default_team).await;
        seed_project(&pool, "Bob's", None, bobs_team).await;

        let titles = |tree: ProjectList| -> Vec<String> {
            tree.entries
                .into_iter()
                .filter_map(|e| match e {
                    ProjectListEntry::Project(p) => Some(p.name),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(
            titles(get_project_list_tree(&pool, alice.id).await.unwrap()),
            ["Shared"]
        );
        assert_eq!(
            titles(get_project_list_tree(&pool, bob.id).await.unwrap()),
            ["Bob's"]
        );
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

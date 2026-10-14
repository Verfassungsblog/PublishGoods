//! Authorization: every resource belongs (directly or through its project) to exactly one
//! team, and a user may act on it if they are a member of that team with a sufficient role.
//!
//! [`has_permission`] is the single check handlers call; it resolves the owning team of the
//! resource and compares the user's [`TeamRole`] against the required one.

use crate::db::repositories::teams;
use crate::db::repositories::users::TeamRole;
use crate::utils::api_helpers::{ApiError, ApiErrorType};
use sqlx::PgPool;
use uuid::Uuid;

/// The kinds of resources that can be permission-checked.
///
/// Sections have no permissions of their own: check [`ResourceType::Project`] with the
/// section's project instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceType {
    Team,
    Folder,
    Project,
    /// A person from the `persons` table, team- or project-scoped. Free-text names are not
    /// persons; they belong to the project that contains them.
    Person,
}

impl ResourceType {
    fn name(self) -> &'static str {
        match self {
            ResourceType::Team => "team",
            ResourceType::Folder => "folder",
            ResourceType::Project => "project",
            ResourceType::Person => "person",
        }
    }
}

/// Returns the id of the team that owns the resource, or `ResourceNotFound` if the resource
/// doesn't exist.
pub async fn resource_team_id(
    pool: &PgPool,
    resource_type: ResourceType,
    resource_id: Uuid,
) -> Result<Uuid, ApiError> {
    let team_id: Option<Uuid> = match resource_type {
        ResourceType::Team => {
            sqlx::query_scalar!("SELECT id FROM teams WHERE id = $1", resource_id)
                .fetch_optional(pool)
                .await
                .map_err(crate::db::repositories::DbError::from)?
        }
        ResourceType::Folder => sqlx::query_scalar!(
            "SELECT owner_team_id FROM project_folders WHERE id = $1",
            resource_id
        )
        .fetch_optional(pool)
        .await
        .map_err(crate::db::repositories::DbError::from)?,
        ResourceType::Project => sqlx::query_scalar!(
            "SELECT owner_team_id FROM projects WHERE id = $1",
            resource_id
        )
        .fetch_optional(pool)
        .await
        .map_err(crate::db::repositories::DbError::from)?,
        // A person is owned by its team directly, or by the team owning its project.
        ResourceType::Person => sqlx::query_scalar!(
            r#"SELECT COALESCE(p.owner_team_id, pr.owner_team_id) AS "team_id"
               FROM persons p
               LEFT JOIN projects pr ON pr.id = p.project_id
               WHERE p.id = $1"#,
            resource_id
        )
        .fetch_optional(pool)
        .await
        .map_err(crate::db::repositories::DbError::from)?
        .flatten(),
    };

    team_id.ok_or_else(|| ApiErrorType::ResourceNotFound(resource_type.name().to_string()).into())
}

/// Checks that `user_id` may act on the resource.
///
/// `required_role` is the *minimum* team role: `None` means any member, `Some(Admin)` means
/// admins and owners, `Some(Owner)` means owners only. A missing resource and a user who isn't
/// a member of its team both yield `ResourceNotFound`, so non-members can't tell a resource
/// they can't access from one that doesn't exist. A member whose role is too low gets
/// `Forbidden`. On success the user's actual role on the owning team is returned, for callers
/// that need to branch on it without a second membership lookup.
///
/// Takes the pool as an extra first argument: there is no global pool, it lives in Rocket's
/// managed state.
pub async fn has_permission(
    pool: &PgPool,
    resource_type: ResourceType,
    resource_id: Uuid,
    user_id: Uuid,
    required_role: Option<TeamRole>,
) -> Result<TeamRole, ApiError> {
    let team_id = resource_team_id(pool, resource_type, resource_id).await?;
    let role = teams::member_role(pool, team_id, user_id)
        .await?
        .ok_or_else(|| ApiErrorType::ResourceNotFound(resource_type.name().to_string()))?;

    match required_role {
        Some(min) if !role.at_least(min) => {
            Err(ApiErrorType::Forbidden("insufficient team role".to_string()).into())
        }
        _ => Ok(role),
    }
}

/// Checks that `user_id` may act on every person in `person_ids` (see [`has_permission`] with
/// [`ResourceType::Person`] and no minimum role), in a single query. An unknown person and one
/// the user can't access both yield `ResourceNotFound`.
pub async fn has_permission_for_persons(
    pool: &PgPool,
    person_ids: &[Uuid],
    user_id: Uuid,
) -> Result<(), ApiError> {
    if person_ids.is_empty() {
        return Ok(());
    }
    let accessible: Vec<Uuid> = sqlx::query_scalar!(
        r#"SELECT p.id AS "id!"
           FROM persons p
           LEFT JOIN projects pr ON pr.id = p.project_id
           JOIN users_teams ut ON ut.team_id = COALESCE(p.owner_team_id, pr.owner_team_id)
           WHERE p.id = ANY($1) AND ut.user_id = $2"#,
        person_ids,
        user_id
    )
    .fetch_all(pool)
    .await
    .map_err(crate::db::repositories::DbError::from)?;
    let accessible: std::collections::HashSet<Uuid> = accessible.into_iter().collect();
    if person_ids.iter().all(|id| accessible.contains(id)) {
        Ok(())
    } else {
        Err(ApiErrorType::ResourceNotFound(ResourceType::Person.name().to_string()).into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::repositories::users;

    struct Fixture {
        team: Uuid,
        folder: Uuid,
        project: Uuid,
        team_person: Uuid,
        project_person: Uuid,
        owner: Uuid,
        admin: Uuid,
        member: Uuid,
        outsider: Uuid,
    }

    async fn fixture(pool: &PgPool) -> Fixture {
        let default_team = users::ensure_default_team(pool).await.unwrap();
        let mut ids = vec![];
        for name in ["owner", "admin", "member", "outsider"] {
            let user = users::insert(
                pool,
                &format!("{name}@example.com"),
                name,
                "hash",
                default_team,
            )
            .await
            .unwrap();
            ids.push(user.id);
        }
        let team = sqlx::query_scalar!("INSERT INTO teams (name) VALUES ('Scoped') RETURNING id")
            .fetch_one(pool)
            .await
            .unwrap();
        for (user, role) in ids.iter().zip(["owner", "admin", "member"]) {
            sqlx::query(
                "INSERT INTO users_teams (user_id, team_id, role) VALUES ($1, $2, $3::team_role)",
            )
            .bind(user)
            .bind(team)
            .bind(role)
            .execute(pool)
            .await
            .unwrap();
        }
        let folder = sqlx::query_scalar!(
            "INSERT INTO project_folders (name, owner_team_id) VALUES ('f', $1) RETURNING id",
            team
        )
        .fetch_one(pool)
        .await
        .unwrap();
        let project = sqlx::query_scalar!(
            "INSERT INTO projects (title, owner_team_id) VALUES ('p', $1) RETURNING id",
            team
        )
        .fetch_one(pool)
        .await
        .unwrap();
        let team_person = sqlx::query_scalar!(
            "INSERT INTO persons (last_names, owner_team_id) VALUES ('T', $1) RETURNING id",
            team
        )
        .fetch_one(pool)
        .await
        .unwrap();
        let project_person = sqlx::query_scalar!(
            "INSERT INTO persons (last_names, project_id) VALUES ('P', $1) RETURNING id",
            project
        )
        .fetch_one(pool)
        .await
        .unwrap();
        Fixture {
            team,
            folder,
            project,
            team_person,
            project_person,
            owner: ids[0],
            admin: ids[1],
            member: ids[2],
            outsider: ids[3],
        }
    }

    fn status<T>(result: Result<T, ApiError>) -> Option<&'static str> {
        match result {
            Ok(_) => None,
            Err(e) => Some(match e.error {
                ApiErrorType::ResourceNotFound(_) => "not_found",
                ApiErrorType::Forbidden(_) => "forbidden",
                _ => "other",
            }),
        }
    }

    #[sqlx::test]
    async fn membership_and_minimum_role_for_every_resource(pool: PgPool) -> sqlx::Result<()> {
        let f = fixture(&pool).await;
        let resources = [
            (ResourceType::Team, f.team),
            (ResourceType::Folder, f.folder),
            (ResourceType::Project, f.project),
            (ResourceType::Person, f.team_person),
            // a project-scoped person is owned by its project's team
            (ResourceType::Person, f.project_person),
        ];
        for (kind, id) in resources {
            let check = |user, role| {
                let pool = pool.clone();
                async move { status(has_permission(&pool, kind, id, user, role).await) }
            };
            // any member
            for user in [f.owner, f.admin, f.member] {
                assert_eq!(check(user, None).await, None, "{kind:?}");
            }
            // non-members can't tell the resource exists
            assert_eq!(check(f.outsider, None).await, Some("not_found"), "{kind:?}");
            // minimum roles
            assert_eq!(check(f.member, Some(TeamRole::Member)).await, None);
            assert_eq!(
                check(f.member, Some(TeamRole::Admin)).await,
                Some("forbidden")
            );
            assert_eq!(check(f.admin, Some(TeamRole::Admin)).await, None);
            assert_eq!(
                check(f.admin, Some(TeamRole::Owner)).await,
                Some("forbidden")
            );
            assert_eq!(check(f.owner, Some(TeamRole::Owner)).await, None);
            assert_eq!(
                check(f.outsider, Some(TeamRole::Member)).await,
                Some("not_found")
            );
        }
        Ok(())
    }

    #[sqlx::test]
    async fn unknown_resources_are_not_found(pool: PgPool) -> sqlx::Result<()> {
        let f = fixture(&pool).await;
        for kind in [
            ResourceType::Team,
            ResourceType::Folder,
            ResourceType::Project,
            ResourceType::Person,
        ] {
            let result = has_permission(&pool, kind, Uuid::new_v4(), f.owner, None).await;
            assert_eq!(status(result), Some("not_found"), "{kind:?}");
        }
        Ok(())
    }

    #[sqlx::test]
    async fn returns_the_callers_role(pool: PgPool) -> sqlx::Result<()> {
        let f = fixture(&pool).await;
        let role = |user| has_permission(&pool, ResourceType::Project, f.project, user, None);
        assert_eq!(role(f.owner).await.unwrap(), TeamRole::Owner);
        assert_eq!(role(f.admin).await.unwrap(), TeamRole::Admin);
        assert_eq!(role(f.member).await.unwrap(), TeamRole::Member);
        Ok(())
    }

    #[sqlx::test]
    async fn bulk_person_check_matches_single_checks(pool: PgPool) -> sqlx::Result<()> {
        let f = fixture(&pool).await;
        let both = [f.team_person, f.project_person];
        assert!(
            has_permission_for_persons(&pool, &[], f.outsider)
                .await
                .is_ok()
        );
        assert!(
            has_permission_for_persons(&pool, &both, f.member)
                .await
                .is_ok()
        );
        // duplicates are fine
        assert!(
            has_permission_for_persons(&pool, &[f.team_person, f.team_person], f.member)
                .await
                .is_ok()
        );
        assert_eq!(
            status(has_permission_for_persons(&pool, &both, f.outsider).await),
            Some("not_found")
        );
        assert_eq!(
            status(
                has_permission_for_persons(&pool, &[f.team_person, Uuid::new_v4()], f.owner).await
            ),
            Some("not_found")
        );
        Ok(())
    }

    #[test]
    fn role_order_is_owner_admin_member() {
        assert!(TeamRole::Owner.at_least(TeamRole::Admin));
        assert!(TeamRole::Owner.at_least(TeamRole::Member));
        assert!(TeamRole::Admin.at_least(TeamRole::Admin));
        assert!(!TeamRole::Admin.at_least(TeamRole::Owner));
        assert!(!TeamRole::Member.at_least(TeamRole::Admin));
    }
}

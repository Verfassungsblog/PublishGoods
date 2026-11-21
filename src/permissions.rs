//! Authorization: every resource belongs (directly or through its project) to exactly one
//! team, and a user may act on it if they are a member of that team with a sufficient role.
//!
//! [`has_permission`] is the single check handlers call; it resolves the owning team of the
//! resource and compares the user's [`TeamRole`] against the required one.

use crate::db::repositories::teams;
use crate::db::repositories::users::TeamRole;
use crate::session::errors::LoginError;
use crate::session::session_guard::Session;
use crate::utils::api_helpers::{ApiError, ApiErrorType};
use rocket::http::Status;
use rocket::request::{FromRequest, Outcome};
use rocket::{Request, State};
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
    /// A project template, including its export formats and asset files.
    Template,
}

impl ResourceType {
    fn name(self) -> &'static str {
        match self {
            ResourceType::Team => "team",
            ResourceType::Folder => "folder",
            ResourceType::Project => "project",
            ResourceType::Person => "person",
            ResourceType::Template => "template",
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
        ResourceType::Template => sqlx::query_scalar!(
            "SELECT owner_team_id FROM project_templates WHERE id = $1",
            resource_id
        )
        .fetch_optional(pool)
        .await
        .map_err(crate::db::repositories::DbError::from)?,
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

/// Marks the resource kind an [`Authorized`] guard checks. The id is read from the path segment
/// right after the static segment [`Resource::COLLECTION`] of the matched route (so routes
/// declare it as `<_>`, e.g. `/api/templates/<_>/assets`).
pub trait Resource {
    const TYPE: ResourceType;
    const COLLECTION: &'static str;
}

/// The minimum team role an [`Authorized`] guard requires.
pub trait MinRole {
    const ROLE: Option<TeamRole>;
}

macro_rules! markers {
    ($($name:ident => $t:ident, $coll:literal;)*) => {$(
        pub struct $name;
        impl Resource for $name {
            const TYPE: ResourceType = ResourceType::$t;
            const COLLECTION: &'static str = $coll;
        }
    )*};
}
markers! {
    TeamResource => Team, "teams";
    ProjectResource => Project, "projects";
    PersonResource => Person, "persons";
    TemplateResource => Template, "templates";
}

/// Any team member passes.
pub struct AnyMember;
impl MinRole for AnyMember {
    const ROLE: Option<TeamRole> = None;
}
/// Team admins and owners pass.
pub struct AdminRole;
impl MinRole for AdminRole {
    const ROLE: Option<TeamRole> = Some(TeamRole::Admin);
}
/// Only team owners pass.
pub struct OwnerRole;
impl MinRole for OwnerRole {
    const ROLE: Option<TeamRole> = Some(TeamRole::Owner);
}

/// Request guard running [`has_permission`] for the resource in the route's path: a handler
/// taking it can't run unless the logged-in user may act on the resource, and can't forget the
/// check. Without a session the guard forwards 401 (to the login redirect).
///
/// Failures are an [`ApiError`] (404 for unknown or inaccessible resources, 403 for a too-low
/// role, 400 for a malformed id). Take the guard bare: Rocket then fails the request before
/// reading its body, and the `/api` catcher ([`api_error_catcher`]) answers with the JSON
/// error. (As `Result<Authorized<..>, ApiError>` the guard would "succeed" with the error and
/// Rocket would read, e.g. upload, the body before the handler can reject it.) Outside `/api`
/// Rocket's catchers answer by status.
pub struct Authorized<R, M = AnyMember> {
    /// Id of the resource.
    pub id: Uuid,
    /// The user's role on the team owning the resource.
    pub role: TeamRole,
    _kind: std::marker::PhantomData<(R, M)>,
}

pub type AuthorizedTeam<M = AnyMember> = Authorized<TeamResource, M>;
pub type AuthorizedProject<M = AnyMember> = Authorized<ProjectResource, M>;
pub type AuthorizedPerson<M = AnyMember> = Authorized<PersonResource, M>;
pub type AuthorizedTemplate<M = AnyMember> = Authorized<TemplateResource, M>;

/// The [`ApiError`] an [`Authorized`] guard failed with, kept for [`api_error_catcher`].
struct GuardFailure(ApiError);

/// Responds to every failed request under `/api` with a JSON [`ApiError`]: the one an
/// [`Authorized`] guard failed with, else one derived from the status. Registered at `/api`.
#[catch(default)]
pub fn api_error_catcher(status: Status, request: &Request<'_>) -> (Status, ApiError) {
    if let Some(GuardFailure(error)) = request.local_cache(|| Option::<GuardFailure>::None) {
        return (status, error.clone());
    }
    let error = match status {
        Status::Unauthorized => ApiErrorType::Unauthorized,
        Status::Forbidden => ApiErrorType::Forbidden(String::new()),
        Status::BadRequest | Status::UnprocessableEntity => {
            ApiErrorType::BadRequest(status.reason_lossy().to_string())
        }
        Status::NotFound => ApiErrorType::ResourceNotFound("resource".to_string()),
        Status::Conflict => ApiErrorType::Conflict(String::new()),
        s if s.class().is_server_error() => ApiErrorType::InternalServerError,
        s => ApiErrorType::Other(s.reason_lossy().to_string()),
    };
    (status, error.into())
}

/// The raw path segment of the resource id: the one after `collection` in the matched route.
fn id_segment<'r>(request: &'r Request<'_>, collection: &str) -> Option<&'r str> {
    // Both the route's and the request's path include the mount base, so positions line up.
    let route = request.route()?;
    let mut route_segments = route.uri.path().split('/').filter(|s| !s.is_empty());
    let index = route_segments.position(|s| s == collection)? + 1;
    route_segments.next()?.starts_with('<').then_some(())?;
    request.uri().path().segments().nth(index)
}

#[rocket::async_trait]
impl<'r, R: Resource + Send + Sync, M: MinRole + Send + Sync> FromRequest<'r> for Authorized<R, M> {
    type Error = ApiError;

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, ApiError> {
        let fail = |e: ApiError| {
            let status = e.status();
            request.local_cache(|| Some(GuardFailure(e.clone())));
            Outcome::Error((status, e))
        };

        let session = match request.guard::<Session>().await {
            Outcome::Success(session) => session,
            // The session store itself is broken: that's our fault, not a missing login.
            Outcome::Error((_, LoginError::Unavailable)) => {
                return fail(ApiErrorType::InternalServerError.into());
            }
            _ => return Outcome::Forward(Status::Unauthorized),
        };
        let pool = match request.guard::<&State<PgPool>>().await {
            Outcome::Success(pool) => pool,
            _ => return fail(ApiErrorType::InternalServerError.into()),
        };

        let Some(raw_id) = id_segment(request, R::COLLECTION) else {
            error!(
                "No <{}> id segment in the route of an Authorized guard",
                R::COLLECTION
            );
            return fail(ApiErrorType::InternalServerError.into());
        };
        let id = match Uuid::parse_str(raw_id) {
            Ok(id) => id,
            Err(e) => return fail(e.into()),
        };
        match has_permission(pool, R::TYPE, id, session.user_id, M::ROLE).await {
            Ok(role) => Outcome::Success(Authorized {
                id,
                role,
                _kind: std::marker::PhantomData,
            }),
            Err(e) => fail(e),
        }
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
        template: Uuid,
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
        let template = sqlx::query_scalar!(
            "INSERT INTO project_templates (name, owner_team_id) VALUES ('t', $1) RETURNING id",
            team
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
            template,
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
            (ResourceType::Template, f.template),
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
            ResourceType::Template,
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

use crate::db::repositories::DbError;
use crate::db::repositories::teams as teams_repo;
use crate::db::repositories::teams::{Invitation, InvitationWithTeam, Team};
use crate::db::repositories::users::{self, TeamRole};
use crate::session::session_guard::Session;
use crate::utils::api_helpers::{APIResponse, APIResult, ApiError, ApiErrorType};
use rocket::State;
use rocket::serde::json::Json;
use sqlx::PgPool;
use std::collections::HashSet;
use uuid::Uuid;

/// Looks up `user_id`'s role on `team_id`, treating "not a member" as
/// [`DbError::NotFound`] (mapped to a 404) so non-members can't distinguish a
/// nonexistent team from one they're simply not part of.
async fn member_role_or_not_found(
    pool: &PgPool,
    team_id: Uuid,
    user_id: Uuid,
) -> Result<TeamRole, ApiError> {
    teams_repo::member_role(pool, team_id, user_id)
        .await?
        .ok_or_else(|| ApiErrorType::ResourceNotFound("team".to_string()).into())
}

/// Fails with a 403 unless `role` is one of `allowed`.
fn require_role(role: TeamRole, allowed: &[TeamRole]) -> Result<(), ApiError> {
    if allowed.contains(&role) {
        Ok(())
    } else {
        Err(ApiErrorType::Forbidden("insufficient team role".to_string()).into())
    }
}

/// Fails with a 403 unless `invitation` is addressed to `user_id`/`user_email` — either
/// directly by user id, or by a case-insensitively matching email. Shared by the
/// self-service accept/decline routes, which (unlike the team-management routes) don't
/// require the requester to already be a team member.
///
/// Takes the email as a plain argument (fetched fresh from the DB by callers) rather than
/// pulling it off `Session`: `Session.user_email` is cached at login and never refreshed,
/// so it goes stale the moment a user changes their email via the Account tab.
fn require_addressed_to(
    invitation: &Invitation,
    user_id: Uuid,
    user_email: &str,
) -> Result<(), ApiError> {
    let addressed_to_requester = invitation.user_id == Some(user_id)
        || invitation
            .email
            .as_deref()
            .is_some_and(|email| email.eq_ignore_ascii_case(user_email));

    if addressed_to_requester {
        Ok(())
    } else {
        Err(ApiErrorType::Forbidden("this invitation is not addressed to you".to_string()).into())
    }
}

/// Fails with a 403 if `team_id` is the shared "Default" team that every user is
/// automatically an `owner` of (see [`users::ensure_default_team`]). That blanket
/// ownership exists only to preserve today's single-tenant "everyone sees everything"
/// behavior — it was never meant to make the Default team manageable like an ordinary
/// team, since any of its many "owners" could otherwise delete it (cascading away every
/// project/folder it still owns) or wipe out everyone else's membership on it.
async fn require_not_default_team(pool: &PgPool, team_id: Uuid) -> Result<(), ApiError> {
    let default_team_id = users::ensure_default_team(pool).await?;
    if team_id == default_team_id {
        Err(
            ApiErrorType::Forbidden("the Default team can't be managed this way".to_string())
                .into(),
        )
    } else {
        Ok(())
    }
}

/// Fails with a 409 if `user_id` is already a member of `team_id`. Guards invitation
/// creation: without this, inviting an existing member (including inviting yourself) lets
/// an owner/admin silently change that member's role on accept — `accept_invitation`
/// upserts the role — bypassing [`set_member_role`](teams_repo::set_member_role)'s
/// dedicated permission path.
async fn require_not_already_member(
    pool: &PgPool,
    team_id: Uuid,
    user_id: Uuid,
) -> Result<(), ApiError> {
    if teams_repo::member_role(pool, team_id, user_id)
        .await?
        .is_some()
    {
        Err(ApiErrorType::Conflict("this user is already a member of the team".to_string()).into())
    } else {
        Ok(())
    }
}

/// GET /api/teams
///
/// Lists every team the requesting user is a member of.
#[get("/api/teams")]
pub async fn list_teams(session: Session, pool: &State<PgPool>) -> APIResult<Vec<Team>> {
    let teams = teams_repo::list_for_user(pool.inner(), session.user_id).await?;
    Ok(APIResponse::from(teams))
}

/// Request body for [`create_team`].
#[derive(Debug, serde::Deserialize)]
pub struct CreateTeamData {
    pub name: String,
}

/// POST /api/teams
///
/// Creates a new team, with the requesting user attached as its `owner`. Unlike the other
/// team routes, this one has no permission requirement — any authenticated user may create
/// a team.
#[post("/api/teams", data = "<data>")]
pub async fn create_team(
    data: Json<CreateTeamData>,
    session: Session,
    pool: &State<PgPool>,
) -> APIResult<Team> {
    let data = data.into_inner();
    let team = teams_repo::create(pool.inner(), &data.name, session.user_id).await?;
    Ok(APIResponse::from(team))
}

/// GET /api/teams/<team_id>
///
/// Fetches a single team, including its members. Only accessible to members of the team;
/// returns 404 for anyone else, matching [`list_teams`] only ever surfacing teams the
/// requesting user belongs to.
#[get("/api/teams/<team_id>")]
pub async fn get_team(team_id: &str, session: Session, pool: &State<PgPool>) -> APIResult<Team> {
    let team_id = Uuid::parse_str(team_id)?;
    let pool = pool.inner();

    member_role_or_not_found(pool, team_id, session.user_id).await?;

    let team = teams_repo::get(pool, team_id).await?;
    Ok(APIResponse::from(team))
}

/// A single member entry in [`PatchTeamData::members`].
#[derive(Debug, serde::Deserialize)]
pub struct PatchTeamMember {
    pub user_id: Uuid,
    pub role: TeamRole,
}

/// Request body for [`patch_team`]: any field left as `None` is left untouched. When
/// `members` is present, it replaces the team's entire membership list — existing members
/// not listed are removed, and every listed `(user_id, role)` pair is created or updated
/// with that role.
#[derive(Debug, serde::Deserialize)]
pub struct PatchTeamData {
    pub name: Option<String>,
    pub members: Option<Vec<PatchTeamMember>>,
}

/// PATCH /api/teams/<team_id>
///
/// Applies a partial update to a team's name and/or membership list, and returns the
/// resulting team so callers don't need a follow-up GET. Both writes run inside a single
/// transaction, so a failure partway through rolls back the whole patch. Requires the
/// requesting user to be an `owner` or `admin` of the team.
///
/// An `admin` may reassign non-owner roles freely, but may not change *who* the team's
/// owner(s) are (add themselves or anyone else as owner, or drop an existing owner) —
/// only an `owner` may touch the owner set. Whoever submits the patch, the resulting
/// membership must still include at least one owner; [`teams_repo::replace_members`]
/// enforces that as the final backstop.
#[patch("/api/teams/<team_id>", data = "<patch>")]
pub async fn patch_team(
    team_id: &str,
    patch: Json<PatchTeamData>,
    session: Session,
    pool: &State<PgPool>,
) -> APIResult<Team> {
    let team_id = Uuid::parse_str(team_id)?;
    let patch = patch.into_inner();
    let pool = pool.inner();

    let role = member_role_or_not_found(pool, team_id, session.user_id).await?;
    require_role(role, &[TeamRole::Owner, TeamRole::Admin])?;
    require_not_default_team(pool, team_id).await?;

    let mut tx = pool.begin().await.map_err(DbError::from)?;

    if let Some(name) = patch.name {
        teams_repo::rename(&mut *tx, team_id, &name).await?;
    }

    if let Some(members) = patch.members {
        if role != TeamRole::Owner {
            let current_members = teams_repo::list_members(&mut *tx, team_id).await?;
            let current_owner_ids: HashSet<Uuid> = current_members
                .iter()
                .filter(|m| m.role == TeamRole::Owner)
                .map(|m| m.user.id)
                .collect();
            let new_owner_ids: HashSet<Uuid> = members
                .iter()
                .filter(|m| m.role == TeamRole::Owner)
                .map(|m| m.user_id)
                .collect();
            if new_owner_ids != current_owner_ids {
                return Err(ApiErrorType::Forbidden(
                    "only an owner may change who owns this team".to_string(),
                )
                .into());
            }
        }

        let members: Vec<(Uuid, TeamRole)> =
            members.into_iter().map(|m| (m.user_id, m.role)).collect();
        teams_repo::replace_members(&mut tx, team_id, &members).await?;
    }

    tx.commit().await.map_err(DbError::from)?;

    let team = teams_repo::get(pool, team_id).await?;
    Ok(APIResponse::from(team))
}

/// POST /api/teams/<team_id>/leave
///
/// Removes the requesting user from a team. Self-service: unlike [`patch_team`]'s
/// membership replacement (which requires the requester to already be an `owner` or
/// `admin`), any member may leave a team they belong to — that's the whole point, since a
/// plain `member` has no other way to remove themselves.
///
/// Fails with 409 if the requester is the team's sole owner: leaving would make the team
/// ownerless, and thus permanently undeletable/unmanageable (mirroring the same guard
/// [`teams_repo::replace_members`] enforces on the patch path). They must hand ownership to
/// someone else first, or delete the team instead.
#[post("/api/teams/<team_id>/leave")]
pub async fn leave_team(team_id: &str, session: Session, pool: &State<PgPool>) -> APIResult<()> {
    let team_id = Uuid::parse_str(team_id)?;
    let pool = pool.inner();

    let role = member_role_or_not_found(pool, team_id, session.user_id).await?;
    require_not_default_team(pool, team_id).await?;

    let mut tx = pool.begin().await.map_err(DbError::from)?;

    if role == TeamRole::Owner {
        let members = teams_repo::list_members(&mut *tx, team_id).await?;
        let owner_count = members.iter().filter(|m| m.role == TeamRole::Owner).count();
        if owner_count <= 1 {
            return Err(ApiErrorType::Conflict(
                "you are the last owner of this team; transfer ownership to someone else or delete the team instead"
                    .to_string(),
            )
            .into());
        }
    }

    teams_repo::remove_member(&mut *tx, team_id, session.user_id).await?;
    tx.commit().await.map_err(DbError::from)?;

    Ok(APIResponse::from(()))
}

/// DELETE /api/teams/<team_id>
///
/// Deletes a team. Requires the requesting user to be an `owner` of the team.
#[delete("/api/teams/<team_id>")]
pub async fn delete_team(team_id: &str, session: Session, pool: &State<PgPool>) -> APIResult<()> {
    let team_id = Uuid::parse_str(team_id)?;
    let pool = pool.inner();

    let role = member_role_or_not_found(pool, team_id, session.user_id).await?;
    require_role(role, &[TeamRole::Owner])?;
    require_not_default_team(pool, team_id).await?;

    teams_repo::delete(pool, team_id).await?;
    Ok(APIResponse::from(()))
}

/// Request body for [`create_invitation`]. Exactly one of `user_id`/`email` must be set:
/// `user_id` invites an existing user directly, `email` invites a not-yet-registered
/// person by address.
#[derive(Debug, serde::Deserialize)]
pub struct CreateInvitationData {
    pub role: TeamRole,
    pub user_id: Option<Uuid>,
    pub email: Option<String>,
}

/// POST /api/teams/<team_id>/invitations
///
/// Invites a user to a team, by id (if they're already registered) or by email. Requires
/// the requesting user to be an `owner` or `admin` of the team.
#[post("/api/teams/<team_id>/invitations", data = "<data>")]
pub async fn create_invitation(
    team_id: &str,
    data: Json<CreateInvitationData>,
    session: Session,
    pool: &State<PgPool>,
) -> APIResult<Invitation> {
    let team_id = Uuid::parse_str(team_id)?;
    let data = data.into_inner();
    let pool = pool.inner();

    let role = member_role_or_not_found(pool, team_id, session.user_id).await?;
    require_role(role, &[TeamRole::Owner, TeamRole::Admin])?;

    let invitation = match (data.user_id, data.email) {
        (Some(user_id), None) => {
            require_not_already_member(pool, team_id, user_id).await?;
            teams_repo::create_invitation_for_user(pool, team_id, data.role, user_id).await?
        }
        (None, Some(email)) => {
            if teams_repo::member_role_by_email(pool, team_id, &email)
                .await?
                .is_some()
            {
                return Err(ApiErrorType::Conflict(
                    "this email belongs to a user who is already a member of the team".to_string(),
                )
                .into());
            }
            teams_repo::create_invitation_for_email(pool, team_id, data.role, &email).await?
        }
        (Some(_), Some(_)) => {
            return Err(ApiErrorType::BadRequest(
                "Provide either user_id or email, not both".to_string(),
            )
            .into());
        }
        (None, None) => {
            return Err(
                ApiErrorType::BadRequest("Provide either user_id or email".to_string()).into(),
            );
        }
    };

    Ok(APIResponse::from(invitation))
}

/// GET /api/teams/<team_id>/invitations
///
/// Lists every pending invitation for a team. Requires the requesting user to be an
/// `owner` or `admin` of the team.
#[get("/api/teams/<team_id>/invitations")]
pub async fn list_invitations(
    team_id: &str,
    session: Session,
    pool: &State<PgPool>,
) -> APIResult<Vec<Invitation>> {
    let team_id = Uuid::parse_str(team_id)?;
    let pool = pool.inner();

    let role = member_role_or_not_found(pool, team_id, session.user_id).await?;
    require_role(role, &[TeamRole::Owner, TeamRole::Admin])?;

    let invitations = teams_repo::list_invitations_for_team(pool, team_id).await?;
    Ok(APIResponse::from(invitations))
}

/// DELETE /api/teams/<team_id>/invitations/<invitation_id>
///
/// Revokes a pending invitation. Requires the requesting user to be an `owner` or `admin`
/// of the team; returns 404 if the invitation doesn't belong to `team_id`.
#[delete("/api/teams/<team_id>/invitations/<invitation_id>")]
pub async fn decline_invitation(
    team_id: &str,
    invitation_id: &str,
    session: Session,
    pool: &State<PgPool>,
) -> APIResult<()> {
    let team_id = Uuid::parse_str(team_id)?;
    let invitation_id = Uuid::parse_str(invitation_id)?;
    let pool = pool.inner();

    let role = member_role_or_not_found(pool, team_id, session.user_id).await?;
    require_role(role, &[TeamRole::Owner, TeamRole::Admin])?;

    let invitation = teams_repo::get_invitation(pool, invitation_id).await?;
    if invitation.team_id != team_id {
        return Err(ApiErrorType::ResourceNotFound("invitation".to_string()).into());
    }

    teams_repo::delete_invitation(pool, invitation_id).await?;
    Ok(APIResponse::from(()))
}

/// POST /api/invitations/<invitation_id>/accept
///
/// Accepts an invitation, adding the requesting user to the invitation's team with the
/// invited role. Self-service: no team role is required (the requester typically isn't a
/// member yet), but the invitation must be addressed to them — either directly by user id,
/// or by an email matching their account's email.
#[post("/api/invitations/<invitation_id>/accept")]
pub async fn accept_invitation(
    invitation_id: &str,
    session: Session,
    pool: &State<PgPool>,
) -> APIResult<()> {
    let invitation_id = Uuid::parse_str(invitation_id)?;
    let pool = pool.inner();

    let invitation = teams_repo::get_invitation(pool, invitation_id).await?;
    let requester = users::get(pool, session.user_id).await?;
    require_addressed_to(&invitation, session.user_id, &requester.email)?;

    teams_repo::accept_invitation(pool, invitation_id, session.user_id).await?;
    Ok(APIResponse::from(()))
}

/// GET /api/invitations
///
/// Lists every pending invitation addressed to the requesting user, across all teams,
/// including each invited team's name. Self-service: unlike [`list_invitations`] (which
/// lists a team's outgoing invitations for its admins), this needs no team role — the
/// requester typically isn't a member of the inviting team yet.
///
/// Looks the requester's email up fresh (rather than using `session.user_email`, which is
/// cached at login and never refreshed) so an invitation addressed to a newly-changed email
/// shows up immediately instead of only after the next login.
#[get("/api/invitations")]
pub async fn list_my_invitations(
    session: Session,
    pool: &State<PgPool>,
) -> APIResult<Vec<InvitationWithTeam>> {
    let pool = pool.inner();
    let requester = users::get(pool, session.user_id).await?;
    let invitations = teams_repo::list_pending_invitations_with_team_name(
        pool,
        session.user_id,
        &requester.email,
    )
    .await?;
    Ok(APIResponse::from(invitations))
}

/// POST /api/invitations/<invitation_id>/decline
///
/// Declines an invitation addressed to the requesting user, without joining the team.
/// Self-service, mirroring [`accept_invitation`]'s addressing check — as opposed to
/// [`decline_invitation`], which lets a team's admins revoke an invitation they sent.
#[post("/api/invitations/<invitation_id>/decline")]
pub async fn decline_my_invitation(
    invitation_id: &str,
    session: Session,
    pool: &State<PgPool>,
) -> APIResult<()> {
    let invitation_id = Uuid::parse_str(invitation_id)?;
    let pool = pool.inner();

    let invitation = teams_repo::get_invitation(pool, invitation_id).await?;
    let requester = users::get(pool, session.user_id).await?;
    require_addressed_to(&invitation, session.user_id, &requester.email)?;

    teams_repo::delete_invitation(pool, invitation_id).await?;
    Ok(APIResponse::from(()))
}

/// Runtime HTTP coverage of the membership-scoping and role checks above (`Session` guard +
/// `State<PgPool>` wiring + `member_role_or_not_found`/`require_role`), mirroring
/// `projects::api::get`'s integration tests.
#[cfg(test)]
mod integration_tests {
    use super::*;
    use crate::db::repositories::users;
    use crate::session::session_storage::SessionStorage;
    use crate::settings::{ExportServer, Settings};
    use argon2::Argon2;
    use argon2::password_hash::{PasswordHasher, SaltString, rand_core::OsRng};
    use rocket::http::{ContentType, Status};
    use rocket::local::asynchronous::Client;
    use rocket_dyn_templates::Template;

    fn dummy_settings() -> Settings {
        Settings {
            app_title: "test".to_string(),
            instance_url: "".to_string(),
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
            smtp_connection_url: "".to_string(),
            mail_from_address: "".to_string(),
            smtp_pool_min_idle: 0,
            smtp_pool_max_size: 0,
            smtp_pool_idle_timeout: 0,
            mail_max_retries: 0,
            mail_base_retry_delay_seconds: 0,
        }
    }

    async fn test_client(pool: PgPool) -> Client {
        // Force the debug profile so Rocket doesn't demand a configured secret_key outside
        // the debug profile (e.g. when tests are built with `cargo test --release`).
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
                    list_teams,
                    get_team,
                    create_team,
                    patch_team,
                    leave_team,
                    delete_team,
                    create_invitation,
                    list_invitations,
                    decline_invitation,
                    accept_invitation,
                    list_my_invitations,
                    decline_my_invitation,
                ],
            );
        Client::tracked(rocket).await.unwrap()
    }

    /// Creates a user with a known password and returns their id.
    async fn seed_user(pool: &PgPool, email: &str, name: &str, default_team: Uuid) -> Uuid {
        let salt = SaltString::generate(&mut OsRng);
        let hash = Argon2::default()
            .hash_password(b"correct horse", &salt)
            .unwrap()
            .to_string();
        users::insert(pool, email, name, &hash, default_team)
            .await
            .unwrap()
            .id
    }

    /// Logs `email` in on `client`, mirroring how a real browser session would carry the
    /// private session cookie across subsequent requests.
    async fn login(client: &Client, email: &str) {
        let response = client
            .post("/login")
            .header(ContentType::Form)
            .body(format!(
                "email={}&password=correct+horse",
                email.replace('@', "%40")
            ))
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::SeeOther);
    }

    #[sqlx::test]
    async fn list_teams_only_returns_membership_over_http(pool: PgPool) -> sqlx::Result<()> {
        let default_team = users::ensure_default_team(&pool).await.unwrap();
        let alice = seed_user(&pool, "alice@example.com", "Alice", default_team).await;
        let bob = seed_user(&pool, "bob@example.com", "Bob", default_team).await;

        let team_a = teams_repo::create(&pool, "Team A", alice).await.unwrap();
        teams_repo::create(&pool, "Team B", bob).await.unwrap();

        let client = test_client(pool.clone()).await;
        login(&client, "alice@example.com").await;

        let response = client.get("/api/teams").dispatch().await;
        assert_eq!(response.status(), Status::Ok);
        let body: serde_json::Value =
            serde_json::from_str(&response.into_string().await.unwrap()).unwrap();
        let names: Vec<String> = body["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect();
        assert!(names.contains(&"Team A".to_string()));
        assert!(!names.contains(&"Team B".to_string()));
        let _ = team_a;
        Ok(())
    }

    #[sqlx::test]
    async fn get_team_returns_not_found_for_non_member(pool: PgPool) -> sqlx::Result<()> {
        let default_team = users::ensure_default_team(&pool).await.unwrap();
        let alice = seed_user(&pool, "alice2@example.com", "Alice", default_team).await;
        let bob = seed_user(&pool, "bob2@example.com", "Bob", default_team).await;
        let _ = bob;

        let team = teams_repo::create(&pool, "Owners Only", alice)
            .await
            .unwrap();

        let client = test_client(pool.clone()).await;
        login(&client, "bob2@example.com").await;

        let response = client
            .get(format!("/api/teams/{}", team.id))
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::NotFound);
        Ok(())
    }

    #[sqlx::test]
    async fn patch_team_forbidden_for_plain_member(pool: PgPool) -> sqlx::Result<()> {
        let default_team = users::ensure_default_team(&pool).await.unwrap();
        let alice = seed_user(&pool, "alice3@example.com", "Alice", default_team).await;
        let carol = seed_user(&pool, "carol3@example.com", "Carol", default_team).await;

        let team = teams_repo::create(&pool, "Team", alice).await.unwrap();
        teams_repo::add_member(&pool, team.id, carol, TeamRole::Member)
            .await
            .unwrap();

        let client = test_client(pool.clone()).await;
        login(&client, "carol3@example.com").await;

        let response = client
            .patch(format!("/api/teams/{}", team.id))
            .header(ContentType::JSON)
            .body(r#"{"name":"Hacked"}"#)
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::Forbidden);
        Ok(())
    }

    #[sqlx::test]
    async fn patch_team_allows_admin_to_rename_and_replace_members(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        let default_team = users::ensure_default_team(&pool).await.unwrap();
        let alice = seed_user(&pool, "alice4@example.com", "Alice", default_team).await;
        let dave = seed_user(&pool, "dave4@example.com", "Dave", default_team).await;
        let erin = seed_user(&pool, "erin4@example.com", "Erin", default_team).await;

        let team = teams_repo::create(&pool, "Old Name", alice).await.unwrap();
        teams_repo::add_member(&pool, team.id, dave, TeamRole::Admin)
            .await
            .unwrap();

        let client = test_client(pool.clone()).await;
        login(&client, "dave4@example.com").await;

        let patch_body = serde_json::json!({
            "name": "New Name",
            "members": [
                {"user_id": alice, "role": "owner"},
                {"user_id": erin, "role": "member"}
            ]
        });
        let response = client
            .patch(format!("/api/teams/{}", team.id))
            .header(ContentType::JSON)
            .body(patch_body.to_string())
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::Ok);
        let body: serde_json::Value =
            serde_json::from_str(&response.into_string().await.unwrap()).unwrap();
        assert_eq!(body["data"]["name"], "New Name");
        let member_ids: Vec<String> = body["data"]["members"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["user"]["id"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(member_ids.len(), 2);
        assert!(member_ids.contains(&alice.to_string()));
        assert!(member_ids.contains(&erin.to_string()));
        // Dave (the patching admin) removed himself via the replacement list.
        assert!(!member_ids.contains(&dave.to_string()));
        Ok(())
    }

    #[sqlx::test]
    async fn leave_team_lets_plain_member_remove_themselves(pool: PgPool) -> sqlx::Result<()> {
        let default_team = users::ensure_default_team(&pool).await.unwrap();
        let alice = seed_user(&pool, "alice30@example.com", "Alice", default_team).await;
        let carol = seed_user(&pool, "carol30@example.com", "Carol", default_team).await;

        let team = teams_repo::create(&pool, "Team", alice).await.unwrap();
        teams_repo::add_member(&pool, team.id, carol, TeamRole::Member)
            .await
            .unwrap();

        let client = test_client(pool.clone()).await;
        login(&client, "carol30@example.com").await;

        let response = client
            .post(format!("/api/teams/{}/leave", team.id))
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::Ok);
        assert_eq!(
            teams_repo::member_role(&pool, team.id, carol)
                .await
                .unwrap(),
            None
        );
        Ok(())
    }

    #[sqlx::test]
    async fn leave_team_allows_one_of_two_owners_to_leave(pool: PgPool) -> sqlx::Result<()> {
        let default_team = users::ensure_default_team(&pool).await.unwrap();
        let alice = seed_user(&pool, "alice31@example.com", "Alice", default_team).await;
        let dave = seed_user(&pool, "dave31@example.com", "Dave", default_team).await;

        let team = teams_repo::create(&pool, "Team", alice).await.unwrap();
        teams_repo::add_member(&pool, team.id, dave, TeamRole::Owner)
            .await
            .unwrap();

        let client = test_client(pool.clone()).await;
        login(&client, "alice31@example.com").await;

        let response = client
            .post(format!("/api/teams/{}/leave", team.id))
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::Ok);
        assert_eq!(
            teams_repo::member_role(&pool, team.id, alice)
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            teams_repo::member_role(&pool, team.id, dave).await.unwrap(),
            Some(TeamRole::Owner)
        );
        Ok(())
    }

    #[sqlx::test]
    async fn leave_team_rejects_sole_owner(pool: PgPool) -> sqlx::Result<()> {
        let default_team = users::ensure_default_team(&pool).await.unwrap();
        let alice = seed_user(&pool, "alice32@example.com", "Alice", default_team).await;

        let team = teams_repo::create(&pool, "Solo Team", alice).await.unwrap();

        let client = test_client(pool.clone()).await;
        login(&client, "alice32@example.com").await;

        let response = client
            .post(format!("/api/teams/{}/leave", team.id))
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::Conflict);
        assert_eq!(
            teams_repo::member_role(&pool, team.id, alice)
                .await
                .unwrap(),
            Some(TeamRole::Owner)
        );
        Ok(())
    }

    #[sqlx::test]
    async fn leave_team_returns_not_found_for_non_member(pool: PgPool) -> sqlx::Result<()> {
        let default_team = users::ensure_default_team(&pool).await.unwrap();
        let alice = seed_user(&pool, "alice33@example.com", "Alice", default_team).await;
        let bob = seed_user(&pool, "bob33@example.com", "Bob", default_team).await;
        let _ = bob;

        let team = teams_repo::create(&pool, "Team", alice).await.unwrap();

        let client = test_client(pool.clone()).await;
        login(&client, "bob33@example.com").await;

        let response = client
            .post(format!("/api/teams/{}/leave", team.id))
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::NotFound);
        Ok(())
    }

    #[sqlx::test]
    async fn leave_team_rejects_leaving_default_team(pool: PgPool) -> sqlx::Result<()> {
        let default_team = users::ensure_default_team(&pool).await.unwrap();
        let alice = seed_user(&pool, "alice34@example.com", "Alice", default_team).await;

        let client = test_client(pool.clone()).await;
        login(&client, "alice34@example.com").await;

        let response = client
            .post(format!("/api/teams/{}/leave", default_team))
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::Forbidden);
        assert_eq!(
            teams_repo::member_role(&pool, default_team, alice)
                .await
                .unwrap(),
            Some(TeamRole::Owner)
        );
        Ok(())
    }

    #[sqlx::test]
    async fn delete_team_requires_owner_not_admin(pool: PgPool) -> sqlx::Result<()> {
        let default_team = users::ensure_default_team(&pool).await.unwrap();
        let alice = seed_user(&pool, "alice5@example.com", "Alice", default_team).await;
        let dave = seed_user(&pool, "dave5@example.com", "Dave", default_team).await;

        let team = teams_repo::create(&pool, "Team", alice).await.unwrap();
        teams_repo::add_member(&pool, team.id, dave, TeamRole::Admin)
            .await
            .unwrap();

        let client = test_client(pool.clone()).await;

        login(&client, "dave5@example.com").await;
        let forbidden = client
            .delete(format!("/api/teams/{}", team.id))
            .dispatch()
            .await;
        assert_eq!(forbidden.status(), Status::Forbidden);

        login(&client, "alice5@example.com").await;
        let ok = client
            .delete(format!("/api/teams/{}", team.id))
            .dispatch()
            .await;
        assert_eq!(ok.status(), Status::Ok);

        assert!(matches!(
            teams_repo::get(&pool, team.id).await,
            Err(DbError::NotFound("team"))
        ));
        Ok(())
    }

    #[sqlx::test]
    async fn create_team_requires_no_permission_and_makes_creator_owner(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        let default_team = users::ensure_default_team(&pool).await.unwrap();
        seed_user(&pool, "frank6@example.com", "Frank", default_team).await;

        let client = test_client(pool.clone()).await;
        login(&client, "frank6@example.com").await;

        let response = client
            .post("/api/teams")
            .header(ContentType::JSON)
            .body(r#"{"name":"Frank's Team"}"#)
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::Ok);
        let body: serde_json::Value =
            serde_json::from_str(&response.into_string().await.unwrap()).unwrap();
        assert_eq!(body["data"]["name"], "Frank's Team");
        let members = body["data"]["members"].as_array().unwrap();
        assert_eq!(members.len(), 1);
        assert_eq!(members[0]["role"], "owner");
        Ok(())
    }

    #[sqlx::test]
    async fn create_invitation_forbidden_for_plain_member(pool: PgPool) -> sqlx::Result<()> {
        let default_team = users::ensure_default_team(&pool).await.unwrap();
        let alice = seed_user(&pool, "alice7@example.com", "Alice", default_team).await;
        let carol = seed_user(&pool, "carol7@example.com", "Carol", default_team).await;

        let team = teams_repo::create(&pool, "Team", alice).await.unwrap();
        teams_repo::add_member(&pool, team.id, carol, TeamRole::Member)
            .await
            .unwrap();

        let client = test_client(pool.clone()).await;
        login(&client, "carol7@example.com").await;

        let response = client
            .post(format!("/api/teams/{}/invitations", team.id))
            .header(ContentType::JSON)
            .body(r#"{"role":"member","email":"new@example.com"}"#)
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::Forbidden);
        Ok(())
    }

    #[sqlx::test]
    async fn create_invitation_rejects_both_user_id_and_email(pool: PgPool) -> sqlx::Result<()> {
        let default_team = users::ensure_default_team(&pool).await.unwrap();
        let alice = seed_user(&pool, "alice8@example.com", "Alice", default_team).await;
        let team = teams_repo::create(&pool, "Team", alice).await.unwrap();

        let client = test_client(pool.clone()).await;
        login(&client, "alice8@example.com").await;

        let response = client
            .post(format!("/api/teams/{}/invitations", team.id))
            .header(ContentType::JSON)
            .body(format!(
                r#"{{"role":"member","user_id":"{}","email":"new@example.com"}}"#,
                alice
            ))
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::BadRequest);
        Ok(())
    }

    #[sqlx::test]
    async fn create_invitation_rejects_neither_user_id_nor_email(pool: PgPool) -> sqlx::Result<()> {
        let default_team = users::ensure_default_team(&pool).await.unwrap();
        let alice = seed_user(&pool, "alice9@example.com", "Alice", default_team).await;
        let team = teams_repo::create(&pool, "Team", alice).await.unwrap();

        let client = test_client(pool.clone()).await;
        login(&client, "alice9@example.com").await;

        let response = client
            .post(format!("/api/teams/{}/invitations", team.id))
            .header(ContentType::JSON)
            .body(r#"{"role":"member"}"#)
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::BadRequest);
        Ok(())
    }

    #[sqlx::test]
    async fn create_invitation_by_email_succeeds_for_admin(pool: PgPool) -> sqlx::Result<()> {
        let default_team = users::ensure_default_team(&pool).await.unwrap();
        let alice = seed_user(&pool, "alice10@example.com", "Alice", default_team).await;
        let dave = seed_user(&pool, "dave10@example.com", "Dave", default_team).await;

        let team = teams_repo::create(&pool, "Team", alice).await.unwrap();
        teams_repo::add_member(&pool, team.id, dave, TeamRole::Admin)
            .await
            .unwrap();

        let client = test_client(pool.clone()).await;
        login(&client, "dave10@example.com").await;

        let response = client
            .post(format!("/api/teams/{}/invitations", team.id))
            .header(ContentType::JSON)
            .body(r#"{"role":"admin","email":"invitee@example.com"}"#)
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::Ok);
        let body: serde_json::Value =
            serde_json::from_str(&response.into_string().await.unwrap()).unwrap();
        assert_eq!(body["data"]["email"], "invitee@example.com");
        assert!(body["data"]["user_id"].is_null());
        assert_eq!(body["data"]["role"], "admin");

        let pending = teams_repo::list_invitations_for_team(&pool, team.id)
            .await
            .unwrap();
        assert_eq!(pending.len(), 1);
        Ok(())
    }

    #[sqlx::test]
    async fn create_invitation_by_user_id_succeeds_for_owner(pool: PgPool) -> sqlx::Result<()> {
        let default_team = users::ensure_default_team(&pool).await.unwrap();
        let alice = seed_user(&pool, "alice11@example.com", "Alice", default_team).await;
        let erin = seed_user(&pool, "erin11@example.com", "Erin", default_team).await;

        let team = teams_repo::create(&pool, "Team", alice).await.unwrap();

        let client = test_client(pool.clone()).await;
        login(&client, "alice11@example.com").await;

        let response = client
            .post(format!("/api/teams/{}/invitations", team.id))
            .header(ContentType::JSON)
            .body(format!(r#"{{"role":"member","user_id":"{}"}}"#, erin))
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::Ok);
        let body: serde_json::Value =
            serde_json::from_str(&response.into_string().await.unwrap()).unwrap();
        assert_eq!(body["data"]["user_id"], erin.to_string());
        assert!(body["data"]["email"].is_null());
        Ok(())
    }

    /// The bug this guards against: an owner could invite themself (or any existing
    /// member) to their own team at a different role, and accepting that invitation would
    /// silently overwrite their real role — bypassing `set_member_role`'s dedicated
    /// permission path entirely.
    #[sqlx::test]
    async fn create_invitation_rejects_self_invite_by_user_id(pool: PgPool) -> sqlx::Result<()> {
        let default_team = users::ensure_default_team(&pool).await.unwrap();
        let alice = seed_user(&pool, "alice15@example.com", "Alice", default_team).await;

        let team = teams_repo::create(&pool, "Team", alice).await.unwrap();

        let client = test_client(pool.clone()).await;
        login(&client, "alice15@example.com").await;

        let response = client
            .post(format!("/api/teams/{}/invitations", team.id))
            .header(ContentType::JSON)
            .body(format!(r#"{{"role":"member","user_id":"{}"}}"#, alice))
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::Conflict);
        assert_eq!(
            teams_repo::member_role(&pool, team.id, alice)
                .await
                .unwrap(),
            Some(TeamRole::Owner)
        );
        Ok(())
    }

    /// Same guard, but via the by-email path, and with a differently-cased address than the
    /// member's account email — matching `require_addressed_to`'s case-insensitive
    /// comparison, since emails aren't case-normalized on input.
    #[sqlx::test]
    async fn create_invitation_rejects_existing_member_by_email_case_insensitive(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        let default_team = users::ensure_default_team(&pool).await.unwrap();
        let alice = seed_user(&pool, "alice16@example.com", "Alice", default_team).await;
        let dave = seed_user(&pool, "dave16@example.com", "Dave", default_team).await;

        let team = teams_repo::create(&pool, "Team", alice).await.unwrap();
        teams_repo::add_member(&pool, team.id, dave, TeamRole::Member)
            .await
            .unwrap();

        let client = test_client(pool.clone()).await;
        login(&client, "alice16@example.com").await;

        let response = client
            .post(format!("/api/teams/{}/invitations", team.id))
            .header(ContentType::JSON)
            .body(r#"{"role":"owner","email":"DAVE16@Example.com"}"#)
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::Conflict);
        assert_eq!(
            teams_repo::member_role(&pool, team.id, dave).await.unwrap(),
            Some(TeamRole::Member)
        );
        Ok(())
    }

    /// Last-line-of-defense check in `accept_invitation` itself: a pending invitation that
    /// predates the addressee becoming a member (the create-time guard can't see into the
    /// future) must not let accepting it silently change their role.
    #[sqlx::test]
    async fn accept_invitation_rejects_when_already_a_member(pool: PgPool) -> sqlx::Result<()> {
        let default_team = users::ensure_default_team(&pool).await.unwrap();
        let alice = seed_user(&pool, "alice17@example.com", "Alice", default_team).await;
        let erin = seed_user(&pool, "erin17@example.com", "Erin", default_team).await;

        let team = teams_repo::create(&pool, "Team", alice).await.unwrap();
        let invitation =
            teams_repo::create_invitation_for_user(&pool, team.id, TeamRole::Owner, erin)
                .await
                .unwrap();

        // Erin joins as a plain member through some other path while the invitation above
        // is still pending.
        teams_repo::add_member(&pool, team.id, erin, TeamRole::Member)
            .await
            .unwrap();

        let client = test_client(pool.clone()).await;
        login(&client, "erin17@example.com").await;

        let response = client
            .post(format!("/api/invitations/{}/accept", invitation.id))
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::Conflict);
        assert_eq!(
            teams_repo::member_role(&pool, team.id, erin).await.unwrap(),
            Some(TeamRole::Member)
        );
        assert!(
            teams_repo::get_invitation(&pool, invitation.id)
                .await
                .is_err()
        );
        Ok(())
    }

    #[sqlx::test]
    async fn list_invitations_forbidden_for_plain_member(pool: PgPool) -> sqlx::Result<()> {
        let default_team = users::ensure_default_team(&pool).await.unwrap();
        let alice = seed_user(&pool, "alice12@example.com", "Alice", default_team).await;
        let carol = seed_user(&pool, "carol12@example.com", "Carol", default_team).await;

        let team = teams_repo::create(&pool, "Team", alice).await.unwrap();
        teams_repo::add_member(&pool, team.id, carol, TeamRole::Member)
            .await
            .unwrap();
        teams_repo::create_invitation_for_email(&pool, team.id, TeamRole::Member, "x@example.com")
            .await
            .unwrap();

        let client = test_client(pool.clone()).await;
        login(&client, "carol12@example.com").await;

        let response = client
            .get(format!("/api/teams/{}/invitations", team.id))
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::Forbidden);
        Ok(())
    }

    #[sqlx::test]
    async fn list_invitations_succeeds_for_admin(pool: PgPool) -> sqlx::Result<()> {
        let default_team = users::ensure_default_team(&pool).await.unwrap();
        let alice = seed_user(&pool, "alice13@example.com", "Alice", default_team).await;
        let dave = seed_user(&pool, "dave13@example.com", "Dave", default_team).await;

        let team = teams_repo::create(&pool, "Team", alice).await.unwrap();
        teams_repo::add_member(&pool, team.id, dave, TeamRole::Admin)
            .await
            .unwrap();
        teams_repo::create_invitation_for_email(
            &pool,
            team.id,
            TeamRole::Member,
            "invitee13@example.com",
        )
        .await
        .unwrap();

        let client = test_client(pool.clone()).await;
        login(&client, "dave13@example.com").await;

        let response = client
            .get(format!("/api/teams/{}/invitations", team.id))
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::Ok);
        let body: serde_json::Value =
            serde_json::from_str(&response.into_string().await.unwrap()).unwrap();
        assert_eq!(body["data"].as_array().unwrap().len(), 1);
        Ok(())
    }

    #[sqlx::test]
    async fn decline_invitation_requires_admin_or_owner(pool: PgPool) -> sqlx::Result<()> {
        let default_team = users::ensure_default_team(&pool).await.unwrap();
        let alice = seed_user(&pool, "alice14@example.com", "Alice", default_team).await;
        let carol = seed_user(&pool, "carol14@example.com", "Carol", default_team).await;

        let team = teams_repo::create(&pool, "Team", alice).await.unwrap();
        teams_repo::add_member(&pool, team.id, carol, TeamRole::Member)
            .await
            .unwrap();
        let invitation = teams_repo::create_invitation_for_email(
            &pool,
            team.id,
            TeamRole::Member,
            "invitee14@example.com",
        )
        .await
        .unwrap();

        let client = test_client(pool.clone()).await;

        login(&client, "carol14@example.com").await;
        let forbidden = client
            .delete(format!(
                "/api/teams/{}/invitations/{}",
                team.id, invitation.id
            ))
            .dispatch()
            .await;
        assert_eq!(forbidden.status(), Status::Forbidden);

        login(&client, "alice14@example.com").await;
        let ok = client
            .delete(format!(
                "/api/teams/{}/invitations/{}",
                team.id, invitation.id
            ))
            .dispatch()
            .await;
        assert_eq!(ok.status(), Status::Ok);

        assert!(matches!(
            teams_repo::get_invitation(&pool, invitation.id).await,
            Err(DbError::NotFound("invitation"))
        ));
        Ok(())
    }

    #[sqlx::test]
    async fn accept_invitation_by_user_id_is_self_service(pool: PgPool) -> sqlx::Result<()> {
        let default_team = users::ensure_default_team(&pool).await.unwrap();
        let alice = seed_user(&pool, "alice15@example.com", "Alice", default_team).await;
        let erin = seed_user(&pool, "erin15@example.com", "Erin", default_team).await;

        let team = teams_repo::create(&pool, "Team", alice).await.unwrap();
        let invitation =
            teams_repo::create_invitation_for_user(&pool, team.id, TeamRole::Admin, erin)
                .await
                .unwrap();

        let client = test_client(pool.clone()).await;
        login(&client, "erin15@example.com").await;

        // Erin isn't a member of the team at all yet, so this must succeed without any team
        // role — the invitation being addressed to her is enough.
        let response = client
            .post(format!("/api/invitations/{}/accept", invitation.id))
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::Ok);

        assert_eq!(
            teams_repo::member_role(&pool, team.id, erin).await.unwrap(),
            Some(TeamRole::Admin)
        );
        assert!(matches!(
            teams_repo::get_invitation(&pool, invitation.id).await,
            Err(DbError::NotFound("invitation"))
        ));
        Ok(())
    }

    #[sqlx::test]
    async fn accept_invitation_by_matching_email_is_self_service(pool: PgPool) -> sqlx::Result<()> {
        let default_team = users::ensure_default_team(&pool).await.unwrap();
        let alice = seed_user(&pool, "alice16@example.com", "Alice", default_team).await;
        let frank = seed_user(&pool, "frank16@example.com", "Frank", default_team).await;

        let team = teams_repo::create(&pool, "Team", alice).await.unwrap();
        let invitation = teams_repo::create_invitation_for_email(
            &pool,
            team.id,
            TeamRole::Member,
            "frank16@example.com",
        )
        .await
        .unwrap();

        let client = test_client(pool.clone()).await;
        login(&client, "frank16@example.com").await;

        let response = client
            .post(format!("/api/invitations/{}/accept", invitation.id))
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::Ok);

        assert_eq!(
            teams_repo::member_role(&pool, team.id, frank)
                .await
                .unwrap(),
            Some(TeamRole::Member)
        );
        Ok(())
    }

    #[sqlx::test]
    async fn accept_invitation_forbidden_for_unrelated_user(pool: PgPool) -> sqlx::Result<()> {
        let default_team = users::ensure_default_team(&pool).await.unwrap();
        let alice = seed_user(&pool, "alice17@example.com", "Alice", default_team).await;
        let erin = seed_user(&pool, "erin17@example.com", "Erin", default_team).await;
        seed_user(&pool, "mallory17@example.com", "Mallory", default_team).await;

        let team = teams_repo::create(&pool, "Team", alice).await.unwrap();
        let invitation =
            teams_repo::create_invitation_for_user(&pool, team.id, TeamRole::Member, erin)
                .await
                .unwrap();

        let client = test_client(pool.clone()).await;
        login(&client, "mallory17@example.com").await;

        let response = client
            .post(format!("/api/invitations/{}/accept", invitation.id))
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::Forbidden);
        Ok(())
    }

    #[sqlx::test]
    async fn list_my_invitations_includes_team_name_and_excludes_others(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        let default_team = users::ensure_default_team(&pool).await.unwrap();
        let alice = seed_user(&pool, "alice18@example.com", "Alice", default_team).await;
        let erin = seed_user(&pool, "erin18@example.com", "Erin", default_team).await;
        seed_user(&pool, "mallory18@example.com", "Mallory", default_team).await;

        let team = teams_repo::create(&pool, "Team Eighteen", alice)
            .await
            .unwrap();
        teams_repo::create_invitation_for_user(&pool, team.id, TeamRole::Member, erin)
            .await
            .unwrap();

        let client = test_client(pool.clone()).await;
        login(&client, "erin18@example.com").await;

        let response = client.get("/api/invitations").dispatch().await;
        assert_eq!(response.status(), Status::Ok);
        let body: serde_json::Value =
            serde_json::from_str(&response.into_string().await.unwrap()).unwrap();
        let invitations = body["data"].as_array().unwrap();
        assert_eq!(invitations.len(), 1);
        assert_eq!(invitations[0]["team_name"], "Team Eighteen");

        login(&client, "mallory18@example.com").await;
        let response = client.get("/api/invitations").dispatch().await;
        assert_eq!(response.status(), Status::Ok);
        let body: serde_json::Value =
            serde_json::from_str(&response.into_string().await.unwrap()).unwrap();
        assert!(body["data"].as_array().unwrap().is_empty());
        Ok(())
    }

    #[sqlx::test]
    async fn decline_my_invitation_is_self_service_but_not_for_others(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        let default_team = users::ensure_default_team(&pool).await.unwrap();
        let alice = seed_user(&pool, "alice19@example.com", "Alice", default_team).await;
        let erin = seed_user(&pool, "erin19@example.com", "Erin", default_team).await;
        seed_user(&pool, "mallory19@example.com", "Mallory", default_team).await;

        let team = teams_repo::create(&pool, "Team", alice).await.unwrap();
        let invitation =
            teams_repo::create_invitation_for_user(&pool, team.id, TeamRole::Member, erin)
                .await
                .unwrap();

        let client = test_client(pool.clone()).await;

        login(&client, "mallory19@example.com").await;
        let forbidden = client
            .post(format!("/api/invitations/{}/decline", invitation.id))
            .dispatch()
            .await;
        assert_eq!(forbidden.status(), Status::Forbidden);

        login(&client, "erin19@example.com").await;
        let ok = client
            .post(format!("/api/invitations/{}/decline", invitation.id))
            .dispatch()
            .await;
        assert_eq!(ok.status(), Status::Ok);

        assert!(matches!(
            teams_repo::get_invitation(&pool, invitation.id).await,
            Err(DbError::NotFound("invitation"))
        ));
        assert_eq!(
            teams_repo::member_role(&pool, team.id, erin).await.unwrap(),
            None
        );
        Ok(())
    }

    #[sqlx::test]
    async fn default_team_cannot_be_deleted_or_patched(pool: PgPool) -> sqlx::Result<()> {
        let default_team = users::ensure_default_team(&pool).await.unwrap();
        seed_user(&pool, "alice20@example.com", "Alice", default_team).await;

        let client = test_client(pool.clone()).await;
        login(&client, "alice20@example.com").await;

        // Alice is an owner of Default by construction (every signup is), so the role
        // check alone would let this through — only the dedicated Default-team guard
        // stops it.
        let delete_response = client
            .delete(format!("/api/teams/{}", default_team))
            .dispatch()
            .await;
        assert_eq!(delete_response.status(), Status::Forbidden);

        let patch_response = client
            .patch(format!("/api/teams/{}", default_team))
            .header(ContentType::JSON)
            .body(r#"{"name":"Hacked Default"}"#)
            .dispatch()
            .await;
        assert_eq!(patch_response.status(), Status::Forbidden);

        assert_eq!(
            teams_repo::get(&pool, default_team).await.unwrap().name,
            "Default"
        );
        Ok(())
    }

    #[sqlx::test]
    async fn patch_team_forbids_admin_from_changing_owner_set(pool: PgPool) -> sqlx::Result<()> {
        let default_team = users::ensure_default_team(&pool).await.unwrap();
        let alice = seed_user(&pool, "alice21@example.com", "Alice", default_team).await;
        let dave = seed_user(&pool, "dave21@example.com", "Dave", default_team).await;

        let team = teams_repo::create(&pool, "Team", alice).await.unwrap();
        teams_repo::add_member(&pool, team.id, dave, TeamRole::Admin)
            .await
            .unwrap();

        let client = test_client(pool.clone()).await;
        login(&client, "dave21@example.com").await;

        // Dave (an admin) tries to promote himself to owner alongside Alice.
        let promote_self = serde_json::json!({
            "members": [
                {"user_id": alice, "role": "owner"},
                {"user_id": dave, "role": "owner"}
            ]
        });
        let response = client
            .patch(format!("/api/teams/{}", team.id))
            .header(ContentType::JSON)
            .body(promote_self.to_string())
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::Forbidden);
        assert_eq!(
            teams_repo::member_role(&pool, team.id, dave).await.unwrap(),
            Some(TeamRole::Admin)
        );

        // Dave also can't drop Alice from the owner set while keeping only himself.
        let drop_owner = serde_json::json!({
            "members": [
                {"user_id": alice, "role": "member"},
                {"user_id": dave, "role": "admin"}
            ]
        });
        let response = client
            .patch(format!("/api/teams/{}", team.id))
            .header(ContentType::JSON)
            .body(drop_owner.to_string())
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::Forbidden);
        assert_eq!(
            teams_repo::member_role(&pool, team.id, alice)
                .await
                .unwrap(),
            Some(TeamRole::Owner)
        );
        Ok(())
    }

    #[sqlx::test]
    async fn patch_team_rejects_removing_the_last_owner(pool: PgPool) -> sqlx::Result<()> {
        let default_team = users::ensure_default_team(&pool).await.unwrap();
        let alice = seed_user(&pool, "alice22@example.com", "Alice", default_team).await;

        let team = teams_repo::create(&pool, "Solo Team", alice).await.unwrap();

        let client = test_client(pool.clone()).await;
        login(&client, "alice22@example.com").await;

        // Alice, the sole owner, tries to remove herself — this must not be allowed to
        // leave the team ownerless (and thus permanently undeletable/unmanageable).
        let response = client
            .patch(format!("/api/teams/{}", team.id))
            .header(ContentType::JSON)
            .body(r#"{"members":[]}"#)
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::Conflict);
        assert_eq!(
            teams_repo::member_role(&pool, team.id, alice)
                .await
                .unwrap(),
            Some(TeamRole::Owner)
        );
        Ok(())
    }

    #[sqlx::test]
    async fn invitation_by_new_email_is_visible_and_acceptable_after_email_change(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        let default_team = users::ensure_default_team(&pool).await.unwrap();
        let alice = seed_user(&pool, "alice23@example.com", "Alice", default_team).await;
        let erin_id = seed_user(&pool, "erin23-old@example.com", "Erin", default_team).await;

        let team = teams_repo::create(&pool, "Team", alice).await.unwrap();

        let client = test_client(pool.clone()).await;
        login(&client, "erin23-old@example.com").await;

        // Erin changes her email — via the repo layer directly, mirroring what
        // PATCH /api/users/<id> does — without logging out, so her session cookie still
        // carries the old email.
        let mut erin = users::get(&pool, erin_id).await.unwrap();
        erin.email = "erin23-new@example.com".to_string();
        users::update(&pool, &erin).await.unwrap();

        // Alice invites Erin's *new* address after the change.
        let invitation = teams_repo::create_invitation_for_email(
            &pool,
            team.id,
            TeamRole::Member,
            "erin23-new@example.com",
        )
        .await
        .unwrap();

        // Erin's still-logged-in session (stale cached email) must still see and be able
        // to accept it, because the check now looks her email up fresh instead of trusting
        // the session.
        let response = client.get("/api/invitations").dispatch().await;
        assert_eq!(response.status(), Status::Ok);
        let body: serde_json::Value =
            serde_json::from_str(&response.into_string().await.unwrap()).unwrap();
        assert_eq!(body["data"].as_array().unwrap().len(), 1);

        let response = client
            .post(format!("/api/invitations/{}/accept", invitation.id))
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::Ok);
        assert_eq!(
            teams_repo::member_role(&pool, team.id, erin_id)
                .await
                .unwrap(),
            Some(TeamRole::Member)
        );
        Ok(())
    }

    #[sqlx::test]
    async fn invitation_email_match_is_case_insensitive_over_http(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        let default_team = users::ensure_default_team(&pool).await.unwrap();
        let alice = seed_user(&pool, "alice24@example.com", "Alice", default_team).await;
        seed_user(&pool, "Erin24@Example.com", "Erin", default_team).await;

        let team = teams_repo::create(&pool, "Team", alice).await.unwrap();
        teams_repo::create_invitation_for_email(
            &pool,
            team.id,
            TeamRole::Member,
            "erin24@example.com",
        )
        .await
        .unwrap();

        let client = test_client(pool.clone()).await;
        login(&client, "Erin24@Example.com").await;

        let response = client.get("/api/invitations").dispatch().await;
        assert_eq!(response.status(), Status::Ok);
        let body: serde_json::Value =
            serde_json::from_str(&response.into_string().await.unwrap()).unwrap();
        assert_eq!(body["data"].as_array().unwrap().len(), 1);
        Ok(())
    }
}

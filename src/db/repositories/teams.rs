//! `teams` / `users_teams` / `invitations`.
//!
//! `Team.members` and `TeamMember.user` aren't plain columns (a team's member list comes
//! from a join against `users`/`users_teams`), so `Team` and `TeamMember` implement
//! `sqlx::FromRow` by hand instead of deriving it: `Team::from_row` always leaves `members`
//! as `vec![]` (filled in separately by [`list_members`], mirroring how `persons::get` fills
//! in `PersonV2::bios` after the fact), and `TeamMember::from_row` reads a `TeamMember`
//! straight out of a `users_teams JOIN users` row. `InvitationWithTeam` (an `Invitation`
//! joined with its team's name) follows the same hand-written pattern. Because `FromRow`
//! (not the compile-time-checked `query_as!` macro) is what consumes these impls, queries
//! that return `Team`, `TeamMember` or `InvitationWithTeam` are runtime-checked. `Invitation`
//! has no such nested shape, so it derives `FromRow` and is fetched with the
//! macro-checked `query_as!`.

use super::DbError;
use crate::db::repositories::users::{TeamRole, UserProfile};
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use sqlx::postgres::{PgExecutor, PgRow};
use sqlx::{FromRow, Row};
use uuid::Uuid;

#[derive(Debug, Clone, serde::Serialize)]
pub struct Team {
    pub id: uuid::Uuid,
    pub name: String,
    pub members: Vec<TeamMember>,
}

impl<'r> FromRow<'r, PgRow> for Team {
    fn from_row(row: &'r PgRow) -> sqlx::Result<Self> {
        Ok(Team {
            id: row.try_get("id")?,
            name: row.try_get("name")?,
            members: Vec::new(),
        })
    }
}

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct Invitation {
    pub id: uuid::Uuid,
    pub team_id: uuid::Uuid,
    pub role: TeamRole,
    pub user_id: Option<uuid::Uuid>,
    pub email: Option<String>,
    pub timestamp: DateTime<Utc>,
}

/// An invitation addressed to a not-yet-member account, joined with the invited team's
/// name — used for self-service display, where the invitee can't look the team up any
/// other way (they're not a member, so `teams::get` would 404 for them).
#[derive(Debug, Clone, serde::Serialize)]
pub struct InvitationWithTeam {
    pub id: uuid::Uuid,
    pub team_id: uuid::Uuid,
    pub team_name: String,
    pub role: TeamRole,
    pub timestamp: DateTime<Utc>,
}

impl<'r> FromRow<'r, PgRow> for InvitationWithTeam {
    fn from_row(row: &'r PgRow) -> sqlx::Result<Self> {
        Ok(InvitationWithTeam {
            id: row.try_get("id")?,
            team_id: row.try_get("team_id")?,
            team_name: row.try_get("team_name")?,
            role: row.try_get("role")?,
            timestamp: row.try_get("timestamp")?,
        })
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TeamMember {
    pub user: UserProfile,
    pub role: TeamRole,
}

impl<'r> FromRow<'r, PgRow> for TeamMember {
    fn from_row(row: &'r PgRow) -> sqlx::Result<Self> {
        Ok(TeamMember {
            user: UserProfile {
                id: row.try_get("id")?,
                name: row.try_get("name")?,
            },
            role: row.try_get("role")?,
        })
    }
}

// ============================================================================
//  Teams
// ============================================================================

/// Fetches a team by id, including its members.
pub async fn get(pool: &PgPool, id: Uuid) -> Result<Team, DbError> {
    let mut team: Team = sqlx::query_as("SELECT id, name FROM teams WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or(DbError::NotFound("team"))?;

    team.members = list_members(pool, id).await?;
    Ok(team)
}

/// Returns every team, ordered by name, including their members.
pub async fn list_all(pool: &PgPool) -> Result<Vec<Team>, DbError> {
    let mut teams: Vec<Team> = sqlx::query_as("SELECT id, name FROM teams ORDER BY name")
        .fetch_all(pool)
        .await?;

    for team in &mut teams {
        team.members = list_members(pool, team.id).await?;
    }
    Ok(teams)
}

/// Returns every team `user_id` belongs to, ordered by name, including their members.
pub async fn list_for_user(pool: &PgPool, user_id: Uuid) -> Result<Vec<Team>, DbError> {
    let mut teams: Vec<Team> = sqlx::query_as(
        "SELECT t.id, t.name FROM teams t
         JOIN users_teams ut ON ut.team_id = t.id
         WHERE ut.user_id = $1
         ORDER BY t.name",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    for team in &mut teams {
        team.members = list_members(pool, team.id).await?;
    }
    Ok(teams)
}

/// Loads a team's members (joined with their user profile), ordered by name.
pub async fn list_members<'e>(
    exec: impl PgExecutor<'e>,
    team_id: Uuid,
) -> Result<Vec<TeamMember>, DbError> {
    let members: Vec<TeamMember> = sqlx::query_as(
        "SELECT u.id, u.name, ut.role FROM users_teams ut
         JOIN users u ON u.id = ut.user_id
         WHERE ut.team_id = $1
         ORDER BY u.name",
    )
    .bind(team_id)
    .fetch_all(exec)
    .await?;
    Ok(members)
}

/// Returns `user_id`'s role on `team_id`, or `None` if they're not a member.
pub async fn member_role<'e>(
    exec: impl PgExecutor<'e>,
    team_id: Uuid,
    user_id: Uuid,
) -> Result<Option<TeamRole>, DbError> {
    let role: Option<TeamRole> = sqlx::query_scalar!(
        r#"SELECT role as "role: TeamRole" FROM users_teams WHERE team_id = $1 AND user_id = $2"#,
        team_id,
        user_id
    )
    .fetch_optional(exec)
    .await?;
    Ok(role)
}

/// Returns the role of whichever member of `team_id` has `email` (case-insensitively), or
/// `None` if no member has it. Case-insensitive to match [`list_pending_invitations`]/
/// [`crate::profile_settings::teams::require_addressed_to`] — email addresses aren't
/// case-normalized on input, so a member's account email and an invitation address can
/// differ only in case.
pub async fn member_role_by_email<'e>(
    exec: impl PgExecutor<'e>,
    team_id: Uuid,
    email: &str,
) -> Result<Option<TeamRole>, DbError> {
    let role: Option<TeamRole> = sqlx::query_scalar!(
        r#"SELECT ut.role as "role: TeamRole" FROM users_teams ut
           JOIN users u ON u.id = ut.user_id
           WHERE ut.team_id = $1 AND LOWER(u.email) = LOWER($2)"#,
        team_id,
        email
    )
    .fetch_optional(exec)
    .await?;
    Ok(role)
}

/// Creates a new team and attaches `owner_user_id` to it as `owner`, in one transaction.
pub async fn create(pool: &PgPool, name: &str, owner_user_id: Uuid) -> Result<Team, DbError> {
    let mut tx = pool.begin().await?;

    let id = sqlx::query_scalar!("INSERT INTO teams (name) VALUES ($1) RETURNING id", name)
        .fetch_one(&mut *tx)
        .await?;

    sqlx::query!(
        "INSERT INTO users_teams (user_id, team_id, role) VALUES ($1, $2, $3)",
        owner_user_id,
        id,
        TeamRole::Owner as TeamRole
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    get(pool, id).await
}

/// Renames a team. Fails with [`DbError::NotFound`] if no team with that id exists.
pub async fn rename<'e>(exec: impl PgExecutor<'e>, id: Uuid, name: &str) -> Result<(), DbError> {
    let result = sqlx::query!("UPDATE teams SET name = $2 WHERE id = $1", id, name)
        .execute(exec)
        .await?;
    if result.rows_affected() == 0 {
        return Err(DbError::NotFound("team"));
    }
    Ok(())
}

/// Deletes a team by id (cascades `users_teams`). Fails with [`DbError::NotFound`] if no
/// team with that id exists.
pub async fn delete<'e>(exec: impl PgExecutor<'e>, id: Uuid) -> Result<(), DbError> {
    let result = sqlx::query!("DELETE FROM teams WHERE id = $1", id)
        .execute(exec)
        .await?;
    if result.rows_affected() == 0 {
        return Err(DbError::NotFound("team"));
    }
    Ok(())
}

/// Adds `user_id` to `team_id` with `role`.
pub async fn add_member<'e>(
    exec: impl PgExecutor<'e>,
    team_id: Uuid,
    user_id: Uuid,
    role: TeamRole,
) -> Result<(), DbError> {
    sqlx::query!(
        "INSERT INTO users_teams (user_id, team_id, role) VALUES ($1, $2, $3)",
        user_id,
        team_id,
        role as TeamRole
    )
    .execute(exec)
    .await?;
    Ok(())
}

/// Changes `user_id`'s role on `team_id`. Fails with [`DbError::NotFound`] if they're not
/// a member of the team.
pub async fn set_member_role<'e>(
    exec: impl PgExecutor<'e>,
    team_id: Uuid,
    user_id: Uuid,
    role: TeamRole,
) -> Result<(), DbError> {
    let result = sqlx::query!(
        "UPDATE users_teams SET role = $3 WHERE team_id = $1 AND user_id = $2",
        team_id,
        user_id,
        role as TeamRole
    )
    .execute(exec)
    .await?;
    if result.rows_affected() == 0 {
        return Err(DbError::NotFound("team member"));
    }
    Ok(())
}

/// Removes `user_id` from `team_id`. Fails with [`DbError::NotFound`] if they're not a
/// member of the team.
pub async fn remove_member<'e>(
    exec: impl PgExecutor<'e>,
    team_id: Uuid,
    user_id: Uuid,
) -> Result<(), DbError> {
    let result = sqlx::query!(
        "DELETE FROM users_teams WHERE team_id = $1 AND user_id = $2",
        team_id,
        user_id
    )
    .execute(exec)
    .await?;
    if result.rows_affected() == 0 {
        return Err(DbError::NotFound("team member"));
    }
    Ok(())
}

/// Replaces a team's entire membership with `members` (delete-then-insert): every current
/// member not listed is removed, and every listed `(user_id, role)` pair is (re-)created
/// with that role. Takes an explicit connection (rather than a generic `PgExecutor`) since
/// it runs multiple statements that must share one transaction with the caller, mirroring
/// `persons::replace_bios`.
///
/// Rejects a `members` list with no `Owner` entry: a team without an owner can never be
/// deleted (`delete_team` requires an owner) or have its membership managed again, so this
/// invariant is enforced here — the one choke point every membership write passes through —
/// rather than trusted to every caller.
pub async fn replace_members(
    exec: &mut sqlx::PgConnection,
    team_id: Uuid,
    members: &[(Uuid, TeamRole)],
) -> Result<(), DbError> {
    if !members.iter().any(|(_, role)| *role == TeamRole::Owner) {
        return Err(DbError::Conflict(
            "a team must always have at least one owner".to_string(),
        ));
    }

    sqlx::query!("DELETE FROM users_teams WHERE team_id = $1", team_id)
        .execute(&mut *exec)
        .await?;

    for (user_id, role) in members {
        sqlx::query!(
            "INSERT INTO users_teams (user_id, team_id, role) VALUES ($1, $2, $3)",
            user_id,
            team_id,
            *role as TeamRole
        )
        .execute(&mut *exec)
        .await?;
    }
    Ok(())
}

// ============================================================================
//  Invitations
// ============================================================================

/// Invites an existing user to a team.
pub async fn create_invitation_for_user<'e>(
    exec: impl PgExecutor<'e>,
    team_id: Uuid,
    role: TeamRole,
    user_id: Uuid,
) -> Result<Invitation, DbError> {
    let invitation = sqlx::query_as!(
        Invitation,
        r#"INSERT INTO invitations (team_id, role, user_id) VALUES ($1, $2, $3)
           RETURNING id, team_id, role as "role: TeamRole", user_id, email, timestamp"#,
        team_id,
        role as TeamRole,
        user_id
    )
    .fetch_one(exec)
    .await?;
    Ok(invitation)
}

/// Invites a not-yet-registered person to a team by email.
pub async fn create_invitation_for_email<'e>(
    exec: impl PgExecutor<'e>,
    team_id: Uuid,
    role: TeamRole,
    email: &str,
) -> Result<Invitation, DbError> {
    let invitation = sqlx::query_as!(
        Invitation,
        r#"INSERT INTO invitations (team_id, role, email) VALUES ($1, $2, $3)
           RETURNING id, team_id, role as "role: TeamRole", user_id, email, timestamp"#,
        team_id,
        role as TeamRole,
        email
    )
    .fetch_one(exec)
    .await?;
    Ok(invitation)
}

/// Fetches an invitation by id.
pub async fn get_invitation<'e>(
    exec: impl PgExecutor<'e>,
    id: Uuid,
) -> Result<Invitation, DbError> {
    sqlx::query_as!(
        Invitation,
        r#"SELECT id, team_id, role as "role: TeamRole", user_id, email, timestamp FROM invitations WHERE id = $1"#,
        id
    )
    .fetch_optional(exec)
    .await?
    .ok_or(DbError::NotFound("invitation"))
}

/// Returns every pending invitation for a team, oldest first.
pub async fn list_invitations_for_team<'e>(
    exec: impl PgExecutor<'e>,
    team_id: Uuid,
) -> Result<Vec<Invitation>, DbError> {
    let invitations = sqlx::query_as!(
        Invitation,
        r#"SELECT id, team_id, role as "role: TeamRole", user_id, email, timestamp FROM invitations WHERE team_id = $1 ORDER BY timestamp"#,
        team_id
    )
    .fetch_all(exec)
    .await?;
    Ok(invitations)
}

/// Returns every pending invitation addressed to an account: either directly by
/// `user_id` (invited after they'd already registered) or by `email` (invited before
/// they existed as a user). The email match is case-insensitive, matching
/// [`crate::profile_settings::teams::require_addressed_to`]'s comparison — email addresses
/// aren't case-normalized on input, so an invitation and an account can differ only in case.
pub async fn list_pending_invitations<'e>(
    exec: impl PgExecutor<'e>,
    user_id: Uuid,
    email: &str,
) -> Result<Vec<Invitation>, DbError> {
    let invitations = sqlx::query_as!(
        Invitation,
        r#"SELECT id, team_id, role as "role: TeamRole", user_id, email, timestamp FROM invitations
           WHERE user_id = $1 OR LOWER(email) = LOWER($2)
           ORDER BY timestamp"#,
        user_id,
        email
    )
    .fetch_all(exec)
    .await?;
    Ok(invitations)
}

/// Like [`list_pending_invitations`], but also returns each invitation's team name —
/// for self-service display to the invitee, who isn't a member yet and so has no other
/// way to look up the team's name. The email match is case-insensitive, matching
/// [`list_pending_invitations`] and [`crate::profile_settings::teams::require_addressed_to`].
pub async fn list_pending_invitations_with_team_name<'e>(
    exec: impl PgExecutor<'e>,
    user_id: Uuid,
    email: &str,
) -> Result<Vec<InvitationWithTeam>, DbError> {
    let invitations: Vec<InvitationWithTeam> = sqlx::query_as(
        "SELECT i.id, i.team_id, t.name as team_name, i.role, i.timestamp
         FROM invitations i
         JOIN teams t ON t.id = i.team_id
         WHERE i.user_id = $1 OR LOWER(i.email) = LOWER($2)
         ORDER BY i.timestamp",
    )
    .bind(user_id)
    .bind(email)
    .fetch_all(exec)
    .await?;
    Ok(invitations)
}

/// Accepts an invitation: adds `accepting_user_id` to the invitation's team with the
/// invited role, then removes the invitation, in one transaction. Fails with
/// [`DbError::NotFound`] if no invitation with that id exists, or [`DbError::Conflict`] if
/// `accepting_user_id` is already a member of the invitation's team — the invitation is
/// discarded as stale in that case rather than silently overwriting their existing role.
/// This is the last line of defense against a member's role being changed via invitation
/// instead of [`set_member_role`]: [`crate::profile_settings::teams::create_invitation`]
/// already rejects invitations addressed to an existing member up front, but that check
/// can't catch an invitation that was pending before the addressee joined, or a second
/// invitation sent while a first one to the same person is still pending.
pub async fn accept_invitation(
    pool: &PgPool,
    invitation_id: Uuid,
    accepting_user_id: Uuid,
) -> Result<(), DbError> {
    let mut tx = pool.begin().await?;

    let invitation = sqlx::query_as!(
        Invitation,
        r#"SELECT id, team_id, role as "role: TeamRole", user_id, email, timestamp FROM invitations WHERE id = $1"#,
        invitation_id
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(DbError::NotFound("invitation"))?;

    let insert_result = sqlx::query!(
        "INSERT INTO users_teams (user_id, team_id, role) VALUES ($1, $2, $3)
         ON CONFLICT (user_id, team_id) DO NOTHING",
        accepting_user_id,
        invitation.team_id,
        invitation.role as TeamRole
    )
    .execute(&mut *tx)
    .await?;

    if insert_result.rows_affected() == 0 {
        sqlx::query!("DELETE FROM invitations WHERE id = $1", invitation_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        return Err(DbError::Conflict(
            "you are already a member of this team".to_string(),
        ));
    }

    sqlx::query!("DELETE FROM invitations WHERE id = $1", invitation_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(())
}

/// Deletes an invitation (declining it, or a team admin revoking it). Fails with
/// [`DbError::NotFound`] if no invitation with that id exists.
pub async fn delete_invitation<'e>(exec: impl PgExecutor<'e>, id: Uuid) -> Result<(), DbError> {
    let result = sqlx::query!("DELETE FROM invitations WHERE id = $1", id)
        .execute(exec)
        .await?;
    if result.rows_affected() == 0 {
        return Err(DbError::NotFound("invitation"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::repositories::users;

    async fn seed_user(pool: &PgPool, email: &str, name: &str, team_id: Uuid) -> Uuid {
        users::insert(pool, email, name, "hash", team_id)
            .await
            .unwrap()
            .id
    }

    #[sqlx::test]
    async fn create_attaches_owner(pool: PgPool) -> sqlx::Result<()> {
        let default_team = users::ensure_default_team(&pool).await.unwrap();
        let owner_id = seed_user(&pool, "owner@example.com", "Owner", default_team).await;

        let team = create(&pool, "Constitution Club", owner_id).await.unwrap();
        assert_eq!(team.name, "Constitution Club");
        assert_eq!(team.members.len(), 1);
        assert_eq!(team.members[0].user.id, owner_id);
        assert_eq!(team.members[0].role, TeamRole::Owner);
        Ok(())
    }

    #[sqlx::test]
    async fn add_set_and_remove_member(pool: PgPool) -> sqlx::Result<()> {
        let default_team = users::ensure_default_team(&pool).await.unwrap();
        let owner_id = seed_user(&pool, "owner2@example.com", "Owner", default_team).await;
        let member_id = seed_user(&pool, "member@example.com", "Member", default_team).await;

        let team = create(&pool, "Team", owner_id).await.unwrap();
        add_member(&pool, team.id, member_id, TeamRole::Member)
            .await
            .unwrap();

        assert_eq!(
            member_role(&pool, team.id, member_id).await.unwrap(),
            Some(TeamRole::Member)
        );

        set_member_role(&pool, team.id, member_id, TeamRole::Admin)
            .await
            .unwrap();
        assert_eq!(
            member_role(&pool, team.id, member_id).await.unwrap(),
            Some(TeamRole::Admin)
        );

        remove_member(&pool, team.id, member_id).await.unwrap();
        assert_eq!(member_role(&pool, team.id, member_id).await.unwrap(), None);
        Ok(())
    }

    #[sqlx::test]
    async fn replace_members_rejects_ownerless_list(pool: PgPool) -> sqlx::Result<()> {
        let default_team = users::ensure_default_team(&pool).await.unwrap();
        let owner_id = seed_user(&pool, "sole-owner@example.com", "Owner", default_team).await;
        let member_id = seed_user(&pool, "member2@example.com", "Member", default_team).await;

        let team = create(&pool, "Team", owner_id).await.unwrap();

        let mut tx = pool.begin().await.unwrap();
        let result = replace_members(&mut tx, team.id, &[(member_id, TeamRole::Member)]).await;
        assert!(matches!(result, Err(DbError::Conflict(_))));
        tx.rollback().await.unwrap();

        // The rejected patch must not have partially applied (the owner is still in place).
        assert_eq!(
            member_role(&pool, team.id, owner_id).await.unwrap(),
            Some(TeamRole::Owner)
        );
        Ok(())
    }

    #[sqlx::test]
    async fn list_for_user_only_returns_membership(pool: PgPool) -> sqlx::Result<()> {
        let default_team = users::ensure_default_team(&pool).await.unwrap();
        let alice = seed_user(&pool, "alice@example.com", "Alice", default_team).await;
        let bob = seed_user(&pool, "bob2@example.com", "Bob", default_team).await;

        let team_a = create(&pool, "Team A", alice).await.unwrap();
        let team_b = create(&pool, "Team B", bob).await.unwrap();

        // Alice is also a member of the default team (every user is attached to it on
        // signup), so she belongs to two teams: Default and Team A.
        let alice_teams = list_for_user(&pool, alice).await.unwrap();
        assert_eq!(alice_teams.len(), 2);
        assert!(alice_teams.iter().any(|t| t.id == team_a.id));
        assert!(alice_teams.iter().all(|t| t.id != team_b.id));
        Ok(())
    }

    #[sqlx::test]
    async fn rename_and_delete(pool: PgPool) -> sqlx::Result<()> {
        let default_team = users::ensure_default_team(&pool).await.unwrap();
        let owner_id = seed_user(&pool, "carol2@example.com", "Carol", default_team).await;
        let team = create(&pool, "Old Name", owner_id).await.unwrap();

        rename(&pool, team.id, "New Name").await.unwrap();
        let fetched = get(&pool, team.id).await.unwrap();
        assert_eq!(fetched.name, "New Name");

        delete(&pool, team.id).await.unwrap();
        assert!(matches!(
            get(&pool, team.id).await,
            Err(DbError::NotFound("team"))
        ));

        let remaining_members = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM users_teams WHERE team_id = $1",
            team.id
        )
        .fetch_one(&pool)
        .await?;
        assert_eq!(remaining_members.unwrap_or(-1), 0);
        Ok(())
    }

    #[sqlx::test]
    async fn invite_existing_user_and_accept(pool: PgPool) -> sqlx::Result<()> {
        let default_team = users::ensure_default_team(&pool).await.unwrap();
        let owner_id = seed_user(&pool, "dave2@example.com", "Dave", default_team).await;
        let invitee_id = seed_user(&pool, "erin2@example.com", "Erin", default_team).await;
        let team = create(&pool, "Invite Team", owner_id).await.unwrap();

        let invitation = create_invitation_for_user(&pool, team.id, TeamRole::Admin, invitee_id)
            .await
            .unwrap();
        assert_eq!(invitation.team_id, team.id);
        assert_eq!(invitation.user_id, Some(invitee_id));

        let pending = list_pending_invitations(&pool, invitee_id, "erin2@example.com")
            .await
            .unwrap();
        assert_eq!(pending.len(), 1);

        let pending_with_team =
            list_pending_invitations_with_team_name(&pool, invitee_id, "erin2@example.com")
                .await
                .unwrap();
        assert_eq!(pending_with_team.len(), 1);
        assert_eq!(pending_with_team[0].team_name, "Invite Team");

        accept_invitation(&pool, invitation.id, invitee_id)
            .await
            .unwrap();

        assert_eq!(
            member_role(&pool, team.id, invitee_id).await.unwrap(),
            Some(TeamRole::Admin)
        );
        assert!(matches!(
            get_invitation(&pool, invitation.id).await,
            Err(DbError::NotFound("invitation"))
        ));
        Ok(())
    }

    #[sqlx::test]
    async fn invite_by_email_then_decline(pool: PgPool) -> sqlx::Result<()> {
        let default_team = users::ensure_default_team(&pool).await.unwrap();
        let owner_id = seed_user(&pool, "frank@example.com", "Frank", default_team).await;
        let team = create(&pool, "Email Invite Team", owner_id).await.unwrap();

        let invitation =
            create_invitation_for_email(&pool, team.id, TeamRole::Member, "grace@example.com")
                .await
                .unwrap();
        assert_eq!(invitation.email, Some("grace@example.com".to_string()));

        let for_team = list_invitations_for_team(&pool, team.id).await.unwrap();
        assert_eq!(for_team.len(), 1);

        delete_invitation(&pool, invitation.id).await.unwrap();
        assert!(
            list_invitations_for_team(&pool, team.id)
                .await
                .unwrap()
                .is_empty()
        );
        Ok(())
    }
}

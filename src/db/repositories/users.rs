//! `users` / `users_teams` / `user_login_attempts` / `teams`.

use super::DbError;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use sqlx::postgres::PgExecutor;
use uuid::Uuid;

/// A user's role within a team, backed by the `team_role` Postgres enum.
#[derive(Debug, sqlx::Type, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[sqlx(type_name = "team_role", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum TeamRole {
    Owner,
    Admin,
    Member,
}

impl TeamRole {
    /// Privilege rank, higher is more privileged. Explicit rather than a derived `Ord`, which
    /// would follow declaration order and make `Owner` the smallest.
    fn rank(self) -> u8 {
        match self {
            TeamRole::Owner => 3,
            TeamRole::Admin => 2,
            TeamRole::Member => 1,
        }
    }

    /// Whether this role is at least as privileged as `min` (Owner > Admin > Member).
    pub fn at_least(self, min: TeamRole) -> bool {
        self.rank() >= min.rank()
    }
}

/// A user account.
#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub name: String,
    #[serde(skip_serializing)]
    pub password_hash: Option<String>,
    pub locked_until: Option<DateTime<Utc>>,
    pub password_reset_token_hash: Option<String>,
    pub password_reset_token_valid_until: Option<DateTime<Utc>>,
}

// Public view of a user account
#[derive(Debug, Clone, serde::Serialize)]
pub struct UserProfile {
    pub id: Uuid,
    pub name: String,
}

/// Name identifying the shared default team (the SQL in [`ensure_default_team`] hard-codes it).
pub const DEFAULT_TEAM_NAME: &str = "Default";

/// Returns the id of the single "Default" team, creating it if it doesn't exist yet.
///
/// Every user/project/folder created by the live app (as opposed to the legacy bincode
/// migration, which creates its own team) is attached to this team as owner, preserving
/// today's single-tenant "everyone sees everything" behavior.
pub async fn ensure_default_team(pool: &PgPool) -> Result<Uuid, DbError> {
    if let Some(id) = sqlx::query_scalar!("SELECT id FROM teams WHERE name = 'Default' LIMIT 1")
        .fetch_optional(pool)
        .await?
    {
        return Ok(id);
    }

    let id = sqlx::query_scalar!("INSERT INTO teams (name) VALUES ('Default') RETURNING id")
        .fetch_one(pool)
        .await?;
    Ok(id)
}

/// Fetches a user by email.
pub async fn find_by_email<'e>(exec: impl PgExecutor<'e>, email: &str) -> Result<User, DbError> {
    sqlx::query_as!(
        User,
        "SELECT id, email, name, password_hash, locked_until, password_reset_token_hash, password_reset_token_valid_until FROM users WHERE LOWER(email) = LOWER($1)",
        email
    )
    .fetch_optional(exec)
    .await?
    .ok_or(DbError::NotFound("user"))
}

/// Fetches a user by id.
pub async fn get<'e>(exec: impl PgExecutor<'e>, id: Uuid) -> Result<User, DbError> {
    sqlx::query_as!(
        User,
        "SELECT id, email, name, password_hash, locked_until, password_reset_token_hash, password_reset_token_valid_until FROM users WHERE id = $1",
        id
    )
    .fetch_optional(exec)
    .await?
    .ok_or(DbError::NotFound("user"))
}

/// Whether the user has verified control of their current email address.
pub async fn is_email_verified<'e>(exec: impl PgExecutor<'e>, id: Uuid) -> Result<bool, DbError> {
    sqlx::query_scalar!("SELECT email_verified FROM users WHERE id = $1", id)
        .fetch_optional(exec)
        .await?
        .ok_or(DbError::NotFound("user"))
}

/// Stores a pending email verification token (already hashed) for the user.
pub async fn set_email_verification_token<'e>(
    exec: impl PgExecutor<'e>,
    id: Uuid,
    token_hash: &str,
    valid_until: DateTime<Utc>,
) -> Result<(), DbError> {
    let result = sqlx::query!(
        "UPDATE users SET email_verification_token_hash = $2, email_verification_token_valid_until = $3 WHERE id = $1",
        id,
        token_hash,
        valid_until
    )
    .execute(exec)
    .await?;
    if result.rows_affected() == 0 {
        return Err(DbError::NotFound("user"));
    }
    Ok(())
}

/// Returns the pending verification token hash and its expiry, if any.
pub async fn get_email_verification_token<'e>(
    exec: impl PgExecutor<'e>,
    id: Uuid,
) -> Result<Option<(String, DateTime<Utc>)>, DbError> {
    let row = sqlx::query!(
        "SELECT email_verification_token_hash, email_verification_token_valid_until FROM users WHERE id = $1",
        id
    )
    .fetch_optional(exec)
    .await?
    .ok_or(DbError::NotFound("user"))?;
    Ok(row
        .email_verification_token_hash
        .zip(row.email_verification_token_valid_until))
}

/// Marks the user's email as verified and discards the pending token.
pub async fn mark_email_verified<'e>(exec: impl PgExecutor<'e>, id: Uuid) -> Result<(), DbError> {
    sqlx::query!(
        "UPDATE users SET email_verified = true, email_verification_token_hash = NULL, email_verification_token_valid_until = NULL WHERE id = $1",
        id
    )
    .execute(exec)
    .await?;
    Ok(())
}

/// Checks whether `email` is already taken by another user, optionally excluding one id
/// (for "is this still free if I keep my own current email" checks during profile updates).
pub async fn email_in_use<'e>(
    exec: impl PgExecutor<'e>,
    email: &str,
    exclude_id: Option<Uuid>,
) -> Result<bool, DbError> {
    let in_use = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM users WHERE LOWER(email) = LOWER($1) AND id IS DISTINCT FROM $2)",
        email,
        exclude_id
    )
    .fetch_one(exec)
    .await?;
    Ok(in_use.unwrap_or(false))
}

/// Creates the user's personal team and makes them its `owner`, inside the caller's
/// transaction. A personal team is an ordinary team that merely starts out with a single
/// member; it's named after the user. `'Default'` is reserved for the shared default team
/// (looked up by name in [`ensure_default_team`]), so a user with that exact name gets a
/// distinguishable team name instead.
pub async fn create_personal_team(
    tx: &mut sqlx::PgConnection,
    user_id: Uuid,
    user_name: &str,
) -> Result<Uuid, DbError> {
    let team_name = if user_name == DEFAULT_TEAM_NAME {
        "Default (personal)"
    } else {
        user_name
    };
    let team_id = sqlx::query_scalar!(
        "INSERT INTO teams (name) VALUES ($1) RETURNING id",
        team_name
    )
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query!(
        "INSERT INTO users_teams (user_id, team_id, role) VALUES ($1, $2, $3)",
        user_id,
        team_id,
        TeamRole::Owner as TeamRole
    )
    .execute(&mut *tx)
    .await?;
    Ok(team_id)
}

/// Inserts a new user, attaches them to `default_team_id` as `owner` and creates their
/// personal team, in one transaction.
pub async fn insert(
    pool: &PgPool,
    email: &str,
    name: &str,
    password_hash: &str,
    default_team_id: Uuid,
) -> Result<User, DbError> {
    insert_with_verification(pool, email, name, password_hash, default_team_id, true).await
}

/// Like [`insert`], but lets the caller state whether the email address is verified. Only
/// verified addresses are matched against pending email invitations.
pub async fn insert_with_verification(
    pool: &PgPool,
    email: &str,
    name: &str,
    password_hash: &str,
    default_team_id: Uuid,
    email_verified: bool,
) -> Result<User, DbError> {
    let mut tx = pool.begin().await?;

    let user = sqlx::query_as!(
        User,
        "INSERT INTO users (email, name, password_hash, email_verified) VALUES ($1, $2, $3, $4)
         RETURNING id, email, name, password_hash, locked_until, password_reset_token_hash, password_reset_token_valid_until",
        email,
        name,
        password_hash,
        email_verified
    )
    .fetch_one(&mut *tx)
    .await?;

    sqlx::query!(
        "INSERT INTO users_teams (user_id, team_id, role) VALUES ($1, $2, $3)",
        user.id,
        default_team_id,
        TeamRole::Owner as TeamRole
    )
    .execute(&mut *tx)
    .await?;

    create_personal_team(&mut tx, user.id, &user.name).await?;

    tx.commit().await?;
    Ok(user)
}

/// Overwrites every field of the user with id `user.id` (except the id itself) with the
/// values on `user`. Callers that want a partial update should first `get` the current row,
/// mutate the fields they care about, and pass the result in here.
pub async fn update<'e>(exec: impl PgExecutor<'e>, user: &User) -> Result<User, DbError> {
    sqlx::query_as!(
        User,
        "UPDATE users SET
            email_verified = (email_verified AND LOWER(email) = LOWER($2)),
            email = $2,
            name = $3,
            password_hash = $4,
            locked_until = $5,
            password_reset_token_hash = $6,
            password_reset_token_valid_until = $7
         WHERE id = $1
         RETURNING id, email, name, password_hash, locked_until, password_reset_token_hash, password_reset_token_valid_until",
        user.id,
        user.email,
        user.name,
        user.password_hash,
        user.locked_until,
        user.password_reset_token_hash,
        user.password_reset_token_valid_until
    )
    .fetch_optional(exec)
    .await?
    .ok_or(DbError::NotFound("user"))
}

/// Deletes a user by id, together with every team they are the last member of (and everything
/// those teams own), so no team is left behind that nobody can reach. Teams that have other
/// members are kept. Fails with [`DbError::NotFound`] if no row with that id exists.
pub async fn delete(pool: &PgPool, id: Uuid) -> Result<(), DbError> {
    let mut tx = pool.begin().await?;
    sqlx::query!(
        "DELETE FROM teams
         WHERE id IN (SELECT team_id FROM users_teams WHERE user_id = $1)
           AND NOT EXISTS (
               SELECT 1 FROM users_teams WHERE team_id = teams.id AND user_id <> $1
           )",
        id
    )
    .execute(&mut *tx)
    .await?;
    let result = sqlx::query!("DELETE FROM users WHERE id = $1", id)
        .execute(&mut *tx)
        .await?;
    if result.rows_affected() == 0 {
        return Err(DbError::NotFound("user"));
    }
    tx.commit().await?;
    Ok(())
}

/// Records a failed login attempt and returns the number of attempts within the last
/// `lockout_window_minutes` minutes (a self-healing rolling window, replacing the old
/// bincode `Vec<u64>` that had to be explicitly reset).
pub async fn record_failed_attempt(
    pool: &PgPool,
    user_id: Uuid,
    lockout_window_minutes: i64,
) -> Result<i64, DbError> {
    sqlx::query!(
        "INSERT INTO user_login_attempts (user_id) VALUES ($1)",
        user_id
    )
    .execute(pool)
    .await?;

    let count = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM user_login_attempts
         WHERE user_id = $1 AND timestamp > now() - make_interval(mins => $2::int)",
        user_id,
        lockout_window_minutes as i32
    )
    .fetch_one(pool)
    .await?;

    Ok(count.unwrap_or(0))
}

/// Sets `locked_until` on a user's account.
pub async fn lock_until<'e>(
    exec: impl PgExecutor<'e>,
    user_id: Uuid,
    until: DateTime<Utc>,
) -> Result<(), DbError> {
    sqlx::query!(
        "UPDATE users SET locked_until = $2 WHERE id = $1",
        user_id,
        until
    )
    .execute(exec)
    .await?;
    Ok(())
}

/// Clears any lockout and forgets past failed attempts (mirrors the old code resetting
/// both `locked_until` and `login_attempts` together on successful login / lock expiry).
pub async fn clear_lockout(pool: &PgPool, user_id: Uuid) -> Result<(), DbError> {
    let mut tx = pool.begin().await?;
    sqlx::query!(
        "UPDATE users SET locked_until = NULL WHERE id = $1",
        user_id
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "DELETE FROM user_login_attempts WHERE user_id = $1",
        user_id
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[sqlx::test]
    async fn insert_and_find_by_email_round_trip(pool: PgPool) -> sqlx::Result<()> {
        let team_id = ensure_default_team(&pool).await.unwrap();
        let user = insert(&pool, "alice@example.com", "Alice", "hash", team_id)
            .await
            .unwrap();

        let found = find_by_email(&pool, "alice@example.com").await.unwrap();
        assert_eq!(found.id, user.id);
        assert_eq!(found.name, "Alice");

        let role: TeamRole = sqlx::query_scalar!(
            r#"SELECT role as "role: TeamRole" FROM users_teams WHERE user_id = $1"#,
            user.id
        )
        .fetch_one(&pool)
        .await?;
        assert_eq!(role, TeamRole::Owner);
        Ok(())
    }

    #[sqlx::test]
    async fn email_in_use_respects_exclude(pool: PgPool) -> sqlx::Result<()> {
        let team_id = ensure_default_team(&pool).await.unwrap();
        let user = insert(&pool, "bob@example.com", "Bob", "hash", team_id)
            .await
            .unwrap();

        assert!(email_in_use(&pool, "bob@example.com", None).await.unwrap());
        assert!(
            !email_in_use(&pool, "bob@example.com", Some(user.id))
                .await
                .unwrap()
        );
        assert!(
            !email_in_use(&pool, "nobody@example.com", None)
                .await
                .unwrap()
        );
        Ok(())
    }

    #[sqlx::test]
    async fn lockout_flow(pool: PgPool) -> sqlx::Result<()> {
        let team_id = ensure_default_team(&pool).await.unwrap();
        let user = insert(&pool, "carol@example.com", "Carol", "hash", team_id)
            .await
            .unwrap();

        let settings = crate::session::login::integration_tests::test_settings();
        let max_login_attempts = settings.max_login_attempts;
        let lockout_window_minutes = settings.lockout_window_minutes;

        let mut last_count = 0;
        for _ in 0..max_login_attempts {
            last_count = record_failed_attempt(&pool, user.id, lockout_window_minutes)
                .await
                .unwrap();
        }
        assert_eq!(last_count, max_login_attempts);

        lock_until(
            &pool,
            user.id,
            Utc::now() + chrono::Duration::minutes(lockout_window_minutes),
        )
        .await
        .unwrap();
        let locked = get(&pool, user.id).await.unwrap();
        assert!(locked.locked_until.is_some());

        clear_lockout(&pool, user.id).await.unwrap();
        let cleared = get(&pool, user.id).await.unwrap();
        assert!(cleared.locked_until.is_none());

        let remaining_attempts = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM user_login_attempts WHERE user_id = $1",
            user.id
        )
        .fetch_one(&pool)
        .await?;
        assert_eq!(remaining_attempts.unwrap_or(-1), 0);
        Ok(())
    }

    #[sqlx::test]
    async fn update_overwrites_all_fields(pool: PgPool) -> sqlx::Result<()> {
        let team_id = ensure_default_team(&pool).await.unwrap();
        let user = insert(&pool, "erin@example.com", "Erin", "hash", team_id)
            .await
            .unwrap();

        let mut changed = user.clone();
        changed.email = "erin2@example.com".to_string();
        changed.name = "Erin Changed".to_string();
        changed.password_hash = Some("new-hash".to_string());
        changed.locked_until = Some(Utc::now());
        changed.password_reset_token_hash = Some("reset-hash".to_string());
        changed.password_reset_token_valid_until = Some(Utc::now());

        let updated = update(&pool, &changed).await.unwrap();
        assert_eq!(updated.id, user.id);
        assert_eq!(updated.email, "erin2@example.com");
        assert_eq!(updated.name, "Erin Changed");
        assert_eq!(updated.password_hash, Some("new-hash".to_string()));
        assert!(updated.locked_until.is_some());
        assert_eq!(
            updated.password_reset_token_hash,
            Some("reset-hash".to_string())
        );
        assert!(updated.password_reset_token_valid_until.is_some());

        // Clearing an Option field back to None must also stick.
        changed.locked_until = None;
        changed.password_reset_token_hash = None;
        changed.password_reset_token_valid_until = None;
        let cleared = update(&pool, &changed).await.unwrap();
        assert!(cleared.locked_until.is_none());
        assert!(cleared.password_reset_token_hash.is_none());
        assert!(cleared.password_reset_token_valid_until.is_none());
        Ok(())
    }

    #[sqlx::test]
    async fn delete_cascades_team_membership(pool: PgPool) -> sqlx::Result<()> {
        let team_id = ensure_default_team(&pool).await.unwrap();
        let user = insert(&pool, "dave@example.com", "Dave", "hash", team_id)
            .await
            .unwrap();

        delete(&pool, user.id).await.unwrap();

        let remaining = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM users_teams WHERE user_id = $1",
            user.id
        )
        .fetch_one(&pool)
        .await?;
        assert_eq!(remaining.unwrap_or(-1), 0);

        assert!(matches!(
            get(&pool, user.id).await,
            Err(DbError::NotFound("user"))
        ));
        Ok(())
    }

    #[sqlx::test]
    async fn delete_removes_teams_the_user_was_last_member_of(pool: PgPool) -> sqlx::Result<()> {
        let default_team = ensure_default_team(&pool).await.unwrap();
        let alice = insert(&pool, "alice@example.com", "Alice", "hash", default_team)
            .await
            .unwrap();
        let bob = insert(&pool, "bob@example.com", "Bob", "hash", default_team)
            .await
            .unwrap();
        let team_of = |name: &'static str| {
            let pool = pool.clone();
            async move {
                sqlx::query_scalar!("SELECT id FROM teams WHERE name = $1", name)
                    .fetch_optional(&pool)
                    .await
                    .unwrap()
            }
        };
        let alice_team = team_of("Alice").await.unwrap();
        let bob_team = team_of("Bob").await.unwrap();
        // a project in Alice's team goes away with it
        let project = sqlx::query_scalar!(
            "INSERT INTO projects (title, owner_team_id) VALUES ('p', $1) RETURNING id",
            alice_team
        )
        .fetch_one(&pool)
        .await?;
        // Alice also joined Bob's team
        sqlx::query!(
            "INSERT INTO users_teams (user_id, team_id, role) VALUES ($1, $2, $3)",
            alice.id,
            bob_team,
            TeamRole::Member as TeamRole
        )
        .execute(&pool)
        .await?;

        delete(&pool, alice.id).await.unwrap();
        assert!(team_of("Alice").await.is_none());
        let project_left =
            sqlx::query_scalar!("SELECT COUNT(*) FROM projects WHERE id = $1", project)
                .fetch_one(&pool)
                .await?;
        assert_eq!(project_left, Some(0));
        // teams that still have members survive
        assert_eq!(team_of("Default").await, Some(default_team));
        assert_eq!(team_of("Bob").await, Some(bob_team));

        // Bob was the last member of his team and of the Default team: both go
        delete(&pool, bob.id).await.unwrap();
        assert!(team_of("Bob").await.is_none());
        assert!(team_of("Default").await.is_none());
        Ok(())
    }

    #[sqlx::test]
    async fn insert_creates_a_personal_team_owned_by_the_user(pool: PgPool) -> sqlx::Result<()> {
        let default_team = ensure_default_team(&pool).await.unwrap();
        let alice = insert(&pool, "alice@example.com", "Alice", "hash", default_team)
            .await
            .unwrap();
        // a second user, to show their personal team isn't shared
        insert(&pool, "bob@example.com", "Bob", "hash", default_team)
            .await
            .unwrap();

        let teams: Vec<(Uuid, String, TeamRole)> = sqlx::query_as(
            "SELECT t.id, t.name, ut.role FROM users_teams ut JOIN teams t ON t.id = ut.team_id
             WHERE ut.user_id = $1 AND t.id <> $2",
        )
        .bind(alice.id)
        .bind(default_team)
        .fetch_all(&pool)
        .await?;
        assert_eq!(teams.len(), 1);
        assert_eq!(teams[0].1, "Alice");
        assert_eq!(teams[0].2, TeamRole::Owner);

        // the personal team is Alice's alone
        let members = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM users_teams WHERE team_id = $1",
            teams[0].0
        )
        .fetch_one(&pool)
        .await?;
        assert_eq!(members, Some(1));
        Ok(())
    }

    #[sqlx::test]
    async fn personal_team_never_takes_the_default_team_name(pool: PgPool) -> sqlx::Result<()> {
        let default_team = ensure_default_team(&pool).await.unwrap();
        insert(&pool, "d@example.com", "Default", "hash", default_team)
            .await
            .unwrap();

        // the by-name lookup must keep finding the original shared team
        assert_eq!(ensure_default_team(&pool).await.unwrap(), default_team);
        let named_default =
            sqlx::query_scalar!("SELECT COUNT(*) FROM teams WHERE name = 'Default'")
                .fetch_one(&pool)
                .await?;
        assert_eq!(named_default, Some(1));
        Ok(())
    }
}

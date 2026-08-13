//! `users` / `users_teams` / `user_login_attempts` / `teams`.

use super::DbError;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use sqlx::postgres::PgExecutor;
use uuid::Uuid;

/// A user's role within a team, backed by the `team_role` Postgres enum.
#[derive(Debug, sqlx::Type, Clone, Copy, PartialEq, Eq)]
#[sqlx(type_name = "team_role", rename_all = "lowercase")]
pub enum TeamRole {
    Owner,
    Admin,
    Member,
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
}

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
        "SELECT id, email, name, password_hash, locked_until FROM users WHERE email = $1",
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
        "SELECT id, email, name, password_hash, locked_until FROM users WHERE id = $1",
        id
    )
    .fetch_optional(exec)
    .await?
    .ok_or(DbError::NotFound("user"))
}

/// Returns every user, ordered by email.
pub async fn list_all<'e>(exec: impl PgExecutor<'e>) -> Result<Vec<User>, DbError> {
    let users = sqlx::query_as!(
        User,
        "SELECT id, email, name, password_hash, locked_until FROM users ORDER BY email"
    )
    .fetch_all(exec)
    .await?;
    Ok(users)
}

/// Checks whether `email` is already taken by another user, optionally excluding one id
/// (for "is this still free if I keep my own current email" checks during profile updates).
pub async fn email_in_use<'e>(
    exec: impl PgExecutor<'e>,
    email: &str,
    exclude_id: Option<Uuid>,
) -> Result<bool, DbError> {
    let in_use = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM users WHERE email = $1 AND id IS DISTINCT FROM $2)",
        email,
        exclude_id
    )
    .fetch_one(exec)
    .await?;
    Ok(in_use.unwrap_or(false))
}

/// Inserts a new user and attaches them to `default_team_id` as `owner`, in one transaction.
pub async fn insert(
    pool: &PgPool,
    email: &str,
    name: &str,
    password_hash: &str,
    default_team_id: Uuid,
) -> Result<User, DbError> {
    let mut tx = pool.begin().await?;

    let user = sqlx::query_as!(
        User,
        "INSERT INTO users (email, name, password_hash) VALUES ($1, $2, $3)
         RETURNING id, email, name, password_hash, locked_until",
        email,
        name,
        password_hash
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

    tx.commit().await?;
    Ok(user)
}

/// Updates whichever of email/name/password_hash are `Some`, leaving the rest unchanged.
pub async fn update_profile<'e>(
    exec: impl PgExecutor<'e>,
    id: Uuid,
    email: Option<&str>,
    name: Option<&str>,
    password_hash: Option<&str>,
) -> Result<User, DbError> {
    sqlx::query_as!(
        User,
        "UPDATE users SET
            email = COALESCE($2, email),
            name = COALESCE($3, name),
            password_hash = COALESCE($4, password_hash)
         WHERE id = $1
         RETURNING id, email, name, password_hash, locked_until",
        id,
        email,
        name,
        password_hash
    )
    .fetch_optional(exec)
    .await?
    .ok_or(DbError::NotFound("user"))
}

/// Deletes a user by id. Fails with [`DbError::NotFound`] if no row with that id exists.
pub async fn delete<'e>(exec: impl PgExecutor<'e>, id: Uuid) -> Result<(), DbError> {
    let result = sqlx::query!("DELETE FROM users WHERE id = $1", id)
        .execute(exec)
        .await?;
    if result.rows_affected() == 0 {
        return Err(DbError::NotFound("user"));
    }
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

        const MAX_LOGIN_ATTEMPTS: i64 = 5;
        const LOCKOUT_WINDOW_MINUTES: i64 = 15;

        let mut last_count = 0;
        for _ in 0..MAX_LOGIN_ATTEMPTS {
            last_count = record_failed_attempt(&pool, user.id, LOCKOUT_WINDOW_MINUTES)
                .await
                .unwrap();
        }
        assert_eq!(last_count, MAX_LOGIN_ATTEMPTS);

        lock_until(&pool, user.id, Utc::now() + chrono::Duration::minutes(15))
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
}

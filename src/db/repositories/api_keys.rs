//! `api_keys` — bearer credentials a user can generate for themselves to authenticate
//! against future API/rendering endpoints. No scoping/permissions yet, see the module's
//! creation task.
//!
//! `ApiKey` never carries `key_hash`, so it's safe to serialize straight back to the
//! client for listing — the secret itself is only ever known at creation time (see
//! [`crate::profile_settings::api_keys::create_api_key`]) or, briefly, while [`verify`]
//! checks it against `ApiKeyWithHash`, a private row type that never leaves this module.
//! Because of that, queries here are runtime-checked with plain `sqlx::query_as`
//! (mirroring `teams::Team`) rather than the compile-time-checked `query_as!` macro, whose
//! column list would otherwise have to match each query's struct exactly.

use super::DbError;
use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordVerifier};
use chrono::{DateTime, Utc};
use sqlx::postgres::PgExecutor;
use uuid::Uuid;

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct ApiKey {
    pub id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub key_prefix: String,
    pub created_at: DateTime<Utc>,
}

/// A raw `api_keys` row including its hash. Unlike [`ApiKey`], never derives `Serialize`
/// and never leaves this module — it exists only so [`verify`] can read `key_hash` without
/// widening any public type to carry it.
#[derive(sqlx::FromRow)]
struct ApiKeyWithHash {
    id: Uuid,
    user_id: Uuid,
    name: String,
    key_prefix: String,
    created_at: DateTime<Utc>,
    key_hash: String,
}

/// Builds the full key string returned once, at creation time, from its `prefix` and
/// `secret` parts — the exact inverse of [`split_key`]. Kept alongside `split_key` (rather
/// than inlined at the call site in the creation route) so the two can never drift apart.
pub fn format_key(prefix: &str, secret: &str) -> String {
    format!("vb_{}_{}", prefix, secret)
}

/// Splits a raw API key of the form `vb_<prefix>_<secret>` into its prefix and secret
/// parts. Returns `None` if the key doesn't have that shape at all (missing the `vb_` tag,
/// or no `_` left to split prefix from secret) — already enough to know the key is invalid
/// without touching the database. Alphanumeric prefixes/secrets never contain `_`
/// themselves (see the creation route), so `split_once` always splits in the right place.
fn split_key(key: &str) -> Option<(&str, &str)> {
    key.strip_prefix("vb_")?.split_once('_')
}

/// Creates a new API key for `user_id`. `key_hash` must already be a password-hash-style
/// digest of the key's secret part (see the creation route) — never the raw key.
pub async fn create<'e>(
    exec: impl PgExecutor<'e>,
    user_id: Uuid,
    name: &str,
    key_prefix: &str,
    key_hash: &str,
) -> Result<ApiKey, DbError> {
    let api_key: ApiKey = sqlx::query_as(
        "INSERT INTO api_keys (user_id, name, key_prefix, key_hash) VALUES ($1, $2, $3, $4)
         RETURNING id, user_id, name, key_prefix, created_at",
    )
    .bind(user_id)
    .bind(name)
    .bind(key_prefix)
    .bind(key_hash)
    .fetch_one(exec)
    .await?;
    Ok(api_key)
}

/// Returns every API key belonging to `user_id`, oldest first.
pub async fn list_for_user<'e>(
    exec: impl PgExecutor<'e>,
    user_id: Uuid,
) -> Result<Vec<ApiKey>, DbError> {
    let keys: Vec<ApiKey> = sqlx::query_as(
        "SELECT id, user_id, name, key_prefix, created_at FROM api_keys WHERE user_id = $1 ORDER BY created_at",
    )
    .bind(user_id)
    .fetch_all(exec)
    .await?;
    Ok(keys)
}

/// Deletes an API key, scoped to `user_id` so a user can never delete someone else's key —
/// this is the only permission check the route needs, mirroring how
/// `teams::remove_member`/`teams::delete` scope their `WHERE` clauses instead of checking
/// ownership separately. Fails with [`DbError::NotFound`] if no key with that id, owned by
/// that user, exists (indistinguishable from the id simply not existing, so a request for
/// someone else's key 404s instead of leaking that it exists).
pub async fn delete<'e>(exec: impl PgExecutor<'e>, id: Uuid, user_id: Uuid) -> Result<(), DbError> {
    let result = sqlx::query("DELETE FROM api_keys WHERE id = $1 AND user_id = $2")
        .bind(id)
        .bind(user_id)
        .execute(exec)
        .await?;
    if result.rows_affected() == 0 {
        return Err(DbError::NotFound("api key"));
    }
    Ok(())
}

/// Verifies a raw API key (as returned once by [`create`]'s caller, via [`format_key`]) and
/// returns the matching [`ApiKey`] on success.
///
/// Looks the key up by its plaintext `key_prefix` first — an indexed equality lookup, since
/// the column is `UNIQUE` — rather than hashing the secret against every row in the table,
/// then verifies the secret against that one row's Argon2 hash. So checking a key costs one
/// lookup plus one hash, regardless of how many keys exist.
///
/// Returns `Ok(None)` for "this key isn't valid" (malformed, unknown prefix, or a secret
/// that doesn't match the stored hash) instead of an error — an invalid key is an expected
/// outcome for an auth check, not a failure, exactly like a wrong password at login (see
/// `session::login::process_login_form`). Callers should treat `None` as unauthenticated.
/// Only an actual database error becomes `Err`.
pub async fn verify<'e>(exec: impl PgExecutor<'e>, key: &str) -> Result<Option<ApiKey>, DbError> {
    let Some((prefix, secret)) = split_key(key) else {
        return Ok(None);
    };

    let row: Option<ApiKeyWithHash> = sqlx::query_as(
        "SELECT id, user_id, name, key_prefix, created_at, key_hash FROM api_keys WHERE key_prefix = $1",
    )
    .bind(prefix)
    .fetch_optional(exec)
    .await?;

    let Some(row) = row else {
        return Ok(None);
    };

    let Ok(parsed_hash) = PasswordHash::new(&row.key_hash) else {
        return Ok(None);
    };
    if Argon2::default()
        .verify_password(secret.as_bytes(), &parsed_hash)
        .is_err()
    {
        return Ok(None);
    }

    Ok(Some(ApiKey {
        id: row.id,
        user_id: row.user_id,
        name: row.name,
        key_prefix: row.key_prefix,
        created_at: row.created_at,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::repositories::users;
    use argon2::PasswordHasher;
    use argon2::password_hash::{SaltString, rand_core::OsRng};
    use sqlx::PgPool;

    async fn seed_user(pool: &PgPool, email: &str, name: &str, team_id: Uuid) -> Uuid {
        users::insert(pool, email, name, "hash", team_id)
            .await
            .unwrap()
            .id
    }

    /// Creates an API key the same way the creation route does: a random-in-spirit
    /// prefix/secret pair, with `secret` Argon2-hashed before storage. Returns the full
    /// `vb_<prefix>_<secret>` key alongside the created row, so tests can call `verify`
    /// with exactly what a real client would send.
    async fn seed_key(
        pool: &PgPool,
        user_id: Uuid,
        name: &str,
        prefix: &str,
        secret: &str,
    ) -> (ApiKey, String) {
        let salt = SaltString::generate(&mut OsRng);
        let key_hash = Argon2::default()
            .hash_password(secret.as_bytes(), &salt)
            .unwrap()
            .to_string();
        let api_key = create(pool, user_id, name, prefix, &key_hash)
            .await
            .unwrap();
        (api_key, format_key(prefix, secret))
    }

    #[sqlx::test]
    async fn create_and_list(pool: PgPool) -> sqlx::Result<()> {
        let default_team = users::ensure_default_team(&pool).await.unwrap();
        let user_id = seed_user(&pool, "alice@example.com", "Alice", default_team).await;

        let key = create(&pool, user_id, "My Key", "abcd1234", "some-hash")
            .await
            .unwrap();
        assert_eq!(key.name, "My Key");
        assert_eq!(key.key_prefix, "abcd1234");

        let keys = list_for_user(&pool, user_id).await.unwrap();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].id, key.id);
        Ok(())
    }

    #[sqlx::test]
    async fn list_only_returns_own_keys(pool: PgPool) -> sqlx::Result<()> {
        let default_team = users::ensure_default_team(&pool).await.unwrap();
        let alice = seed_user(&pool, "alice2@example.com", "Alice", default_team).await;
        let bob = seed_user(&pool, "bob2@example.com", "Bob", default_team).await;

        create(&pool, alice, "Alice's Key", "prefix1a", "hash1")
            .await
            .unwrap();
        create(&pool, bob, "Bob's Key", "prefix1b", "hash2")
            .await
            .unwrap();

        let alice_keys = list_for_user(&pool, alice).await.unwrap();
        assert_eq!(alice_keys.len(), 1);
        assert_eq!(alice_keys[0].name, "Alice's Key");
        Ok(())
    }

    #[sqlx::test]
    async fn delete_is_scoped_to_owner(pool: PgPool) -> sqlx::Result<()> {
        let default_team = users::ensure_default_team(&pool).await.unwrap();
        let alice = seed_user(&pool, "alice3@example.com", "Alice", default_team).await;
        let bob = seed_user(&pool, "bob3@example.com", "Bob", default_team).await;

        let key = create(&pool, alice, "Alice's Key", "prefix2a", "hash1")
            .await
            .unwrap();

        // Bob can't delete Alice's key.
        assert!(matches!(
            delete(&pool, key.id, bob).await,
            Err(DbError::NotFound("api key"))
        ));
        assert_eq!(list_for_user(&pool, alice).await.unwrap().len(), 1);

        // Alice can.
        delete(&pool, key.id, alice).await.unwrap();
        assert_eq!(list_for_user(&pool, alice).await.unwrap().len(), 0);
        Ok(())
    }

    #[sqlx::test]
    async fn verify_succeeds_for_the_matching_key(pool: PgPool) -> sqlx::Result<()> {
        let default_team = users::ensure_default_team(&pool).await.unwrap();
        let alice = seed_user(&pool, "alice4@example.com", "Alice", default_team).await;
        let (api_key, key) = seed_key(&pool, alice, "My Key", "abcd1234", "s3cret-value").await;

        let verified = verify(&pool, &key).await.unwrap();
        assert_eq!(verified.map(|k| k.id), Some(api_key.id));
        Ok(())
    }

    #[sqlx::test]
    async fn verify_rejects_a_tampered_secret(pool: PgPool) -> sqlx::Result<()> {
        let default_team = users::ensure_default_team(&pool).await.unwrap();
        let alice = seed_user(&pool, "alice5@example.com", "Alice", default_team).await;
        seed_key(&pool, alice, "My Key", "abcd1235", "s3cret-value").await;

        // Right prefix, wrong secret -- must not verify just because the row was found.
        let tampered = format_key("abcd1235", "wrong-secret");
        assert!(verify(&pool, &tampered).await.unwrap().is_none());
        Ok(())
    }

    #[sqlx::test]
    async fn verify_rejects_an_unknown_prefix(pool: PgPool) -> sqlx::Result<()> {
        let default_team = users::ensure_default_team(&pool).await.unwrap();
        let alice = seed_user(&pool, "alice6@example.com", "Alice", default_team).await;
        seed_key(&pool, alice, "My Key", "abcd1236", "s3cret-value").await;

        let unknown = format_key("zzzzzzzz", "s3cret-value");
        assert!(verify(&pool, &unknown).await.unwrap().is_none());
        Ok(())
    }

    #[sqlx::test]
    async fn verify_rejects_a_malformed_key(pool: PgPool) -> sqlx::Result<()> {
        // No "vb_" tag at all.
        assert!(
            verify(&pool, "not-a-valid-key-at-all")
                .await
                .unwrap()
                .is_none()
        );
        // Has the tag, but nothing after it to split into prefix/secret.
        assert!(verify(&pool, "vb_missing-secret").await.unwrap().is_none());
        Ok(())
    }
}

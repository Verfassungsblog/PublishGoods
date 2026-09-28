use crate::db::repositories::api_keys as api_keys_repo;
use crate::db::repositories::api_keys::ApiKey;
use crate::session::session_guard::Session;
use crate::utils::api_helpers::{APIResponse, APIResult, ApiError, ApiErrorType};
use argon2::Argon2;
use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHasher, SaltString};
use rand::distr::{Alphanumeric, SampleString};
use rocket::State;
use rocket::serde::json::Json;
use sqlx::PgPool;
use uuid::Uuid;

/// GET /api/api-keys
///
/// Lists every API key belonging to the requesting user. Never includes the key secret —
/// that's only ever returned once, at creation time, by [`create_api_key`].
#[get("/api/api-keys")]
pub async fn list_api_keys(session: Session, pool: &State<PgPool>) -> APIResult<Vec<ApiKey>> {
    let keys = api_keys_repo::list_for_user(pool.inner(), session.user_id).await?;
    Ok(APIResponse::from(keys))
}

/// Request body for [`create_api_key`].
#[derive(Debug, serde::Deserialize)]
pub struct CreateApiKeyData {
    pub name: String,
}

/// Response body for [`create_api_key`]: the created key's metadata, plus `key` — the only
/// time the full secret is ever available, since only its Argon2 hash is stored.
#[derive(Debug, serde::Serialize)]
pub struct CreatedApiKey {
    #[serde(flatten)]
    pub api_key: ApiKey,
    pub key: String,
}

/// POST /api/api-keys
///
/// Creates a new API key for the requesting user. The key is `vb_<prefix>_<secret>`: the
/// 8-character `prefix` is stored in plaintext (it identifies the key in listings and would
/// later let an auth check look up the row without hashing against every key in the table),
/// while the 40-character `secret` is hashed with Argon2 before storage — mirroring how
/// password-reset tokens are hashed in `session::login` — so the raw key can't be recovered
/// from the database, even by an administrator. Requires no particular role; any
/// authenticated user may create keys for themselves. Keys have no scope yet.
#[post("/api/api-keys", data = "<data>")]
pub async fn create_api_key(
    data: Json<CreateApiKeyData>,
    session: Session,
    pool: &State<PgPool>,
) -> APIResult<CreatedApiKey> {
    let name = data.into_inner().name.trim().to_string();
    if name.is_empty() {
        return Err(ApiErrorType::BadRequest("API key name can't be empty".to_string()).into());
    }

    let prefix = Alphanumeric.sample_string(&mut rand::rng(), 8);
    let secret = Alphanumeric.sample_string(&mut rand::rng(), 40);

    let salt = SaltString::generate(&mut OsRng);
    let key_hash = Argon2::default()
        .hash_password(secret.as_bytes(), &salt)
        .map_err(|e| {
            error!("Couldn't hash API key: {}", e);
            ApiError::from(ApiErrorType::InternalServerError)
        })?
        .to_string();

    let api_key =
        api_keys_repo::create(pool.inner(), session.user_id, &name, &prefix, &key_hash).await?;

    Ok(APIResponse::from(CreatedApiKey {
        api_key,
        key: api_keys_repo::format_key(&prefix, &secret),
    }))
}

/// DELETE /api/api-keys/<key_id>
///
/// Deletes an API key. Self-service, scoped to the requesting user's own keys —
/// [`api_keys_repo::delete`]'s `user_id` filter means another user's key id 404s instead of
/// leaking whether it exists.
#[delete("/api/api-keys/<key_id>")]
pub async fn delete_api_key(key_id: &str, session: Session, pool: &State<PgPool>) -> APIResult<()> {
    let key_id = Uuid::parse_str(key_id)?;
    api_keys_repo::delete(pool.inner(), key_id, session.user_id).await?;
    Ok(APIResponse::from(()))
}

/// Runtime HTTP coverage of the per-user scoping, mirroring `teams::integration_tests`.
#[cfg(test)]
mod integration_tests {
    use super::*;
    use crate::db::repositories::users;
    use crate::session::session_storage::SessionStorage;
    use crate::settings::{ExportServer, Settings};
    use argon2::password_hash::rand_core::OsRng as ArgonOsRng;
    use argon2::password_hash::{PasswordHash, PasswordVerifier};
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
                    list_api_keys,
                    create_api_key,
                    delete_api_key,
                ],
            );
        Client::tracked(rocket).await.unwrap()
    }

    async fn seed_user(pool: &PgPool, email: &str, name: &str, default_team: Uuid) -> Uuid {
        let salt = SaltString::generate(&mut ArgonOsRng);
        let hash = Argon2::default()
            .hash_password(b"correct horse", &salt)
            .unwrap()
            .to_string();
        users::insert(pool, email, name, &hash, default_team)
            .await
            .unwrap()
            .id
    }

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
    async fn create_api_key_returns_key_matching_stored_hash(pool: PgPool) -> sqlx::Result<()> {
        let default_team = users::ensure_default_team(&pool).await.unwrap();
        seed_user(&pool, "alice@example.com", "Alice", default_team).await;

        let client = test_client(pool.clone()).await;
        login(&client, "alice@example.com").await;

        let response = client
            .post("/api/api-keys")
            .header(ContentType::JSON)
            .body(r#"{"name":"My Key"}"#)
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::Ok);
        let body: serde_json::Value =
            serde_json::from_str(&response.into_string().await.unwrap()).unwrap();
        let key = body["data"]["key"].as_str().unwrap().to_string();
        let prefix = body["data"]["key_prefix"].as_str().unwrap().to_string();
        assert!(key.starts_with(&format!("vb_{}_", prefix)));

        // The stored hash must verify against the secret part of the returned key, not
        // the full "vb_<prefix>_<secret>" string.
        let secret = key.strip_prefix(&format!("vb_{}_", prefix)).unwrap();
        let key_id = uuid::Uuid::parse_str(body["data"]["id"].as_str().unwrap()).unwrap();
        let stored_hash: String = sqlx::query_scalar("SELECT key_hash FROM api_keys WHERE id = $1")
            .bind(key_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        let parsed = PasswordHash::new(&stored_hash).unwrap();
        assert!(
            Argon2::default()
                .verify_password(secret.as_bytes(), &parsed)
                .is_ok()
        );
        Ok(())
    }

    #[sqlx::test]
    async fn list_api_keys_excludes_secret_fields(pool: PgPool) -> sqlx::Result<()> {
        let default_team = users::ensure_default_team(&pool).await.unwrap();
        seed_user(&pool, "alice2@example.com", "Alice", default_team).await;

        let client = test_client(pool.clone()).await;
        login(&client, "alice2@example.com").await;

        client
            .post("/api/api-keys")
            .header(ContentType::JSON)
            .body(r#"{"name":"My Key"}"#)
            .dispatch()
            .await;

        let response = client.get("/api/api-keys").dispatch().await;
        assert_eq!(response.status(), Status::Ok);
        let body: serde_json::Value =
            serde_json::from_str(&response.into_string().await.unwrap()).unwrap();
        let entries = body["data"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].get("key").is_none());
        assert!(entries[0].get("key_hash").is_none());
        assert!(entries[0]["key_prefix"].is_string());
        Ok(())
    }

    #[sqlx::test]
    async fn delete_api_key_is_forbidden_for_other_users(pool: PgPool) -> sqlx::Result<()> {
        let default_team = users::ensure_default_team(&pool).await.unwrap();
        seed_user(&pool, "alice3@example.com", "Alice", default_team).await;
        seed_user(&pool, "bob3@example.com", "Bob", default_team).await;

        let client = test_client(pool.clone()).await;
        login(&client, "alice3@example.com").await;
        let create_response = client
            .post("/api/api-keys")
            .header(ContentType::JSON)
            .body(r#"{"name":"My Key"}"#)
            .dispatch()
            .await;
        let body: serde_json::Value =
            serde_json::from_str(&create_response.into_string().await.unwrap()).unwrap();
        let key_id = body["data"]["id"].as_str().unwrap();

        login(&client, "bob3@example.com").await;
        let delete_response = client
            .delete(format!("/api/api-keys/{}", key_id))
            .dispatch()
            .await;
        assert_eq!(delete_response.status(), Status::NotFound);

        login(&client, "alice3@example.com").await;
        let list_response = client.get("/api/api-keys").dispatch().await;
        let body: serde_json::Value =
            serde_json::from_str(&list_response.into_string().await.unwrap()).unwrap();
        assert_eq!(body["data"].as_array().unwrap().len(), 1);
        Ok(())
    }

    #[sqlx::test]
    async fn list_api_keys_requires_authentication(pool: PgPool) -> sqlx::Result<()> {
        let client = test_client(pool.clone()).await;
        let response = client.get("/api/api-keys").dispatch().await;
        assert_eq!(response.status(), Status::Unauthorized);
        Ok(())
    }
}

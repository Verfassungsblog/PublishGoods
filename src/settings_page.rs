use crate::db::repositories::users::{self, User};
use crate::session::session_guard::Session;
use rocket::State;
use rocket_dyn_templates::Template;
use sqlx::PgPool;

/// GET /settings
///
/// Renders the settings page, listing all users currently registered in the system.
#[get("/settings")]
pub async fn settings_page(_session: Session, pool: &State<PgPool>) -> Template {
    let users = users::list_all(pool.inner()).await.unwrap_or_default();
    Template::render("settings", users)
}

pub mod api {
    use super::User;
    use crate::db::repositories::users;
    use crate::session::session_guard::Session;
    use crate::utils::api_helpers::{APIResponse, APIResult, ApiErrorType};
    use argon2::password_hash::rand_core::OsRng;
    use argon2::{Argon2, PasswordHasher};
    use chrono::{DateTime, Utc};
    use rocket::State;
    use rocket::serde::json::Json;
    use sqlx::PgPool;

    #[derive(serde::Deserialize)]
    pub struct NewUser {
        username: String,
        password: String,
        email: String,
    }

    /// Hashes a plaintext password with Argon2 using a freshly generated salt.
    fn hash_password(password: &str) -> String {
        let salt = argon2::password_hash::SaltString::generate(&mut OsRng);
        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .unwrap()
            .to_string()
    }

    /// Insert a new user
    #[post("/api/users", data = "<new_user>")]
    pub async fn add_user(
        new_user: Json<NewUser>,
        _session: Session,
        pool: &State<PgPool>,
    ) -> APIResult<User> {
        let new_user = new_user.into_inner();
        let pool = pool.inner();

        if users::email_in_use(pool, &new_user.email, None).await? {
            return Err(ApiErrorType::BadRequest("Email already in use".to_string()).into());
        }

        let password_hash = hash_password(&new_user.password);
        let default_team_id = users::ensure_default_team(pool).await?;
        let user = users::insert(
            pool,
            &new_user.email,
            &new_user.username,
            &password_hash,
            default_team_id,
        )
        .await?;
        Ok(APIResponse::from(user))
    }

    /// Partial update payload for [`update_user`]. All fields except `id` are optional;
    /// only the fields present are applied. `locked_until` is a tri-state: absent means
    /// "don't touch", `Some(None)` clears the lockout, `Some(Some(secs))` locks the account
    /// until the given Unix timestamp.
    #[derive(serde::Deserialize)]
    pub struct PatchUser {
        pub id: uuid::Uuid,
        pub email: Option<String>,
        pub name: Option<String>,
        pub password: Option<String>,
        pub locked_until: Option<Option<u64>>,
    }

    /// PATCH /api/users/<id>
    ///
    /// Updates the given fields of a user's profile (email, name, password). If `locked_until`
    /// is present, additionally sets or clears the account's lockout timestamp.
    #[patch("/api/users/<id>", data = "<new_user>")]
    pub async fn update_user(
        id: String,
        new_user: Json<PatchUser>,
        _session: Session,
        pool: &State<PgPool>,
    ) -> APIResult<User> {
        let id = uuid::Uuid::parse_str(&id)?;
        let patch = new_user.into_inner();
        let pool = pool.inner();

        if let Some(new_email) = &patch.email
            && users::email_in_use(pool, new_email, Some(id)).await?
        {
            return Err(ApiErrorType::BadRequest("Email already in use".to_string()).into());
        }

        let password_hash = patch.password.as_deref().map(hash_password);
        let user = users::update_profile(
            pool,
            id,
            patch.email.as_deref(),
            patch.name.as_deref(),
            password_hash.as_deref(),
        )
        .await?;

        let user = if let Some(locked_until) = patch.locked_until {
            match locked_until {
                Some(secs) => {
                    let until = DateTime::<Utc>::from_timestamp(secs as i64, 0).ok_or(
                        ApiErrorType::BadRequest("Invalid locked_until timestamp".to_string()),
                    )?;
                    users::lock_until(pool, id, until).await?;
                }
                None => {
                    users::clear_lockout(pool, id).await?;
                }
            }
            users::get(pool, id).await?
        } else {
            user
        };

        Ok(APIResponse::from(user))
    }

    /// Delete a user
    #[delete("/api/users/<id>")]
    pub async fn delete_user(id: String, session: Session, pool: &State<PgPool>) -> APIResult<()> {
        let id = uuid::Uuid::parse_str(&id)?;

        if id == session.user_id {
            return Err(ApiErrorType::BadRequest("Cannot delete own user".to_string()).into());
        }

        users::delete(pool.inner(), id).await?;
        Ok(APIResponse::from(()))
    }
}

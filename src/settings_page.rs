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
    use crate::mailer::Mailer;
    use crate::projects::api::Patch;
    use crate::session::session_guard::Session;
    use crate::session::session_storage::SessionStorage;
    use crate::settings::Settings;
    use crate::utils::api_helpers::{APIResponse, APIResult, ApiError, ApiErrorType};
    use crate::utils::hash_password;
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

    /// GET /api/users/me
    ///
    /// Returns the profile of the currently logged-in user.
    #[get("/api/users/me")]
    pub async fn get_current_user(session: Session, pool: &State<PgPool>) -> APIResult<User> {
        let mut user = users::get(pool.inner(), session.user_id).await?;

        user.password_hash = None;
        user.password_reset_token_hash = None;
        user.password_reset_token_valid_until = None;

        Ok(APIResponse::from(user))
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

        let password_hash = hash_password(&new_user.password)
            .map_err(|_| ApiError::from(ApiErrorType::InternalServerError))?;
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

    /// Applies the profile fields of a [`PatchUser`] (email, name, password) onto a [`User`],
    /// leaving absent fields untouched. `id` and `locked_until` are handled by the caller.
    impl Patch<PatchUser, User> for User {
        fn patch(&mut self, patch: PatchUser) -> User {
            let mut new_user = self.clone();

            if let Some(email) = patch.email {
                new_user.email = email;
            }
            if let Some(name) = patch.name {
                new_user.name = name;
            }
            if let Some(password) = patch.password {
                new_user.password_hash =
                    Some(hash_password(&password).expect("Couldn't hash password."));
            }

            new_user
        }
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
        mailer: &State<Mailer>,
        settings: &State<Settings>,
    ) -> APIResult<User> {
        let id = uuid::Uuid::parse_str(&id)?;
        let patch = new_user.into_inner();
        let pool = pool.inner();

        if let Some(new_email) = &patch.email
            && users::email_in_use(pool, new_email, Some(id)).await?
        {
            return Err(ApiErrorType::BadRequest("Email already in use".to_string()).into());
        }

        let locked_until = patch.locked_until;
        let user_before = users::get(pool, id).await?;
        let user = user_before.clone().patch(patch);
        let user = users::update(pool, &user).await?;

        let user = if let Some(locked_until) = locked_until {
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

    /// Confirmation payload for [`delete_own_account`].
    #[derive(serde::Deserialize)]
    pub struct DeleteOwnAccount {
        email: String,
    }

    /// Deletes the currently logged-in user's own account.
    ///
    /// Requires the account's email to be repeated as confirmation (checked against the
    /// current DB value, not the possibly-stale email cached on the session), and
    /// invalidates the session immediately on success.
    #[delete("/api/users/me", data = "<confirm>")]
    pub async fn delete_own_account(
        confirm: Json<DeleteOwnAccount>,
        session: Session,
        pool: &State<PgPool>,
        session_storage: &State<SessionStorage>,
    ) -> APIResult<()> {
        let user = users::get(pool.inner(), session.user_id).await?;

        if !confirm.email.trim().eq_ignore_ascii_case(&user.email) {
            return Err(
                ApiErrorType::BadRequest("Email confirmation does not match.".to_string()).into(),
            );
        }

        users::delete(pool.inner(), session.user_id).await?;
        session_storage.remove_session(session.id.clone());

        Ok(APIResponse::from(()))
    }
}

//! Self-service account endpoints used by the profile settings page's account tab.
//! Users can only ever read, change or delete their own account.

use crate::db::repositories::users::{self, User};
use crate::projects::api::Patch;
use crate::session::session_guard::Session;
use crate::session::session_storage::SessionStorage;
use crate::utils::api_helpers::{APIResponse, APIResult, ApiErrorType};
use crate::utils::hash_password;
use rocket::State;
use rocket::serde::json::Json;
use sqlx::PgPool;

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

/// Partial update payload for [`update_user`]. All fields except `id` are optional;
/// only the fields present are applied.
#[derive(serde::Deserialize)]
pub struct PatchUser {
    pub id: uuid::Uuid,
    pub email: Option<String>,
    pub name: Option<String>,
    pub password: Option<String>,
}

/// Applies the profile fields of a [`PatchUser`] (email, name, password) onto a [`User`],
/// leaving absent fields untouched.
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
/// Updates the given fields of the logged-in user's own profile (email, name, password).
/// Requests for any other user id are rejected.
#[patch("/api/users/<id>", data = "<new_user>")]
pub async fn update_user(
    id: String,
    new_user: Json<PatchUser>,
    session: Session,
    pool: &State<PgPool>,
) -> APIResult<User> {
    let id = uuid::Uuid::parse_str(&id)?;
    let patch = new_user.into_inner();
    let pool = pool.inner();

    if id != session.user_id {
        return Err(
            ApiErrorType::Forbidden("You can only change your own account.".to_string()).into(),
        );
    }

    if let Some(new_email) = &patch.email
        && users::email_in_use(pool, new_email, Some(id)).await?
    {
        return Err(ApiErrorType::BadRequest("Email already in use".to_string()).into());
    }

    let user_before = users::get(pool, id).await?;
    let user = user_before.clone().patch(patch);
    let user = users::update(pool, &user).await?;

    Ok(APIResponse::from(user))
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

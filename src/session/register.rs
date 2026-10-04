use crate::db::repositories::DbError;
use crate::db::repositories::users::{self, User};
use crate::mailer::Mailer;
use crate::session::verify_email::send_verification_email;
use crate::settings::Settings;
use crate::utils::hash_password;
use rocket::State;
use rocket::form::Form;
use rocket::http::Status;
use rocket::response::Redirect;
use rocket_dyn_templates::Template;
use sqlx::PgPool;
use std::collections::BTreeMap;

/// Why a registration was rejected. [`RegisterError::code`] doubles as the `error` query
/// parameter the registration page is redirected back to (rendered as `error_<code>`).
#[derive(Debug)]
pub enum RegisterError {
    PasswordsDontMatch,
    PasswordTooShort,
    EmailInvalid,
    NameEmpty,
    EmailTaken,
    Internal,
}

impl RegisterError {
    pub fn code(&self) -> &'static str {
        match self {
            RegisterError::PasswordsDontMatch => "pw-dont-match",
            RegisterError::PasswordTooShort => "pw-too-short",
            RegisterError::EmailInvalid => "email-invalid",
            RegisterError::NameEmpty => "name-empty",
            RegisterError::EmailTaken => "email-taken",
            RegisterError::Internal => "internal",
        }
    }
}

/// Parses `email` as an address and returns it in its canonical form (trimmed, lowercase),
/// or `None` if it isn't a plain `local@domain` address. Emails are stored in this form so
/// that comparisons between accounts and invitations don't depend on case.
pub fn normalize_email(email: &str) -> Option<String> {
    let address: lettre::Address = email.trim().parse().ok()?;
    Some(address.to_string().to_lowercase())
}

/// Validates the submitted details and creates the account (attached to the default team
/// like every other user, see [`users::insert`]). Shared by the public registration and
/// the registration via team invitation. `email_verified` must only be true if the caller
/// has proof that the registrant controls the address (the mailed invitation token).
pub async fn create_account(
    pool: &PgPool,
    email: &str,
    name: &str,
    password: &str,
    password2: &str,
    email_verified: bool,
    min_password_length: usize,
) -> Result<User, RegisterError> {
    let name = name.trim();

    if name.is_empty() {
        return Err(RegisterError::NameEmpty);
    }
    let email = normalize_email(email).ok_or(RegisterError::EmailInvalid)?;
    if password != password2 {
        return Err(RegisterError::PasswordsDontMatch);
    }
    if password.chars().count() < min_password_length {
        return Err(RegisterError::PasswordTooShort);
    }

    if users::email_in_use(pool, &email, None).await.map_err(|e| {
        error!("Couldn't check email on registration: {}", e);
        RegisterError::Internal
    })? {
        return Err(RegisterError::EmailTaken);
    }

    let hashed_pw = hash_password(password).map_err(|e| {
        error!("Couldn't hash password: {}", e);
        RegisterError::Internal
    })?;

    let default_team = users::ensure_default_team(pool).await.map_err(|e| {
        error!("Couldn't get default team on registration: {}", e);
        RegisterError::Internal
    })?;

    users::insert_with_verification(pool, &email, name, &hashed_pw, default_team, email_verified)
        .await
        .map_err(|e| match e {
            // Lost a race against a concurrent registration with the same email.
            DbError::Conflict(_) => RegisterError::EmailTaken,
            e => {
                error!("Couldn't create user: {}", e);
                RegisterError::Internal
            }
        })
}

/// Renders the registration page. `team_name` (set when registering via an invitation) is
/// shown as the inviting team; `token` is passed through as a hidden field; a `Some(email)`
/// is prefilled and not editable.
pub fn render_register_page(
    action: &str,
    error: Option<&str>,
    team_name: Option<&str>,
    token: Option<&str>,
    locked_email: Option<&str>,
    min_password_length: usize,
) -> Template {
    let mut context: BTreeMap<String, String> = BTreeMap::new();
    context.insert("action".to_string(), action.to_string());
    context.insert(
        "min_password_length".to_string(),
        min_password_length.to_string(),
    );
    if let Some(error) = error {
        context.insert(format!("error_{}", error), "true".to_string());
    }
    if let Some(team_name) = team_name {
        context.insert("team_name".to_string(), team_name.to_string());
    }
    if let Some(token) = token {
        context.insert("token".to_string(), token.to_string());
    }
    if let Some(email) = locked_email {
        context.insert("email".to_string(), email.to_string());
        context.insert("email_locked".to_string(), "true".to_string());
    }
    Template::render("register", context)
}

/// GET /register
///
/// Public registration page. 404s unless `public_registration` is enabled.
#[get("/register?<error>")]
pub fn register_page(error: Option<&str>, settings: &State<Settings>) -> Result<Template, Status> {
    if !settings.public_registration {
        return Err(Status::NotFound);
    }
    Ok(render_register_page(
        "/register",
        error,
        None,
        None,
        None,
        settings.min_password_length,
    ))
}

#[derive(FromForm)]
pub struct RegisterForm {
    pub email: String,
    pub name: String,
    pub password: String,
    pub password2: String,
}

/// POST /register
///
/// Creates an unverified account and mails a verification link; the user can only log in
/// after following it. 404s unless `public_registration` is enabled.
#[post("/register", data = "<form>")]
pub async fn process_register_form(
    form: Form<RegisterForm>,
    pool: &State<PgPool>,
    settings: &State<Settings>,
    mailer: &State<Mailer>,
) -> Result<Redirect, Status> {
    if !settings.public_registration {
        return Err(Status::NotFound);
    }
    let form = form.into_inner();

    match create_account(
        pool,
        &form.email,
        &form.name,
        &form.password,
        &form.password2,
        false,
        settings.min_password_length,
    )
    .await
    {
        Ok(user) => {
            // If the mail can't be sent the user can request another one from the login page.
            if let Err(e) = send_verification_email(pool, mailer, settings, &user).await {
                error!("Couldn't send verification email: {}", e);
            }
            Ok(Redirect::to("/login?error=verification-sent"))
        }
        Err(e) => Ok(Redirect::to(format!("/register?error={}", e.code()))),
    }
}

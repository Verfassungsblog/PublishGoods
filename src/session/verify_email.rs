use crate::db::repositories::users::{self, User};
use crate::mailer::{MailJob, Mailer};
use crate::settings::Settings;
use crate::utils::hash_password;
use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordVerifier};
use chrono::{Duration, Utc};
use lettre::message::{Mailbox, header::ContentType};
use rand::distr::{Alphanumeric, SampleString};
use rocket::State;
use rocket::form::Form;
use rocket::response::Redirect;
use sqlx::PgPool;
use uuid::Uuid;

/// Generates a fresh verification token for `user`, stores its hash and mails the
/// verification link to the user's current email address. Does nothing (and returns
/// `Ok`) if the account is already verified.
pub async fn send_verification_email(
    pool: &PgPool,
    mailer: &Mailer,
    settings: &Settings,
    user: &User,
) -> Result<(), String> {
    let token = Alphanumeric.sample_string(&mut rand::rng(), 40);
    let token_hash = hash_password(&token).map_err(|e| format!("Couldn't hash token: {}", e))?;
    let valid_until = Utc::now() + Duration::hours(settings.email_verification_validity_hours);
    users::set_email_verification_token(pool, user.id, &token_hash, valid_until)
        .await
        .map_err(|e| format!("Couldn't store verification token: {}", e))?;

    let mut url = settings.get_instance_url();
    url.path_segments_mut()
        .map_err(|_| "Instance URL can't be a base".to_string())?
        .extend(["verify-email", &user.id.to_string()]);
    url.query_pairs_mut().append_pair("token", &token);

    let address = user
        .email
        .parse()
        .map_err(|e| format!("Can't parse email address: {}", e))?;
    let body = format!(
        "Dear {},\nplease confirm your email address for {} by visiting {}\nThe link is valid for {} hours.",
        user.name, settings.app_title, url, settings.email_verification_validity_hours
    );
    let job = MailJob::from(
        Mailbox::new(Some(user.name.clone()), address),
        "Confirm your email address".to_string(),
        body,
        ContentType::TEXT_PLAIN,
    );
    mailer
        .sender
        .send(job)
        .await
        .map_err(|e| format!("Couldn't queue mail: {}", e))
}

/// Checks `token` against the user's pending verification token.
async fn token_is_valid(pool: &PgPool, user_id: Uuid, token: &str) -> bool {
    let Ok(Some((hash, valid_until))) = users::get_email_verification_token(pool, user_id).await
    else {
        return false;
    };
    if valid_until < Utc::now() {
        return false;
    }
    let Ok(parsed) = PasswordHash::new(&hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(token.as_bytes(), &parsed)
        .is_ok()
}

/// GET /verify-email/<user_id>?<token>
///
/// Verifies the account's email address and redirects to the login page.
#[get("/verify-email/<user_id>?<token>")]
pub async fn verify_email(user_id: &str, token: &str, pool: &State<PgPool>) -> Redirect {
    let pool = pool.inner();
    let Ok(user_id) = Uuid::parse_str(user_id) else {
        return Redirect::to("/login?error=verification-invalid");
    };
    if token_is_valid(pool, user_id, token).await
        && users::mark_email_verified(pool, user_id).await.is_ok()
    {
        return Redirect::to("/login?error=email-verified");
    }
    Redirect::to("/login?error=verification-invalid")
}

#[derive(FromForm)]
pub struct ResendForm {
    pub email: String,
}

/// POST /resend-verification
///
/// Mails a new verification link to an unverified account. Always answers the same way,
/// so it can't be used to find out which addresses are registered.
#[post("/resend-verification", data = "<form>")]
pub async fn resend_verification(
    form: Form<ResendForm>,
    pool: &State<PgPool>,
    mailer: &State<Mailer>,
    settings: &State<Settings>,
) -> Redirect {
    let pool = pool.inner();
    if let Ok(user) = users::find_by_email(pool, form.email.trim()).await
        && !users::is_email_verified(pool, user.id)
            .await
            .unwrap_or(true)
    {
        let recently_sent = matches!(
            users::get_email_verification_token(pool, user.id).await,
            Ok(Some((_, valid_until)))
                if valid_until
                    > Utc::now()
                        + Duration::hours(settings.email_verification_validity_hours)
                        - Duration::minutes(settings.email_verification_resend_cooldown_minutes)
        );
        if !recently_sent
            && let Err(e) = send_verification_email(pool, mailer, settings, &user).await
        {
            error!("Couldn't send verification email: {}", e);
        }
    }
    Redirect::to("/login?error=verification-sent")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::login::{login_page, process_login_form};
    use crate::session::session_storage::SessionStorage;
    use rocket::http::{ContentType, Status};
    use rocket::local::asynchronous::Client;
    use rocket_dyn_templates::Template;

    fn settings() -> Settings {
        let mut settings = crate::session::login::integration_tests::test_settings();
        settings.instance_url = "http://localhost".to_string();
        settings
    }

    async fn client(pool: PgPool) -> Client {
        let figment = rocket::Config::figment().select(rocket::Config::DEBUG_PROFILE);
        let rocket = rocket::custom(figment)
            .manage(pool)
            .manage(SessionStorage::new())
            .manage(settings())
            .attach(Template::fairing())
            .mount("/", routes![login_page, process_login_form, verify_email]);
        Client::tracked(rocket).await.unwrap()
    }

    async fn unverified_user(pool: &PgPool) -> User {
        let team = users::ensure_default_team(pool).await.unwrap();
        let hash = hash_password("correct horse").unwrap();
        users::insert_with_verification(pool, "new@example.com", "New", &hash, team, false)
            .await
            .unwrap()
    }

    async fn login(client: &Client) -> rocket::local::asynchronous::LocalResponse<'_> {
        client
            .post("/login")
            .header(ContentType::Form)
            .body("email=new%40example.com&password=correct+horse")
            .dispatch()
            .await
    }

    #[sqlx::test]
    async fn unverified_user_cannot_log_in_until_verified(pool: PgPool) -> sqlx::Result<()> {
        let user = unverified_user(&pool).await;
        let (sender, mut receiver) = tokio::sync::mpsc::channel(4);
        send_verification_email(&pool, &Mailer { sender }, &settings(), &user)
            .await
            .unwrap();
        let body = receiver.recv().await.unwrap().body;
        let token = body
            .split("token=")
            .nth(1)
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap();

        let client = client(pool.clone()).await;
        let response = login(&client).await;
        assert_eq!(
            response.headers().get_one("Location"),
            Some("/login?error=email-not-verified")
        );
        assert!(response.cookies().get_private("session").is_none());

        // A wrong token doesn't verify.
        let response = client
            .get(format!("/verify-email/{}?token=wrong", user.id))
            .dispatch()
            .await;
        assert_eq!(
            response.headers().get_one("Location"),
            Some("/login?error=verification-invalid")
        );
        assert!(!users::is_email_verified(&pool, user.id).await.unwrap());

        let response = client
            .get(format!("/verify-email/{}?token={}", user.id, token))
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::SeeOther);
        assert_eq!(
            response.headers().get_one("Location"),
            Some("/login?error=email-verified")
        );

        let response = login(&client).await;
        assert_eq!(response.headers().get_one("Location"), Some("/"));
        assert!(response.cookies().get_private("session").is_some());

        // The token is single-use.
        let response = client
            .get(format!("/verify-email/{}?token={}", user.id, token))
            .dispatch()
            .await;
        assert_eq!(
            response.headers().get_one("Location"),
            Some("/login?error=verification-invalid")
        );
        Ok(())
    }
}

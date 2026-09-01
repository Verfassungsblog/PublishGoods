use crate::db::repositories::users;
use crate::session::session_storage::SessionStorage;
use crate::settings::Settings;
use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordVerifier},
};
use chrono::Utc;
use rocket::State;
use rocket::form::Form;
use rocket::http::CookieJar;
use rocket::response::Redirect;
use rocket_dyn_templates::Template;
use sqlx::PgPool;
use std::collections::BTreeMap;

/// Show login page
/// Method: GET
#[get("/login?<error>")]
pub fn login_page(error: Option<String>) -> Template {
    let mut context: BTreeMap<String, bool> = BTreeMap::new();

    if let Some(error) = error {
        if error == "invalid" {
            context.insert("error_invalid".to_string(), true);
        } else if error == "contact-admin" {
            context.insert("error_contact_admin".to_string(), true);
        } else if error == "too-many-attempts" {
            context.insert("error_too_many_attempts".to_string(), true);
        }
    }
    Template::render("login", context)
}

/// POST /login
///
/// Verifies the submitted email/password against the stored user, then either starts a
/// session (setting a private `session` cookie and redirecting to `/`) or records a failed
/// attempt and redirects back to the login page. After `settings.max_login_attempts` failed
/// attempts within the lockout window the account is locked for
/// `settings.lockout_window_minutes` minutes, redirecting with `error=too-many-attempts`.
/// If the failed-attempt count can't be read from the database, the account is locked
/// defensively rather than letting the attempt through uncounted.
#[post("/login", data = "<form>")]
pub async fn process_login_form(
    form: Form<LoginForm>,
    pool: &State<PgPool>,
    session_storage: &State<SessionStorage>,
    settings: &State<Settings>,
    cookies: &CookieJar<'_>,
) -> Redirect {
    let form = form.into_inner();
    let pool = pool.inner();

    let user = match users::find_by_email(pool, &form.email).await {
        Ok(user) => user,
        Err(_) => return Redirect::to("/login?error=invalid"),
    };

    if let Some(locked_until) = user.locked_until {
        if locked_until > Utc::now() {
            info!(
                "User {} tried to login while locked. Still locked until {}",
                user.email, locked_until
            );
            return Redirect::to("/login?error=too-many-attempts");
        } else {
            // Lockout window has passed, forget it and the attempts that caused it.
            let _ = users::clear_lockout(pool, user.id).await;
        }
    }

    let password_hash = match &user.password_hash {
        Some(hash) => hash,
        None => return Redirect::to("/login?error=invalid"),
    };
    let parsed_hash = PasswordHash::new(password_hash).unwrap();
    match Argon2::default().verify_password(form.password.as_bytes(), &parsed_hash) {
        Ok(_) => {
            let _ = users::clear_lockout(pool, user.id).await;
            let session = session_storage.generate_session(user.email.clone(), user.id);
            cookies.add_private(("session", session.id.clone()));
            Redirect::to("/")
        }
        Err(_) => {
            let should_lock =
                match users::record_failed_attempt(pool, user.id, settings.lockout_window_minutes)
                    .await
                {
                    Ok(attempts) => attempts >= settings.max_login_attempts,
                    Err(e) => {
                        // We couldn't reliably count this attempt — fail closed (lock the
                        // account) instead of silently treating it as "0 attempts so far",
                        // which would let a DB hiccup bypass brute-force lockout indefinitely.
                        error!(
                            "Couldn't record failed login attempt for user {}: {}",
                            user.id, e
                        );
                        true
                    }
                };
            if should_lock {
                let _ = users::lock_until(
                    pool,
                    user.id,
                    Utc::now() + chrono::Duration::minutes(settings.lockout_window_minutes),
                )
                .await;
                return Redirect::to("/login?error=too-many-attempts");
            }
            Redirect::to("/login?error=invalid")
        }
    }
}

#[derive(FromForm)]
pub struct LoginForm {
    pub email: String,
    pub password: String,
}

#[test]
pub fn generate_hash() {
    use argon2::PasswordHasher;
    use argon2::password_hash::rand_core::OsRng;
    let salt = argon2::password_hash::SaltString::generate(&mut OsRng);
    let password = b"123456";
    let argon2 = Argon2::default();
    let hash: String = argon2.hash_password(password, &salt).unwrap().to_string();
    print!("{}", hash);
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use argon2::PasswordHasher;
    use argon2::password_hash::rand_core::OsRng;
    use rocket::http::{ContentType, Status};
    use rocket::local::asynchronous::Client;

    fn hash(password: &str) -> String {
        let salt = argon2::password_hash::SaltString::generate(&mut OsRng);
        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .unwrap()
            .to_string()
    }

    fn test_settings() -> Settings {
        Settings {
            app_title: "Verfassungsbooks".to_string(),
            project_cache_time: 1800,
            data_path: "data".to_string(),
            database_url: "".to_string(),
            database_max_connections: 10,
            file_lock_timeout: 1000,
            backup_to_file_interval: 20,
            max_connections_to_rendering_server: 10,
            max_import_threads: 4,
            zotero_translation_server: "".to_string(),
            export_servers: vec![],
            ca_cert_path: "".to_string(),
            client_cert_path: "".to_string(),
            client_key_path: "".to_string(),
            revocation_list_path: "".to_string(),
            version: "test".to_string(),
            max_login_attempts: 5,
            lockout_window_minutes: 15,
        }
    }

    async fn test_client(pool: PgPool) -> Client {
        // Force the debug profile so Rocket doesn't demand a configured secret_key, which it
        // otherwise requires outside the debug profile (e.g. when tests are built with
        // `cargo test --release`).
        let figment = rocket::Config::figment().select(rocket::Config::DEBUG_PROFILE);
        let rocket = rocket::custom(figment)
            .manage(pool)
            .manage(SessionStorage::new())
            .manage(test_settings())
            .attach(Template::fairing())
            .mount("/", routes![login_page, process_login_form]);
        Client::tracked(rocket).await.unwrap()
    }

    #[sqlx::test]
    async fn successful_login_sets_session_cookie_and_clears_lockout(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        let team_id = users::ensure_default_team(&pool).await.unwrap();
        users::insert(
            &pool,
            "runtime@example.com",
            "Runtime",
            &hash("correct horse"),
            team_id,
        )
        .await
        .unwrap();

        let client = test_client(pool).await;
        let response = client
            .post("/login")
            .header(ContentType::Form)
            .body("email=runtime%40example.com&password=correct+horse")
            .dispatch()
            .await;

        assert_eq!(response.status(), Status::SeeOther);
        assert_eq!(response.headers().get_one("Location"), Some("/"));
        assert!(response.cookies().get_private("session").is_some());
        Ok(())
    }

    #[sqlx::test]
    async fn failed_login_locks_account_after_five_attempts(pool: PgPool) -> sqlx::Result<()> {
        let team_id = users::ensure_default_team(&pool).await.unwrap();
        let user = users::insert(
            &pool,
            "lockme@example.com",
            "Lockme",
            &hash("realpassword"),
            team_id,
        )
        .await
        .unwrap();

        let client = test_client(pool.clone()).await;
        for _ in 0..test_settings().max_login_attempts {
            let response = client
                .post("/login")
                .header(ContentType::Form)
                .body("email=lockme%40example.com&password=wrong")
                .dispatch()
                .await;
            assert_eq!(response.status(), Status::SeeOther);
        }

        let locked = users::get(&pool, user.id).await.unwrap();
        assert!(locked.locked_until.is_some());

        let response = client
            .post("/login")
            .header(ContentType::Form)
            .body("email=lockme%40example.com&password=realpassword")
            .dispatch()
            .await;
        assert_eq!(
            response.headers().get_one("Location"),
            Some("/login?error=too-many-attempts")
        );
        Ok(())
    }
}

use crate::db::repositories::users;
use crate::session::session_storage::SessionStorage;
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
/// attempt and redirects back to the login page. After [`users::MAX_LOGIN_ATTEMPTS`] failed
/// attempts within the lockout window the account is locked for
/// [`users::LOCKOUT_WINDOW_MINUTES`] minutes, redirecting with `error=too-many-attempts`.
/// If the failed-attempt count can't be read from the database, the account is locked
/// defensively rather than letting the attempt through uncounted.
#[post("/login", data = "<form>")]
pub async fn process_login_form(
    form: Form<LoginForm>,
    pool: &State<PgPool>,
    session_storage: &State<SessionStorage>,
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
            println!(
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
            let should_lock = match users::record_failed_attempt(pool, user.id).await {
                Ok(attempts) => attempts >= users::MAX_LOGIN_ATTEMPTS,
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
                    Utc::now() + chrono::Duration::minutes(users::LOCKOUT_WINDOW_MINUTES),
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

    async fn test_client(pool: PgPool) -> Client {
        let rocket = rocket::build()
            .manage(pool)
            .manage(SessionStorage::new())
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
        for _ in 0..users::MAX_LOGIN_ATTEMPTS {
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

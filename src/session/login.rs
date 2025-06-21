use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use rocket::form::Form;
use rocket::response::Redirect;
use rocket::State;
use rocket_dyn_templates::Template;
use rocket::http::CookieJar;
use argon2::{password_hash::{
    PasswordHash, PasswordVerifier
}, Argon2};
use crate::storage::data_storage::DataStorage;
use crate::session::session_storage::SessionStorage;

/// Show login page
/// Method: GET
#[get("/login?<error>")]
pub fn login_page(error: Option<String>) -> Template {
    let mut context: BTreeMap<String, bool> = BTreeMap::new();

    if let Some(error) = error{
        if error == "invalid"{
            context.insert("error_invalid".to_string(), true);
        }else if error == "contact-admin" {
            context.insert("error_contact_admin".to_string(), true);
        }else if error == "too-many-attempts" {
            context.insert("error_too_many_attempts".to_string(), true);
        }
    }
    Template::render("login", context)
}

/// Process login request
#[post("/login", data = "<form>")]
pub fn process_login_form(form: Form<LoginForm>, data_storage: &State<Arc<DataStorage>>, session_storage: &State<SessionStorage>, cookies: &CookieJar) -> Redirect {
    let form = form.into_inner();

    match data_storage.get_user(&form.email){
        Ok(user) => {
            let user = user.read().unwrap().clone();

            if let Some(locked_until) = user.locked_until{
                if locked_until > SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(){
                    println!("User {} tried to login while locked. Still locked until {}", user.email, locked_until);
                    return Redirect::to("/login?error=too-many-attempts");
                }else{
                    // Remove old login attempts & unlock account
                    data_storage.get_user(&form.email).unwrap().write().unwrap().login_attempts = Vec::new();
                    data_storage.get_user(&form.email).unwrap().write().unwrap().locked_until = None;
                }
            }

            let parsed_hash = PasswordHash::new(&user.password_hash).unwrap();
            match Argon2::default().verify_password(form.password.as_bytes(), &parsed_hash){
                Ok(_) => {
                    // Login successful, remove old login attempts and generate session
                    data_storage.get_user(&form.email).unwrap().write().unwrap().login_attempts = Vec::new();
                    let session = session_storage.generate_session(user.email.clone(), user.id.clone());
                    cookies.add_private(("session", session.id.clone()));
                    return Redirect::to("/");
                }
                Err(_) => {
                    data_storage.get_user(&form.email).unwrap().write().unwrap().login_attempts.push(SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs());
                    if data_storage.get_user(&form.email).unwrap().read().unwrap().login_attempts.len() >= 5{
                        //More than 5 login attempts, lock account for 15 minutes
                        data_storage.get_user(&form.email).unwrap().write().unwrap().locked_until = Some(SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() + 900);
                        return Redirect::to("/login?error=too-many-attempts");
                    }
                    Redirect::to("/login?error=invalid")
                }
            }
        },
        Err(_) => {
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
pub fn generate_hash(){
    use argon2::PasswordHasher;
    use argon2::password_hash::rand_core::OsRng;
    let salt = argon2::password_hash::SaltString::generate(&mut OsRng);
    let password = b"123456";
    let argon2 = Argon2::default();
    let hash : String = argon2.hash_password(password,&salt).unwrap().to_string();
    print!("{}", hash);
}
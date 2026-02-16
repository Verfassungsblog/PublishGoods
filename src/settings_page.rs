use crate::session::session_guard::Session;
use crate::storage::data_storage::DataStorage;
use crate::storage::User;
use rocket::State;
use rocket_dyn_templates::Template;
use std::sync::Arc;

#[get("/settings")]
pub async fn settings_page(_session: Session, data_storage: &State<Arc<DataStorage>>) -> Template {
    let data_storage = data_storage;
    let users: Vec<User> = data_storage
        .data
        .read()
        .unwrap()
        .login_data
        .iter()
        .map(|x| x.1.read().unwrap().clone())
        .collect();
    Template::render("settings", users)
}

pub mod api {
    use crate::projects::api::Patch;
    use crate::session::session_guard::Session;
    use crate::settings::Settings;
    use crate::storage::data_storage::DataStorage;
    use crate::storage::User;
    use crate::utils::api_helpers::{APIResponse, APIResult, ApiErrorType};
    use argon2::password_hash::rand_core::OsRng;
    use argon2::{Argon2, PasswordHasher};
    use rocket::serde::json::Json;
    use rocket::State;
    use std::sync::Arc;

    #[derive(serde::Deserialize)]
    struct NewUser {
        username: String,
        password: String,
        email: String,
    }

    /// Insert a new user
    #[post("/api/users", data = "<new_user>")]
    pub async fn add_user(
        new_user: Json<NewUser>,
        _session: Session,
        data_storage: &State<Arc<DataStorage>>,
        settings: &State<Settings>,
    ) -> APIResult<User> {
        let new_user = new_user.into_inner();
        let data_storage = data_storage;

        // Check if user with this email already exists
        if data_storage
            .data
            .read()
            .unwrap()
            .login_data
            .iter()
            .any(|x| x.1.read().unwrap().email == new_user.email)
        {
            return Err(ApiErrorType::BadRequest("Email already in use".to_string()).into());
        }

        let user = User::new(new_user.email, new_user.username, new_user.password);
        data_storage
            .insert_user(user.clone(), settings)
            .await
            .unwrap();
        Ok(APIResponse::from(user))
    }

    #[derive(serde::Deserialize)]
    pub struct PatchUser {
        pub id: uuid::Uuid,
        pub email: Option<String>,
        pub name: Option<String>,
        pub password: Option<String>,
        pub locked_until: Option<Option<u64>>,
        pub login_attempts: Option<Vec<u64>>,
    }

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
                let salt = argon2::password_hash::SaltString::generate(&mut OsRng);
                let password_hash = Argon2::default()
                    .hash_password(&password.as_bytes(), &salt)
                    .unwrap()
                    .to_string();
                new_user.password_hash = password_hash;
            }
            if let Some(locked_until) = patch.locked_until {
                new_user.locked_until = locked_until;
            }
            if let Some(login_attempts) = patch.login_attempts {
                new_user.login_attempts = login_attempts;
            }
            new_user
        }
    }

    /// Update a user
    #[patch("/api/users/<id>", data = "<new_user>")]
    pub fn update_user(
        id: String,
        new_user: Json<PatchUser>,
        _session: Session,
        data_storage: &State<Arc<DataStorage>>,
    ) -> APIResult<User> {
        // Parse id or return error
        let id = uuid::Uuid::parse_str(&id)?;

        let new_user = new_user.into_inner();
        let data_storage = data_storage;

        let mut data = data_storage.data.write().unwrap();

        //Check email is changed + email is already in use
        if let Some(new_email) = new_user.email.clone() {
            if new_email != data.login_data.get(&id).unwrap().read().unwrap().email {
                if data
                    .login_data
                    .iter()
                    .any(|x| x.1.read().unwrap().email == new_email)
                {
                    return Err(ApiErrorType::BadRequest("Email already in use".to_string()).into());
                }
            }
        }

        match data.login_data.get_mut(&id) {
            Some(user) => {
                let mut user = user.write().unwrap();
                let new_user = user.patch(new_user);
                *user = new_user.clone();
                Ok(APIResponse::from(new_user))
            }
            None => Err(ApiErrorType::ResourceNotFound("user".to_string()).into()),
        }
    }

    /// Delete a user
    #[delete("/api/users/<id>")]
    pub async fn delete_user(
        id: String,
        session: Session,
        data_storage: &State<Arc<DataStorage>>,
    ) -> APIResult<()> {
        // Parse id or return error
        let id = uuid::Uuid::parse_str(&id)?;

        if id == session.user_id {
            return Err(ApiErrorType::BadRequest("Cannot delete own user".to_string()).into());
        }

        let data_storage = data_storage;
        let mut data = data_storage.data.write().unwrap();

        match data.login_data.remove(&id) {
            Some(_) => Ok(APIResponse::from(())),
            None => Err(ApiErrorType::ResourceNotFound("user".to_string()).into()),
        }
    }
}

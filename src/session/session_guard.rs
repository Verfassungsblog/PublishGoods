use rocket::{Request, State};
use rocket::http::Status;
use rocket::request::{FromRequest, Outcome};
use rocket::serde::{Deserialize, Serialize};
use crate::session::errors::LoginError;
use crate::session::session_storage::SessionStorage;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session{
    pub id: String,
    pub user_id: uuid::Uuid,
    pub valid_until: std::time::SystemTime,
    pub user_email: String,
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for Session {
    type Error = LoginError;

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let storage : &State<SessionStorage> = match request.guard::<&State<SessionStorage>>().await {
            Outcome::Success(storage) => storage,
            _ => return Outcome::Error((Status::Unauthorized,LoginError::Unavailable)),
        };
        debug!("Cookies: {}", request.cookies().iter().map(|cookie|cookie.to_string()).collect::<String>());
        match request.cookies().get_private("session") {
            Some(cookie) => match storage.get_session(cookie.value().to_string(), true) {
                Some(cookie) => {
                    Outcome::Success(cookie.clone())
                }
                None => Outcome::Error((Status::Unauthorized, LoginError::Missing)),
            },
            None => Outcome::Error((Status::Unauthorized, LoginError::Missing)),
        }
    }
}
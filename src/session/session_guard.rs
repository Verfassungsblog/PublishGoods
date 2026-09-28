use crate::session::errors::LoginError;
use crate::session::session_storage::SessionStorage;
use rocket::http::Status;
use rocket::request::{FromRequest, Outcome};
use rocket::serde::{Deserialize, Serialize};
use rocket::{Request, State};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub user_id: uuid::Uuid,
    pub valid_until: std::time::SystemTime,
    pub user_email: String,
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for Session {
    type Error = LoginError;

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let storage: &State<SessionStorage> = match request.guard::<&State<SessionStorage>>().await
        {
            Outcome::Success(storage) => storage,
            _ => return Outcome::Error((Status::Unauthorized, LoginError::Unavailable)),
        };
        debug!(
            "Cookies: {}",
            request
                .cookies()
                .iter()
                .map(|cookie| cookie.to_string())
                .collect::<String>()
        );
        match request.cookies().get_private("session") {
            Some(cookie) => match storage.get_session(cookie.value().to_string(), true) {
                Some(cookie) => Outcome::Success(cookie.clone()),
                None => Outcome::Error((Status::Unauthorized, LoginError::Missing)),
            },
            None => Outcome::Error((Status::Unauthorized, LoginError::Missing)),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct APISession {
    pub id: Uuid,
    pub user_id: Uuid,
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for APISession {
    type Error = ();
    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let pg_pool: &PgPool = match request.guard::<&State<PgPool>>().await {
            Outcome::Success(res) => res.inner(),
            _ => {
                error!("Couldn't retrieve PgPool.");
                return Outcome::Error((Status::InternalServerError, ()));
            }
        };

        let raw_api_key = match request.headers().get_one("X-API-KEY") {
            Some(api_key) => api_key,
            None => return Outcome::Error((Status::Unauthorized, ())),
        };

        // Validate API Key
        match crate::db::repositories::api_keys::verify(pg_pool, raw_api_key).await {
            Ok(api_key) => match api_key {
                Some(api_details) => Outcome::Success(APISession {
                    id: api_details.id,
                    user_id: api_details.user_id,
                }),
                None => Outcome::Error((Status::Unauthorized, ())),
            },
            Err(e) => {
                error!("DB Error checking api key: {}", e);
                Outcome::Error((Status::InternalServerError, ()))
            }
        }
    }
}

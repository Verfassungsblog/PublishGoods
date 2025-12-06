use crate::projects::api::{DeprecatedApiError, DeprecatedApiResult};
use crate::settings::Settings;
use crate::storage::data_storage::current::DataStorageError;
use crate::storage::project_storage::{ProjectData, ProjectStorage, ProjectStorageError};
use rocket::http::{ContentType, Status};
use rocket::response::Responder;
use rocket::serde::json::Json;
use rocket::{Request, Response, State};
use serde::Serialize;
use std::io::{Cursor, Error, ErrorKind};
use std::sync::{Arc, RwLock};

/// Attempts to parse a string as a UUID.
///
/// Returns `Ok(uuid)` if the input string is a valid UUID.
/// Otherwise, logs the error and returns an `ApiError::BadRequest` wrapped as JSON.
///
/// # Arguments
/// * `uuid` - String slice containing the UUID to be parsed.
pub fn parse_uuid(uuid: &str) -> Result<uuid::Uuid, Json<DeprecatedApiResult<DeprecatedApiError>>> {
    match uuid::Uuid::parse_str(uuid) {
        Ok(uuid) => Ok(uuid),
        Err(e) => {
            eprintln!("Couldn't parse UUID: {}", e);
            Err(DeprecatedApiResult::new_error(
                DeprecatedApiError::BadRequest("Invalid UUID".to_string()),
            ))
        }
    }
}
/// Asynchronously retrieves a project entry wrapped in an `Arc<RwLock<ProjectData>>` by its UUID.
///
/// Returns `Ok(project_entry)` if found, otherwise returns a not found error as JSON.
///
/// # Arguments
/// * `project_id` - Reference to the project's UUID.
/// * `settings` - State containing runtime settings.
/// * `project_storage` - Shared project storage backend.
pub async fn get_project(
    project_id: &uuid::Uuid,
    settings: &State<Settings>,
    project_storage: Arc<ProjectStorage>,
) -> Result<Arc<RwLock<ProjectData>>, Json<DeprecatedApiResult<DeprecatedApiError>>> {
    match project_storage.get_project(&project_id, settings).await {
        Ok(project_entry) => Ok(project_entry.clone()),
        Err(_) => {
            eprintln!("Couldn't get project with id {}", project_id);
            Err(DeprecatedApiResult::new_error(DeprecatedApiError::NotFound))
        }
    }
}

/// Represents the new standard API result type, holding either a valid response or an error.
pub type APIResult<T> = Result<APIResponse<T>, ApiError>;

/// Enumeration of possible new-style API error types.
#[derive(Serialize, Debug)]
pub enum ApiErrorType {
    /// Session invalid or expired
    Unauthorized,
    /// A request parameter could not be parsed. Contains the parameter name.
    UnparsableParameter(String),
    /// The requested resource does not exist. Contains the resource name.
    ResourceNotFound(String),
    /// A generic internal server error.
    InternalServerError,
    /// Arbitrary error, contains a message.
    Other(String),
}

/// Translates from old `ApiError` values to the new error type representation.
impl From<DeprecatedApiError> for ApiErrorType {
    fn from(value: DeprecatedApiError) -> Self {
        match value {
            DeprecatedApiError::NotFound => ApiErrorType::ResourceNotFound("unknown".to_string()),
            DeprecatedApiError::BadRequest(x) => ApiErrorType::Other(x),
            DeprecatedApiError::Unauthorized => ApiErrorType::Unauthorized,
            DeprecatedApiError::InternalServerError => ApiErrorType::InternalServerError,
            DeprecatedApiError::Conflict(x) => ApiErrorType::Other(x),
            DeprecatedApiError::Other(x) => ApiErrorType::Other(x),
        }
    }
}

/// The new-style API error object for JSON responses.
#[derive(Debug, Serialize)]
pub struct ApiError {
    /// The type/category of the error.
    pub error: ApiErrorType,
    /// Optional detailed error description for clients.
    pub error_description: Option<String>,
}

impl ApiError {
    /// Creates a new `NewApiError` with the given error type and optional description.
    ///
    /// # Arguments
    /// * `error_type` - Kind of error as `NewApiErrorType`.
    /// * `error_description` - Optional detailed error description.
    pub fn new(error_type: ApiErrorType, error_description: Option<String>) -> Self {
        ApiError {
            error: error_type,
            error_description,
        }
    }
}

/// Allows Rocket to send `ApiError` as a JSON response, with status code selected based on the error type.
impl<'r> Responder<'r, 'static> for ApiError {
    fn respond_to(self, request: &'r Request<'_>) -> rocket::response::Result<'static> {
        debug!("Responding with error {:?}", self.error);

        let status = match self.error {
            ApiErrorType::Unauthorized => Status::Unauthorized,
            ApiErrorType::UnparsableParameter(_) => Status::BadRequest,
            ApiErrorType::ResourceNotFound(_) => Status::NotFound,
            ApiErrorType::InternalServerError => Status::InternalServerError,
            ApiErrorType::Other(_) => Status::ImATeapot,
        };

        let mut response = Response::new();
        let serialized_body = serde_json::to_string(&self).unwrap();
        response.set_sized_body(serialized_body.len(), Cursor::new(serialized_body));
        response.set_status(status);
        response.set_header(ContentType::JSON);

        Ok(response)
    }
}

/// Converts error types to a detailed `NewApiError` instance, filling error descriptions with standard text.
impl From<ApiErrorType> for ApiError {
    fn from(value: ApiErrorType) -> Self {
        let error_description = match &value{
            ApiErrorType::Unauthorized => Some("You session is invalid or expired. Please login.".to_string()),
            ApiErrorType::UnparsableParameter(parameter) => Some(format!("Value of parameter {} is not parsable.", parameter)),
            ApiErrorType::ResourceNotFound(resource_name) => Some(format!("The requested resource {} couldn't be found.", resource_name)),
            ApiErrorType::InternalServerError => Some("An interal error occured. Please contact the administrator if this problem persists.".to_string()),
            ApiErrorType::Other(_) => None,
        };

        ApiError {
            error: value,
            error_description,
        }
    }
}

/// Converts UUID parsing errors into a standardized API error indicating the parameter was unparsable.
impl From<uuid::Error> for ApiError {
    fn from(value: uuid::Error) -> Self {
        ApiErrorType::UnparsableParameter("uuid".to_string()).into()
    }
}

/// Converts project storage errors to the API error response format.
impl From<ProjectStorageError> for ApiError {
    fn from(value: ProjectStorageError) -> Self {
        match value {
            ProjectStorageError::ProjectNotFound => {
                ApiErrorType::ResourceNotFound(String::from("project")).into()
            }
            _ => {
                error!("Internal Server Error: {:?}", value);
                ApiErrorType::InternalServerError.into()
            }
        }
    }
}

/// Convert data storage errors to the API error response format to enable usage of ? operator
impl From<DataStorageError> for ApiError {
    fn from(value: DataStorageError) -> Self {
        match value {
            DataStorageError::NotFound(detail) => ApiErrorType::ResourceNotFound(detail).into(),
        }
    }
}

impl From<std::io::Error> for ApiError {
    fn from(value: Error) -> Self {
        match value.kind() {
            ErrorKind::NotFound => {
                warn!("IO Not Found error: {:?}", value);
                ApiErrorType::ResourceNotFound(String::from("path")).into()
            }
            _ => {
                error!("Internal Server Error: {:?}", value);
                ApiErrorType::InternalServerError.into()
            }
        }
    }
}

/// New-style standard API response wrapping a serializable data result.
#[derive(Serialize, Debug)]
pub struct APIResponse<T: Serialize> {
    /// The actual returned value/data as part of the response.
    data: T,
}

impl<'r, T: Serialize + std::fmt::Debug> Responder<'r, 'static> for APIResponse<T> {
    /// Implements Rocket responder for `NewAPIResponse`, returning JSON data and HTTP 200.
    fn respond_to(self, request: &'r Request<'_>) -> rocket::response::Result<'static> {
        debug!("Responding with data {:?}", self.data);

        let mut response = Response::new();
        let serialized_body = serde_json::to_string(&self).unwrap();
        response.set_sized_body(serialized_body.len(), Cursor::new(serialized_body));
        response.set_status(Status::Ok);
        response.set_header(ContentType::JSON);

        Ok(response)
    }
}

/// Allows convenient implicit conversion from a serializable value to a `APIResponse` for API return types.
impl<T: Serialize> From<T> for APIResponse<T> {
    fn from(value: T) -> Self {
        APIResponse { data: value }
    }
}

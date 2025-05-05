use std::collections::VecDeque;
use std::sync::Arc;

use rocket::form::Form;
use rocket::fs::TempFile;
use rocket::http::ContentType;
use rocket::serde::json::Json;
use rocket::State;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::data_storage::ProjectStorage;
use crate::import::processing::{ImportJob, ImportProcessor, ImportStatus, ImportStatusPoll};
use crate::import::wordpress::{Category, CategoryTree, CoAuthor, PostAcf, PostData, PostDataType, PostPreview, RenderedContent, WordpressAPI, WordpressAPIContext, WordpressAPIError};
use crate::projects::api::{ApiError, ApiResult};
use crate::session::session_guard::Session;
use crate::settings::Settings;

/// Form data for file upload request
#[derive(FromForm)]
struct FileUpload<'r>{
    /// Vector of temporary files uploaded by user
    files: Vec<TempFile<'r>>,
    /// Optional bibliography file in BibTeX format
    bib_file: Option<TempFile<'r>>,
    /// Unique identifier for the project
    project_id: String,
    /// Whether footnotes should be converted to endnotes during processing
    convert_footnotes_to_endnotes: bool,
}

/// Configuration for importing content to WordPress
#[derive(Serialize, Deserialize)]
struct WordpressImport{
    /// The unique identifier of the target project
    project_id: uuid::Uuid,
    /// Whether to convert footnotes to endnotes in final output
    endnotes: bool,
    /// Collection of links that need to be processed
    links: Vec<String>,
    /// Whether to increase heading levels by one
    shift_headings: bool,
    /// Whether to convert internal links to their WordPress equivalents
    convert_links: bool
}

#[post("/api/import/upload", data = "<upload>")]
pub async fn import_from_upload(mut upload: Form<FileUpload<'_>>, _session: Session, settings: &State<Settings>, _project_storage: &State<Arc<ProjectStorage>>, import_processor: &State<Arc<ImportProcessor>>) -> Json<ApiResult<uuid::Uuid>>{
    debug!("Uploading file for import for project {}", upload.project_id);

    let mut file_paths: VecDeque<(String, ContentType)> = VecDeque::new();

    // Persisting the files to disk
    for file in upload.files.iter_mut(){
        debug!("Processing file {}", file.name().unwrap());

        let path = format!("{}/temp/{}", settings.data_path, Uuid::new_v4());
        file.copy_to(path.clone()).await.unwrap();
        let content_type = match file.content_type(){
            Some(content_type) => content_type,
            None => return ApiResult::new_error(ApiError::BadRequest("Invalid file type".to_string()))
        };

        file_paths.push_back((path, content_type.clone()));
    }

    // Persisting bib file
    let bib_file_path = match upload.bib_file.as_mut(){
        Some(file) => {
            let path = format!("{}/temp/{}", settings.data_path, Uuid::new_v4());
            file.copy_to(path.clone()).await.unwrap();
            Some(path)
        },
        None => None
    };

    let project_id = match uuid::Uuid::parse_str(&upload.project_id){
        Ok(id) => id,
        Err(_) => return ApiResult::new_error(ApiError::BadRequest("Invalid project id".to_string()))
    };

    let id = uuid::Uuid::new_v4();
    let import_job = ImportJob{
        id,
        project_id,
        length: file_paths.len() as usize,
        processed: 0,
        files_to_process: Some(file_paths),
        convert_footnotes_to_endnotes: upload.convert_footnotes_to_endnotes,
        bib_file: bib_file_path,
        wordpress_post_links_to_convert: None,
        status: ImportStatus::Pending,
        shift_headings_up: false,
        convert_links: false
    };

    import_processor.job_queue.write().unwrap().push_back(import_job);

    ApiResult::new_data(id)
}

#[post("/api/import/wordpress", data = "<job>")]
pub async fn import_from_wordpress(job: Json<WordpressImport>, _session: Session, _settings: &State<Settings>, import_processor: &State<Arc<ImportProcessor>>) -> Json<ApiResult<uuid::Uuid>>{
    let id = Uuid::new_v4();

    let import_job = ImportJob{
        id,
        project_id: job.project_id,
        length: job.links.len(),
        processed: 0,
        files_to_process: None,
        convert_footnotes_to_endnotes: job.endnotes,
        wordpress_post_links_to_convert: Some(<Vec<std::string::String> as Clone>::clone(&job.links).into()),
        status: ImportStatus::Pending,
        bib_file: None,
        shift_headings_up: job.shift_headings,
        convert_links: job.convert_links
    };

    import_processor.job_queue.write().unwrap().push_back(import_job);
    ApiResult::new_data(id)
}

/// Endpoint to fetch WordPress categories as a hierarchical category tree.
///
/// This endpoint interacts with the WordPress API to retrieve the category structure 
/// and returns it as a JSON response. The client must provide the base URL of the 
/// WordPress API for this operation.
///
/// # Arguments
/// - `base_url`: A query string parameter representing the base URL of the WordPress site. 
///   Do NOT include the protocol (e.g. https://)!
/// - `_session`: Session Request Guard, making sure only users with a valid session can access this route
///
/// # Returns
/// A `Json` response containing an `ApiResult` type:
/// - On success: A JSON payload containing the hierarchical `CategoryTree`.
/// - On failure: A JSON error response depending on the nature of the error.
///
/// # Errors
/// The following errors may be returned based on the outcome of the WordPress API interaction:
/// - `ApiError::InternalServerError`:
///   - If there's an internal issue while initializing the WordPress API instance.
///   - If there's a serialization/deserialization issue when processing the response.
///   - If there's an unexpected response or unsupported error during API communication.
/// - `ApiError::BadRequest`: If the provided `base_url` is invalid or cannot be resolved.
/// - A valid but empty data set if no categories are found.
///
#[get("/api/import/wordpress/categories?<base_url>")]
pub async fn get_wordpress_categories(base_url: String, _session: Session) -> Json<ApiResult<CategoryTree>>{
    let wordpress_api = match WordpressAPI::new(base_url.clone()){
        Ok(api) => api,
        Err(e) => {
            error!("{:?}", e);
            return ApiResult::new_error(ApiError::InternalServerError)
        }
    };

    let categories = match wordpress_api.get_category_tree().await{
        Ok(categories) => categories,
        Err(e) => {
            return match e{
                WordpressAPIError::SerdeParsingError => {
                    ApiResult::new_error(ApiError::InternalServerError)
                },
                WordpressAPIError::ReqwestError => {
                    ApiResult::new_error(ApiError::InternalServerError)
                },
                WordpressAPIError::InvalidURL => {
                    info!("Invalid url for wordpress api: {}", base_url);
                    ApiResult::new_error(ApiError::BadRequest("Invalid URL".to_string()))
                },
                WordpressAPIError::NotFound => {
                    info!("No categories found for wordpress api: {}", base_url);
                    ApiResult::new_data(vec![].into())
                }
                WordpressAPIError::Unsupported(e) => {
                    ApiResult::new_error(ApiError::InternalServerError)
                }
                WordpressAPIError::UnexpectedResponse => {
                    ApiResult::new_error(ApiError::InternalServerError)
                }
            }
        }
    };

    ApiResult::new_data(categories)
}

/// Request parameters for generating a preview
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct PreviewRequest {
    /// Base URL of the WordPress site
    base_url: String,
    /// List of category IDs to include posts from
    include_categories: Option<Vec<usize>>,
    /// List of category IDs to exclude posts from
    exclude_categories: Option<Vec<usize>>,
    /// Only include posts published before this date
    before: Option<chrono::NaiveDate>,
    /// Only include posts modified before this date
    modified_before: Option<chrono::NaiveDate>,
    /// Only include posts published after this date
    after: Option<chrono::NaiveDate>,
    /// Only include posts modified after this date
    modified_after: Option<chrono::NaiveDate>,
    /// Number of posts per page
    per_page: Option<usize>,
    /// Page number to retrieve
    page: Option<usize>
}

/// Response for a ['PreviewRequest']
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct PostPreviewReponse{
    /// Total number of posts that matched the request (not just the ones returned as preview)
    number_of_posts: usize,
    /// Preview of x posts, depending on the per_page setting in the request
    previews: Vec<PostPreview>
}
    
/// POST /api/import/wordpress/posts-preview
/// 
/// Fetches a preview of WordPress posts based on the provided filtering criteria.
/// 
/// # Request Body
/// Expects a JSON body containing [`PreviewRequest`] with filtering parameters for WordPress posts.
/// 
/// # Returns
/// Returns a JSON response containing:
/// - On success: [`ApiResult`] with [`PostPreviewReponse`] containing the total number of matched posts and previews
/// - On error:
///   - [`ApiError::BadRequest`] if the provided URL is invalid
///   - [`ApiError::NotFound`] if the WordPress site cannot be found
///   - [`ApiError::InternalServerError`] for other errors
/// 
/// # Authentication
/// Requires a valid session
#[post("/api/import/wordpress/posts-preview", data = "<preview_request>")]
pub async fn get_wordpress_posts_preview(preview_request: Json<PreviewRequest>, _session: Session) -> Json<ApiResult<PostPreviewReponse>> {
    let preview_request = preview_request.into_inner();

    let wordpress_api = match WordpressAPI::new(preview_request.base_url.clone()){
        Ok(api) => api,
        Err(e) => {
            error!("{:?}", e);
            return ApiResult::new_error(ApiError::InternalServerError)
        }
    };

    let res = wordpress_api.get_posts(WordpressAPIContext::Embed, preview_request.page, preview_request.per_page, None, preview_request.after, preview_request.modified_after, preview_request.before, preview_request.modified_before, None, preview_request.include_categories, preview_request.exclude_categories).await;

    match res{
        Ok(res) => {
            let data = match res.data{
                PostDataType::PostPreviews(data) => data,
                _ => return ApiResult::new_error(ApiError::InternalServerError),
            };
            
            ApiResult::new_data(PostPreviewReponse{
                number_of_posts: res.number_of_records,
                previews: data,
            })
        }
        Err(e) => {
            warn!("WordpressAPIError occured getting posts preview: {:?}", e);
            match e{
                WordpressAPIError::InvalidURL => {ApiResult::new_error(ApiError::BadRequest("Invalid URL".to_string()))}
                WordpressAPIError::NotFound => {
                    ApiResult::new_error(ApiError::NotFound)
                },
                _ => {
                    ApiResult::new_error(ApiError::InternalServerError)
                }
            }
        }
    }
}

#[get("/api/import/status/<id>")]
pub async fn poll_import_status(id: String, _session: Session, import_processor: &State<Arc<ImportProcessor>>) -> Json<ApiResult<ImportStatusPoll>>{
    let job_archive = import_processor.job_archive.read().unwrap();

    let id = match uuid::Uuid::parse_str(&id){
        Ok(id) => id,
        Err(_) => return ApiResult::new_error(ApiError::BadRequest("Invalid job id".to_string()))
    };

    match job_archive.get(&id){
        Some(job) =>{
            let job = job.read().unwrap();
            let status = ImportStatusPoll{
                status: job.status.clone(),
                processed: job.processed,
                length: job.length,
            };
            return ApiResult::new_data(status);
        },
        None => ()
    }
    let job_queue = import_processor.job_queue.read().unwrap();

    let job = job_queue.iter().find(|job| job.id == id);
    match job{
        Some(job) => ApiResult::new_data(ImportStatusPoll{
            status: job.status.clone(),
            processed: job.processed,
            length: job.length,
        }),
        None => ApiResult::new_error(ApiError::NotFound)
    }
}
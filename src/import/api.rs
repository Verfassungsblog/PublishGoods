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
use crate::import::processing::{FileImportData, ImportJob, ImportJobData, ImportProcessor, ImportStatus, WordpressFilterData};
use crate::import::wordpress::{CategoryTree, PostDataType, PostPreview, WordpressAPI, WordpressAPIContext, WordpressAPIError};
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
    shift_headings_up: bool,
    convert_links: bool,
}

/// Data structure used for WordPress import operations
#[derive(Serialize, Deserialize)]
pub enum WordpressImportData {
    /// Collection of WordPress URLs to be processed for import
    WordpressLinks(Vec<String>),
    /// Filter criteria for WordPress content import, containing host URL and various filtering options
    WordpressFilter(WordpressFilterData),
}

/// Configuration for importing content to WordPress
#[derive(Serialize, Deserialize)]
struct WordpressImportRequest{
    /// The unique identifier of the target project
    project_id: Uuid,
    /// Data to be imported
    data: WordpressImportData,
    /// Whether to convert footnotes to endnotes in final output
    endnotes: bool,
    /// Whether to decrease heading levels by one so h2 becomes h1
    shift_headings: bool,
    /// Whether to convert links to citations
    convert_links: bool
}

/// POST /api/import/upload
///
/// Creates a new import job for files uploaded via a multipart form.
///
/// # Arguments
///
/// * `upload` - Form data containing files to import and import settings
///
/// # Returns
///
/// Returns a UUID that can be used to track the import job status.
/// On error returns:
/// - BadRequest if project_id is invalid or files have invalid content type
#[post("/api/import/upload", data = "<upload>")]
pub async fn import_from_upload(mut upload: Form<FileUpload<'_>>, _session: Session, settings: &State<Settings>, _project_storage: &State<Arc<ProjectStorage>>, import_processor: &State<Arc<ImportProcessor>>) -> Json<ApiResult<Uuid>>{
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

    let project_id = match Uuid::parse_str(&upload.project_id){
        Ok(id) => id,
        Err(_) => return ApiResult::new_error(ApiError::BadRequest("Invalid project id".to_string()))
    };

    let id = Uuid::new_v4();
    let import_job = ImportJob{
        id,
        project_id,
        convert_footnotes_to_endnotes: upload.convert_footnotes_to_endnotes,
        shift_headings_up: upload.shift_headings_up,
        convert_links: upload.convert_links,
        import_data: ImportJobData::FileImport(FileImportData{
            files_to_process: file_paths,
            bib_file: bib_file_path,
        }),
    };

    import_processor.job_queue.write().unwrap().push_back(import_job);

    ApiResult::new_data(id)
}

/// POST /api/import/wordpress
///
/// Creates a new WordPress import job and adds it to the import processor queue.
/// Requires a valid session.
///
/// # Parameters
/// * `job` - Import configuration containing target project, import data and processing options
///
/// # Returns
/// Returns a UUID identifying the created import job
#[post("/api/import/wordpress", data = "<job>")]
pub async fn import_from_wordpress(job: Json<WordpressImportRequest>, _session: Session, _settings: &State<Settings>, import_processor: &State<Arc<ImportProcessor>>) -> Json<ApiResult<Uuid>>{
    let id = Uuid::new_v4();

    let job = job.into_inner();

    let import_job_data = match job.data{
        WordpressImportData::WordpressLinks(links) => {
            ImportJobData::WordpressLinks(links)
        }
        WordpressImportData::WordpressFilter(filter_data) => {
            ImportJobData::WordpressFilter(filter_data)
        }
    };

    let import_job = ImportJob{
        id,
        project_id: job.project_id,
        convert_footnotes_to_endnotes: job.endnotes,
        shift_headings_up: job.shift_headings,
        convert_links: job.convert_links,
        import_data: import_job_data,
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
                WordpressAPIError::Unsupported(_) => {
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

/// GET /api/import/status/<id>
///
/// Returns the current status of an import job identified by its UUID.
///
/// # Arguments
/// * `id` - UUID string identifying the import job
/// * `_session` - Ensures request comes from authenticated user
/// * `import_processor` - State containing import job queue and archive
///
/// # Returns
/// Returns a JSON response containing either:
/// * Success: The current `ImportStatus` of the job
/// * Error: 
///   - `BadRequest` if the provided ID is not a valid UUID
///   - `NotFound` if no job with the given ID exists
#[get("/api/import/status/<id>")]
pub async fn poll_import_status(id: String, _session: Session, import_processor: &State<Arc<ImportProcessor>>) -> Json<ApiResult<ImportStatus>>{
    let id = match Uuid::parse_str(&id){
        Ok(id) => id,
        Err(_) => return ApiResult::new_error(ApiError::BadRequest("Invalid job id".to_string()))
    };
    // Try to find job with id in archive
    match import_processor.job_archive.read().unwrap().get(&id){
        Some(status) =>{
            return ApiResult::new_data(status.clone());
        },
        None => ()
    }
    let job_queue = import_processor.job_queue.read().unwrap();
    // Job not in archive yet, try to find it in job queue
    let job = job_queue.iter().find(|job| job.id == id);
    match job{
        Some(_) => ApiResult::new_data(ImportStatus::Pending),
        None => ApiResult::new_error(ApiError::NotFound)
    }
}
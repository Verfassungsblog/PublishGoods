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
use crate::import::wordpress::{Category, CategoryTree, WordpressAPI, WordpressAPIError};
use crate::projects::api::{ApiError, ApiResult};
use crate::session::session_guard::Session;
use crate::settings::Settings;

#[derive(FromForm)]
struct FileUpload<'r>{
    files: Vec<TempFile<'r>>,
    bib_file: Option<TempFile<'r>>,
    project_id: String,
    convert_footnotes_to_endnotes: bool,
}

#[derive(Serialize, Deserialize)]
struct WordpressImport{
    project_id: uuid::Uuid,
    endnotes: bool,
    links: Vec<String>,
    shift_headings: bool,
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

/// Get wordpress categories
#[get("/api/import/wordpress/categories?<base_url>")]
pub async fn get_wordpress_categories(base_url: String, _session: Session, _settings: &State<Settings>) -> Json<ApiResult<CategoryTree>>{
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
            }
        }
    };

    ApiResult::new_data(categories)
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
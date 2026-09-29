//! External rendering API.
//!
//! Lets API clients render content from an external data source (currently a WordPress post)
//! without creating a project: the content is imported into memory, exported through the regular
//! rendering pipeline and the results are offered as download links that expire after one hour.
use crate::export::rendering_manager::{RenderingJob, RenderingManager};
use crate::import::processing::{
    ContentImportOptions, ImportError, ImportProcessor, ImportedSection, MediaStore,
    WordpressPostLocation,
};

use crate::session::session_guard::APISession;
use crate::settings::Settings;
use crate::storage::project_storage::ProjectData;
use crate::storage::project_storage::current::{BibEntryOrFolder, Bibliography};
use crate::utils::api_helpers::{ApiError, ApiErrorType};
use crate::utils::timeout::{limit_from_seconds, with_timeout};
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use dashmap::mapref::entry::Entry;
use rand::distr::{Alphanumeric, SampleString};
use rocket::State;
use rocket::fs::NamedFile;
use rocket::serde::json::Json;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use uuid::Uuid;
use vb_exchange::projects::ProjectSettingsV5;
use vb_exchange::{RenderingError, RenderingStatus};

/// How often expired jobs get removed.
const SWEEP_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Deserialize)]
pub struct RenderingSettings {
    pub csl_style: Option<String>,
    pub csl_language_code: Option<String>,
    pub add_soft_hyphens: bool,
    /// uuid of the template to be used for the rendering
    pub template_id: Uuid,
    /// List of export formats to render
    pub export_formats: Vec<String>,
    /// Convert footnotes to endnotes
    #[serde(default)]
    pub convert_footnotes_to_endnotes: bool,
    /// Shift all headings up one level (h2 becomes h1)
    #[serde(default)]
    pub shift_headings_up: bool,
    /// Try to convert external links into citations
    #[serde(default)]
    pub convert_links: bool,
    /// Import the author names of the posts. Enabled by default.
    #[serde(default = "default_true")]
    pub import_author_names: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Deserialize)]
pub struct ExternalAPIRenderingRequest {
    #[serde(default = "uuid::Uuid::new_v4")]
    pub job_id: Uuid,
    pub data_source: WordpressPostLocation,
    pub rendering_settings: RenderingSettings,
    /// Webhook to be called when rendering finished
    pub webhook_url: Option<String>,
    /// Book metadata (title, authors, ...) to use for the rendered project. If omitted, it is
    /// derived from the imported content instead.
    #[serde(default)]
    pub project_metadata: Option<crate::storage::project_storage::ProjectMetadata>,
}

/// Serializes as `{"type": "ImportError", "reason": <ImportError>}` or
/// `{"type": "RenderError", "reason": <RenderingError>}`, so the shape of `reason` is
/// predictable from `type` instead of being keyed by the variant name itself.
#[derive(Serialize, Clone, Debug)]
#[serde(tag = "type", content = "reason")]
pub enum ExternalAPIRenderingError {
    ImportError(ImportError),
    RenderError(RenderingError),
}

#[derive(Serialize, Clone, Debug)]
pub struct ExternalAPIRenderingResult {
    /// One download uri per rendered file
    pub download_uris: Vec<String>,
    /// The download uris stop working at this point
    pub expires_at: DateTime<Utc>,
}

/// Serializes as `{"status": "Pending"}` for variants without data, or
/// `{"status": "Finished", "details": <ExternalAPIRenderingResult>}` /
/// `{"status": "Failed", "details": <ExternalAPIRenderingError>}` for the two that carry a
/// payload. `status` is always a plain string, so clients can switch on it without also having
/// to know whether the value is a string or an object.
#[derive(Serialize, Clone, Debug)]
#[serde(tag = "status", content = "details")]
pub enum ExternalAPIRenderingStatusEnum {
    Pending,
    ImportRunning,
    RenderingPreprocessing,
    QueuedForRendering,
    Rendering,
    Finished(ExternalAPIRenderingResult),
    Failed(ExternalAPIRenderingError),
}

#[derive(Serialize, Clone, Debug)]
pub struct ExternalAPIRenderingStatus {
    pub job_id: Uuid,
    #[serde(flatten)]
    pub status: ExternalAPIRenderingStatusEnum,
}

/// Why a job couldn't be accepted.
#[derive(Debug)]
pub enum SubmitError {
    /// The request is invalid, contains a description of what's wrong
    Invalid(String),
    /// A job with the same id already exists
    Conflict,
    Internal,
}

/// The files that can be downloaded for a finished job.
struct Download {
    token: String,
    dir: PathBuf,
    files: Vec<String>,
    expires_at: DateTime<Utc>,
}

impl From<std::io::Error> for ExternalAPIRenderingError {
    fn from(e: std::io::Error) -> Self {
        error!("IO error in external rendering job: {}", e);
        ExternalAPIRenderingError::RenderError(RenderingError::Other("IO error".to_string()))
    }
}

impl From<ImportError> for ExternalAPIRenderingError {
    fn from(e: ImportError) -> Self {
        ExternalAPIRenderingError::ImportError(e)
    }
}

impl From<RenderingError> for ExternalAPIRenderingError {
    fn from(e: RenderingError) -> Self {
        ExternalAPIRenderingError::RenderError(e)
    }
}

impl Download {
    /// Returns the path of `filename` if `token` matches, the download hasn't expired and the
    /// file belongs to the job.
    fn resolve(&self, token: &str, filename: &str) -> Option<PathBuf> {
        if !constant_time_eq(self.token.as_bytes(), token.as_bytes())
            || Utc::now() >= self.expires_at
            || !self.files.iter().any(|f| f == filename)
        {
            return None;
        }
        Some(self.dir.join(filename))
    }

    /// Builds one absolute download uri per file, below `instance_url`.
    fn uris(&self, instance_url: &str, job_id: Uuid) -> Option<Vec<String>> {
        let mut uris = vec![];
        for file in &self.files {
            let mut uri = url::Url::parse(instance_url).ok()?;
            uri.path_segments_mut().ok()?.pop_if_empty().extend([
                "api",
                "v1",
                "rendering_jobs",
                &job_id.to_string(),
                "files",
                &self.token,
                file,
            ]);
            uris.push(uri.to_string());
        }
        Some(uris)
    }
}

struct JobEntry {
    /// User owning the api key that created the job
    owner: Uuid,
    /// Id under which the rendering shows up in [`RenderingManager::requests_archive`]. Not the
    /// job id, so a client-chosen job id can never collide with an internal rendering request.
    render_request_id: Uuid,
    status: ExternalAPIRenderingStatusEnum,
    /// When the job finished or failed
    finished_at: Option<Instant>,
    download: Option<Download>,
}

pub struct ExternalRenderManager {
    /// Copy of the global settings
    settings: Settings,
    /// Used to render the imported content
    rendering_manager: Arc<RenderingManager>,
    /// Only used for its `fetch_*` functions: content is imported into memory
    importer: ImportProcessor,
    http_client: reqwest::Client,
    /// Information about jobs that are waiting, running, finished or failed
    job_archive: DashMap<Uuid, JobEntry>,
    /// Limits the number of jobs processed concurrently to `max_external_rendering_jobs`.
    /// Waiting for a permit is fair, so jobs are processed in the order they were submitted.
    slots: Arc<Semaphore>,
    /// How long a finished job's results stay downloadable, from `settings.external_rendering_result_validity`.
    result_validity: Duration,
}

impl ExternalRenderManager {
    pub fn init(
        settings: Settings,
        pool: sqlx::PgPool,
        rendering_manager: Arc<RenderingManager>,
    ) -> Arc<Self> {
        let max_jobs = (settings.max_external_rendering_jobs as usize).max(1);
        let result_validity = Duration::from_secs(settings.external_rendering_result_validity);
        let manager = Arc::new(ExternalRenderManager {
            importer: ImportProcessor::new(settings.clone(), pool),
            settings,
            rendering_manager,
            http_client: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
            job_archive: DashMap::new(),
            slots: Arc::new(Semaphore::new(max_jobs)),
            result_validity,
        });

        // Jobs only live in memory, so whatever is left on disk from a previous run is orphaned.
        let _ = std::fs::remove_dir_all(manager.base_dir());

        let manager_cpy = Arc::clone(&manager);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(SWEEP_INTERVAL).await;
                manager_cpy.remove_expired_jobs().await;
            }
        });

        manager
    }

    fn base_dir(&self) -> PathBuf {
        PathBuf::from(format!("{}/external_renders", self.settings.data_path))
    }

    /// Validates the request and queues it. The job is processed as soon as a slot is free.
    pub async fn submit(
        self: &Arc<Self>,
        owner: Uuid,
        request: ExternalAPIRenderingRequest,
    ) -> Result<ExternalAPIRenderingStatus, SubmitError> {
        self.validate(&request).await?;

        let job_id = request.job_id;
        match self.job_archive.entry(job_id) {
            Entry::Occupied(_) => return Err(SubmitError::Conflict),
            Entry::Vacant(vacant) => {
                vacant.insert(JobEntry {
                    owner,
                    render_request_id: Uuid::new_v4(),
                    status: ExternalAPIRenderingStatusEnum::Pending,
                    finished_at: None,
                    download: None,
                });
            }
        }

        Self::process_job(Arc::clone(self), request);

        Ok(ExternalAPIRenderingStatus {
            job_id,
            status: ExternalAPIRenderingStatusEnum::Pending,
        })
    }

    async fn validate(&self, request: &ExternalAPIRenderingRequest) -> Result<(), SubmitError> {
        let invalid = |msg: &str| Err(SubmitError::Invalid(msg.to_string()));

        // `data_source` is validated once, by the import manager, when the job actually runs.

        if let Some(webhook) = &request.webhook_url {
            match url::Url::parse(webhook) {
                Ok(url) if matches!(url.scheme(), "http" | "https") && url.host().is_some() => {}
                _ => return invalid("webhook_url: invalid url"),
            }
        }

        let rendering = &request.rendering_settings;
        if rendering.export_formats.is_empty() {
            return invalid("rendering_settings: at least one export format is required");
        }
        if let Some(style) = &rendering.csl_style
            && !self.rendering_manager.csl_data.styles.contains_key(style)
        {
            return invalid("rendering_settings: unknown csl_style");
        }
        Ok(())
    }

    /// Returns the status of a job, or None if the job doesn't exist (anymore) or belongs to
    /// another user.
    pub fn get_status(&self, job_id: Uuid, owner: Uuid) -> Option<ExternalAPIRenderingStatus> {
        let entry = self.job_archive.get(&job_id)?;
        if entry.owner != owner {
            return None;
        }

        let status = match &entry.status {
            // While the rendering pipeline is working on the job it knows the more detailed status
            ExternalAPIRenderingStatusEnum::RenderingPreprocessing
            | ExternalAPIRenderingStatusEnum::QueuedForRendering
            | ExternalAPIRenderingStatusEnum::Rendering => {
                match self
                    .rendering_manager
                    .requests_archive
                    .read()
                    .unwrap()
                    .get(&entry.render_request_id)
                {
                    Some(status) => map_rendering_status(status),
                    // The rendering pipeline is done, the result is being collected
                    None => ExternalAPIRenderingStatusEnum::Rendering,
                }
            }
            other => other.clone(),
        };
        Some(ExternalAPIRenderingStatus { job_id, status })
    }

    /// Returns the path of a downloadable file if `token` is valid for the job, the file
    /// belongs to the job and the download hasn't expired yet.
    pub fn resolve_download(&self, job_id: Uuid, token: &str, filename: &str) -> Option<PathBuf> {
        let entry = self.job_archive.get(&job_id)?;
        entry.download.as_ref()?.resolve(token, filename)
    }

    fn set_status(&self, job_id: Uuid, status: ExternalAPIRenderingStatusEnum) {
        if let Some(mut entry) = self.job_archive.get_mut(&job_id) {
            entry.status = status;
        }
    }

    fn process_job(manager: Arc<Self>, job: ExternalAPIRenderingRequest) {
        tokio::spawn(async move {
            let job_id = job.job_id;
            let webhook_url = job.webhook_url.clone();

            // Wait for a free slot, other jobs may still be running
            let permit = manager.slots.clone().acquire_owned().await;

            // Run in its own task so a panic can't leave the job hanging in a running state
            let worker = Arc::clone(&manager);
            let outcome = tokio::spawn(async move { worker.run_job(&job).await }).await;
            drop(permit);

            let outcome = outcome.unwrap_or_else(|e| {
                error!("External rendering job {} panicked: {}", job_id, e);
                Err(ExternalAPIRenderingError::RenderError(
                    RenderingError::Other("Internal error".to_string()),
                ))
            });

            let status = match outcome {
                Ok(download) => {
                    let result = match manager.download_uris(job_id, &download) {
                        Ok(uris) => {
                            ExternalAPIRenderingStatusEnum::Finished(ExternalAPIRenderingResult {
                                download_uris: uris,
                                expires_at: download.expires_at,
                            })
                        }
                        Err(e) => ExternalAPIRenderingStatusEnum::Failed(e),
                    };
                    if let Some(mut entry) = manager.job_archive.get_mut(&job_id) {
                        entry.download = Some(download);
                    }
                    result
                }
                Err(e) => ExternalAPIRenderingStatusEnum::Failed(e),
            };

            if let Some(mut entry) = manager.job_archive.get_mut(&job_id) {
                entry.status = status.clone();
                entry.finished_at = Some(Instant::now());
            }

            if let Some(webhook_url) = webhook_url {
                manager
                    .call_webhook(&webhook_url, &ExternalAPIRenderingStatus { job_id, status })
                    .await;
            }
        });
    }

    /// Imports the data source and renders it. On success the results are in the returned
    /// [`Download`]'s directory, on failure nothing is left on disk.
    async fn run_job(
        &self,
        job: &ExternalAPIRenderingRequest,
    ) -> Result<Download, ExternalAPIRenderingError> {
        let job_dir = self.base_dir().join(job.job_id.to_string());
        let media = MediaStore {
            dir: job_dir.join("uploads"),
            url_prefix: format!("/external/{}/uploads", job.job_id),
        };
        let result_dir = job_dir.join("results");

        let result = self.import_and_render(job, &media, &result_dir).await;

        // Downloaded media is only needed while rendering
        let _ = tokio::fs::remove_dir_all(&media.dir).await;
        if result.is_err() {
            let _ = tokio::fs::remove_dir_all(&job_dir).await;
        }
        result
    }

    async fn import_and_render(
        &self,
        job: &ExternalAPIRenderingRequest,
        media: &MediaStore,
        result_dir: &PathBuf,
    ) -> Result<Download, ExternalAPIRenderingError> {
        let job_id = job.job_id;
        let rendering = &job.rendering_settings;

        self.set_status(job_id, ExternalAPIRenderingStatusEnum::ImportRunning);

        let options = ContentImportOptions {
            convert_footnotes_to_endnotes: rendering.convert_footnotes_to_endnotes,
            shift_headings_up: rendering.shift_headings_up,
            convert_links: rendering.convert_links,
            import_author_names: rendering.import_author_names,
        };
        let import_timeout = self.settings.import_timeout;
        let imported = with_timeout(
            limit_from_seconds(import_timeout),
            self.importer
                .fetch_wordpress(&job.data_source, media, options),
        )
        .await
        .map_err(|_| {
            warn!(
                "Import for job {} timed out after {} seconds",
                job_id, import_timeout
            );
            ExternalAPIRenderingError::ImportError(ImportError::Timeout)
        })??;
        if imported.is_empty() {
            return Err(ExternalAPIRenderingError::ImportError(
                ImportError::WordPressApiError(
                    crate::import::wordpress::WordpressAPIError::NotFound,
                ),
            ));
        }

        let render_request_id = {
            let entry = self.job_archive.get(&job_id).ok_or_else(|| {
                ExternalAPIRenderingError::RenderError(RenderingError::Other(
                    "Job was removed".to_string(),
                ))
            })?;
            entry.render_request_id
        };
        // Insert into `requests_archive` before advertising `RenderingPreprocessing`, so a
        // concurrent `get_status` call always finds a matching entry there instead of racing into
        // the `None` branch that's meant only for the finished-and-collecting case.
        self.rendering_manager
            .requests_archive
            .write()
            .unwrap()
            .insert(render_request_id, RenderingStatus::PreparingOnLocal);
        self.set_status(
            job_id,
            ExternalAPIRenderingStatusEnum::RenderingPreprocessing,
        );

        // Share the rendering server connection limit with the internal queue: hold a permit for
        // the whole preparation + send, not just while queued.
        let _permit = self
            .rendering_manager
            .slots
            .clone()
            .acquire_owned()
            .await
            .expect("semaphore is never closed");

        let render_result = RenderingManager::prepare_and_send(
            Arc::clone(&self.rendering_manager),
            RenderingJob {
                request_id: render_request_id,
                project_data: build_project_data(imported, rendering, job.project_metadata.clone()),
                uploads_dir: media.dir.clone(),
                sections: None,
                export_formats: rendering.export_formats.clone(),
                result_dir: Some(result_dir.clone()),
            },
        )
        .await;
        drop(_permit);
        let final_status = self
            .rendering_manager
            .requests_archive
            .write()
            .unwrap()
            .remove(&render_request_id);

        render_result?;
        let dir = match final_status {
            Some(RenderingStatus::SavedOnLocal(_, dir)) => dir,
            Some(RenderingStatus::Failed(e)) => {
                return Err(ExternalAPIRenderingError::RenderError(e));
            }
            _ => {
                return Err(ExternalAPIRenderingError::RenderError(
                    RenderingError::NoResultFiles,
                ));
            }
        };

        let mut files = vec![];
        let mut read_dir = tokio::fs::read_dir(&dir).await?;
        while let Some(entry) = read_dir.next_entry().await? {
            if entry.file_type().await?.is_file()
                && let Some(name) = entry.file_name().to_str()
            {
                files.push(name.to_string());
            }
        }
        if files.is_empty() {
            return Err(ExternalAPIRenderingError::RenderError(
                RenderingError::NoResultFiles,
            ));
        }
        files.sort();

        Ok(Download {
            token: Alphanumeric.sample_string(&mut rand::rng(), 40),
            dir,
            files,
            expires_at: Utc::now() + self.result_validity,
        })
    }

    fn download_uris(
        &self,
        job_id: Uuid,
        download: &Download,
    ) -> Result<Vec<String>, ExternalAPIRenderingError> {
        download
            .uris(&self.settings.instance_url, job_id)
            .ok_or_else(|| {
                error!("instance_url is not a valid base url for download links");
                ExternalAPIRenderingError::RenderError(RenderingError::Other(
                    "Server misconfigured".to_string(),
                ))
            })
    }

    /// Absolute url of `GET /api/v1/rendering_jobs/<job_id>`, used as the `Location` header of
    /// the 202 response returned when the job is submitted.
    fn status_uri(&self, job_id: Uuid) -> Option<String> {
        let mut uri = url::Url::parse(&self.settings.instance_url).ok()?;
        uri.path_segments_mut().ok()?.pop_if_empty().extend([
            "api",
            "v1",
            "rendering_jobs",
            &job_id.to_string(),
        ]);
        Some(uri.to_string())
    }

    /// Notifies the webhook about the final status of a job. Failures are only logged.
    async fn call_webhook(&self, url: &str, status: &ExternalAPIRenderingStatus) {
        match self.http_client.post(url).json(status).send().await {
            Ok(response) if response.status().is_success() => {}
            Ok(response) => warn!(
                "Webhook for job {} responded with {}",
                status.job_id,
                response.status()
            ),
            Err(e) => warn!("Couldn't call webhook for job {}: {}", status.job_id, e),
        }
    }

    /// Removes finished jobs (and their files) once they're older than `result_validity`.
    async fn remove_expired_jobs(&self) {
        let expired: Vec<Uuid> = self
            .job_archive
            .iter()
            .filter(|entry| {
                entry
                    .finished_at
                    .is_some_and(|finished| finished.elapsed() >= self.result_validity)
            })
            .map(|entry| *entry.key())
            .collect();

        for job_id in expired {
            // Delete the files before freeing the job id: while the job is still archived, a
            // resubmission with the same id is rejected, so it can't have its files deleted here.
            let dir = self.base_dir().join(job_id.to_string());
            if let Err(e) = tokio::fs::remove_dir_all(&dir).await
                && e.kind() != std::io::ErrorKind::NotFound
            {
                warn!("Couldn't remove expired rendering job {:?}: {}", dir, e);
            }
            self.job_archive.remove(&job_id);
        }
    }
}

/// Maps the status of the internal rendering pipeline to the status shown to API clients.
fn map_rendering_status(status: &RenderingStatus) -> ExternalAPIRenderingStatusEnum {
    match status {
        RenderingStatus::QueuedOnLocal | RenderingStatus::PreparingOnLocal => {
            ExternalAPIRenderingStatusEnum::RenderingPreprocessing
        }
        RenderingStatus::PreparedOnLocal
        | RenderingStatus::SendToRenderingServer
        | RenderingStatus::RequestingTemplate
        | RenderingStatus::TransmittingTemplate
        | RenderingStatus::QueuedOnRendering => ExternalAPIRenderingStatusEnum::QueuedForRendering,
        // The final status is set by the job itself after collecting the result files
        RenderingStatus::Running
        | RenderingStatus::Finished(_)
        | RenderingStatus::SavedOnLocal(_, _)
        | RenderingStatus::Failed(_) => ExternalAPIRenderingStatusEnum::Rendering,
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Assembles an in-memory project from the imported sections. If `metadata_override` is given,
/// it is used as-is; otherwise the book metadata is derived from the imported content: a single
/// post provides title, authors and so on.
fn build_project_data(
    imported: Vec<ImportedSection>,
    settings: &RenderingSettings,
    metadata_override: Option<crate::storage::project_storage::ProjectMetadata>,
) -> ProjectData {
    let mut bibliography = HashMap::new();
    let mut sections = vec![];
    for imported_section in imported {
        for entry in imported_section.bib_entries {
            bibliography.insert(entry.key, BibEntryOrFolder::BibEntry(entry));
        }
        sections.push(imported_section.section);
    }

    let metadata = metadata_override.unwrap_or_else(|| {
        let mut metadata = crate::storage::project_storage::ProjectMetadata::default();
        if let [section] = sections.as_slice() {
            let m = &section.metadata;
            metadata.title = m.title.clone();
            metadata.subtitle = m.subtitle.clone();
            metadata.authors = Some(m.authors.clone());
            metadata.web_url = m.web_url.clone();
            metadata.identifiers = Some(m.identifiers.clone());
            metadata.published = m.published;
            metadata.languages = m.lang.map(|lang| vec![lang]);
        } else {
            metadata.title = "Imported posts".to_string();
        }
        metadata
    });

    ProjectData {
        name: metadata.title.clone(),
        description: None,
        template_id: settings.template_id,
        last_interaction: 0,
        metadata: Some(metadata),
        settings: Some(ProjectSettingsV5 {
            toc_enabled: sections.len() > 1,
            csl_style: settings.csl_style.clone(),
            csl_language_code: settings.csl_language_code.clone(),
            add_soft_hyphens: settings.add_soft_hyphens,
            ..Default::default()
        }),
        sections,
        bibliography: Bibliography {
            entries: bibliography,
        },
    }
}

/// 202 Accepted carrying a `Location` header that points at the job's status url, per RFC 7231
/// ("the server SHOULD include ... a Location header field with a URI that ... allows the user
/// agent to monitor the status of the request").
pub struct RenderJobAccepted<R>(Option<String>, R);

impl<'r, 'o: 'r, R: rocket::response::Responder<'r, 'o>> rocket::response::Responder<'r, 'o>
    for RenderJobAccepted<R>
{
    fn respond_to(self, req: &'r rocket::Request<'_>) -> rocket::response::Result<'o> {
        let mut response = rocket::Response::build_from(self.1.respond_to(req)?);
        response.status(rocket::http::Status::Accepted);
        if let Some(location) = self.0 {
            response.raw_header("Location", location);
        }
        response.ok()
    }
}

/// POST /api/v1/rendering_jobs
///
/// Creates a new rendering job. The job is processed asynchronously, poll
/// `GET /api/v1/rendering_jobs/<id>` or supply a `webhook_url` to get the result.
///
/// Requires the `X-API-KEY` header. `job_id` is optional and defaults to a random uuid; pass one
/// explicitly to make the request idempotent (resubmitting the same `job_id` returns a 409
/// Conflict instead of creating a second job). `data_source` accepts either a `WordPressByURL`
/// variant (a link to a single post) or a `WordPressBySlug` variant (`host` + `slug`).
/// `rendering_settings.template_id` and every entry of `export_formats` must exist on that
/// template. `project_metadata` is optional; every one of its fields is optional too, and any
/// field left out (or set to `null`) is instead derived from the imported post.
///
/// Example request body:
/// ```json
/// {
///   "data_source": {
///     "WordPressByURL": "https://example.org/2024/01/01/my-post/"
///   },
///   "rendering_settings": {
///     "csl_style": "din-1505-2",
///     "csl_language_code": "de-DE",
///     "add_soft_hyphens": true,
///     "template_id": "3fa85f64-5717-4562-b3fc-2c963f66afa6",
///     "export_formats": ["pdf", "epub"],
///     "convert_footnotes_to_endnotes": false,
///     "shift_headings_up": true,
///     "convert_links": true,
///     "import_author_names": true
///   },
///   "webhook_url": "https://example.org/webhooks/rendering-finished",
///   "project_metadata": {
///     "title": "My Book",
///     "subtitle": "A collection of posts",
///     "authors": [
///       {"NameString": "Jane Doe"}
///     ],
///     "editors": null,
///     "web_url": "https://example.org/2024/01/01/my-post/",
///     "identifiers": null,
///     "published": "2024-01-01",
///     "languages": null,
///     "number_of_pages": null,
///     "short_abstract": null,
///     "long_abstract": null,
///     "keywords": null,
///     "ddc": null,
///     "license": null,
///     "series": null,
///     "volume": null,
///     "edition": null,
///     "publisher": null,
///     "custom_fields": {}
///   }
/// }
/// ```
///
/// `data_source` can instead be:
/// ```json
/// { "WordPressBySlug": { "host": "example.org", "slug": "my-post" } }
/// ```
///
/// Optional fields (`job_id`, `csl_style`, `csl_language_code`, `webhook_url`,
/// `project_metadata`, and every `rendering_settings` field with a `default` note above) may be
/// omitted entirely instead of set to `null`. The smallest valid request is:
/// ```json
/// {
///   "data_source": { "WordPressByURL": "https://example.org/2024/01/01/my-post/" },
///   "rendering_settings": {
///     "add_soft_hyphens": false,
///     "template_id": "3fa85f64-5717-4562-b3fc-2c963f66afa6",
///     "export_formats": ["pdf"]
///   }
/// }
/// ```
///
/// 202 Accepted, with a `Location` header pointing at `GET /api/v1/rendering_jobs/<id>` and a
/// body of identical shape (see below):
/// ```json
/// {
///   "job_id": "8c2e6f2a-8b0a-4a3e-9c1a-3e6b2f8d9a10",
///   "status": "Pending"
/// }
/// ```
#[post("/api/v1/rendering_jobs", format = "json", data = "<data>")]
pub async fn add_render_job(
    api_session: APISession,
    manager: &State<Arc<ExternalRenderManager>>,
    data: Json<ExternalAPIRenderingRequest>,
) -> Result<RenderJobAccepted<Json<ExternalAPIRenderingStatus>>, ApiError> {
    let manager = manager.inner();
    match manager.submit(api_session.user_id, data.into_inner()).await {
        Ok(status) => {
            let location = manager.status_uri(status.job_id);
            if location.is_none() {
                error!("instance_url is not a valid base url for the Location header");
            }
            Ok(RenderJobAccepted(location, Json(status)))
        }
        Err(SubmitError::Invalid(msg)) => Err(ApiErrorType::BadRequest(msg).into()),
        Err(SubmitError::Conflict) => Err(ApiErrorType::Conflict(
            "a rendering job with this job_id already exists".to_string(),
        )
        .into()),
        Err(SubmitError::Internal) => Err(ApiErrorType::InternalServerError.into()),
    }
}

/// GET /api/v1/rendering_jobs/<id>
///
/// Returns the status of a rendering job. Finished jobs are available for one hour. Requires the
/// `X-API-KEY` header of the api key that created the job; a job created by another user (or an
/// unknown id) yields a 404.
///
/// `status` moves through `Pending` -> `ImportRunning` -> `RenderingPreprocessing` ->
/// `QueuedForRendering` -> `Rendering` and ends in either `Finished` or `Failed`. `status` is
/// always a plain string; `Finished` and `Failed` additionally carry a `details` object next to
/// it:
///
/// ```json
/// { "job_id": "8c2e6f2a-8b0a-4a3e-9c1a-3e6b2f8d9a10", "status": "Pending" }
/// ```
/// ```json
/// { "job_id": "8c2e6f2a-8b0a-4a3e-9c1a-3e6b2f8d9a10", "status": "Rendering" }
/// ```
///
/// Finished, with one download uri per rendered file. The download uris need no api key and
/// expire at the same time as the job itself:
/// ```json
/// {
///   "job_id": "8c2e6f2a-8b0a-4a3e-9c1a-3e6b2f8d9a10",
///   "status": "Finished",
///   "details": {
///     "download_uris": [
///       "https://example.org/api/v1/rendering_jobs/8c2e6f2a-8b0a-4a3e-9c1a-3e6b2f8d9a10/files/aB3dE6fG9hJk/My%20Book.pdf",
///       "https://example.org/api/v1/rendering_jobs/8c2e6f2a-8b0a-4a3e-9c1a-3e6b2f8d9a10/files/aB3dE6fG9hJk/My%20Book.epub"
///     ],
///     "expires_at": "2024-01-01T13:00:00Z"
///   }
/// }
/// ```
///
/// Failed, wrapping either an `ImportError` or a `RenderError` (see those enums for the full
/// list of variants) under `details.type` / `details.reason`:
/// ```json
/// {
///   "job_id": "8c2e6f2a-8b0a-4a3e-9c1a-3e6b2f8d9a10",
///   "status": "Failed",
///   "details": { "type": "ImportError", "reason": "Timeout" }
/// }
/// ```
#[get("/api/v1/rendering_jobs/<id>")]
pub fn get_render_job_status(
    api_session: APISession,
    manager: &State<Arc<ExternalRenderManager>>,
    id: String,
) -> Result<Json<ExternalAPIRenderingStatus>, ApiError> {
    let id = Uuid::parse_str(&id)?;
    manager
        .get_status(id, api_session.user_id)
        .map(Json)
        .ok_or_else(|| ApiErrorType::ResourceNotFound("rendering_job".to_string()).into())
}

/// GET /api/v1/rendering_jobs/<id>/files/<token>/<filename>
///
/// Downloads a rendered file. The random token in the url authorizes the download, so no api key
/// is needed. Only valid for one hour after the job finished.
#[get("/api/v1/rendering_jobs/<id>/files/<token>/<filename>")]
pub async fn download_render_result(
    manager: &State<Arc<ExternalRenderManager>>,
    id: &str,
    token: &str,
    filename: &str,
) -> Result<NamedFile, ApiError> {
    let not_found = || ApiError::from(ApiErrorType::ResourceNotFound("file".to_string()));
    let id = Uuid::parse_str(id)?;
    let path = manager
        .resolve_download(id, token, filename)
        .ok_or_else(not_found)?;
    NamedFile::open(path).await.map_err(|_| not_found())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::project_storage::current::PersonUuidOrString;
    use crate::storage::project_storage::sections::{Section, SectionMetadata};

    fn download(expires_in: chrono::Duration) -> Download {
        Download {
            token: "secret".to_string(),
            dir: PathBuf::from("/results"),
            files: vec!["book.pdf".to_string(), "my book.epub".to_string()],
            expires_at: Utc::now() + expires_in,
        }
    }

    #[test]
    fn download_requires_matching_token_and_known_file() {
        let d = download(chrono::Duration::hours(1));
        assert_eq!(
            d.resolve("secret", "book.pdf"),
            Some(PathBuf::from("/results/book.pdf"))
        );
        assert_eq!(d.resolve("wrong!", "book.pdf"), None);
        assert_eq!(d.resolve("secret", "other.pdf"), None);
        assert_eq!(d.resolve("secret", "../book.pdf"), None);
    }

    #[test]
    fn expired_download_is_rejected() {
        let d = download(chrono::Duration::seconds(-1));
        assert_eq!(d.resolve("secret", "book.pdf"), None);
    }

    #[test]
    fn download_uris_are_percent_encoded_and_keep_instance_path() {
        let d = download(chrono::Duration::hours(1));
        let id = Uuid::nil();
        let uris = d.uris("https://example.org/prefix/", id).unwrap();
        assert_eq!(
            uris[1],
            format!(
                "https://example.org/prefix/api/v1/rendering_jobs/{id}/files/secret/my%20book.epub"
            )
        );
        assert!(d.uris("not a url", id).is_none());
    }

    #[test]
    fn status_is_always_a_plain_string_with_payload_alongside_it() {
        let job_id = Uuid::nil();

        let pending = ExternalAPIRenderingStatus {
            job_id,
            status: ExternalAPIRenderingStatusEnum::Pending,
        };
        assert_eq!(
            serde_json::to_value(&pending).unwrap(),
            serde_json::json!({"job_id": job_id, "status": "Pending"})
        );

        let finished = ExternalAPIRenderingStatus {
            job_id,
            status: ExternalAPIRenderingStatusEnum::Finished(ExternalAPIRenderingResult {
                download_uris: vec!["https://example.org/book.pdf".to_string()],
                expires_at: DateTime::parse_from_rfc3339("2024-01-01T13:00:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
            }),
        };
        assert_eq!(
            serde_json::to_value(&finished).unwrap(),
            serde_json::json!({
                "job_id": job_id,
                "status": "Finished",
                "details": {
                    "download_uris": ["https://example.org/book.pdf"],
                    "expires_at": "2024-01-01T13:00:00Z"
                }
            })
        );

        let failed = ExternalAPIRenderingStatus {
            job_id,
            status: ExternalAPIRenderingStatusEnum::Failed(ExternalAPIRenderingError::ImportError(
                ImportError::Timeout,
            )),
        };
        assert_eq!(
            serde_json::to_value(&failed).unwrap(),
            serde_json::json!({
                "job_id": job_id,
                "status": "Failed",
                "details": {"type": "ImportError", "reason": "Timeout"}
            })
        );
    }

    #[test]
    fn rendering_status_maps_to_external_status() {
        use ExternalAPIRenderingStatusEnum as E;
        assert!(matches!(
            map_rendering_status(&RenderingStatus::PreparingOnLocal),
            E::RenderingPreprocessing
        ));
        assert!(matches!(
            map_rendering_status(&RenderingStatus::QueuedOnRendering),
            E::QueuedForRendering
        ));
        assert!(matches!(
            map_rendering_status(&RenderingStatus::Running),
            E::Rendering
        ));
    }

    fn imported(title: &str, author: &str) -> ImportedSection {
        ImportedSection {
            section: Section {
                id: Some(Uuid::new_v4()),
                css_classes: vec![],
                sub_sections: vec![],
                content: vec![],
                visible_in_toc: true,
                metadata: SectionMetadata {
                    title: title.to_string(),
                    toc_title_subtitle_override: None,
                    subtitle: None,
                    authors: vec![PersonUuidOrString::NameString(author.to_string())],
                    editors: vec![],
                    web_url: Some("https://example.org/post".to_string()),
                    identifiers: vec![],
                    published: None,
                    last_changed: None,
                    lang: None,
                    custom_fields: HashMap::new(),
                },
            },
            bib_entries: vec![],
        }
    }

    fn settings() -> RenderingSettings {
        RenderingSettings {
            csl_style: Some("apa".to_string()),
            csl_language_code: None,
            add_soft_hyphens: true,
            template_id: Uuid::new_v4(),
            export_formats: vec!["pdf".to_string()],
            convert_footnotes_to_endnotes: false,
            shift_headings_up: false,
            convert_links: false,
            import_author_names: true,
        }
    }

    #[test]
    fn import_options_are_optional_and_default_to_previous_behaviour() {
        let json = serde_json::json!({
            "add_soft_hyphens": false,
            "template_id": Uuid::nil(),
            "export_formats": ["pdf"],
        });
        let s: RenderingSettings = serde_json::from_value(json).unwrap();
        assert!(!s.convert_footnotes_to_endnotes);
        assert!(!s.shift_headings_up);
        assert!(!s.convert_links);
        assert!(s.import_author_names);
    }

    #[test]
    fn import_options_can_be_set() {
        let json = serde_json::json!({
            "add_soft_hyphens": false,
            "template_id": Uuid::nil(),
            "export_formats": ["pdf"],
            "convert_footnotes_to_endnotes": true,
            "shift_headings_up": true,
            "convert_links": true,
            "import_author_names": false,
        });
        let s: RenderingSettings = serde_json::from_value(json).unwrap();
        assert!(s.convert_footnotes_to_endnotes && s.shift_headings_up && s.convert_links);
        assert!(!s.import_author_names);
    }

    #[test]
    fn single_post_provides_book_metadata() {
        let data = build_project_data(vec![imported("A post", "Jane")], &settings(), None);
        let metadata = data.metadata.unwrap();
        assert_eq!(metadata.title, "A post");
        assert_eq!(data.name, "A post");
        assert_eq!(
            metadata.authors,
            Some(vec![PersonUuidOrString::NameString("Jane".to_string())])
        );
        let project_settings = data.settings.unwrap();
        assert_eq!(project_settings.csl_style.as_deref(), Some("apa"));
        assert!(project_settings.add_soft_hyphens);
        assert!(!project_settings.toc_enabled);
        assert_eq!(data.sections.len(), 1);
    }

    #[test]
    fn multiple_posts_get_generic_title_and_toc() {
        let data = build_project_data(
            vec![imported("One", "A"), imported("Two", "B")],
            &settings(),
            None,
        );
        assert_eq!(data.metadata.unwrap().title, "Imported posts");
        assert!(data.settings.unwrap().toc_enabled);
        assert_eq!(data.sections.len(), 2);
    }

    /// The exact request bodies from `add_render_job`'s doc comment, parsed to catch drift
    /// between the documentation and [`ExternalAPIRenderingRequest`]'s actual fields.
    #[test]
    fn documented_request_bodies_still_deserialize() {
        let full: ExternalAPIRenderingRequest = serde_json::from_str(
            r#"{
              "data_source": { "WordPressByURL": "https://example.org/2024/01/01/my-post/" },
              "rendering_settings": {
                "csl_style": "din-1505-2",
                "csl_language_code": "de-DE",
                "add_soft_hyphens": true,
                "template_id": "3fa85f64-5717-4562-b3fc-2c963f66afa6",
                "export_formats": ["pdf", "epub"],
                "convert_footnotes_to_endnotes": false,
                "shift_headings_up": true,
                "convert_links": true,
                "import_author_names": true
              },
              "webhook_url": "https://example.org/webhooks/rendering-finished",
              "project_metadata": {
                "title": "My Book",
                "subtitle": "A collection of posts",
                "authors": [
                  {"NameString": "Jane Doe"}
                ],
                "editors": null,
                "web_url": "https://example.org/2024/01/01/my-post/",
                "identifiers": null,
                "published": "2024-01-01",
                "languages": null,
                "number_of_pages": null,
                "short_abstract": null,
                "long_abstract": null,
                "keywords": null,
                "ddc": null,
                "license": null,
                "series": null,
                "volume": null,
                "edition": null,
                "publisher": null,
                "custom_fields": {}
              }
            }"#,
        )
        .expect("full documented request body must deserialize");
        assert_eq!(full.rendering_settings.export_formats, vec!["pdf", "epub"]);
        assert_eq!(full.project_metadata.unwrap().title, "My Book");

        let by_slug: WordpressPostLocation = serde_json::from_str(
            r#"{ "WordPressBySlug": { "host": "example.org", "slug": "my-post" } }"#,
        )
        .expect("documented WordPressBySlug variant must deserialize");
        assert!(matches!(
            by_slug,
            WordpressPostLocation::WordPressBySlug { .. }
        ));

        let smallest: ExternalAPIRenderingRequest = serde_json::from_str(
            r#"{
              "data_source": { "WordPressByURL": "https://example.org/2024/01/01/my-post/" },
              "rendering_settings": {
                "add_soft_hyphens": false,
                "template_id": "3fa85f64-5717-4562-b3fc-2c963f66afa6",
                "export_formats": ["pdf"]
              }
            }"#,
        )
        .expect("smallest documented request body must deserialize");
        assert!(smallest.webhook_url.is_none());
        assert!(smallest.project_metadata.is_none());
        assert!(!smallest.rendering_settings.convert_footnotes_to_endnotes);
    }

    /// The exact "Finished" response body shown in `get_render_job_status`'s doc comment,
    /// serialized from a real `ExternalAPIRenderingStatus` and compared byte-for-byte against
    /// the documented JSON (parsed as [`serde_json::Value`] so field order doesn't matter).
    /// `ExternalAPIRenderingStatus` is response-only (no `Deserialize`), so this checks the
    /// documentation against the serializer directly rather than parsing the doc's JSON back.
    #[test]
    fn documented_finished_response_body_matches_the_serialized_type() {
        let documented: serde_json::Value = serde_json::from_str(
            r#"{
              "job_id": "8c2e6f2a-8b0a-4a3e-9c1a-3e6b2f8d9a10",
              "status": "Finished",
              "details": {
                "download_uris": [
                  "https://example.org/api/v1/rendering_jobs/8c2e6f2a-8b0a-4a3e-9c1a-3e6b2f8d9a10/files/aB3dE6fG9hJk/My%20Book.pdf",
                  "https://example.org/api/v1/rendering_jobs/8c2e6f2a-8b0a-4a3e-9c1a-3e6b2f8d9a10/files/aB3dE6fG9hJk/My%20Book.epub"
                ],
                "expires_at": "2024-01-01T13:00:00Z"
              }
            }"#,
        )
        .unwrap();

        let actual = ExternalAPIRenderingStatus {
            job_id: Uuid::parse_str("8c2e6f2a-8b0a-4a3e-9c1a-3e6b2f8d9a10").unwrap(),
            status: ExternalAPIRenderingStatusEnum::Finished(ExternalAPIRenderingResult {
                download_uris: vec![
                    "https://example.org/api/v1/rendering_jobs/8c2e6f2a-8b0a-4a3e-9c1a-3e6b2f8d9a10/files/aB3dE6fG9hJk/My%20Book.pdf".to_string(),
                    "https://example.org/api/v1/rendering_jobs/8c2e6f2a-8b0a-4a3e-9c1a-3e6b2f8d9a10/files/aB3dE6fG9hJk/My%20Book.epub".to_string(),
                ],
                expires_at: DateTime::parse_from_rfc3339("2024-01-01T13:00:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
            }),
        };

        assert_eq!(serde_json::to_value(&actual).unwrap(), documented);
    }
}

/// Runtime HTTP coverage: submits requests to the real Rocket routes (over a real Postgres pool
/// via `#[sqlx::test]`) and checks the responses against exactly the shapes documented on
/// [`add_render_job`] and [`get_render_job_status`]. `ExternalRenderManager` needs a real
/// `RenderingManager`, which in turn needs a TLS `ClientConfig` for talking to rendering
/// servers; none of these tests reach that far (they only exercise `submit`'s synchronous
/// validation and status/download lookups), so the config is built with no client cert and an
/// empty trust store rather than pulling in real certificates.
#[cfg(test)]
mod integration_tests {
    use super::*;
    use crate::db::repositories::{api_keys as api_keys_repo, users};
    use crate::export::rendering_manager::RenderingManager;
    use crate::settings::ExportServer;
    use crate::utils::csl::CslData;
    use argon2::Argon2;
    use argon2::password_hash::rand_core::OsRng as ArgonOsRng;
    use argon2::password_hash::{PasswordHasher, SaltString};
    use rocket::http::{Header, Status};
    use rocket::local::asynchronous::Client;
    use sqlx::PgPool;
    use std::path::PathBuf;

    fn test_settings(data_path: String) -> Settings {
        Settings {
            app_title: "test".to_string(),
            instance_url: "https://example.org".to_string(),
            project_cache_time: 0,
            data_path,
            database_url: "".to_string(),
            database_max_connections: 1,
            file_lock_timeout: 0,
            backup_to_file_interval: 0,
            max_connections_to_rendering_server: 1,
            max_import_threads: 1,
            max_external_rendering_jobs: 1,
            external_rendering_result_validity: 3600,
            rendering_server_timeout: 0,
            rendering_server_connect_timeout: 0,
            rendering_preprocessing_timeout: 0,
            import_timeout: 5,
            zotero_translation_server: "".to_string(),
            export_servers: vec![ExportServer {
                hostname: "".to_string(),
                port: 0,
                domain_name: "".to_string(),
            }],
            ca_cert_path: "".to_string(),
            client_cert_path: "".to_string(),
            client_key_path: "".to_string(),
            revocation_list_path: "".to_string(),
            version: "test".to_string(),
            max_login_attempts: 5,
            lockout_window_minutes: 15,
            smtp_connection_url: "".to_string(),
            mail_from_address: "".to_string(),
            smtp_pool_min_idle: 0,
            smtp_pool_max_size: 0,
            smtp_pool_idle_timeout: 0,
            mail_max_retries: 0,
            mail_base_retry_delay_seconds: 0,
        }
    }

    /// A fresh directory with `csl_locales`/`csl_styles` symlinked in from the real `data/`
    /// dir (`CslData::new` reads those unconditionally), isolated per test so parallel
    /// `#[sqlx::test]` runs can't step on each other's `external_renders` job files -
    /// `ExternalRenderManager::init` wipes that directory on startup.
    fn isolated_data_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vb_render_api_test_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let repo_data = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data");
        std::os::unix::fs::symlink(repo_data.join("csl_locales"), dir.join("csl_locales")).unwrap();
        std::os::unix::fs::symlink(repo_data.join("csl_styles"), dir.join("csl_styles")).unwrap();
        dir
    }

    /// Creates a user with an API key, returning the key's owner id and its raw `vb_..._...`
    /// value for the `X-API-KEY` header - the same shape `create_api_key`'s endpoint hands out.
    async fn seed_api_key(pool: &PgPool) -> (Uuid, String) {
        let default_team = users::ensure_default_team(pool).await.unwrap();
        let user = users::insert(
            pool,
            "render-api-test@example.com",
            "Render API Test",
            "unused-password-hash",
            default_team,
        )
        .await
        .unwrap();

        let prefix = Alphanumeric.sample_string(&mut rand::rng(), 8);
        let secret = Alphanumeric.sample_string(&mut rand::rng(), 40);
        let salt = SaltString::generate(&mut ArgonOsRng);
        let key_hash = Argon2::default()
            .hash_password(secret.as_bytes(), &salt)
            .unwrap()
            .to_string();
        api_keys_repo::create(pool, user.id, "test key", &prefix, &key_hash)
            .await
            .unwrap();

        (user.id, api_keys_repo::format_key(&prefix, &secret))
    }

    async fn test_client(pool: PgPool, settings: Settings) -> Client {
        let csl_data = Arc::new(CslData::new(&settings));
        let client_config = tokio_rustls::rustls::ClientConfig::builder_with_protocol_versions(&[
            &tokio_rustls::rustls::version::TLS13,
        ])
        .with_root_certificates(tokio_rustls::rustls::RootCertStore::empty())
        .with_no_client_auth();

        let rendering_manager = RenderingManager::start(
            settings.clone(),
            pool.clone(),
            csl_data,
            Arc::new(client_config),
        );
        let external_manager =
            ExternalRenderManager::init(settings.clone(), pool.clone(), rendering_manager);

        // Force the debug profile so Rocket doesn't demand a configured secret_key outside it.
        let figment = rocket::Config::figment().select(rocket::Config::DEBUG_PROFILE);
        let rocket = rocket::custom(figment)
            .manage(pool)
            .manage(external_manager)
            .mount(
                "/",
                routes![
                    add_render_job,
                    get_render_job_status,
                    download_render_result
                ],
            );
        Client::tracked(rocket).await.unwrap()
    }

    fn minimal_request_body(job_id: Uuid) -> serde_json::Value {
        serde_json::json!({
            "job_id": job_id,
            "data_source": { "WordPressByURL": "https://example.org/2024/01/01/my-post/" },
            "rendering_settings": {
                "add_soft_hyphens": false,
                "template_id": Uuid::new_v4(),
                "export_formats": ["pdf"]
            }
        })
    }

    #[sqlx::test]
    async fn submitting_a_job_returns_the_documented_202_body_and_location_header(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        let data_dir = isolated_data_dir();
        let settings = test_settings(data_dir.to_string_lossy().to_string());
        let (_, api_key) = seed_api_key(&pool).await;
        let client = test_client(pool, settings).await;

        let job_id = Uuid::new_v4();
        let response = client
            .post("/api/v1/rendering_jobs")
            .header(rocket::http::ContentType::JSON)
            .header(Header::new("X-API-KEY", api_key))
            .body(minimal_request_body(job_id).to_string())
            .dispatch()
            .await;

        assert_eq!(response.status(), Status::Accepted);
        assert_eq!(
            response.headers().get_one("Location"),
            Some(format!("https://example.org/api/v1/rendering_jobs/{job_id}").as_str())
        );
        let body: serde_json::Value =
            serde_json::from_str(&response.into_string().await.unwrap()).unwrap();
        assert_eq!(
            body,
            serde_json::json!({"job_id": job_id, "status": "Pending"})
        );

        let _ = std::fs::remove_dir_all(&data_dir);
        Ok(())
    }

    #[sqlx::test]
    async fn resubmitting_the_same_job_id_returns_409(pool: PgPool) -> sqlx::Result<()> {
        let data_dir = isolated_data_dir();
        let settings = test_settings(data_dir.to_string_lossy().to_string());
        let (_, api_key) = seed_api_key(&pool).await;
        let client = test_client(pool, settings).await;

        let job_id = Uuid::new_v4();
        let body = minimal_request_body(job_id).to_string();
        let first = client
            .post("/api/v1/rendering_jobs")
            .header(rocket::http::ContentType::JSON)
            .header(Header::new("X-API-KEY", api_key.clone()))
            .body(body.clone())
            .dispatch()
            .await;
        assert_eq!(first.status(), Status::Accepted);

        let second = client
            .post("/api/v1/rendering_jobs")
            .header(rocket::http::ContentType::JSON)
            .header(Header::new("X-API-KEY", api_key))
            .body(body)
            .dispatch()
            .await;
        assert_eq!(second.status(), Status::Conflict);

        let _ = std::fs::remove_dir_all(&data_dir);
        Ok(())
    }

    #[sqlx::test]
    async fn invalid_webhook_url_is_rejected_before_a_job_is_created(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        let data_dir = isolated_data_dir();
        let settings = test_settings(data_dir.to_string_lossy().to_string());
        let (_, api_key) = seed_api_key(&pool).await;
        let client = test_client(pool, settings).await;

        let mut body = minimal_request_body(Uuid::new_v4());
        body["webhook_url"] = serde_json::json!("not a url");

        let response = client
            .post("/api/v1/rendering_jobs")
            .header(rocket::http::ContentType::JSON)
            .header(Header::new("X-API-KEY", api_key))
            .body(body.to_string())
            .dispatch()
            .await;

        assert_eq!(response.status(), Status::BadRequest);

        let _ = std::fs::remove_dir_all(&data_dir);
        Ok(())
    }

    #[sqlx::test]
    async fn unknown_job_id_returns_404(pool: PgPool) -> sqlx::Result<()> {
        let data_dir = isolated_data_dir();
        let settings = test_settings(data_dir.to_string_lossy().to_string());
        let (_, api_key) = seed_api_key(&pool).await;
        let client = test_client(pool, settings).await;

        let response = client
            .get(format!("/api/v1/rendering_jobs/{}", Uuid::new_v4()))
            .header(Header::new("X-API-KEY", api_key))
            .dispatch()
            .await;

        assert_eq!(response.status(), Status::NotFound);

        let _ = std::fs::remove_dir_all(&data_dir);
        Ok(())
    }

    #[sqlx::test]
    async fn unknown_download_token_returns_404_without_an_api_key(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        let data_dir = isolated_data_dir();
        let settings = test_settings(data_dir.to_string_lossy().to_string());
        let client = test_client(pool, settings).await;

        let response = client
            .get(format!(
                "/api/v1/rendering_jobs/{}/files/wrong-token/book.pdf",
                Uuid::new_v4()
            ))
            .dispatch()
            .await;

        assert_eq!(response.status(), Status::NotFound);

        let _ = std::fs::remove_dir_all(&data_dir);
        Ok(())
    }
}

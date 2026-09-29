use crate::db::repositories::{bibliography, projects, sections, templates};
use crate::export::preprocessing::prepare_project;
use crate::export::zip::create_zip_from_bytes;
use crate::settings::{ExportServer, Settings};
use crate::storage::project_storage::ProjectData;
use crate::utils::csl::CslData;
use crate::utils::timeout::{limit_from_seconds, with_timeout};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use tokio::net::TcpStream;
use tokio::sync::Semaphore;
use tokio_rustls::rustls::ClientConfig;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::{TlsConnector, TlsStream};
use vb_exchange::export_formats::ExportFormat;
use vb_exchange::{
    CommunicationError, FilesOnMemoryOrHarddrive, Message, RenderingError, RenderingRequest,
    RenderingStatus, TemplateContents, TemplateDataResult, read_message, send_message,
};

#[derive(Default, Serialize, Deserialize)]
/// Unprepared Rendering Request, send by User.
/// Gets prepared & converted to a [vb_exchange::RenderingRequest]
pub struct LocalRenderingRequest {
    /// Randomly generated uuid
    pub request_id: uuid::Uuid,
    /// id of the project to render
    pub project_id: uuid::Uuid,
    /// list of export formats slugs that should be rendered
    pub export_formats: Vec<String>,
    /// list of section ids to be prepared, or None if all should be prepared
    pub sections: Option<Vec<uuid::Uuid>>,
}

/// A fully loaded project that is ready to be prepared and sent to a rendering server.
///
/// Unlike a [`LocalRenderingRequest`] it doesn't reference a project in the database, so it can
/// also be used for projects that only exist in memory (e.g. external rendering jobs).
pub struct RenderingJob {
    /// Id under which status updates are stored in [`RenderingManager::requests_archive`]
    pub request_id: uuid::Uuid,
    /// The project to render. `template_id` must reference an existing template.
    pub project_data: ProjectData,
    /// Directory containing the files uploaded to the project
    pub uploads_dir: PathBuf,
    /// list of section ids to be prepared, or None if all should be prepared
    pub sections: Option<Vec<uuid::Uuid>>,
    /// list of export formats slugs that should be rendered
    pub export_formats: Vec<String>,
    /// If set, every result file is written to this directory individually (no zip is created).
    /// If None, results are written to a fresh directory in `data/temp` and zipped if there are multiple.
    pub result_dir: Option<PathBuf>,
}

/// Returns `name`, or `name` with a numeric suffix before the extension (`main-2.pdf`) if a file
/// with that name was already used. Different export formats can produce equally named files.
fn unique_file_name(
    name: &std::ffi::OsStr,
    used: &mut std::collections::HashSet<String>,
) -> String {
    let path = std::path::Path::new(name);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
    let ext = path.extension().and_then(|e| e.to_str());
    let mut candidate = name.to_string_lossy().into_owned();
    let mut n = 1;
    while !used.insert(candidate.clone()) {
        n += 1;
        candidate = match ext {
            Some(ext) => format!("{stem}-{n}.{ext}"),
            None => format!("{stem}-{n}"),
        };
    }
    candidate
}

/// Coordinates local preparation and remote rendering of projects: owns the queue of
/// pending [`LocalRenderingRequest`]s, tracks their status, and dispatches them to
/// rendering servers over TLS.
pub struct RenderingManager {
    /// Settings loaded from configuration files, containing global application settings such as titles, data paths, and server configurations.
    pub settings: Settings,
    /// Postgres connection pool.
    pub pool: sqlx::PgPool,
    /// Loaded Citation Style Language (CSL) data, including available locales and styles.
    pub csl_data: Arc<CslData>,
    /// Archive of rendering requests, storing their UUIDs with corresponding rendering status
    pub requests_archive: RwLock<HashMap<uuid::Uuid, RenderingStatus>>,
    /// Queue of rendering requests, storing each local rendering request in the order they are to be processed.
    pub rendering_queue: RwLock<VecDeque<LocalRenderingRequest>>,
    /// Atomic counter managing round-robin selection of the next rendering server to be used for distributing rendering tasks.
    pub next_rendering_server_to_use: Arc<AtomicU64>,
    /// Loaded configuration for the rendering client, including certificates, paths, and connection settings.
    pub client_config: Arc<ClientConfig>,
    /// Limits how many rendering requests are prepared & sent to a rendering server concurrently,
    /// shared by both the internal queue and [`Self::prepare_and_send`] callers outside of it (e.g.
    /// external rendering jobs), so `max_connections_to_rendering_server` is enforced globally.
    pub slots: Arc<Semaphore>,
}

impl RenderingManager {
    /// Creates a new `RenderingManager` and spawns a background task that continuously
    /// polls the rendering queue and dispatches queued requests to a rendering server
    /// (up to `max_connections_to_rendering_server` concurrent tasks). Returns a shared
    /// `Arc<RenderingManager>` handle for submitting and tracking rendering requests.
    pub fn start(
        settings: Settings,
        pool: sqlx::PgPool,
        csl_data: Arc<CslData>,
        client_config: Arc<ClientConfig>,
    ) -> Arc<RenderingManager> {
        let rendering_manager = RenderingManager {
            settings: settings.clone(),
            pool,
            csl_data,
            requests_archive: RwLock::new(HashMap::new()),
            rendering_queue: RwLock::new(VecDeque::new()),
            next_rendering_server_to_use: Arc::new(AtomicU64::new(0)),
            client_config,
            slots: Arc::new(Semaphore::new(
                settings.max_connections_to_rendering_server as usize,
            )),
        };

        let rendering_manager = Arc::new(rendering_manager);
        let rendering_manager_cpy = rendering_manager.clone();

        // Start thread that checks for new rendering requests and sends them to a rendering server.
        tokio::spawn(async move {
            let slots = rendering_manager_cpy.slots.clone();

            loop {
                // Check if there are any new rendering requests and a slot is free
                let rendering_requests_len =
                    rendering_manager_cpy.rendering_queue.read().unwrap().len();
                if rendering_requests_len > 0
                    && let Ok(permit) = Arc::clone(&slots).try_acquire_owned()
                {
                    debug!("Starting new thread to prepare & send rendering data");
                    {
                        let mut rendering_queue =
                            rendering_manager_cpy.rendering_queue.write().unwrap();

                        // Move the rendering request out of the vector, put it into the archive and start rendering
                        let request = match rendering_queue.pop_front() {
                            Some(req) => req,
                            // Permit is dropped here, freeing the slot again
                            None => continue,
                        };

                        debug!("Found a new rendering request.");

                        // Update status
                        if let Some(status) = rendering_manager_cpy
                            .requests_archive
                            .write()
                            .unwrap()
                            .get_mut(&request.request_id)
                        {
                            *status = RenderingStatus::PreparingOnLocal
                        }

                        let request_id_cpy = request.request_id;

                        let rendering_manager_cpy2 = rendering_manager_cpy.clone();

                        // Start rendering in a new thread
                        tokio::spawn(async move {
                            // Held until the job finishes, freeing the slot again
                            let _permit = permit;
                            match Self::prepare_and_send_to_server(
                                Arc::clone(&rendering_manager_cpy2),
                                request,
                            )
                            .await
                            {
                                Ok(_) => {}
                                Err(e) => {
                                    error!("Couldn't render project: {:?}", e);
                                    // Update status:
                                    if let Some(status) = rendering_manager_cpy2
                                        .requests_archive
                                        .write()
                                        .unwrap()
                                        .get_mut(&request_id_cpy)
                                    {
                                        *status = RenderingStatus::Failed(e)
                                    }
                                }
                            }
                        });
                    }
                }

                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        });

        rendering_manager.clone()
    }
    /// Loads a project (title, description, template, metadata, settings, sections and
    /// bibliography) from the database and hands it off to [`Self::prepare_and_send`].
    async fn prepare_and_send_to_server(
        rendering_manager: Arc<RenderingManager>,
        request: LocalRenderingRequest,
    ) -> Result<(), RenderingError> {
        let pool = &rendering_manager.pool;

        let title = projects::get_title(pool, request.project_id)
            .await
            .map_err(|_| RenderingError::ProjectNotFound)?;
        let description = projects::get_description(pool, request.project_id)
            .await
            .map_err(|_| RenderingError::ProjectNotFound)?;
        let template_id = projects::get_template_id(pool, request.project_id)
            .await
            .map_err(|_| RenderingError::ProjectNotFound)?
            .ok_or(RenderingError::TemplateNotFound)?;
        let metadata = projects::get_metadata(pool, request.project_id)
            .await
            .map_err(|_| RenderingError::ProjectNotFound)?;
        let settings = projects::get_settings(pool, request.project_id)
            .await
            .map_err(|_| RenderingError::ProjectNotFound)?;
        let project_sections = sections::get_tree_for_project_with_content(
            pool,
            &rendering_manager.settings,
            request.project_id,
        )
        .await
        .map_err(|_| RenderingError::ProjectNotFound)?;
        let bib = bibliography::get_all_for_project(pool, request.project_id)
            .await
            .map_err(|_| RenderingError::ProjectNotFound)?;

        let project_data: ProjectData = ProjectData {
            name: title,
            description,
            template_id,
            last_interaction: 0,
            metadata: Some(metadata),
            settings: Some(settings),
            sections: project_sections,
            bibliography: bib,
        };

        Self::prepare_and_send(
            rendering_manager,
            RenderingJob {
                request_id: request.request_id,
                project_data,
                uploads_dir: PathBuf::from(format!("data/projects/{}/uploads", request.project_id)),
                sections: request.sections,
                export_formats: request.export_formats,
                result_dir: None,
            },
        )
        .await
    }

    /// Prepares `job.project_data` for rendering, packs its uploads, sends everything to the next
    /// available rendering server and waits until the rendering finished or failed.
    ///
    /// Status updates and the final result are stored in `requests_archive` under `job.request_id`.
    pub async fn prepare_and_send(
        rendering_manager: Arc<RenderingManager>,
        job: RenderingJob,
    ) -> Result<(), RenderingError> {
        let template_id = job.project_data.template_id;
        let project_data = job.project_data;

        // Get current version id of the template
        let template_version_id = match templates::get(&rendering_manager.pool, template_id).await {
            Ok(template) => template.version,
            Err(_) => return Err(RenderingError::TemplateNotFound),
        };

        let RenderingJob {
            request_id,
            uploads_dir,
            sections,
            export_formats,
            result_dir,
            ..
        } = job;

        // Prepare project and pack uploaded files
        let preprocessing_timeout = rendering_manager.settings.rendering_preprocessing_timeout;
        let preprocessing = async {
            let prepared_project = prepare_project(
                project_data,
                rendering_manager.pool.clone(),
                rendering_manager.csl_data.clone(),
                sections,
                &uploads_dir,
            )
            .await?;

            // Check if upload directory exists:
            let uploads = if uploads_dir.exists() {
                vb_exchange::recursive_read_dir_async(uploads_dir)
                    .await
                    .map_err(|e| {
                        RenderingError::Other(format!("IO Error packing uploaded files: {}", e))
                    })?
            } else {
                Vec::new()
            };
            Ok::<_, RenderingError>((prepared_project, uploads))
        };
        let (prepared_project, uploads) =
            with_timeout(limit_from_seconds(preprocessing_timeout), preprocessing)
                .await
                .map_err(|_| {
                    error!(
                        "Preprocessing timed out after {} seconds.",
                        preprocessing_timeout
                    );
                    RenderingError::Other(format!(
                        "Preprocessing timed out after {} seconds.",
                        preprocessing_timeout
                    ))
                })??;

        let request = RenderingRequest {
            request_id,
            prepared_project,
            project_uploaded_files: FilesOnMemoryOrHarddrive::Memory(uploads),
            template_id,
            template_version_id,
            export_formats,
        };

        // Update status
        if let Some(status) = rendering_manager
            .requests_archive
            .write()
            .unwrap()
            .get_mut(&request.request_id)
        {
            *status = RenderingStatus::PreparedOnLocal
        }

        // Send to server
        let num_of_rendering_servers = rendering_manager.settings.export_servers.len();
        if num_of_rendering_servers == 0 {
            error!("Error: No rendering servers configured.");
            return Err(RenderingError::Other(
                "No rendering server configured".to_string(),
            ));
        }

        // Figure out which rendering server to use next
        let mut next_rendering_server = rendering_manager
            .next_rendering_server_to_use
            .load(Ordering::SeqCst);
        // Reset counter to 0 if we used the last rendering server in list
        if next_rendering_server >= num_of_rendering_servers as u64 {
            next_rendering_server = 0;
        }
        let first_tried_server = next_rendering_server;

        rendering_manager
            .next_rendering_server_to_use
            .fetch_add(1, Ordering::SeqCst);

        debug!("Using rendering server no {}", next_rendering_server);

        let rendering_server_timeout =
            limit_from_seconds(rendering_manager.settings.rendering_server_timeout);
        let connect_timeout =
            limit_from_seconds(rendering_manager.settings.rendering_server_connect_timeout);

        let tls_stream;
        loop {
            let export_server_data = rendering_manager
                .settings
                .export_servers
                .get(next_rendering_server as usize)
                .unwrap();

            debug!("Connection to Server.");
            // A server that doesn't answer in time is treated like an unreachable one, so the next is tried
            let connection = with_timeout(
                connect_timeout,
                Self::connect_to_server(rendering_manager.clone(), export_server_data),
            )
            .await
            .unwrap_or(Err(RenderingError::ConnectionToRenderingServerFailed));
            match connection {
                Ok(res) => {
                    tls_stream = res;
                    break;
                }
                Err(e) => {
                    match e {
                        RenderingError::ConnectionToRenderingServerFailed => {
                            error!(
                                "Warning: Couldn't connect to rendering server no. {}. Trying next available.",
                                next_rendering_server + 1
                            );
                            // Connection failed, try another server.

                            next_rendering_server += 1;

                            if next_rendering_server >= num_of_rendering_servers as u64 {
                                next_rendering_server = 0;
                            }

                            // Fail after we tried all other remaining servers
                            if next_rendering_server == first_tried_server {
                                error!("Couldn't find any working rendering servers.");
                                return Err(e);
                            }
                        }
                        _ => {
                            return Err(e);
                        }
                    }
                }
            }
        }

        debug!("Connected, sending request to server.");

        with_timeout(
            rendering_server_timeout,
            Self::send_to_server(tls_stream, request, rendering_manager.clone(), result_dir),
        )
        .await
        .map_err(|_| {
            error!(
                "Rendering timed out after {} seconds.",
                rendering_manager.settings.rendering_server_timeout
            );
            RenderingError::Other(format!(
                "Rendering timed out after {} seconds.",
                rendering_manager.settings.rendering_server_timeout
            ))
        })??;
        Ok(())
    }

    async fn connect_to_server(
        rendering_manager: Arc<RenderingManager>,
        export_server: &ExportServer,
    ) -> Result<TlsStream<TcpStream>, RenderingError> {
        let connector = TlsConnector::from(rendering_manager.client_config.clone());
        let stream =
            match TcpStream::connect(format!("{}:{}", export_server.hostname, export_server.port))
                .await
            {
                Ok(res) => res,
                Err(e) => {
                    error!("Couldn't connect to export server: {}", e);
                    return Err(RenderingError::ConnectionToRenderingServerFailed);
                }
            };

        let domain: ServerName =
            export_server
                .domain_name
                .clone()
                .try_into()
                .unwrap_or_else(|_| {
                    panic!(
                        "Warning: Invalid DNS name for export server: {}",
                        export_server.domain_name
                    )
                });
        match connector.connect(domain, stream).await {
            Ok(res) => Ok(res.into()),
            Err(e) => {
                error!("Couldn't initiate tls stream: {}", e);
                Err(RenderingError::ConnectionToRenderingServerFailed)
            }
        }
    }
    async fn send_to_server(
        mut tls_stream: TlsStream<TcpStream>,
        request: RenderingRequest,
        rendering_manager: Arc<RenderingManager>,
        result_dir: Option<PathBuf>,
    ) -> Result<(), RenderingError> {
        let request_id = request.request_id;
        if let Err(_) = send_message(&mut tls_stream, Message::RenderingRequest(request)).await {
            return Err(RenderingError::CommunicationError);
        }
        debug!("Request sent to server.");

        // Update status
        if let Some(status) = rendering_manager
            .requests_archive
            .write()
            .unwrap()
            .get_mut(&request_id)
        {
            *status = RenderingStatus::SendToRenderingServer
        }
        // From here we get new status updates from the rendering server

        loop {
            debug!("Waiting for response from server.");
            match read_message(&mut tls_stream).await {
                Ok(msg) => match msg {
                    Message::TemplateDataRequest(req) => {
                        debug!(
                            "Template Data requested: Template {} with version {}.",
                            req.template_id, req.template_version_id
                        );
                        // Update status
                        if let Some(status) = rendering_manager
                            .requests_archive
                            .write()
                            .unwrap()
                            .get_mut(&request_id)
                        {
                            *status = RenderingStatus::TransmittingTemplate
                        }

                        let template_files = match TemplateContents::from_path(PathBuf::from(
                            format!("data/templates/{}/", req.template_id),
                        ))
                        .await
                        {
                            Ok(res) => res,
                            Err(e) => {
                                error!("Couldn't package template contents: {}", e);
                                return Err(RenderingError::TemplateNotFound);
                            }
                        };

                        let export_formats: HashMap<String, ExportFormat> = match templates::get(
                            &rendering_manager.pool,
                            req.template_id,
                        )
                        .await
                        {
                            Err(_) => {
                                error!(
                                    "Couldn't find template {} requested from rendering server.",
                                    req.template_id.clone()
                                );
                                return Err(RenderingError::TemplateNotFound);
                            }
                            Ok(template) => template.export_formats,
                        };

                        let data = TemplateDataResult {
                            template_id: req.template_id,
                            template_version_id: req.template_version_id, //Warning: Currently we do not check if the template hasn't changed since queuing (template_version_id doesnt get checked)
                            contents: template_files,
                            export_formats,
                        };

                        if let Err(_) =
                            send_message(&mut tls_stream, Message::TemplateDataResult(data)).await
                        {
                            return Err(RenderingError::CommunicationError);
                        }
                    }
                    Message::CommunicationError(err) => {
                        error!("Communication error: {:?}", err);
                        return Err(RenderingError::CommunicationError);
                    }
                    Message::RenderingRequestStatus(status) => {
                        match status {
                            RenderingStatus::Finished(mut res) => {
                                // Finished, update status and save files to file system, generate zip if necessary
                                let individual_files_only = result_dir.is_some();
                                let res_dir = result_dir.clone().unwrap_or_else(|| {
                                    PathBuf::from(format!("data/temp/{}", uuid::Uuid::new_v4()))
                                });
                                if let Err(e) = tokio::fs::create_dir_all(&res_dir).await {
                                    error!("Couldn't create dir: {}", e);
                                }

                                if res.files.len() > 1 && !individual_files_only {
                                    // More than 1 file -> load files into res_dir + create zip
                                    let res_path = res_dir.join("result.zip");

                                    for file in res.files.clone() {
                                        let file_path = res_dir.join(file.name);
                                        if let Err(e) =
                                            tokio::fs::write(&file_path, file.content).await
                                        {
                                            error!("Couldn't save rendering result to file: {}", e);
                                            return Err(RenderingError::Other(
                                                "Couldn't save rendering output.".to_string(),
                                            ));
                                        }
                                    }

                                    let res_path2 = res_path.clone();
                                    let task = tokio::task::spawn_blocking(move || {
                                        create_zip_from_bytes(res.files, res_path2)
                                    })
                                    .await;
                                    match task {
                                        Ok(res) => match res {
                                            Ok(_) => {
                                                rendering_manager
                                                    .requests_archive
                                                    .write()
                                                    .unwrap()
                                                    .insert(
                                                        request_id,
                                                        RenderingStatus::SavedOnLocal(
                                                            res_path, res_dir,
                                                        ),
                                                    );
                                            }
                                            Err(e) => {
                                                error!("IO error creating result zip: {}", e);
                                                rendering_manager
                                                    .requests_archive
                                                    .write()
                                                    .unwrap()
                                                    .insert(
                                                        request_id,
                                                        RenderingStatus::Failed(
                                                            RenderingError::Other(
                                                                "IO Error".to_string(),
                                                            ),
                                                        ),
                                                    );
                                            }
                                        },
                                        Err(e) => {
                                            error!("Join error: {}", e);
                                            rendering_manager
                                                .requests_archive
                                                .write()
                                                .unwrap()
                                                .insert(
                                                    request_id,
                                                    RenderingStatus::Failed(RenderingError::Other(
                                                        "Join Error".to_string(),
                                                    )),
                                                );
                                        }
                                    }
                                } else if individual_files_only && !res.files.is_empty() {
                                    let mut first_file_path = None;
                                    let mut used_names = std::collections::HashSet::new();
                                    for file in res.files {
                                        let Some(name) =
                                            std::path::Path::new(&file.name).file_name()
                                        else {
                                            continue;
                                        };
                                        let file_path =
                                            res_dir.join(unique_file_name(name, &mut used_names));
                                        if let Err(e) =
                                            tokio::fs::write(&file_path, file.content).await
                                        {
                                            error!("Couldn't save rendering result to file: {}", e);
                                            return Err(RenderingError::Other(
                                                "Couldn't save rendering output.".to_string(),
                                            ));
                                        }
                                        first_file_path.get_or_insert(file_path);
                                    }
                                    let status = match first_file_path {
                                        Some(path) => RenderingStatus::SavedOnLocal(path, res_dir),
                                        None => {
                                            RenderingStatus::Failed(RenderingError::NoResultFiles)
                                        }
                                    };
                                    rendering_manager
                                        .requests_archive
                                        .write()
                                        .unwrap()
                                        .insert(request_id, status);
                                } else if let Some(file) = res.files.pop() {
                                    let file_path = res_dir.join(file.name);
                                    if let Err(e) = tokio::fs::write(&file_path, file.content).await
                                    {
                                        error!("Couldn't save rendering result to file: {}", e);
                                        return Err(RenderingError::Other(
                                            "Couldn't save rendering output.".to_string(),
                                        ));
                                    }
                                    rendering_manager.requests_archive.write().unwrap().insert(
                                        request_id,
                                        RenderingStatus::SavedOnLocal(file_path, res_dir),
                                    );
                                } else {
                                    rendering_manager.requests_archive.write().unwrap().insert(
                                        request_id,
                                        RenderingStatus::Failed(RenderingError::NoResultFiles),
                                    );
                                }
                                break;
                            }
                            RenderingStatus::Failed(e) => {
                                // Failed, update status and return
                                rendering_manager
                                    .requests_archive
                                    .write()
                                    .unwrap()
                                    .insert(request_id, RenderingStatus::Failed(e.clone()));
                                return Err(e);
                            }
                            _ => {
                                // Update status
                                rendering_manager
                                    .requests_archive
                                    .write()
                                    .unwrap()
                                    .insert(request_id, status);
                            }
                        }
                    }
                    _ => {
                        let _ = send_message(
                            &mut tls_stream,
                            Message::CommunicationError(CommunicationError::UnexpectedMessageType),
                        )
                        .await;
                        return Err(RenderingError::CommunicationError);
                    }
                },
                Err(_) => return Err(RenderingError::CommunicationError),
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    #[test]
    fn equally_named_result_files_get_numbered() {
        let mut used = std::collections::HashSet::new();
        assert_eq!(
            unique_file_name(OsStr::new("main.pdf"), &mut used),
            "main.pdf"
        );
        assert_eq!(
            unique_file_name(OsStr::new("main.pdf"), &mut used),
            "main-2.pdf"
        );
        assert_eq!(
            unique_file_name(OsStr::new("main.pdf"), &mut used),
            "main-3.pdf"
        );
        assert_eq!(unique_file_name(OsStr::new("book"), &mut used), "book");
        assert_eq!(unique_file_name(OsStr::new("book"), &mut used), "book-2");
    }
}

use crate::export::preprocessing::prepare_project;
use crate::export::zip::create_zip_from_bytes;
use crate::settings::{ExportServer, Settings};
use crate::storage::data_storage::DataStorage;
use crate::storage::project_storage::{ProjectData, ProjectStorage};
use crate::utils::csl::CslData;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use tokio::net::TcpStream;
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

pub struct RenderingManager {
    /// Settings loaded from configuration files, containing global application settings such as titles, data paths, and server configurations.
    pub settings: Settings,
    /// Persistent memory storage for small data such as users, passwords, or login attempts, shared between tasks.
    pub data_storage: Arc<DataStorage>,
    /// In-memory project storage, holding and managing the state of all projects loaded at startup.
    pub project_storage: Arc<ProjectStorage>,
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
}

impl RenderingManager {
    pub fn start(
        settings: Settings,
        data_storage: Arc<DataStorage>,
        project_storage: Arc<ProjectStorage>,
        csl_data: Arc<CslData>,
        client_config: Arc<ClientConfig>,
    ) -> Arc<RenderingManager> {
        let rendering_manager = RenderingManager {
            settings: settings.clone(),
            data_storage,
            project_storage,
            csl_data,
            requests_archive: RwLock::new(HashMap::new()),
            rendering_queue: RwLock::new(VecDeque::new()),
            next_rendering_server_to_use: Arc::new(AtomicU64::new(0)),
            client_config,
        };

        let rendering_manager = Arc::new(rendering_manager);
        let rendering_manager_cpy = rendering_manager.clone();

        // Start thread that checks for new rendering requests and sends them to a rendering server.
        tokio::spawn(async move {
            let running_threads: Arc<AtomicU64> = Arc::new(AtomicU64::new(0));

            loop {
                // Check if there are any new rendering requests

                let rendering_requests_len =
                    rendering_manager_cpy.rendering_queue.read().unwrap().len();
                if rendering_requests_len > 0
                    && rendering_manager_cpy
                        .settings
                        .max_connections_to_rendering_server
                        > running_threads.load(std::sync::atomic::Ordering::Relaxed)
                {
                    debug!("Starting new thread to prepare & send rendering data");
                    {
                        let mut rendering_queue =
                            rendering_manager_cpy.rendering_queue.write().unwrap();

                        // Move the rendering request out of the vector, put it into the archive and start rendering
                        let request = match rendering_queue.pop_front() {
                            Some(req) => req,
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

                        running_threads.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let running_threads_clone = Arc::clone(&running_threads);

                        // Start rendering in a new thread
                        tokio::spawn(async move {
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
                            running_threads_clone
                                .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                        });
                    }
                }

                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        });

        rendering_manager.clone()
    }
    async fn prepare_and_send_to_server(
        rendering_manager: Arc<RenderingManager>,
        request: LocalRenderingRequest,
    ) -> Result<(), RenderingError> {
        let project_data: ProjectData = match rendering_manager
            .project_storage
            .get_project(&request.project_id, &rendering_manager.settings)
            .await
        {
            Ok(project) => project.read().unwrap().clone(),
            Err(_) => return Err(RenderingError::ProjectNotFound),
        };

        let template_id = project_data.template_id;
        // Get current version id of the template
        let template_version_id = match rendering_manager
            .data_storage
            .data
            .templates
            .get(&template_id)
        {
            Some(template) => template.read().unwrap().version.unwrap(),
            None => return Err(RenderingError::TemplateNotFound),
        };

        // Prepare project
        let prepared_project = prepare_project(
            project_data,
            rendering_manager.data_storage.clone(),
            rendering_manager.csl_data.clone(),
            request.sections,
            &request.project_id,
        )
        .await?;

        // Pack uploaded files
        // Check if upload directory exists:
        let upload_dir = PathBuf::from(format!("data/projects/{}/uploads", &request.project_id));
        let uploads = if upload_dir.exists() {
            match vb_exchange::recursive_read_dir_async(upload_dir).await {
                Ok(uploads) => uploads,
                Err(e) => {
                    return Err(RenderingError::Other(format!(
                        "IO Error packing uploaded files: {}",
                        e
                    )));
                }
            }
        } else {
            Vec::new()
        };

        let request = RenderingRequest {
            request_id: request.request_id,
            prepared_project,
            project_uploaded_files: FilesOnMemoryOrHarddrive::Memory(uploads),
            template_id,
            template_version_id,
            export_formats: request.export_formats,
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

        let tls_stream;
        loop {
            let export_server_data = rendering_manager
                .settings
                .export_servers
                .get(next_rendering_server as usize)
                .unwrap();

            debug!("Connection to Server.");
            match Self::connect_to_server(rendering_manager.clone(), export_server_data).await {
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

        Self::send_to_server(tls_stream, request, rendering_manager.clone()).await?;
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

                        let export_formats: HashMap<String, ExportFormat> = match rendering_manager
                            .data_storage
                            .data
                            .templates
                            .get(&req.template_id)
                        {
                            None => {
                                error!(
                                    "Couldn't find template {} requested from rendering server.",
                                    req.template_id.clone()
                                );
                                return Err(RenderingError::TemplateNotFound);
                            }
                            Some(template) => template.read().unwrap().export_formats.clone(),
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
                                let res_dir =
                                    PathBuf::from(format!("data/temp/{}", uuid::Uuid::new_v4()));
                                if let Err(e) = tokio::fs::create_dir(&res_dir).await {
                                    error!("Couldn't create dir: {}", e);
                                }

                                if res.files.len() > 1 {
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

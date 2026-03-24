use crate::projects::websocket::WebsocketMessage::{
    DOCUPDATE, RECEIVEDDOCUPDATE, REMOVECURSOR, SETCURSOR,
};
use crate::session::session_guard::Session;
use crate::settings::Settings;
use crate::storage::data_storage::DataStorage;
use crate::storage::project_storage::ProjectStorage;
use dashmap::DashMap;
use dashmap::mapref::one::{Ref, RefMut};
use rocket::State;
use rocket::futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Error;
use std::sync::Arc;
use std::time::SystemTime;
use tokio::sync::broadcast;
use uuid::Uuid;
use yrs::StateVector;
use yrs::updates::decoder::Decode;
use yrs::{Doc, ReadTxn, Transact, Update};

pub struct WebsocketManager {
    documents: Arc<DashMap<uuid::Uuid, DocumentState>>,
    project_storage: Arc<ProjectStorage>,
    settings: Arc<Settings>,
}

impl WebsocketManager {
    pub fn new(project_storage: Arc<ProjectStorage>, settings: Arc<Settings>) -> Self {
        Self {
            documents: Arc::new(DashMap::new()),
            project_storage,
            settings,
        }
    }

    pub fn get_document(&self, document_id: &uuid::Uuid) -> Option<Ref<'_, Uuid, DocumentState>> {
        self.documents.get(document_id)
    }

    pub fn get_document_mut(
        &self,
        document_id: &uuid::Uuid,
    ) -> Option<dashmap::mapref::one::RefMut<'_, uuid::Uuid, DocumentState>> {
        self.documents.get_mut(document_id)
    }

    /// Loads a document from storage, decodes it, and insert it into the websocket manager
    /// Doesn't return the document state, only adds it to the documents [DashMap].
    ///
    /// Params:
    /// - project_id: The project the document belongs to
    /// - document_id: The id of the document to load
    /// - client_id: The id of the client that requested the document, will be inserted into the document's clients list
    ///
    /// Returns None if the document is already loaded or an error occurs
    pub async fn load_document_from_storage(
        &self,
        project_id: &uuid::Uuid,
        document_id: &uuid::Uuid,
        client_id: Option<&Uuid>,
    ) -> Option<()> {
        // Check if the document is already loaded
        if self.documents.contains_key(document_id) {
            return None;
        }
        debug!("Creating ydoc for section {}", document_id);
        let ydoc = Doc::new();
        let project_lock = self
            .project_storage
            .get_project(project_id, &self.settings)
            .await
            .unwrap()
            .clone();
        let binary_update = project_lock
            .read()
            .unwrap()
            .get_section(&document_id)
            .map(|x| x.content.clone());

        drop(project_lock); // release lock

        if let Some(binary_update) = binary_update {
            let update = match Update::decode_v1(&binary_update) {
                Ok(update) => update,
                Err(e) => {
                    error!("Couldn't decode section {}: {}", document_id, e);
                    return None;
                }
            };
            if let Err(e) = ydoc.transact_mut().apply_update(update) {
                error!(
                    "Couldn't apply update to ydoc for section {}: {}",
                    document_id, e
                );
                return None;
            }
        }

        let (tx, _) = broadcast::channel(100);
        let save_requester = Arc::new(tokio::sync::Notify::new());

        self.spawn_document_save_worker(project_id, document_id, save_requester.clone())
            .await;
        let doc_state = DocumentState {
            broadcast_tx: tx,
            save_requester: save_requester.clone(),
            active_clients: client_id.map(|x| vec![x.clone()]).unwrap_or_default(),
            doc: ydoc,
        };
        self.documents.insert(document_id.clone(), doc_state);
        Some(())
    }

    /// Combines [load_document_from_storage] and [get_document].
    ///
    /// Returns None if the document is neither in the documents DashMap nor could be loaded from storage
    pub async fn get_or_load_from_storage(
        &self,
        project_id: &uuid::Uuid,
        document_id: &uuid::Uuid,
        client_id: Option<&Uuid>,
    ) -> Option<Ref<'_, Uuid, DocumentState>> {
        match self.get_document(document_id) {
            Some(doc) => Some(doc),
            None => {
                if let Some(_) = self
                    .load_document_from_storage(project_id, document_id, client_id)
                    .await
                {
                    return self.get_document(document_id);
                } else {
                    // new document
                    let (tx, _) = broadcast::channel(100);
                    let save_requester = Arc::new(tokio::sync::Notify::new());

                    self.spawn_document_save_worker(
                        project_id,
                        document_id,
                        save_requester.clone(),
                    )
                    .await;

                    let mut active_clients = vec![];
                    if let Some(client_id) = client_id {
                        active_clients.push(client_id.clone());
                    }
                    let doc_state = DocumentState {
                        broadcast_tx: tx,
                        save_requester: save_requester.clone(),
                        active_clients,
                        doc: Doc::new(),
                    };
                    self.documents.insert(document_id.clone(), doc_state);
                    return self.get_document(document_id);
                }
                None
            }
        }
    }

    /// Combines [`self.load_document_from_storage`] and [self.get_document_mut].
    ///
    /// Returns None if the document is neither in the documents DashMap nor could be loaded from storage
    pub async fn get_mut_or_load_from_storage(
        &self,
        project_id: &uuid::Uuid,
        document_id: &uuid::Uuid,
        client_id: Option<&Uuid>,
    ) -> Option<RefMut<'_, Uuid, DocumentState>> {
        match self.get_document_mut(document_id) {
            Some(doc) => Some(doc),
            None => {
                if let Some(_) = self
                    .load_document_from_storage(project_id, document_id, client_id)
                    .await
                {
                    return self.get_document_mut(document_id);
                }
                None
            }
        }
    }

    async fn spawn_document_save_worker(
        &self,
        project_id: &uuid::Uuid,
        document_id: &uuid::Uuid,
        save_requester: Arc<tokio::sync::Notify>,
    ) {
        let project_id = project_id.clone();
        let document_id = document_id.clone();
        let documents = Arc::clone(&self.documents);
        let project_storage = Arc::clone(&self.project_storage);
        let settings = Arc::clone(&self.settings);

        tokio::task::spawn(async move {
            let mut last_save_time = std::time::Instant::now();

            loop {
                save_requester.notified().await; // Wait for notification to save. Even if notify_one() is called multiple times we will only save one time
                debug!(
                    "Save for document {} requested, {:?} elapsed since last safe.",
                    document_id,
                    last_save_time.elapsed()
                );
                if last_save_time.elapsed() < std::time::Duration::from_millis(500) {
                    debug!(
                        "Postponed save for document {} because it was already saved recently.",
                        document_id
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    // Wait 500ms before saving again
                }
                last_save_time = std::time::Instant::now();
                match documents.get(&document_id) {
                    None => {
                        debug!(
                            "Document {} no longer in DashMap, killing save worker",
                            document_id
                        );
                        break;
                    }
                    Some(doc) => {
                        match doc
                            .save_document_to_storage(
                                &project_id.clone(),
                                &document_id.clone(),
                                project_storage.clone(),
                                &settings.clone(),
                            )
                            .await
                        {
                            Ok(_) => {
                                debug!("Saved document {}", document_id);
                            }
                            Err(_) => {
                                error!("Failed to save document {}", document_id);
                            }
                        }
                    }
                }
            }
        });
    }
}

/// Represents a single document (e.g. a section).
///
/// Holds the actual content as a [Doc] and a list of all active clients
/// Also makes sure that the document is saved to the project_storage on changes.
///
/// ## Saving Logic
/// Since we don't want to do unnecessary saves to the project_storage (e.g. when many clients
/// are doing changes in a short time frame), we need to debounce the save operation.
/// Each document has its own tokio task for saving and gets notified of changes via a tokio Notify notification.
/// The thread keeps track of the last save time and waits for at least 500ms before saving again.
#[derive(Clone)]
pub struct DocumentState {
    /// channel to broadcast messages to all clients in this document
    pub broadcast_tx: broadcast::Sender<BroadcastMessage>,
    /// notify channel
    pub save_requester: Arc<tokio::sync::Notify>,
    /// list of client ids currently connected to this document
    pub active_clients: Vec<uuid::Uuid>,
    /// actual yrs document
    pub doc: Doc,
}

impl DocumentState {
    pub fn spawn_document_save_worker() {
        tokio::task::spawn(async move {});
    }

    pub async fn save_document_to_storage(
        &self,
        project_id: &uuid::Uuid,
        document_id: &uuid::Uuid,
        project_storage: Arc<ProjectStorage>,
        settings: &Settings,
    ) -> Result<(), ()> {
        let binary_update = self.doc.transact().encode_diff_v1(&StateVector::default());

        let project_lock = project_storage
            .get_project(project_id, &settings)
            .await
            .map_err(|_| ())?;

        {
            let mut project_write = project_lock.write().map_err(|_| ())?;
            project_write.last_interaction = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs();
            if let Some(section) = project_write.get_section_mut(document_id) {
                section.content = binary_update;
            } else {
                return Err(());
            }
        }

        project_storage
            .save_project_to_disk(project_id, &settings)
            .await
            .map_err(|_| ())?;

        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct BroadcastMessage {
    pub sender_id: uuid::Uuid,
    pub message: WebsocketMessage,
}

/// Application-level WebSocket messages encoded as a single identifier byte followed by a payload.
/// The first byte of an incoming frame selects the variant:
/// - `10` => `CONNECT` (JSON)
/// - `11` => `WELCOME` (JSON)
/// - `20` => `GETDOC` (raw bytes)
/// - `30` => `DOCUPDATE` (raw bytes)
/// - `31` => `RECEIVEDDOCUPDATE` (no payload)
/// - `40` => `SETCURSOR` (JSON)
/// - `41` => `REMOVECURSOR` (JSON)
/// - `50` => `DISCONNECT` (JSON)
/// - `60` => `ERROR` (JSON)
#[derive(Clone, Debug)]
pub enum WebsocketMessage {
    /// Client requests to connect to a document session.
    CONNECT(ConnectMessage),
    /// Server acknowledges a successful connection and assigns a client identifier.
    WELCOME(WelcomeMessage),
    /// Client requests the current document contents; payload contains their StateVector
    GETDOC(Vec<u8>),
    /// Document delta/update payload; payload is an encoded yrs/yjs update
    DOCUPDATE(Vec<u8>),
    /// Document update received
    RECEIVEDDOCUPDATE,
    /// Client announces or updates its selection/cursor location.
    SETCURSOR(SetCursorMessage),
    /// Client removes its cursor/selection from the shared state.
    REMOVECURSOR(RemoveCursorMessage),
    /// Client indicates it is disconnecting from the document session. They may connect to another document or disconnect completely.
    DISCONNECT(DisconnectMessage),
    /// Server reports an error condition.
    ERROR(ErrorMessage),
}
/// Errors that can occur while decoding/encoding `WebsocketMessage` frames.
#[derive(Debug)]
pub enum WebsocketDecodeEncodeError {
    /// The provided frame had no bytes, so no message identifier could be read.
    EmptyMessage,
    /// The first byte did not match any known message identifier.
    UnknownMessageType,
    /// JSON (de)serialization failed for a message whose payload is JSON.
    SerdeError(serde_json::Error),
}
/// Converts a `serde_json::Error` into `WebsocketDecodeEncodeError` to simplify decoding code.
impl From<serde_json::Error> for WebsocketDecodeEncodeError {
    fn from(value: Error) -> Self {
        WebsocketDecodeEncodeError::SerdeError(value)
    }
}

impl Into<Vec<u8>> for WebsocketMessage {
    fn into(self) -> Vec<u8> {
        let mut res = Vec::new();
        match self {
            WebsocketMessage::CONNECT(msg) => {
                res.push(10);
                res.extend(serde_json::to_vec(&msg).unwrap());
            }
            WebsocketMessage::WELCOME(msg) => {
                res.push(11);
                res.extend(serde_json::to_vec(&msg).unwrap());
            }
            WebsocketMessage::GETDOC(data) => {
                res.push(20);
                res.extend(data);
            }
            WebsocketMessage::DOCUPDATE(data) => {
                res.push(30);
                res.extend(data);
            }
            WebsocketMessage::RECEIVEDDOCUPDATE => {
                res.push(31);
            }
            WebsocketMessage::SETCURSOR(msg) => {
                res.push(40);
                res.extend(serde_json::to_vec(&msg).unwrap());
            }
            WebsocketMessage::REMOVECURSOR(msg) => {
                res.push(41);
                res.extend(serde_json::to_vec(&msg).unwrap());
            }
            WebsocketMessage::DISCONNECT(msg) => {
                res.push(50);
                res.extend(serde_json::to_vec(&msg).unwrap());
            }
            WebsocketMessage::ERROR(msg) => {
                res.push(60);
                res.extend(serde_json::to_vec(&msg).unwrap());
            }
        }
        res
    }
}
/// Decodes a WebSocket frame (as bytes) into a `WebsocketMessage`.
/// Expects the first byte to be a message identifier; the remaining bytes are the payload.
impl TryFrom<Vec<u8>> for WebsocketMessage {
    type Error = WebsocketDecodeEncodeError;
    fn try_from(value: Vec<u8>) -> Result<Self, Self::Error> {
        let identifier_byte = match value.get(0) {
            Some(byte) => byte,
            None => return Err(WebsocketDecodeEncodeError::EmptyMessage),
        };
        match identifier_byte {
            10 => Ok(WebsocketMessage::CONNECT(serde_json::from_slice::<
                ConnectMessage,
            >(&value[1..])?)),
            11 => Ok(WebsocketMessage::WELCOME(serde_json::from_slice::<
                WelcomeMessage,
            >(&value[1..])?)),
            20 => Ok(WebsocketMessage::GETDOC(value[1..].to_vec())),
            30 => Ok(WebsocketMessage::DOCUPDATE(value[1..].to_vec())),
            31 => Ok(WebsocketMessage::RECEIVEDDOCUPDATE),
            40 => Ok(WebsocketMessage::SETCURSOR(serde_json::from_slice::<
                SetCursorMessage,
            >(&value[1..])?)),
            41 => Ok(WebsocketMessage::REMOVECURSOR(serde_json::from_slice::<
                RemoveCursorMessage,
            >(&value[1..])?)),
            50 => Ok(WebsocketMessage::DISCONNECT(serde_json::from_slice::<
                DisconnectMessage,
            >(&value[1..])?)),
            60 => Ok(WebsocketMessage::ERROR(serde_json::from_slice::<
                ErrorMessage,
            >(&value[1..])?)),
            &_ => Err(WebsocketDecodeEncodeError::UnknownMessageType),
        }
    }
}
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ConnectMessage {
    /// Identifier of the document the client wants to join.
    pub document_id: uuid::Uuid,
}
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct WelcomeMessage {
    /// Server-assigned identifier for this client connection.
    pub client_id: uuid::Uuid,
}
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SetCursorMessage {
    /// Identifier of the client whose cursor/selection is being updated.
    pub client_id: uuid::Uuid,
    /// Identifier of the editor block the cursor refers to.
    pub block_id: uuid::Uuid,
    /// Start offset of the cursor/selection within the referenced block.
    pub start: usize,
    /// Optional end offset for a selection; `None` represents a caret without a selection range.
    pub end: Option<usize>,
}
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RemoveCursorMessage {
    /// Identifier of the client whose cursor/selection should be removed.
    pub client_id: uuid::Uuid,
}
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DisconnectMessage {
    /// Identifier of the client that is disconnecting.
    pub client_id: uuid::Uuid,
}
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ErrorMessage {
    /// (HTTP-like) Status code representing the error category.
    pub status: u16,
    /// Human-readable error description.
    pub error: String,
}

#[get("/api/projects/<project_id>/websocket")]
pub async fn websocket<'a>(
    ws: ws::WebSocket,
    _session: Session,
    project_id: &'a str,
    project_storage: &'a State<Arc<ProjectStorage>>,
    settings: &'a State<Settings>,
    _data_storage: &'a State<Arc<DataStorage>>,
    websocket_manager: &'a State<Arc<WebsocketManager>>,
) -> ws::Channel<'a> {
    let project_storage = project_storage.inner().clone();
    let settings = settings.inner().clone();
    let project_id = match uuid::Uuid::parse_str(project_id) {
        Ok(project_id) => project_id,
        Err(e) => {
            // Invalid project ID, return error via WebSocket
            error!("Failed to parse project ID: {}", e);
            let error_ws_msg = WebsocketMessage::ERROR(ErrorMessage {
                status: 400,
                error: "Invalid project id".to_string(),
            });
            let data: Vec<u8> = error_ws_msg.into();
            return ws.channel(move |mut stream| {
                Box::pin(async move {
                    let _ = stream.send(data.into()).await;
                    Ok(())
                })
            });
        }
    };
    ws.channel(move |mut stream| Box::pin(async move {
        let client_id = uuid::Uuid::new_v4();
        let mut document_id : Option<uuid::Uuid> = None;
        let mut broadcast_rx : Option<broadcast::Receiver<BroadcastMessage>> = None;

        loop {
            tokio::select! {
                // Handle messages from the client
                msg_result = stream.next() => {
                    let result = match msg_result {
                        Some(result) => result,
                        None => break, // Stream closed
                    };
                    let msg = match result {
                        Ok(msg) => msg,
                        Err(_) => break, // Connection closed
                    };

                    let data = msg.into_data();
                    let ws_msg = match WebsocketMessage::try_from(data) {
                        Ok(msg) => msg,
                        Err(e) => {
                            error!("Failed to parse WebSocket message: {:?}", e);
                            let error_msg = match e {
                                WebsocketDecodeEncodeError::EmptyMessage => {
                                    warn!("Received empty WebSocket message, ignoring");
                                    Some(ErrorMessage {
                                        status: 400,
                                        error: "Empty message".to_string(),
                                    })
                                }
                                WebsocketDecodeEncodeError::UnknownMessageType => {
                                    warn!("Received WebSocket message with unknown type, ignoring");
                                    Some(ErrorMessage {
                                        status: 400,
                                        error: "Unknown message type".to_string(),
                                    })
                                }
                                WebsocketDecodeEncodeError::SerdeError(json_err) => {
                                    error!("JSON deserialization error in WebSocket message: {}", json_err);
                                    Some(ErrorMessage {
                                        status: 400,
                                        error: format!("JSON error: {}", json_err),
                                    })
                                }
                            };

                            if let Some(msg) = error_msg {
                                let error_ws_msg = WebsocketMessage::ERROR(msg);
                                let data: Vec<u8> = error_ws_msg.into();
                                if let Err(e) = stream.send(data.into()).await {
                                    error!("Failed to send error message: {}", e);
                                    break;
                                }
                            }
                            continue;
                        }
                    };

                    let result = handle_client_msg(
                        ws_msg,
                        &client_id,
                        &project_id,
                        &mut document_id,
                        &mut broadcast_rx,
                        websocket_manager.inner().clone(),
                        project_storage.clone(),
                        &settings,
                    ).await;

                    match result {
                        Ok(responses) => {
                            for response in responses {
                                let data: Vec<u8> = response.into();
                                if let Err(e) = stream.send(data.into()).await {
                                    error!("Failed to send response: {}", e);
                                    break;
                                }
                            }
                        }
                        Err(err) => {
                            let data: Vec<u8> = WebsocketMessage::ERROR(err).into();
                            if let Err(e) = stream.send(data.into()).await {
                                error!("Failed to send handle_client_msg error: {}", e);
                                break;
                            }
                        }
                    }
                },
                // Handle messages from the broadcast channel
                Some(broadcast_msg) = async {
                    if let Some(rx) = broadcast_rx.as_mut() {
                        rx.recv().await.ok()
                    } else {
                        None
                    }
                } => {
                    if broadcast_msg.sender_id != client_id { // Don't send messages back to the sender
                        let data: Vec<u8> = broadcast_msg.message.into();
                        if let Err(e) = stream.send(data.into()).await {
                            error!("Failed to send broadcast message: {}", e);
                            break;
                        }
                    }
                },
                else => break,
            }
        }

        // Cleanup on disconnect
        //todo: send remove cursor message to all clients
        if let Some(doc_id) = document_id {
            if let Some(mut state) = websocket_manager.documents.get_mut(&doc_id) {
                state.active_clients.retain(|&id| id != client_id);
                if state.active_clients.is_empty() {
                    let state_to_save = state.clone();
                    let save_worker_notifier = state.save_requester.clone();
                    drop(state); // Release lock before saving and potentially removing

                    // Re-check and remove atomically
                    websocket_manager.documents.remove_if(&doc_id, |_, s| {
                        s.active_clients.is_empty()
                    });

                    // If it was removed, save it
                    if !websocket_manager.documents.contains_key(&doc_id) {
                        save_worker_notifier.notify_one(); // Shutdown save worker notifier.
                        let _ = state_to_save.save_document_to_storage(
                            &project_id,
                            &doc_id,
                            project_storage.clone(),
                            &settings
                        ).await;
                    }
                }
            }
        }

        Ok(())
    }))
}

async fn handle_client_msg(
    msg: WebsocketMessage,
    client_id: &uuid::Uuid,
    project_id: &uuid::Uuid,
    document_id: &mut Option<uuid::Uuid>,
    broadcast_rx: &mut Option<broadcast::Receiver<BroadcastMessage>>,
    websocket_manager: Arc<WebsocketManager>,
    project_storage: Arc<ProjectStorage>,
    settings: &Settings,
) -> Result<Vec<WebsocketMessage>, ErrorMessage> {
    match msg {
        WebsocketMessage::CONNECT(msg) => {
            debug!("Received CONNECT message from client {}", client_id);
            *document_id = Some(msg.document_id);
            let doc_id = msg.document_id;

            let state = websocket_manager
                .get_or_load_from_storage(project_id, &doc_id, Some(client_id))
                .await
                .unwrap();

            broadcast_rx.replace(state.broadcast_tx.subscribe());

            let responses = vec![WebsocketMessage::WELCOME(WelcomeMessage {
                client_id: *client_id,
            })];

            Ok(responses)
        }
        WebsocketMessage::GETDOC(raw_statevec) => {
            debug!("Received GETDOC message from client {}", client_id);
            if let Some(doc_id) = document_id {
                let statevec: StateVector = match StateVector::decode_v1(&raw_statevec) {
                    Ok(statevec) => statevec,
                    Err(e) => {
                        error!("Failed to decode StateVector: {}", e);
                        return Err(ErrorMessage {
                            status: 400,
                            error: format!("Failed to decode StateVector: {}", e),
                        });
                    }
                };
                if let Some(state) = websocket_manager.get_document_mut(&doc_id) {
                    let binary_update = state.doc.transact().encode_diff_v1(&statevec);
                    return Ok(vec![WebsocketMessage::DOCUPDATE(binary_update)]);
                }
            }
            Err(ErrorMessage {
                status: 404,
                error: "Document not found. Make sure you connect first and provide a valid document id.".to_string(),
            })
        }
        WebsocketMessage::DOCUPDATE(raw_update) => {
            debug!("Received DOCUPDATE message from client {}", client_id);

            if let Some(doc_id) = document_id {
                // Decode update from client
                let update: Update = match Update::decode_v1(&raw_update) {
                    Ok(update) => update,
                    Err(e) => {
                        error!("Failed to decode update: {}", e);
                        return Err(ErrorMessage {
                            status: 400,
                            error: format!("Failed to decode update: {}", e),
                        });
                    }
                };

                // Apply update to server document state
                let doc = match websocket_manager.get_document_mut(doc_id){
                    Some(doc) => doc,
                    None => {
                        return Err(ErrorMessage {
                            status: 404,
                            error: "Document not found. Make sure you connect first and provide a valid document id.".to_string(),
                        })
                    }
                };

                let mut txn = doc.doc.transact_mut();
                if let Err(e) = txn.apply_update(update) {
                    error!("Failed to apply yrs update: {}", e);
                    return Err(ErrorMessage {
                        status: 500,
                        error: format!("Failed to apply update: {}", e),
                    });
                }

                // Broadcast update to all clients (except the sender)
                let sender = doc.broadcast_tx.clone();
                drop(txn); // must drop txn before dropping doc or using it

                // Notify save worker to save the document
                doc.save_requester.notify_one();

                drop(doc);

                let _ = sender.send(BroadcastMessage {
                    sender_id: *client_id,
                    message: DOCUPDATE(raw_update),
                });
            }
            Ok(vec![RECEIVEDDOCUPDATE])
        }
        WebsocketMessage::SETCURSOR(msg) => {
            // Broadcast update to all clients (except the sender)
            if let Some(doc_id) = document_id {
                let doc = match websocket_manager.get_document_mut(doc_id) {
                    Some(doc) => doc,
                    None => {
                        return Err(ErrorMessage {
                            status: 404,
                            error: "Document not found. Make sure you connect first and provide a valid document id.".to_string(),
                        })
                    }
                };

                let sender = doc.broadcast_tx.clone();
                drop(doc);
                let _ = sender.send(BroadcastMessage {
                    sender_id: *client_id,
                    message: SETCURSOR(msg),
                });
            }
            Ok(vec![])
        }
        WebsocketMessage::REMOVECURSOR(msg) => {
            // Broadcast update to all clients (except the sender)
            if let Some(doc_id) = document_id {
                let doc = match websocket_manager.get_document_mut(doc_id) {
                    Some(doc) => doc,
                    None => {
                        return Err(ErrorMessage {
                            status: 404,
                            error: "Document not found. Make sure you connect first and provide a valid document id.".to_string(),
                        })
                    }
                };

                let sender = doc.broadcast_tx.clone();
                drop(doc);
                let _ = sender.send(BroadcastMessage {
                    sender_id: *client_id,
                    message: REMOVECURSOR(msg),
                });
            }
            Ok(vec![])
        }
        WebsocketMessage::DISCONNECT(_msg) => {
            // Remove client from document state, remove document state if no clients are left
            if let Some(doc_id) = document_id {
                let doc_id = *doc_id;

                if let Some(mut state) = websocket_manager.get_document_mut(&doc_id) {
                    let sender = state.broadcast_tx.clone();
                    let _ = sender.send(BroadcastMessage {
                        sender_id: *client_id,
                        message: REMOVECURSOR(RemoveCursorMessage {
                            client_id: *client_id,
                        }),
                    });
                    state.active_clients.retain(|&id| id != *client_id);
                    if state.active_clients.is_empty() {
                        let state_to_save = state.clone();
                        let save_worker_notifier = state.save_requester.clone();
                        drop(state);

                        websocket_manager
                            .documents
                            .remove_if(&doc_id, |_, s| s.active_clients.is_empty());

                        if !websocket_manager.documents.contains_key(&doc_id) {
                            save_worker_notifier.notify_one(); // Notify the save worker to shut itself down
                            let _ = state_to_save
                                .save_document_to_storage(
                                    project_id,
                                    &doc_id,
                                    project_storage.clone(),
                                    settings,
                                )
                                .await;
                        }
                    }
                }
                *document_id = None;
            }
            Ok(vec![])
        }
        _ => {
            error!("Unexpected websocket message: {:?}", msg);
            Ok(vec![])
        }
    }
}

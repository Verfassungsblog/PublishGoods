use crate::session::session_guard::Session;
use crate::settings::Settings;
use crate::storage::data_storage::DataStorage;
use crate::storage::project_storage::ProjectStorage;
use dashmap::DashMap;
use rocket::futures::{SinkExt, StreamExt};
use rocket::State;
use serde::{Deserialize, Serialize};
use serde_json::Error;
use std::sync::Arc;
use tokio::sync::broadcast;
use yrs::Doc;

pub struct WebsocketManager {
    pub documents: DashMap<uuid::Uuid, DocumentState>,
}

pub struct DocumentState {
    pub broadcast_tx: broadcast::Sender<BroadcastMessage>,
    pub active_clients: Vec<uuid::Uuid>,
    pub doc: Doc,
}

impl DocumentState {
    pub async fn create_doc(
        &mut self,
        project_id: uuid::Uuid,
        document_id: uuid::Uuid,
        project_storage: Arc<ProjectStorage>,
        project_settings: Arc<Settings>,
    ) -> Self {
        let mut ydoc = Doc::new();
        let project_lock = project_storage
            .get_project(&project_id, &project_settings)
            .await
            .unwrap()
            .clone();
        let project = project_lock.read().unwrap();

        //TODO!

        unimplemented!()
    }
}

#[derive(Clone, Debug)]
pub struct BroadcastMessage {
    pub sender_id: uuid::Uuid,
    pub message: WebsocketMessage,
}

impl WebsocketManager {
    pub fn new() -> Self {
        Self {
            documents: DashMap::new(),
        }
    }
}

/// Application-level WebSocket messages encoded as a single identifier byte followed by a payload.
/// The first byte of an incoming frame selects the variant:
/// - `10` => `CONNECT` (JSON)
/// - `11` => `WELCOME` (JSON)
/// - `20` => `GETDOC` (raw bytes)
/// - `30` => `DOCUPDATE` (raw bytes)
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
    /// Client requests the current document contents; payload is treated as opaque bytes.
    GETDOC(Vec<u8>),
    /// Document delta/update payload; payload is treated as opaque bytes.
    DOCUPDATE(Vec<u8>),
    /// Client announces or updates its selection/cursor location.
    SETCURSOR(SetCursorMessage),
    /// Client removes its cursor/selection from the shared state.
    REMOVECURSOR(RemoveCursorMessage),
    /// Client indicates it is disconnecting.
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
pub fn websocket<'a>(
    ws: ws::WebSocket,
    _session: Session,
    project_id: &'a str,
    project_storage: &'a State<Arc<ProjectStorage>>,
    _data_storage: &'a State<Arc<DataStorage>>,
    websocket_manager: &'a State<Arc<WebsocketManager>>,
) -> ws::Channel<'a> {
    ws.channel(move |mut stream| Box::pin(async move {
        let mut client_id = uuid::Uuid::new_v4();
        let mut document_id : Option<uuid::Uuid> = None;
        let mut broadcast_rx : Option<broadcast::Receiver<BroadcastMessage>> = None;

        loop {
            tokio::select! {
                // Handle messages from the client
                Some(result) = stream.next() => {
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
                                let _ = stream.send(data.into()).await;
                            }
                            continue;
                        }
                    };

                    match ws_msg {
                        WebsocketMessage::CONNECT(msg) => {
                            document_id = Some(msg.document_id);
                            let (tx, rx) = {
                                let mut state = websocket_manager.documents.entry(msg.document_id).or_insert_with(|| {
                                    debug!("Creating new broadcast channel for document {}", msg.document_id);
                                    unimplemented!();
                                    /*
                                    let (tx, _) = broadcast::channel(100);

                                    //todo!
                                    DocumentState {
                                        broadcast_tx: tx,
                                        active_clients: Vec::new(),
                                    }

                                     */
                                });
                                debug!("Reusing existing broadcast channel for document {}", msg.document_id);
                                state.active_clients.push(client_id);
                                (state.broadcast_tx.clone(), state.broadcast_tx.subscribe())
                            };
                            broadcast_rx = Some(rx);

                            let welcome = WebsocketMessage::WELCOME(WelcomeMessage { client_id });
                            let data: Vec<u8> = welcome.into();
                            let _ = stream.send(data.into()).await;
                        },
                        WebsocketMessage::DOCUPDATE(_) | WebsocketMessage::SETCURSOR(_) | WebsocketMessage::REMOVECURSOR(_) | WebsocketMessage::GETDOC(_) => {
                            if let Some(doc_id) = document_id {
                                if let Some(state) = websocket_manager.documents.get(&doc_id) {
                                    let _ = state.broadcast_tx.send(BroadcastMessage {
                                        sender_id: client_id,
                                        message: ws_msg,
                                    });
                                }
                            }
                        },
                        WebsocketMessage::DISCONNECT(msg) => {
                            client_id = msg.client_id;
                            break;
                        },
                        _ => {}
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
                        let _ = stream.send(data.into()).await;
                    }
                }
            }
        }

        // Cleanup on disconnect
        if let Some(doc_id) = document_id {
            if let Some(mut state) = websocket_manager.documents.get_mut(&doc_id) {
                state.active_clients.retain(|&id| id != client_id);
            }
        }

        Ok(())
    }))
}

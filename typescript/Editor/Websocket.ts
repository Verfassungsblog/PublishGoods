import {show_alert} from "../tools";

export enum WebsocketMessageType {
    CONNECT = 10,
    WELCOME = 11,
    GETDOC = 20,
    DOCUPDATE = 30,
    RECEIVEDDOCUPDATE = 31,
    SETCURSOR = 40,
    REMOVECURSOR = 41,
    DISCONNECT = 50,
    ERROR = 60,
}
export interface ConnectMessage {
    document_id: string;
}

export interface WelcomeMessage {
    client_id: string;
}

export interface SetCursorMessage {
    client_id: string;
    block_id: string;
    start: number;
    end: number | null;
}

export interface RemoveCursorMessage {
    client_id: string;
}

export interface DisconnectMessage {
    client_id: string;
}

export interface ErrorMessage {
    status: number;
    error: string;
}

export interface OpenMessage {
    /** True if this is a re-established connection (not the first one of this client). */
    reconnect: boolean;
}

export type WebsocketEventMap = {
    'open': OpenMessage;
    'welcome': WelcomeMessage;
    'docUpdate': Uint8Array;
    'receivedDocUpdate': null;
    'setCursor': SetCursorMessage;
    'removeCursor': RemoveCursorMessage;
    'error': ErrorMessage;
    'disconnect': DisconnectMessage;
};

export function WebsocketClient(projectId: string) {
    let ws: WebSocket | null = null;
    let clientId: string | null = null;
    let connection_end_requested: boolean = false;
    let first_connection: boolean = true;
    let connection_lost_alerted: boolean = false;
    let reconnect_attempts: number = 0;
    let reconnect_timer: ReturnType<typeof setTimeout> | null = null;

    const handlers: { [K in keyof WebsocketEventMap]?: ((data: WebsocketEventMap[K]) => void)[] } = {};

    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    const url = `${protocol}//${window.location.host}/api/projects/${projectId}/websocket`;

    const RECONNECT_BASE_DELAY_MS = 1000;
    const RECONNECT_MAX_DELAY_MS = 30000;

    /**
     * Exponential backoff with jitter: ~1s, 2s, 4s, ... capped at 30s (each randomized to 50-100%).
     */
    function nextReconnectDelay(): number {
        const exp = Math.min(RECONNECT_MAX_DELAY_MS, RECONNECT_BASE_DELAY_MS * 2 ** reconnect_attempts);
        reconnect_attempts++;
        return exp * (0.5 + Math.random() * 0.5);
    }

    function scheduleReconnect(): void {
        if (reconnect_timer !== null) return;
        const delay = nextReconnectDelay();
        reconnect_timer = setTimeout(() => {
            reconnect_timer = null;
            if (connection_end_requested) return;
            // Failures are handled by onclose scheduling the next attempt.
            connect().catch(() => {});
        }, delay);
    }

    function connect(): Promise<void> {
        return new Promise((resolve, reject) => {
            const socket = new WebSocket(url);
            ws = socket;
            socket.binaryType = 'arraybuffer';
            let opened = false;
            let opened_time = 0;

            socket.onopen = () => {
                opened = true;
                opened_time = performance.now();
                console.log('WebSocket connected');
                // Also a reconnect if the very first attempts failed and an alert was shown.
                const reconnect = !first_connection || connection_lost_alerted;
                first_connection = false;
                resolve();
                // Every (re)connection is a fresh server-side session, so listeners must redo the handshake.
                trigger('open', { reconnect });
            };

            socket.onerror = (error) => {
                console.error('WebSocket error:', error);
                // Only meaningful while the promise is still pending; rejecting a settled promise is a no-op.
                if (!opened) reject(error);
            };

            socket.onmessage = (event) => {
                handleMessage(event.data);
            };

            socket.onclose = (event) => {
                // Ignore close events of sockets that were already replaced (but don't strand their promise).
                if (ws !== socket) {
                    if (!opened) reject(new Error(`WebSocket replaced before opening (code ${event.code})`));
                    return;
                }
                ws = null;
                clientId = null;
                console.log(
                    `WebSocket closed: code=${event.code} reason=${JSON.stringify(event.reason)} ` +
                    `wasClean=${event.wasClean} openFor=${opened ? Math.round(performance.now() - opened_time) + 'ms' : 'never opened'}`
                );
                if (!opened) reject(new Error(`WebSocket closed before opening (code ${event.code})`));
                if(!connection_end_requested) { // Only try to reconnect if disconnect wasn't requested by the user
                    console.log('WebSocket closed. Trying to reconnect...');
                    // Signal to user that connection is lost (once, not per retry)
                    if (!connection_lost_alerted) {
                        connection_lost_alerted = true;
                        show_alert('Verbindung zum Server verloren. Versuche, die Verbindung wiederherzustellen...', 'warning');
                    }
                    scheduleReconnect();
                }
            };
        });
    }

    function on<K extends keyof WebsocketEventMap>(event: K, handler: (data: WebsocketEventMap[K]) => void): void {
        if (!handlers[event]) {
            handlers[event] = [];
        }
        handlers[event]!.push(handler);
    }

    function trigger<K extends keyof WebsocketEventMap>(event: K, data: WebsocketEventMap[K]): void {
        const eventHandlers = handlers[event];
        if (eventHandlers) {
            eventHandlers.forEach(handler => handler(data));
        }
    }

    function handleMessage(data: ArrayBuffer): void {
        const view = new Uint8Array(data);
        if (view.length === 0) return;

        const type = view[0] as WebsocketMessageType;
        const payload = view.slice(1);

        switch (type) {
            case WebsocketMessageType.WELCOME:
                const welcome: WelcomeMessage = JSON.parse(new TextDecoder().decode(payload));
                clientId = welcome.client_id;
                // Only now is the connection proven usable: the server accepted the session. Resetting
                // the backoff on mere socket open would let a server that accepts and immediately closes
                // be retried at the minimum delay forever.
                reconnect_attempts = 0;
                if (connection_lost_alerted) {
                    connection_lost_alerted = false;
                    show_alert('Verbindung zum Server wiederhergestellt!', 'success');
                }
                trigger('welcome', welcome);
                break;
            case WebsocketMessageType.DOCUPDATE:
                trigger('docUpdate', payload);
                break;
            case WebsocketMessageType.RECEIVEDDOCUPDATE:
                trigger('receivedDocUpdate', null);
                break;
            case WebsocketMessageType.SETCURSOR:
                const setCursor: SetCursorMessage = JSON.parse(new TextDecoder().decode(payload));
                trigger('setCursor', setCursor);
                break;
            case WebsocketMessageType.REMOVECURSOR:
                const removeCursor: RemoveCursorMessage = JSON.parse(new TextDecoder().decode(payload));
                trigger('removeCursor', removeCursor);
                break;
            case WebsocketMessageType.DISCONNECT:
                const disconnect: DisconnectMessage = JSON.parse(new TextDecoder().decode(payload));
                trigger('disconnect', disconnect);
                break;
            case WebsocketMessageType.ERROR:
                const error: ErrorMessage = JSON.parse(new TextDecoder().decode(payload));
                trigger('error', error);
                break;
            default:
                console.warn('Unknown WebSocket message type:', type);
        }
    }

    function sendConnect(documentId: string): void {
        connection_end_requested = false;
        const msg: ConnectMessage = { document_id: documentId };
        sendJson(WebsocketMessageType.CONNECT, msg);
    }

    function sendGetDoc(stateVector: Uint8Array): void {
        sendBinary(WebsocketMessageType.GETDOC, stateVector);
    }

    function sendDocUpdate(update: Uint8Array): void {
        sendBinary(WebsocketMessageType.DOCUPDATE, update);
    }

    function sendSetCursor(blockId: string, start: number, end: number | null): void {
        if (!clientId) return;
        const msg: SetCursorMessage = {
            client_id: clientId,
            block_id: blockId,
            start,
            end
        };
        sendJson(WebsocketMessageType.SETCURSOR, msg);
    }

    function sendRemoveCursor(): void {
        if (!clientId) return;
        const msg: RemoveCursorMessage = { client_id: clientId };
        sendJson(WebsocketMessageType.REMOVECURSOR, msg);
    }

    function sendDisconnect(): void {
        connection_end_requested = true;
        first_connection = true;
        if (reconnect_timer !== null) {
            clearTimeout(reconnect_timer);
            reconnect_timer = null;
        }
        if (clientId && ws && ws.readyState === WebSocket.OPEN) {
            const msg: DisconnectMessage = { client_id: clientId };
            sendJson(WebsocketMessageType.DISCONNECT, msg);
        }
        ws?.close();
    }

    function sendJson(type: WebsocketMessageType, msg: any): void {
        const json = JSON.stringify(msg);
        const payload = new TextEncoder().encode(json);
        sendBinary(type, payload);
    }

    function sendBinary(type: WebsocketMessageType, payload: Uint8Array): void {
        if (!ws || ws.readyState !== WebSocket.OPEN) {
            console.error('WebSocket is not open');
            show_alert("Couldn't save changes: Connection to server lost!", "danger");
            return;
        }
        const data = new Uint8Array(1 + payload.length);
        data[0] = type;
        data.set(payload, 1);
        ws.send(data);
    }

    function getClientId(): string | null {
        return clientId;
    }

    return {
        connect,
        on,
        sendConnect,
        sendGetDoc,
        sendDocUpdate,
        sendSetCursor,
        sendRemoveCursor,
        sendDisconnect,
        getClientId
    };
}

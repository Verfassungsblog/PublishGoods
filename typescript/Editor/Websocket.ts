
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

export type WebsocketEventMap = {
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
    const handlers: { [K in keyof WebsocketEventMap]?: ((data: WebsocketEventMap[K]) => void)[] } = {};

    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    const url = `${protocol}//${window.location.host}/api/projects/${projectId}/websocket`;

    function connect(): Promise<void> {
        return new Promise((resolve, reject) => {
            ws = new WebSocket(url);
            ws.binaryType = 'arraybuffer';

            ws.onopen = () => {
                console.log('WebSocket connected');
                resolve();
            };

            ws.onerror = (error) => {
                console.error('WebSocket error:', error);
                reject(error);
            };

            ws.onmessage = (event) => {
                handleMessage(event.data);
            };

            ws.onclose = () => {
                console.log('WebSocket closed');
                ws = null;
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
        if (!clientId) return;
        const msg: DisconnectMessage = { client_id: clientId };
        sendJson(WebsocketMessageType.DISCONNECT, msg);
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

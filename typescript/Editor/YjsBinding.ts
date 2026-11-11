import * as Y from 'yjs';
import {WebsocketClient} from './Websocket';

/**
 * Enable verbose debug logging for Yjs update origins and stacks.
 *
 * Keep this `false` by default to avoid noisy logs and to prevent DevTools from
 * eagerly inspecting complex Yjs objects during frequent updates.
 */
const DEBUG_YJS_UPDATES = false;

/**
 * Creates a Yjs document binding backed by the project websocket.
 *
 * Responsibilities:
 * - Maintain a local `Y.Doc`.
 * - Redo the CONNECT handshake on every (re)connect and request the document state after `welcome`.
 * - Apply server updates (`origin = 'server'`).
 * - Send local updates back to the server (everything except `origin = 'server'`).
 *
 * @param projectId Project UUID (string) used to create the websocket client.
 * @param documentId Document UUID (string) used to connect and scope messages.
 */
export function YjsBinding(projectId: string, documentId: string) {
    const doc = new Y.Doc();
    const ws = WebsocketClient(projectId);
    let destroyed = false;

    /**
     * Sets up websocket handlers to keep `doc` in sync with the server.
     *
     * @param documentId The document id to connect to.
     */
    function setupWebsocket(documentId: string) {
        // Set after a reconnect: local edits made while offline never reached the server, so once the
        // server state has been merged we push our full state (Yjs updates are idempotent).
        let pushLocalStateAfterSync = false;

        // Every (re)connection is a new server-side session that doesn't know the document yet, so the
        // CONNECT -> WELCOME -> GETDOC handshake has to be repeated each time the socket opens.
        ws.on('open', ({reconnect}) => {
            if (destroyed) return;
            console.log(reconnect ? 'WebSocket reopened, sending CONNECT...' : 'WebSocket open, sending CONNECT...');
            pushLocalStateAfterSync = reconnect;
            ws.sendConnect(documentId);
        });

        ws.on('welcome', () => {
            if (destroyed) return;
            console.log('Received welcome from server, requesting document state...');
            
            // Request the document state
            const stateVector = Y.encodeStateVector(doc);
            ws.sendGetDoc(stateVector);
        });

        ws.on('docUpdate', (update: Uint8Array) => {
            if (destroyed) return;
            console.log('Received document update, length:', update.length);
            Y.applyUpdate(doc, update, 'server');

            if (pushLocalStateAfterSync) {
                pushLocalStateAfterSync = false;
                ws.sendDocUpdate(Y.encodeStateAsUpdate(doc));
            }

            trigger('update', doc);

            if (DEBUG_YJS_UPDATES) {
                console.log('Current document state (debug):', {
                    guid: (doc as any).guid,
                    clientID: (doc as any).clientID,
                });
            }
        });

        doc.on('update', (update, origin) => {
            if (destroyed) return;

            if (DEBUG_YJS_UPDATES) {
                const stack = new Error().stack;
                console.log('[YjsBinding] doc.update', {
                    origin,
                    length: update?.length,
                    stack,
                });
            }

            if (origin !== 'server') {
                if (update.length <= 2 && update[0] === 0 && (update.length === 1 || update[1] === 0)) {
                    return;
                }
                console.log('Local update, sending to server... Origin:', origin, 'Length:', update.length);
                ws.sendDocUpdate(update);
            }
        });

        ws.on('error', (err) => {
            if (destroyed) return;
            console.error('YjsBinding websocket error:', err);
        });

        ws.connect().catch(err => {
            if (destroyed) return;
            console.error('Failed to connect to websocket:', err);
        });
    }

    const handlers: { [key: string]: ((data: any) => void)[] } = {};

    /**
     * Registers a handler for a binding event.
     *
     * Currently supported events:
     * - `update`: triggered after a server update has been applied to the `Y.Doc`.
     */
    function on(event: string, handler: (data: any) => void) {
        if (!handlers[event]) {
            handlers[event] = [];
        }
        handlers[event].push(handler);
    }

    /**
     * Triggers a binding event.
     *
     * @param event Event name.
     * @param data Payload passed to handlers.
     */
    function trigger(event: string, data: any) {
        if (handlers[event]) {
            handlers[event].forEach(h => h(data));
        }
    }

    setupWebsocket(documentId);

    /**
     * Returns the underlying `Y.Doc` instance.
     */
    function getDoc(): Y.Doc {
        return doc;
    }

    return {
        getDoc,
        on,
        /**
         * Destroys the binding and disconnects the websocket.
         *
         * The binding becomes inert after destruction.
         */
        destroy: () => {
            if (destroyed) return;
            destroyed = true;
            console.log("Destroying YjsBinding for document:", documentId);
            ws.sendDisconnect();
        }
    };
}

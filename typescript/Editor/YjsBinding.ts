import * as Y from 'yjs';
import {WebsocketClient} from './Websocket';

export function YjsBinding(projectId: string, documentId: string) {
    const doc = new Y.Doc();
    const ws = WebsocketClient(projectId);
    let destroyed = false;

    function setupWebsocket(documentId: string) {
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
            
            trigger('update', doc);
            
            // Debug print the document state
            console.log('Current document state:');
            console.log(doc);
        });

        doc.on('update', (update, origin) => {
            if (destroyed) return;
            if (origin !== 'server') {
                if (update.length <= 2 && update[0] === 0 && (update.length === 1 || update[1] === 0)) {
                    // console.log('Empty local update, skipping send to server');
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

        ws.connect().then(() => {
            if (destroyed) return;
            console.log('WebSocket open, sending CONNECT...');
            ws.sendConnect(documentId);
        }).catch(err => {
            if (destroyed) return;
            console.error('Failed to connect to websocket:', err);
        });
    }

    const handlers: { [key: string]: ((data: any) => void)[] } = {};

    function on(event: string, handler: (data: any) => void) {
        if (!handlers[event]) {
            handlers[event] = [];
        }
        handlers[event].push(handler);
    }

    function trigger(event: string, data: any) {
        if (handlers[event]) {
            handlers[event].forEach(h => h(data));
        }
    }

    setupWebsocket(documentId);

    function getDoc(): Y.Doc {
        return doc;
    }

    return {
        getDoc,
        on,
        destroy: () => {
            if (destroyed) return;
            destroyed = true;
            console.log("Destroying YjsBinding for document:", documentId);
            ws.sendDisconnect();
        }
    };
}

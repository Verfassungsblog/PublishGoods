import {main_col} from "./Editor";
import {YjsBinding} from "./YjsBinding";
import {state} from "./Main";
import EditorJS from "@editorjs/editorjs";
// @ts-ignore
import Header from "@editorjs/header";
// @ts-ignore
import List from "@editorjs/list";
// @ts-ignore
import Quote from "@editorjs/quote";
// @ts-ignore
import Raw from "@editorjs/raw";
// @ts-ignore
import ImageTool from "@editorjs/image";
import {createMutex} from "./Mutex";

let currentEditor: EditorJS | null = null;
let currentYjsBinding: any = null;
let currentEditorBinding: any = null;

export async function showSectionEditor(section_id: string){
    console.log("Loading section editor for section: " + section_id);

    // Cleanup previous instances
    if (currentEditor) {
        try {
            // Best-effort wait for EditorJS init before tearing down to avoid internal state issues.
            // (We intentionally swallow errors here because we are in cleanup.)
            await (currentEditor as any).isReady?.catch?.((): void => undefined);

            const destroyFn = (currentEditor as any).destroy;
            if (typeof destroyFn === 'function') {
                destroyFn.call(currentEditor);
            } else {
                console.warn('Editor instance has no destroy() function, skipping destroy');
            }
        } catch (e) {
            console.error("Error destroying editor:", e);
        }
        currentEditor = null;
    }
    if (currentYjsBinding) {
        currentYjsBinding.destroy();
        currentYjsBinding = null;
    }
    if (currentEditorBinding) {
        currentEditorBinding.destroy();
        currentEditorBinding = null;
    }

    // @ts-ignore
    main_col.innerHTML = Handlebars.templates.editor_section_view({});

    const yjsBinding = YjsBinding(state.project_id, section_id);
    currentYjsBinding = yjsBinding;
    const doc = yjsBinding.getDoc();

    const editor = new EditorJS({
        holder: 'section_content_blocks_inner',
        tools: {
            header: Header,
            list: List,
            quote: Quote,
            raw: Raw,
            image: ImageTool,
        },
        data: { blocks: [] },
        onReady: () => {
            console.log('Editor.js is ready!');
        },
        onChange: (api, event) => {
            if (currentEditorBinding) {
                currentEditorBinding.onBlockEventEditorJS(api, event);
            }
        }
    });
    currentEditor = editor;

    currentEditorBinding = createEditorBinding(doc, editor);

    yjsBinding.on('update', () => {
        currentEditorBinding.initialRender();
    });
}

function createEditorBinding(doc: any, editor: EditorJS) {
    const yBlocks = doc.getArray('blocks');
    // Keep a simple UUID set so we can ignore EditorJS events that are side-effects of
    // initial render / remote renders.
    const internalStore = new Map<string, true>();
    const mutex = createMutex();
    let initialRenderDone = false;
    let initialRenderPromise: Promise<void> | null = null;
    let destroyed = false;

    // Serial queue for event processing to avoid race conditions during async save()
    let eventQueue = Promise.resolve();

    function toFiniteIndex(rawIndex: any, fallback: number): number {
        const n = typeof rawIndex === 'number' ? rawIndex : Number(rawIndex);
        if (!Number.isFinite(n)) return fallback;
        return Math.max(0, Math.floor(n));
    }

    function clampIndex(index: number, len: number): number {
        if (index < 0) return 0;
        if (index > len) return len;
        return index;
    }

    function getUuidFromEditorTarget(target: any, fallback?: string | null): string | null {
        const fromAttr = target?.holder?.getAttribute?.("data-y2-uuid") || null;
        return fromAttr || fallback || null;
    }

    function setUuidOnEditorTarget(target: any, uuid: string) {
        if (target?.holder?.setAttribute) {
            target.holder.setAttribute("data-y2-uuid", uuid);
        }
    }

    function findYjsIndexByUuid(uuid: string): number {
        const arr = yBlocks.toArray();
        return arr.findIndex((b: any) => normalizeBlockData(b).uuid === uuid);
    }

    async function renderBlockIntoEditor(editorBlock: any, index: number) {
        const normalized = normalizeBlockData(editorBlock);
        // Pass uuid as EditorJS block id, so onChange gives us stable identifiers.
        editor.blocks.insert(normalized.type, normalized.data, null, index, false, undefined, normalized.uuid);

        const blockApi = editor.blocks.getBlockByIndex(index);
        if (blockApi?.holder && normalized.uuid) {
            blockApi.holder.setAttribute("data-y2-uuid", normalized.uuid);
            internalStore.set(normalized.uuid, true);
        }
    }

    function onBlockEventEditorJS(api: any, event: any) {
        if (destroyed) return;
        if (!initialRenderDone) return;

        const events = Array.isArray(event) ? event : [event];
        
        // Push processing to the queue
        eventQueue = eventQueue.then(() => processEvents(events)).catch(err => {
            console.error("onBlockEventEditorJS: Error in event queue:", err);
        });

        async function processEvents(evs: any[]) {
            if (destroyed) return;
            const eventDataList: any[] = [];

            for (const ev of evs) {
                let skippedByMutex = false;
                mutex(() => {}, () => { skippedByMutex = true; });
                if (skippedByMutex) {
                    // console.log("onBlockEventEditorJS: Event skipped by mutex:", ev.type);
                    continue;
                }

                const index = ev.detail.index;
                const blockId = ev.detail.blockId;
                const target = ev.detail.target;

                let uuid: string | null = getUuidFromEditorTarget(target, blockId);

                let savedData: any = null;
                if (target && ev.type !== 'block-removed') {
                    try {
                        savedData = await target.save();
                    } catch (e) {
                        console.error("onBlockEventEditorJS: Failed to save block data:", e);
                    }
                }

                eventDataList.push({ type: ev.type, index, uuid, target, savedData });
            }

            if (eventDataList.length === 0) return;

            doc.transact(() => {
                mutex(() => {
                    if (destroyed) return;
                    for (const data of eventDataList) {
                        let uuid: string | null = data.uuid || (data.savedData ? (data.savedData.id || data.savedData.uuid) : null);
                        
                        if (!uuid && data.type === 'block-added') {
                            // @ts-ignore
                            uuid = crypto.randomUUID();
                            setUuidOnEditorTarget(data.target, uuid);
                        }

                        if (!uuid) {
                            console.warn("onBlockEventEditorJS: Missing UUID for event:", data.type);
                            continue;
                        }

                        switch (data.type) {
                            case "block-added":
                                if (internalStore.has(uuid)) {
                                    break;
                                }
                                if (!data.savedData) {
                                    console.warn("onBlockEventEditorJS: block-added missing savedData:", uuid);
                                    break;
                                }

                                // Ensure EditorJS block has a uuid for future events.
                                setUuidOnEditorTarget(data.target, uuid);

                                const newBlock = {
                                    id: uuid,
                                    uuid: uuid,
                                    type: data.savedData.type,
                                    data: data.savedData.data,
                                };

                                const insertIndex = clampIndex(
                                    toFiniteIndex(data.index, yBlocks.length),
                                    yBlocks.length
                                );
                                yBlocks.insert(insertIndex, [newBlock]);
                                internalStore.set(uuid, true);
                                break;

                            case "block-removed":
                                const removeIndex = findYjsIndexByUuid(uuid);
                                if (removeIndex !== -1) {
                                    yBlocks.delete(removeIndex);
                                    internalStore.delete(uuid);
                                } else {
                                    console.warn("onBlockEventEditorJS: block-removed could not find block in Yjs with uuid:", uuid);
                                }
                                break;

                            case "block-changed":
                                if (!data.savedData) break;
                                setUuidOnEditorTarget(data.target, uuid);

                                const editorBlock = {
                                    id: uuid,
                                    uuid: uuid,
                                    type: data.savedData.type,
                                    data: data.savedData.data,
                                };

                                const changeIndex = findYjsIndexByUuid(uuid);
                                if (changeIndex !== -1) {
                                    yBlocks.delete(changeIndex);
                                    yBlocks.insert(changeIndex, [editorBlock]);
                                    internalStore.set(uuid, true);
                                } else {
                                    // Fallback: insert at event index if we can't find it.
                                    const insertIndex = clampIndex(
                                        toFiniteIndex(data.index, yBlocks.length),
                                        yBlocks.length
                                    );
                                    yBlocks.insert(insertIndex, [editorBlock]);
                                    internalStore.set(uuid, true);
                                }
                                break;
                        }
                    }
                });
            }, 'local');
        }
    }

    function initialRender() {
        if (destroyed) return;
        if (initialRenderDone) return;
        if (initialRenderPromise) return;

        initialRenderPromise = (async () => {
            await editor.isReady;
            if (destroyed) return;
            if (initialRenderDone) return;

            await new Promise<void>((resolve, reject) => {
                mutex(() => {
                    if (destroyed) {
                        reject(new Error('initialRender aborted: binding destroyed'));
                        return;
                    }
                    const blocksToRender = yBlocks.toArray().map((b: any) => {
                        const n = normalizeBlockData(b);
                        return { id: n.uuid, type: n.type, data: n.data };
                    });

                    internalStore.clear();

                    // Let EditorJS manage clearing internally. Calling `editor.blocks.clear()` here
                    // can race with internal `render()->clear()` and triggers "Can't find a Block to remove".
                    editor.render({ blocks: blocksToRender } as any)
                        .then(() => {
                            mutex(() => {
                                if (destroyed) {
                                    resolve();
                                    return;
                                }
                                for (let i = 0; i < editor.blocks.getBlocksCount(); i++) {
                                    const blockApi = editor.blocks.getBlockByIndex(i);
                                    const uuid = blocksToRender[i]?.id || blockApi?.id;
                                    if (blockApi?.holder && uuid) {
                                        blockApi.holder.setAttribute("data-y2-uuid", uuid);
                                        internalStore.set(uuid, true);
                                    }
                                }
                                initialRenderDone = true;
                                startObserver();
                            });
                            resolve();
                        })
                        .catch((e: any) => {
                            console.error("initialRender: editor.render failed", e);
                            reject(e);
                        });
                });
            });
        })().finally(() => {
            initialRenderPromise = null;
        });
    }

    let yObserver: any = null;

    function startObserver() {
        if (yObserver) return;
        
        yObserver = (eventArray: any[], transaction: any) => {
            if (destroyed) return;
            if (transaction.origin !== 'server') return;
            if (!initialRenderDone) return;
            if (initialRenderPromise) return;
            
            mutex(() => {
                if (destroyed) return;
                for (const event of eventArray) {
                    let index = 0;
                    for (const delta of event.changes.delta) {
                        if (delta.retain) {
                            index += delta.retain;
                        } else if (delta.insert) {
                            for (const block of delta.insert) {
                                try {
                                    renderBlockIntoEditor(block, index);
                                } catch (e) {
                                    console.error("Error inserting remote block at index", index, e);
                                }
                                index++;
                            }
                        } else if (delta.delete) {
                            for (let i = 0; i < delta.delete; i++) {
                                try {
                                    const blockApi = editor.blocks.getBlockByIndex(index);
                                    if (blockApi) {
                                        const uuid = blockApi.holder.getAttribute("data-y2-uuid") || blockApi.id;
                                        if (uuid) internalStore.delete(uuid);
                                        editor.blocks.delete(index);
                                    } else {
                                        console.warn("startObserver: Attempted to delete block at index", index, "but it was not found in EditorJS. State might be out of sync.");
                                    }
                                } catch (e) {
                                    console.error("Error deleting block from remote update at index", index, e);
                                }
                            }
                        }
                    }
                }
            });
        };
        yBlocks.observeDeep(yObserver);
    }

    return {
        onBlockEventEditorJS,
        initialRender,
        destroy: () => {
            destroyed = true;
            if (yObserver) {
                yBlocks.unobserveDeep(yObserver);
            }
        }
    };
}

function normalizeBlockData(block: any) {
    const blockData = typeof block.toJSON === 'function' ? block.toJSON() : block;

    // Normalize properties
    let type = blockData.type || blockData.block_type || blockData.blockType || blockData.kind || blockData.tool;
    let data = blockData.data;
    let uuid = blockData.uuid || blockData.id;

    // 1. Try to infer type from data if it's a tagged union (e.g., { Paragraph: { ... } })
    if (data && typeof data === 'object' && !type) {
        const keys = Object.keys(data);
        if (keys.length === 1) {
            const key = keys[0];
            const possibleTypes = ["Paragraph", "Heading", "Header", "List", "Quote", "Image", "Raw", "Table"];
            if (possibleTypes.includes(key)) {
                type = key.toLowerCase();
                data = data[key];
            }
        }
    }

    // 2. Try to infer type from blockData keys if missing (tagged union at top level)
    if (!type) {
        const possibleTypes = ["paragraph", "header", "heading", "list", "quote", "image", "raw", "table"];
        for (const pt of possibleTypes) {
            const capitalized = pt.charAt(0).toUpperCase() + pt.slice(1);
            if (blockData[pt] || (blockData[capitalized] && typeof blockData[capitalized] === 'object')) {
                type = pt;
                if (!data) data = blockData[pt] || blockData[capitalized];
                break;
            }
        }
    }

    // 3. Fallback: if data contains text/level/items etc directly, it might be a flat structure
    if (!type && data && typeof data === 'object') {
        if (data.text !== undefined && data.level !== undefined) type = "header";
        else if (data.text !== undefined) type = "paragraph";
        else if (data.items !== undefined) type = "list";
        else if (data.html !== undefined) type = "raw";
        else if (data.file !== undefined) type = "image";
    }

    // 4. Normalize specific type names
    if (typeof type === 'string') {
        type = type.toLowerCase();
        if (type === "heading") type = "header";
    }

    if (!type) {
        console.warn("normalizeBlockData: Failed to find type for block, falling back to 'paragraph'. Raw data:", blockData);
        type = "paragraph";
    }

    return {
        type,
        data: data || {},
        id: uuid,
        uuid: uuid
    };
}
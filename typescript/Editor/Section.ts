import {init, main_col} from "./Editor";
import {YjsBinding} from "./YjsBinding";
import {state} from "./Main";
import EditorJS from "@editorjs/editorjs";
import * as Y from 'yjs';
import DiffMatchPatch from 'diff-match-patch';
// @ts-ignore
import Header from "@editorjs/header";
// @ts-ignore
import List from "@editorjs/list";
// @ts-ignore
import Quote from "@editorjs/quote";
// @ts-ignoreO
import Raw from "@editorjs/raw";
// @ts-ignore
import ImageTool from "@editorjs/image";
// @ts-ignore
import Strikethrough from "@sotaproject/strikethrough";
import {NoteTool} from "./Tools/NoteTool";
import {CitationTool} from "./Tools/CitationTool";
import {CustomStyleTool} from "./Tools/CustomStyleTool";
import {BlockStyleTune} from "./Tools/BlockStyleTune";
import {createMutex} from "./Mutex";
import {FixedLinkTool} from "./Tools/FixedLinkTool";
import {PersonsAPI, PersonUuidOrString, SectionAPI} from "../api_requests";
import {add_search, show_alert} from "../tools";

const personsApi = new (PersonsAPI as any)();
const sectionApi = new (SectionAPI as any)();

let currentEditor: EditorJS | null = null;
let currentYjsBinding: any = null;
let currentEditorBinding: any = null;

/**
 * Mounts the section editor view and binds EditorJS to the section's Yjs document.
 *
 * This function is responsible for:
 * - Cleaning up any previous editor/binding instances.
 * - Rendering the section editor template.
 * - Creating the `YjsBinding` websocket-backed doc.
 * - Creating the EditorJS instance.
 * - Wiring EditorJS `onChange` into the binding.
 *
 * @param content_path Colon-separated path from top-level section to leaf (ids joined by ':').
 */
export async function showSectionEditor(content_path: string){
    console.log("Loading section editor for section path: " + content_path);
    const pathParts = (content_path || '').split(':').filter(Boolean);
    const section_id = pathParts[pathParts.length - 1]; // leaf id for Yjs binding
    state.active_section_id = section_id;

    if (!section_id) {
        console.error('showSectionEditor: No leaf section_id could be derived from content_path:', content_path);
        return;
    }

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

    // Fetch section metadata (expand authors/editors for UI) using full content_path
    let sectionData: any = null;
    try{
        const resp = await fetch(`/api/projects/${state.project_id}/sections/${content_path}?expand=authors,editors`, {
            credentials: 'include'
        });
        if(!resp.ok){
            console.error('Failed to fetch section metadata', resp.status, await resp.text());
        }else{
            sectionData = await resp.json();
        }
    }catch(e){
        console.error('Error fetching section metadata', e);
    }

    // Render template with metadata if available
    // @ts-ignore
    main_col.innerHTML = Handlebars.templates.editor_section_view({data: sectionData?.data || sectionData || {}});

    // Update active section in sidebar
    const sidebarSections = document.querySelectorAll('.sidebar-contents-section');
    sidebarSections.forEach(section => {
        const body = section.querySelector('.sidebar-contents-section-body');
        if (body) {
            if (section.getAttribute('data-section-id') === section_id) {
                body.classList.add('active');
            } else {
                body.classList.remove('active');
            }
        }
    });

    // Wire up metadata show/hide and change handlers
    const actualData = sectionData?.data || sectionData;
    if (actualData && actualData.metadata) {
        try {
            setupSectionMetadataUI(content_path, actualData);
        } catch(e) {
            console.warn('setupSectionMetadataUI failed', e);
        }
    }

    // Use the full content_path as the Yjs document id to match the backend route used for metadata
    console.log('Initializing YjsBinding for documentId (content_path):', content_path);
    const yjsBinding = YjsBinding(state.project_id, section_id);
    currentYjsBinding = yjsBinding;
    const doc = yjsBinding.getDoc();

    const editor = new EditorJS({
        holder: 'section_content_blocks_inner',
        tools: {
            // Enable inline toolbar for headers so formatting (z.B. Link, Bold, Italic) erscheint
            header: {
                class: Header,
                inlineToolbar: true
            },
            // Enable inline toolbar for lists and quotes as well
            list: {
                class: List,
                inlineToolbar: true
            } as any,
            quote: {
                class: Quote,
                inlineToolbar: true
            } as any,
            raw: Raw,
            image: {
                class: ImageTool,
                config: {
                    endpoints: {
                        byFile: '/api/projects/'+state.project_id+'/uploads',
                        byUrl: '/api/fetch_image',
                    }
                }
            },
            strikethrough: Strikethrough,
            /** Override EditorJS built-in inline `link` tool with our custom implementation */
            link: FixedLinkTool,
            note: NoteTool,
            citation: CitationTool,
            custom_style_tool: CustomStyleTool,
            block_style_tune: BlockStyleTune
        },
        tunes: ['block_style_tune'],
        data: { blocks: [] },
        onReady: () => {
            console.log('Editor.js is ready!');
            NoteTool.add_all_show_note_settings_listeners();
            CitationTool.add_all_show_note_settings_listeners();
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
        console.log('Yjs update received, triggering initialRender');
        currentEditorBinding.initialRender();
    });
}

/**
 * Creates a two-way binding between a Yjs `blocks` array and an EditorJS instance.
 *
 * The binding:
 * - Applies local EditorJS edits into Yjs (`origin = 'local'`).
 * - Applies remote Yjs updates from the server into EditorJS without echoing them back.
 * - Stores a stable Yjs uuid per EditorJS block using the `data-y2-uuid` holder attribute.
 *
 * @param doc Yjs document.
 * @param editor EditorJS instance.
 */
function createEditorBinding(doc: any, editor: EditorJS) {
    /** Enable verbose debug logging for the binding. Keep `false` by default. */
    const DEBUG_BINDING = false;
    const yBlocks = doc.getArray('blocks');
    // Keep a simple UUID set so we can ignore EditorJS events that are side-effects of
    // initial render / remote renders.
    const internalStore = new Map<string, true>();
    const mutex = createMutex();
    let initialRenderDone = false;
    let initialRenderPromise: Promise<void> | null = null;
    let destroyed = false;

    // Some EditorJS operations (notably `blocks.update`) can trigger `onChange` asynchronously,
    // after we already released our mutex. Use an explicit suppression counter to prevent
    // remote-applied changes from being echoed back into Yjs.
    let suppressEditorEvents = 0;

    // Additional guard: even with suppression, EditorJS may emit `block-changed` after a remote
    // `blocks.update`/`blocks.insert` due to internal normalization. Track remote-touched block ids
    // for a short time window and ignore corresponding EditorJS events.
    const remoteTouchedUntil = new Map<string, number>();
    const REMOTE_TOUCH_TTL_MS = 2500;

    // EditorJS manages its own internal block ids. Sometimes during updates (notably when changing
    // header level) EditorJS may re-create block DOM and temporarily lose our `data-y2-uuid` marker.
    // Keep a secondary mapping so we can still resolve our uuid and suppress echo writes.
    const editorIdToUuid = new Map<string, string>();

    /**
     * Marks a Yjs block uuid as recently modified by a remote update.
     *
     * We use this to suppress EditorJS `onChange` events that are side-effects of applying
     * remote updates (prevents ping-pong loops).
     */
    function markRemoteTouched(uuid: string | null | undefined) {
        if (!uuid) return;
        remoteTouchedUntil.set(uuid, Date.now() + REMOTE_TOUCH_TTL_MS);
    }

    /**
     * Returns `true` if the given uuid was modified by a remote update very recently.
     */
    function wasRecentlyRemoteTouched(uuid: string | null | undefined): boolean {
        if (!uuid) return false;
        const until = remoteTouchedUntil.get(uuid);
        if (!until) return false;
        if (Date.now() > until) {
            remoteTouchedUntil.delete(uuid);
            return false;
        }
        return true;
    }

    /**
     * Executes `fn` while suppressing EditorJS events for a short time window.
     *
     * EditorJS can emit `onChange` asynchronously after we apply an update.
     */
    function withSuppressedEditorEvents(fn: () => void) {
        suppressEditorEvents++;
        try {
            fn();
        } finally {
            // Defer unsuppression to cover async `onChange` emissions.
            setTimeout(() => {
                suppressEditorEvents = Math.max(0, suppressEditorEvents - 1);
            }, 2000);
        }
    }

    // Serial queue for event processing to avoid race conditions during async save()
    let eventQueue = Promise.resolve();

    const dmp = new DiffMatchPatch();

    /**
     * Applies a diff between the current `ytext` content and `next` using `diff-match-patch`.
     *
     * This preserves Yjs text structure and avoids replacing whole blocks for small edits.
     */
    function applyStringDiffToYText(ytext: Y.Text, next: string): boolean {
        const prev = ytext.toString();
        if (prev === next) return true;

        const diffs = dmp.diff_main(prev, next, false);
        dmp.diff_cleanupEfficiency(diffs);

        let index = 0;
        for (const [op, text] of diffs) {
            if (op === DiffMatchPatch.DIFF_EQUAL) {
                index += text.length;
            } else if (op === DiffMatchPatch.DIFF_DELETE) {
                if (text.length > 0) {
                    ytext.delete(index, text.length);
                }
            } else if (op === DiffMatchPatch.DIFF_INSERT) {
                if (text.length > 0) {
                    ytext.insert(index, text);
                    index += text.length;
                }
            }
        }

        return true;
    }

    /**
     * Builds a Yjs block representation from EditorJS `target.save()` output.
     *
     * Mirrors the backend yrs schema (see Rust `NewContentBlock -> MapPrelim`):
     * `{ id, type, data: {...}, tunes?: {...} }`, storing text fields as `Y.Text`.
     */
    function yBlockFromEditorSaved(uuid: string, toolName: string | null, savedData: any): any {
        // Mirror the yrs schema produced by `NewContentBlock -> MapPrelim` on the backend:
        // { id, type, data: { text|level|items|html|... }, tunes?: { block_style_tunes: { css_classes } } }
        const type = toolName || savedData?.type;
        const data = savedData?.data || {};
        const tunes = savedData?.tunes;

        const yBlock = new Y.Map();
        yBlock.set('id', uuid);
        if (type) {
            yBlock.set('type', type);
        }

        const yData = new Y.Map();

        if (type === 'paragraph' && typeof data.text === 'string') {
            const yText = new Y.Text();
            yText.insert(0, data.text);
            yData.set('text', yText);
        } else if (type === 'header') {
            if (typeof data.text === 'string') {
                const yText = new Y.Text();
                yText.insert(0, data.text);
                yData.set('text', yText);
            }
            if (data.level !== undefined) yData.set('level', data.level);
        } else if (type === 'raw' && typeof data.html === 'string') {
            const yText = new Y.Text();
            yText.insert(0, data.html);
            yData.set('html', yText);
        } else if (type === 'quote') {
            if (typeof data.text === 'string') {
                const yText = new Y.Text();
                yText.insert(0, data.text);
                yData.set('text', yText);
            }
            if (typeof data.caption === 'string') {
                const yCaption = new Y.Text();
                yCaption.insert(0, data.caption);
                yData.set('caption', yCaption);
            }
            if (data.alignment !== undefined) yData.set('alignment', data.alignment);
        } else if (type === 'list') {
            if (data.style !== undefined) yData.set('style', data.style);
            const yItems = new Y.Array();
            if (Array.isArray(data.items)) {
                for (const it of data.items) {
                    const yItem = new Y.Text();
                    yItem.insert(0, String(it ?? ''));
                    yItems.push([yItem]);
                }
            }
            yData.set('items', yItems);
        } else if (type === 'image') {
            if (data.file && typeof data.file === 'object') {
                const yFile = new Y.Map();
                if (data.file.url !== undefined) yFile.set('url', data.file.url);
                if (data.file.filename !== undefined) yFile.set('filename', data.file.filename);
                yData.set('file', yFile);
            }
            if (typeof data.caption === 'string') {
                const yCaption = new Y.Text();
                yCaption.insert(0, data.caption);
                yData.set('caption', yCaption);
            }
            if (data.withBorder !== undefined) yData.set('withBorder', data.withBorder);
            if (data.withBackground !== undefined) yData.set('withBackground', data.withBackground);
            if (data.stretched !== undefined) yData.set('stretched', data.stretched);
        } else {
            console.warn('[SectionBinding] Unsupported block type', type, data);
            // Unknown tool or unexpected payload: keep a plain JSON snapshot for now.
            yBlock.set('data', data);
        }

        if (!yBlock.has('data')) {
            yBlock.set('data', yData);
        }

        // Preserve tunes if present (esp. block_style_tunes css_classes)
        if (tunes && typeof tunes === 'object') {
            yBlock.set('tunes', tunes);
        }

        return yBlock;
    }

    /**
     * Attempts to apply an EditorJS change to an existing Yjs block without replacing the block.
     *
     * For supported block types, this updates the relevant `Y.Text` fields using minimal diffs
     * and updates simple scalar fields (e.g. header `level`).
     *
     * @returns `true` if applied in-place; `false` if the caller should fall back to a full block replacement.
     */
    function tryApplyInPlaceTextUpdate(existing: any, toolName: string | null, savedData: any): boolean {
        if (!existing || typeof existing !== 'object') return false;
        // We only support Y.Map-based blocks for in-place updates.
        if (!(existing instanceof Y.Map)) return false;

        const nextType = toolName || savedData?.type;
        const nextData = savedData?.data || {};
        const nextTunes = savedData?.tunes;

        const currentType = existing.get('type');
        if (currentType && nextType && currentType !== nextType) return false;
        if (!currentType && nextType) {
            // If an earlier write inserted a block without type, fix it in-place.
            existing.set('type', nextType);
        }

        // Sync tunes if they changed
        if (nextTunes !== undefined) {
            // Use a simple JSON comparison for tunes as they are typically small
            const currentTunes = existing.get('tunes');
            if (JSON.stringify(currentTunes) !== JSON.stringify(nextTunes)) {
                existing.set('tunes', nextTunes);
            }
        } else if (existing.has('tunes')) {
            existing.delete('tunes');
        }

        const yData = existing.get('data');
        if (!(yData instanceof Y.Map)) return false;

        // Paragraph/header/quote/raw store their text fields as Y.Text on the backend.
        if (nextType === 'paragraph') {
            const yText = yData.get('text');
            if (yText instanceof Y.Text && typeof nextData.text === 'string') {
                return applyStringDiffToYText(yText, nextData.text);
            }
            return false;
        }
        if (nextType === 'header') {
            const yText = yData.get('text');
            if (yText instanceof Y.Text && typeof nextData.text === 'string') {
                applyStringDiffToYText(yText, nextData.text);
            } else if (typeof nextData.text === 'string') {
                return false;
            }
            if (nextData.level !== undefined) {
                yData.set('level', nextData.level);
            }
            return true;
        }
        if (nextType === 'raw') {
            const yHtml = yData.get('html');
            if (yHtml instanceof Y.Text && typeof nextData.html === 'string') {
                return applyStringDiffToYText(yHtml, nextData.html);
            }
            return false;
        }
        if (nextType === 'quote') {
            const yText = yData.get('text');
            const yCaption = yData.get('caption');
            if (yText instanceof Y.Text && typeof nextData.text === 'string') {
                applyStringDiffToYText(yText, nextData.text);
            } else if (typeof nextData.text === 'string') {
                return false;
            }
            if (yCaption instanceof Y.Text && typeof nextData.caption === 'string') {
                applyStringDiffToYText(yCaption, nextData.caption);
            }
            if (nextData.alignment !== undefined) yData.set('alignment', nextData.alignment);
            return true;
        }
        // List and image changes can be more structural; keep fallback path for now.
        return false;
    }

    /**
     * Resolves the EditorJS tool name for an onChange event.
     *
     * `target.save()` typically returns only the tool data and may omit the type.
     * We prefer:
     * - `target.name` (Block API),
     * - then `editor.blocks.getBlockByIndex(index)?.name`,
     * - then `savedData.type`.
     */
    function getToolNameForEvent(index: any, target: any, savedData: any): string | null {
        // EditorJS Block API has `name` (tool key) and `save()` returns only tool data.
        // Prefer block api / target name over savedData fields.
        const fromTarget = target?.name;
        if (typeof fromTarget === 'string' && fromTarget.length > 0) return fromTarget;

        const idx = toFiniteIndex(index, -1);
        if (idx >= 0) {
            const blockApi: any = editor.blocks.getBlockByIndex(idx);
            const fromBlockApi = blockApi?.name;
            if (typeof fromBlockApi === 'string' && fromBlockApi.length > 0) return fromBlockApi;
        }

        const fromSaved = savedData?.type;
        if (typeof fromSaved === 'string' && fromSaved.length > 0) return fromSaved;
        return null;
    }

    /**
     * Normalizes an EditorJS event index to a non-negative integer.
     */
    function toFiniteIndex(rawIndex: any, fallback: number): number {
        const n = typeof rawIndex === 'number' ? rawIndex : Number(rawIndex);
        if (!Number.isFinite(n)) return fallback;
        return Math.max(0, Math.floor(n));
    }

    /**
     * Clamps an index into the `[0..len]` range.
     */
    function clampIndex(index: number, len: number): number {
        if (index < 0) return 0;
        if (index > len) return len;
        return index;
    }

    /**
     * Resolves the binding uuid for an EditorJS block event.
     *
     * Primary source: `target.holder[data-y2-uuid]`.
     * Fallback: EditorJS internal id (`blockId`) which we map to a uuid via `editorIdToUuid`.
     */
    function getUuidFromEditorTarget(target: any, fallback?: string | null): string | null {
        const fromAttr = target?.holder?.getAttribute?.("data-y2-uuid") || null;
        if (fromAttr) return fromAttr;

        // If holder marker is missing, try resolving via EditorJS internal block id.
        const fb = fallback || null;
        if (fb && editorIdToUuid.has(fb)) {
            return editorIdToUuid.get(fb) || fb;
        }
        return fb;
    }

    /**
     * Attaches the binding uuid to an EditorJS block (DOM holder attribute) and caches it.
     */
    function setUuidOnEditorTarget(target: any, uuid: string) {
        if (target?.holder?.setAttribute) {
            target.holder.setAttribute("data-y2-uuid", uuid);
        }
        if (target?.id && typeof target.id === 'string') {
            editorIdToUuid.set(target.id, uuid);
        }
    }

    /**
     * Finds the index of a Yjs block by its binding uuid.
     */
    function findYjsIndexByUuid(uuid: string): number {
        const arr = yBlocks.toArray();
        return arr.findIndex((b: any) => normalizeBlockData(b).uuid === uuid);
    }

    /**
     * Inserts a single Yjs block into EditorJS at `index` and attaches the binding uuid.
     */
    async function renderBlockIntoEditor(editorBlock: any, index: number) {
        const normalized = normalizeBlockData(editorBlock);

        // IMPORTANT: Do NOT try to force EditorJS block ids to match our Yjs uuids.
        // EditorJS manages its own internal ids; attempting to override them can break
        // `blocks.update()` ("Incorrect index"). We store our uuid on the holder attribute.
        editor.blocks.insert(normalized.type, normalized.data, null, index, false);

        const blockApi: any = editor.blocks.getBlockByIndex(index);
        if (blockApi?.holder && normalized.uuid) {
            blockApi.holder.setAttribute("data-y2-uuid", normalized.uuid);
            internalStore.set(normalized.uuid, true);
            if (typeof blockApi.id === 'string') {
                editorIdToUuid.set(blockApi.id, normalized.uuid);
            }
        }
    }

    /**
     * Finds an EditorJS block API object by its internal id.
     */
    function findEditorBlockApiById(editorId: string): any | null {
        for (let i = 0; i < editor.blocks.getBlocksCount(); i++) {
            const blockApi: any = editor.blocks.getBlockByIndex(i);
            if (blockApi?.id === editorId) return blockApi;
        }
        return null;
    }

    /**
     * Finds an EditorJS block API object by our binding uuid (`data-y2-uuid`).
     */
    function findEditorBlockApiByUuid(uuid: string): any | null {
        for (let i = 0; i < editor.blocks.getBlocksCount(); i++) {
            const blockApi: any = editor.blocks.getBlockByIndex(i);
            const attr = blockApi?.holder?.getAttribute?.('data-y2-uuid');
            if (attr === uuid) return blockApi;
        }
        return null;
    }

    /**
     * Full safety-net rerender from the current Yjs document state.
     *
     * We try hard to apply remote updates in-place. This is only used if EditorJS
     * gets out of sync (e.g. throws/rejects during `blocks.update`).
     */
    async function rerenderAllFromYjs(reason: string) {
        try {
            // This is a safety-net fallback when we can't reliably apply an in-place update
            // (e.g. EditorJS throws "Incorrect index"). It should be rare; we log it.
            console.warn('[SectionBinding] Fallback to full editor.render()', {
                reason,
                stack: new Error().stack,
            });

            if (DEBUG_BINDING) {
                console.warn('[SectionBinding] rerenderAllFromYjs (debug)', reason);
            }

            const yArr = yBlocks.toArray();
            const blocksToRender = yArr.map((b: any) => {
                const n = normalizeBlockData(b);
                return { type: n.type, data: n.data };
            });

            internalStore.clear();
            await editor.render({ blocks: blocksToRender } as any);

            // Re-attach uuids in order.
            for (let i = 0; i < editor.blocks.getBlocksCount(); i++) {
                const blockApi: any = editor.blocks.getBlockByIndex(i);
                const uuid = normalizeBlockData(yArr[i]).uuid;
                if (blockApi?.holder && uuid) {
                    blockApi.holder.setAttribute('data-y2-uuid', uuid);
                    internalStore.set(uuid, true);
                }
            }
            NoteTool.add_all_show_note_settings_listeners();
            CitationTool.add_all_show_note_settings_listeners();
        } catch (e) {
            console.error('[SectionBinding] rerenderAllFromYjs failed', e);
        }
    }

    let rerenderTimer: any = null;

    /**
     * Debounces full rerenders to avoid hammering EditorJS if multiple failures happen at once.
     */
    function scheduleRerender(reason: string) {
        if (rerenderTimer) return;
        rerenderTimer = setTimeout(() => {
            rerenderTimer = null;
            void rerenderAllFromYjs(reason);
        }, 50);
    }

    // Debounced, per-block deep update application to avoid hammering EditorJS during rapid typing.
    // IMPORTANT: don't capture Yjs/Editor state too early; recompute inside the timer, otherwise
    // EditorJS may have re-indexed blocks and `blocks.update` may throw "Incorrect index".
    const deepUpdateTimers = new Map<string, any>();

    /**
     * Schedules an in-place update of a single EditorJS block from the latest Yjs state.
     *
     * Used for deep Yjs events (e.g. `Y.Text` changes inside `data`).
     */
    function scheduleDeepBlockUpdate(index: number) {
        const yBlockNow = yBlocks.get(index);
        const uuid = normalizeBlockData(yBlockNow).uuid;
        if (!uuid) {
            scheduleRerender('deep change (missing uuid)');
            return;
        }

        const prev = deepUpdateTimers.get(uuid);
        if (prev) clearTimeout(prev);

        deepUpdateTimers.set(uuid, setTimeout(() => {
            deepUpdateTimers.delete(uuid);

            // Re-read latest Yjs value.
            const yBlock = yBlocks.get(index);
            const normalized = normalizeBlockData(yBlock);

            // Resolve EditorJS internal id by our uuid mapping (prefer uuid, fallback to index).
            const blockApi: any = findEditorBlockApiByUuid(uuid) || editor.blocks.getBlockByIndex(index);
            const editorId = blockApi?.id;
            if (!editorId) {
                scheduleRerender('deep change (missing editor block)');
                return;
            }

            try {
                const maybePromise = (editor as any).blocks.update(editorId, normalized.data, normalized.tunes);
                if (maybePromise && typeof maybePromise.catch === 'function') {
                    maybePromise.catch((e: any) => {
                        console.error('startObserver: editor.blocks.update failed', e);
                        scheduleRerender('blocks.update failed');
                    });
                }

                // Best-effort: keep uuid marker + mapping attached even if EditorJS re-created holder.
                setTimeout(() => {
                    try {
                        const byId = findEditorBlockApiById(editorId);
                        if (byId?.holder && uuid) {
                            byId.holder.setAttribute('data-y2-uuid', uuid);
                            editorIdToUuid.set(editorId, uuid);
                        }
                    } catch {
                        // ignore
                    }
                    NoteTool.add_all_show_note_settings_listeners();
                    CitationTool.add_all_show_note_settings_listeners();
                }, 0);
            } catch (e) {
                console.error('startObserver: editor.blocks.update threw', e);
                scheduleRerender('blocks.update threw');
            }
        }, 0));
    }

    /**
     * Handles EditorJS `onChange` events and applies them into Yjs.
     *
     * This method is intentionally conservative to prevent feedback loops:
     * - ignores events during initial render
     * - ignores events while suppressed
     * - ignores events for uuids recently touched by remote updates
     */
    function onBlockEventEditorJS(api: any, event: any) {
        if (destroyed) return;
        if (!initialRenderDone) return;
        if (suppressEditorEvents > 0) return;

        const events = Array.isArray(event) ? event : [event];
        
        // Push processing to the queue
        eventQueue = eventQueue.then(() => processEvents(events)).catch(err => {
            console.error("onBlockEventEditorJS: Error in event queue:", err);
        });

        /**
         * Serially processes EditorJS events, snapshots `target.save()` results,
         * and applies the aggregated changes to Yjs in a single transaction.
         */
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
                const fromIndex = ev.detail.fromIndex;
                const toIndex = ev.detail.toIndex;
                const blockId = ev.detail.blockId || ev.detail.target?.id;
                const target = ev.detail.target;

                let uuid: string | null = getUuidFromEditorTarget(target, blockId);

                // If this block was just updated/inserted due to a remote Yjs update, ignore the
                // echo event so we don't write the same change back into Yjs.
                if (wasRecentlyRemoteTouched(uuid)) {
                    continue;
                }

                let savedData: any = null;
                if (target && ev.type !== 'block-removed') {
                    try {
                        savedData = await target.save();
                    } catch (e) {
                        console.error("onBlockEventEditorJS: Failed to save block data:", e);
                    }
                }

                const toolName = getToolNameForEvent(index, target, savedData);
                eventDataList.push({ type: ev.type, index, fromIndex, toIndex, uuid, target, toolName, savedData });
            }

            if (eventDataList.length === 0) return;

            if (DEBUG_BINDING) {
                console.log('[SectionBinding] applying EditorJS events to Yjs', eventDataList.map(e => ({
                    type: e.type,
                    index: e.index,
                    uuid: e.uuid,
                    toolName: e.toolName,
                })));
                console.log('[SectionBinding] stack', new Error().stack);
            }

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

                                const newBlock = yBlockFromEditorSaved(uuid, data.toolName || null, data.savedData);

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

                                const changeIndex = findYjsIndexByUuid(uuid);
                                if (changeIndex !== -1) {
                                    const existing = yBlocks.get(changeIndex);
                                    const inPlaceOk = tryApplyInPlaceTextUpdate(existing, data.toolName || null, data.savedData);
                                    if (inPlaceOk) {
                                        internalStore.set(uuid, true);
                                        break;
                                    }

                                    // Fallback: replace whole block (e.g. structural changes or legacy JSON blocks)
                                    const replacement = yBlockFromEditorSaved(uuid, data.toolName || null, data.savedData);
                                    yBlocks.delete(changeIndex);
                                    yBlocks.insert(changeIndex, [replacement]);
                                    internalStore.set(uuid, true);
                                } else {
                                    // Fallback: insert at event index if we can't find it.
                                    const insertIndex = clampIndex(
                                        toFiniteIndex(data.index, yBlocks.length),
                                        yBlocks.length
                                    );
                                    const replacement = yBlockFromEditorSaved(uuid, data.toolName || null, data.savedData);
                                    yBlocks.insert(insertIndex, [replacement]);
                                    internalStore.set(uuid, true);
                                }
                                break;
                            case "block-moved":
                                // block-moved detail contains 'fromIndex' and 'toIndex'.
                                const moveFrom = findYjsIndexByUuid(uuid);
                                const moveTo = data.toIndex !== undefined ? toFiniteIndex(data.toIndex, yBlocks.length) : toFiniteIndex(data.index, yBlocks.length);

                                if (moveFrom !== -1 && moveFrom !== moveTo) {
                                    const blockToMove = yBlocks.get(moveFrom).clone();
                                    
                                    // Use a single transaction for the move to ensure consistency
                                    yBlocks.delete(moveFrom);
                                    
                                    // If we moved a block from a lower index to a higher index, 
                                    // deleting it from its original position (moveFrom) shifts all 
                                    // subsequent blocks down by 1 in the Y.Array.
                                    // EditorJS's 'toIndex' (moveTo) represents the desired final position.
                                    const finalMoveTo = clampIndex(moveTo, yBlocks.length);
                                    console.log(`[SectionBinding] block-moved: uuid=${uuid}, moveFrom=${moveFrom}, moveTo=${moveTo}, finalMoveTo=${finalMoveTo}, yLen=${yBlocks.length}`);

                                    yBlocks.insert(finalMoveTo, [blockToMove]);
                                    internalStore.set(uuid, true);
                                }
                                break;
                        }
                    }
                });
            }, 'local');
        }
    }

    /**
     * Performs the initial EditorJS render from the current Yjs `blocks` array.
     *
     * This is invoked after we have applied at least one server update.
     */
    function initialRender() {
        if (destroyed) return;
        if (initialRenderDone) return;
        if (initialRenderPromise) return;

        initialRenderPromise = (async () => {
            await editor.isReady;
            if (destroyed) return;
            if (initialRenderDone) return;

            const blocksToRenderRaw = yBlocks.toArray();
            if (blocksToRenderRaw.length === 0) {
                console.log('initialRender: yBlocks is empty, postponing until content arrives');
                return;
            }

            await new Promise<void>((resolve, reject) => {
                mutex(() => {
                    if (destroyed) {
                        reject(new Error('initialRender aborted: binding destroyed'));
                        return;
                    }
                    const blocksToRender = blocksToRenderRaw.map((b: any) => {
                        const n = normalizeBlockData(b);
                        // Do not pass `id` to EditorJS; we track our uuid via holder attribute.
                        return { type: n.type, data: n.data, tunes: n.tunes };
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
                                const yArr = yBlocks.toArray();
                                editorIdToUuid.clear();
                                for (let i = 0; i < editor.blocks.getBlocksCount(); i++) {
                                    const blockApi: any = editor.blocks.getBlockByIndex(i);
                                    const uuid = normalizeBlockData(yArr[i]).uuid;
                                    if (blockApi?.holder && uuid) {
                                        blockApi.holder.setAttribute("data-y2-uuid", uuid);
                                        internalStore.set(uuid, true);
                                        if (typeof blockApi.id === 'string') {
                                            editorIdToUuid.set(blockApi.id, uuid);
                                        }
                                    }
                                }
                                initialRenderDone = true;
                                NoteTool.add_all_show_note_settings_listeners();
                                CitationTool.add_all_show_note_settings_listeners();
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

    /**
     * Starts observing the Yjs `blocks` array (deep) and applies remote updates into EditorJS.
     */
    function startObserver() {
        if (yObserver) return;

        /**
         * Yjs observer callback for remote (server-origin) updates.
         *
         * We only handle `transaction.origin === 'server'` here. Local edits are applied
         * from EditorJS into Yjs in `onBlockEventEditorJS`.
         */
        function yObserverHandler(eventArray: any[], transaction: any) {
            if (destroyed) return;
            if (transaction.origin !== 'server') return;
            if (!initialRenderDone) return;
            if (initialRenderPromise) return;
            
            // Apply remote changes while suppressing EditorJS onChange echoes.
            withSuppressedEditorEvents(() => mutex(() => {
                if (destroyed) return;
                for (const event of eventArray) {
                    // IMPORTANT: observeDeep emits events from nested types as well.
                    // Many nested types (especially Y.Text) also have `changes.delta`, but those deltas
                    // are NOT array-of-blocks deltas. Only treat events targeting the top-level blocks
                    // array as structural insert/delete/reorder.
                    const isBlocksArrayEvent = event?.target === yBlocks;
                    if (!isBlocksArrayEvent) {
                        // Deep changes (e.g. Y.Text updates inside a block's data map)
                        // Try to update the affected block in-place via `blocks.update` (by EditorJS
                        // internal id resolved from our `data-y2-uuid`), with safe fallback to a full
                        // rerender if EditorJS rejects the update.
                        try {
                            const path = event?.path as any[] | undefined;
                            const topKey = Array.isArray(path) ? path[0] : null;
                            if (typeof topKey === 'number') {
                                const yBlock = yBlocks.get(topKey);
                                const normalized = normalizeBlockData(yBlock);
                                if (normalized?.uuid) {
                                    markRemoteTouched(normalized.uuid);
                                    scheduleDeepBlockUpdate(topKey);
                                }
                            }
                        } catch (e) {
                            console.error('startObserver: Failed to apply deep remote update', e);
                        }
                        continue;
                    }

                    // Array-level changes (insert/delete/reorder)
                    if (!event?.changes?.delta) {
                        continue;
                    }

                    let index = 0;
                    for (const delta of event.changes.delta) {
                        if (delta.retain) {
                            index += delta.retain;
                        } else if (delta.insert) {
                            for (const _block of delta.insert) {
                                try {
                                    // Use the integrated element from `yBlocks` instead of the delta payload.
                                    // The delta payload may contain non-integrated Yjs types, which will throw
                                    // "Invalid access: Add Yjs type to a document before reading data."
                                    const yBlock = yBlocks.get(index);
                                    const normalized = normalizeBlockData(yBlock);
                                    markRemoteTouched(normalized.uuid);
                                    renderBlockIntoEditor(yBlock, index);
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
                                        markRemoteTouched(uuid);
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
            }));
        }

        yObserver = yObserverHandler;
        yBlocks.observeDeep(yObserver);
    }

    return {
        onBlockEventEditorJS,
        initialRender,
        /**
         * Destroys the binding and stops observing Yjs.
         */
        destroy: () => {
            destroyed = true;
            if (yObserver) {
                yBlocks.unobserveDeep(yObserver);
            }
        }
    };
}

// ===== Section Metadata UI logic =====

function debounce<F extends (...args: any[]) => void>(fn: F, delay = 400) {
    let t: any;
    return (...args: any[]) => {
        clearTimeout(t);
        t = setTimeout(() => fn(...args), delay);
    };
}

async function patchSectionMeta(content_path: string, metadataPatch: any) {
    try {
        const resp = await fetch(`/api/projects/${state.project_id}/sections/${content_path}`, {
            method: 'PATCH',
            credentials: 'include',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ metadata: metadataPatch })
        });
        if (!resp.ok) {
            console.error('PATCH section metadata failed', resp.status, await resp.text());
        }
    } catch (e) {
        console.error('PATCH section metadata error', e);
    }
}

function setupSectionMetadataUI(content_path: string, sectionData: any) {
    if (!sectionData || !sectionData.metadata) {
        return;
    }
    const collapsed = document.getElementsByClassName('editor_section_view_collapsed_metadata')[0] as HTMLElement | undefined;
    const panel = document.getElementsByClassName('editor_section_view_metadata')[0] as HTMLElement | undefined;
    const showBtn = document.getElementById('section_show_metadata');
    const hideBtn = document.getElementById('section_hide_metadata');

    if (showBtn && panel && collapsed) {
        showBtn.addEventListener('click', () => {
            panel.classList.remove('hide');
            collapsed.classList.add('hide');
        });
    }
    if (hideBtn && panel && collapsed) {
        hideBtn.addEventListener('click', () => {
            panel.classList.add('hide');
            collapsed.classList.remove('hide');
        });
    }

    // Helpers to reflect title/subtitle in collapsed header live
    const collapsedTitle = collapsed?.querySelector('h1');
    const collapsedSubtitle = collapsed?.querySelector('h2');

    const onTitleChange = debounce((text: string) => {
        if (collapsedTitle) collapsedTitle.textContent = text;
        const pathParts = (content_path || '').split(':').filter(Boolean);
        const section_id = pathParts[pathParts.length - 1];
        const sidebarSection = document.querySelector(`.sidebar-contents-section[data-section-id="${section_id}"]`);
        if (sidebarSection) {
            const titleSpan = sidebarSection.querySelector('.section-title');
            if (titleSpan) {
                titleSpan.textContent = text || '[No title]';
            }
        }
        patchSectionMeta(content_path, { title: text });
    });
    const onSubtitleChange = debounce((text: string) => {
        if (collapsedSubtitle) collapsedSubtitle.textContent = text || '';
        patchSectionMeta(content_path, { subtitle: text && text.trim().length ? text : null });
    });

    const titleEl = document.getElementById('section_metadata_title');
    if (titleEl) {
        titleEl.addEventListener('input', () => onTitleChange((titleEl as HTMLElement).textContent || ''));
        titleEl.addEventListener('blur', () => onTitleChange((titleEl as HTMLElement).textContent || ''));
    }
    const subtitleEl = document.getElementById('section_metadata_subtitle');
    if (subtitleEl) {
        subtitleEl.addEventListener('input', () => onSubtitleChange((subtitleEl as HTMLElement).textContent || ''));
        subtitleEl.addEventListener('blur', () => onSubtitleChange((subtitleEl as HTMLElement).textContent || ''));
    }

    const tocOverride = document.getElementById('section_metadata_toc_title_subtitle_override') as HTMLInputElement | null;
    if (tocOverride) {
        tocOverride.addEventListener('input', debounce(() => {
            const v = tocOverride.value?.trim();
            patchSectionMeta(content_path, { toc_title_subtitle_override: v ? v : null });
        }));
    }

    const webUrl = document.getElementById('section_metadata_web_url') as HTMLInputElement | null;
    if (webUrl) {
        webUrl.addEventListener('input', debounce(() => {
            const v = webUrl.value?.trim();
            patchSectionMeta(content_path, { web_url: v ? v : null });
        }));
    }

    const langSel = document.getElementById('section_metadata_lang') as HTMLSelectElement | null;
    if (langSel) {
        langSel.addEventListener('change', () => {
            const v = langSel.value;
            patchSectionMeta(content_path, { lang: v === 'none' ? null : v });
        });
    }

    const published = document.getElementById('section_metadata_published') as HTMLInputElement | null;
    if (published) {
        const handler = debounce(() => {
            const v = published.value?.trim();
            patchSectionMeta(content_path, { published: v ? v : null });
        });
        published.addEventListener('input', handler);
        published.addEventListener('change', handler);
    }

    const delBtn = document.getElementById('section_delete');
    if (delBtn) {
        delBtn.addEventListener('click', async () => {
            if (!confirm("Are you sure you want to delete this section and ALL its contents? This cannot be undone.")) return;
            try {
                await sectionApi.delete_section(state.project_id, content_path);
                // Redirect to project root or reload project
                init();
            } catch (e) {
                console.error("Failed to delete section", e);
                show_alert("Failed to delete section.", "error");
            }
        });
    }

    // Authors and Editors
    let dragged_section_metadata_element: HTMLElement | null = null;

    const patchPersonsOrder = async (type: 'authors' | 'editors') => {
        const container = document.getElementById(`section_metadata_${type}_div`) as HTMLElement | null;
        if (!container) return;

        const elems = Array.from(container.querySelectorAll<HTMLElement>(`.section_metadata_persons_div[data-group='${type}']`));
        const next: PersonUuidOrString[] = [];

        for (const el of elems) {
            const entry_type = el.getAttribute('data-entry-type');
            if (entry_type === 'Person') {
                const id = el.getAttribute('data-id');
                if (id !== null) next.push({ PersonUuid: id });
            } else if (entry_type === 'NameString') {
                const name = el.getAttribute('data-name');
                if (name !== null) next.push({ NameString: name });
            }
        }

        const patch: any = {};
        patch[type] = next;
        await patchSectionMeta(content_path, patch);

        // Keep local copy in sync
        sectionData.metadata[type] = next;

        // Refresh expanded data for stable rendering
        const r = await fetch(`/api/projects/${state.project_id}/sections/${content_path}?expand=authors,editors`, { credentials: 'include' });
        if (r.ok) {
            const resp = await r.json();
            const data = resp?.data || resp;
            sectionData.metadata.authors = data?.metadata?.authors || [];
            sectionData.metadata.editors = data?.metadata?.editors || [];
            sectionData.metadata.authors_expanded = data?.metadata?.authors_expanded || [];
            sectionData.metadata.editors_expanded = data?.metadata?.editors_expanded || [];
        }
    };

    const addDragAndDropListeners = (type: 'authors' | 'editors') => {
        const container = document.getElementById(`section_metadata_${type}_div`) as HTMLElement | null;
        if (!container) return;

        const dragElements = Array.from(container.querySelectorAll<HTMLElement>(`.section_metadata_persons_div[data-group='${type}']`));
        const dropZones: HTMLElement[] = [];

        const firstDropzone = container.querySelector<HTMLElement>('.first_dropzone');
        if (firstDropzone) dropZones.push(firstDropzone);
        dropZones.push(...Array.from(container.querySelectorAll<HTMLElement>('.section_metadata_person_div_after')));

        for (const element of dragElements) {
            element.addEventListener('dragstart', (e) => {
                dragged_section_metadata_element = e.currentTarget as HTMLElement;

                const draggedId = dragged_section_metadata_element.getAttribute('data-id');
                const parent = dragged_section_metadata_element.parentElement;

                for (const dz of dropZones) {
                    if (!draggedId) {
                        dz.classList.add('dragactive');
                        continue;
                    }

                    // Don't highlight the first dropzone when dragging the first element
                    if (dz.classList.contains('first_dropzone') && parent && parent.children.length > 1) {
                        const firstElement = parent.children[1] as HTMLElement;
                        if (firstElement.getAttribute('data-id') === draggedId) {
                            continue;
                        }
                    }

                    const afterId = dz.getAttribute('data-dropzone-after');
                    if (afterId && afterId === draggedId) {
                        continue;
                    }

                    dz.classList.add('dragactive');
                }
            });

            element.addEventListener('dragend', () => {
                dragged_section_metadata_element = null;
                for (const dz of dropZones) {
                    dz.classList.remove('dragactive');
                    dz.classList.remove('dragover');
                }
            });
        }

        for (const dropzone of dropZones) {
            dropzone.addEventListener('dragenter', (e) => {
                const zone = e.currentTarget as HTMLElement;

                if (!dragged_section_metadata_element) return;

                // Don't show drop opportunity for first dropzone for first element
                if (zone.classList.contains('first_dropzone')) {
                    const parent = dragged_section_metadata_element.parentElement;
                    if (parent && parent.children.length > 1) {
                        const firstElement = parent.children[1] as HTMLElement;
                        if (firstElement.getAttribute('data-id') === dragged_section_metadata_element.getAttribute('data-id')) {
                            return;
                        }
                    }
                }

                if (
                    dragged_section_metadata_element.getAttribute('data-group') === type &&
                    dragged_section_metadata_element.getAttribute('data-id') !== zone.getAttribute('data-dropzone-after')
                ) {
                    zone.classList.add('dragover');
                }
            });

            dropzone.addEventListener('dragleave', (e) => {
                const zone = e.currentTarget as HTMLElement;
                zone.classList.remove('dragover');
            });

            dropzone.addEventListener('dragover', (e) => {
                e.preventDefault();
            });

            dropzone.addEventListener('drop', (e) => {
                const zone = e.currentTarget as HTMLElement;

                if (!dragged_section_metadata_element || dragged_section_metadata_element.getAttribute('data-group') !== type) {
                    return;
                }

                const draggedId = dragged_section_metadata_element.getAttribute('data-id');
                const dropzoneId = zone.getAttribute('data-dropzone-after');

                if (draggedId === dropzoneId) {
                    return;
                }

                if (zone.classList.contains('first_dropzone')) {
                    const parent = dragged_section_metadata_element.parentElement;
                    if (parent && parent.children.length > 1) {
                        const firstElement = parent.children[1] as HTMLElement;
                        if (firstElement.getAttribute('data-id') === draggedId) {
                            return;
                        }
                    }

                    zone.classList.remove('dragover');
                    dragged_section_metadata_element.parentNode?.removeChild(dragged_section_metadata_element);
                    zone.insertAdjacentElement('afterend', dragged_section_metadata_element);
                } else {
                    zone.classList.remove('dragover');
                    dragged_section_metadata_element.parentNode?.removeChild(dragged_section_metadata_element);
                    zone.parentElement?.insertAdjacentElement('afterend', dragged_section_metadata_element);
                }

                patchPersonsOrder(type).then(() => {
                    renderAuthorsEditors(type);
                });
            });
        }
    };

    function renderAuthorsEditors(type: 'authors' | 'editors') {
        const div = document.getElementById(`section_metadata_${type}_div`);
        if (!div) return;
        // Keep the first_dropzone
        const dropzone = div.querySelector('.first_dropzone');
        div.innerHTML = '';
        if (dropzone) div.appendChild(dropzone);

        const list = type === 'authors' ? (sectionData.metadata.authors_expanded || []) : (sectionData.metadata.editors_expanded || []);

        list.forEach((person: any) => {
            // @ts-ignore
            div.insertAdjacentHTML('beforeend', Handlebars.templates[`editor_section_${type}_li`](person));
        });
        
        // Add remove listeners (template uses `.section_metadata_authors_remove` / `.section_metadata_editors_remove`)
        const removeBtnClass = type === 'authors' ? '.section_metadata_authors_remove' : '.section_metadata_editors_remove';
        div.querySelectorAll(removeBtnClass).forEach(btn => {
            btn.addEventListener('click', async (e) => {
                e.preventDefault();
                const entry = (e.currentTarget as HTMLElement).closest('.section_metadata_persons_div') as HTMLElement | null;
                if (!entry) return;

                const entryType = entry.getAttribute('data-entry-type');
                const id = entry.getAttribute('data-id');
                const name = entry.getAttribute('data-name');

                let currentList: PersonUuidOrString[] = (type === 'authors' ? sectionData.metadata.authors : sectionData.metadata.editors) || [];

                if (entryType === 'Person' && id) {
                    currentList = currentList.filter(p => !('PersonUuid' in p) || p.PersonUuid !== id);
                } else if (entryType === 'NameString' && name) {
                    currentList = currentList.filter(p => !('NameString' in p) || p.NameString !== name);
                } else {
                    return;
                }

                const patch: any = {};
                patch[type] = currentList;
                await patchSectionMeta(content_path, patch);

                // Update local data and rerender
                sectionData.metadata[type] = currentList;
                // We need expanded data for rendering, so refetching is easiest
                const r = await fetch(`/api/projects/${state.project_id}/sections/${content_path}?expand=authors,editors`, { credentials: 'include' });
                if (r.ok) {
                    const resp = await r.json();
                    const data = resp?.data || resp;
                    sectionData.metadata.authors = data?.metadata?.authors || [];
                    sectionData.metadata.editors = data?.metadata?.editors || [];
                    sectionData.metadata.authors_expanded = data?.metadata?.authors_expanded || [];
                    sectionData.metadata.editors_expanded = data?.metadata?.editors_expanded || [];
                }
                renderAuthorsEditors(type);
            });
        });

        addDragAndDropListeners(type);
    }

    const setupPersonSearch = (type: 'authors' | 'editors') => {
        const searchbar = document.getElementById(`section_metadata_search_${type}`) as HTMLInputElement | null;
        const results = document.getElementById(`section_metadata_search_${type}_results`) as HTMLElement | null;
        if (searchbar && results) {
            add_search(
                searchbar,
                results,
                personsApi.send_search_person_request.bind(personsApi),
                // @ts-ignore
                Handlebars.templates.search_person_li,
                async (selected: HTMLElement) => {
                    const person_id = selected.getAttribute("data-person-id");
                    if (!person_id) return;
                    
                    let currentList: PersonUuidOrString[] = (type === 'authors' ? sectionData.metadata.authors : sectionData.metadata.editors) || [];
                    if (currentList.some(p => 'PersonUuid' in p && p.PersonUuid === person_id)) {
                        show_alert("Already added.", "warning");
                        return;
                    }

                    currentList.push({ PersonUuid: person_id });
                    const patch: any = {};
                    patch[type] = currentList;
                    await patchSectionMeta(content_path, patch);

                    // Update and rerender
                    const r = await fetch(`/api/projects/${state.project_id}/sections/${content_path}?expand=authors,editors`, { credentials: 'include' });
                    if (r.ok) {
                        const resp = await r.json();
                        const data = resp?.data || resp;
                        sectionData.metadata.authors = data?.metadata?.authors || [];
                        sectionData.metadata.editors = data?.metadata?.editors || [];
                        sectionData.metadata.authors_expanded = data?.metadata?.authors_expanded || [];
                        sectionData.metadata.editors_expanded = data?.metadata?.editors_expanded || [];
                    }
                    renderAuthorsEditors(type);
                }
            );

            searchbar.addEventListener("keydown", async (e: KeyboardEvent) => {
                if (e.key !== "Enter") return;
                const value = searchbar.value.trim();
                if (!value) return;
                searchbar.value = "";

                let currentList: PersonUuidOrString[] = (type === 'authors' ? sectionData.metadata.authors : sectionData.metadata.editors) || [];
                currentList.push({ NameString: value });
                const patch: any = {};
                patch[type] = currentList;
                await patchSectionMeta(content_path, patch);

                // Update and rerender
                const r = await fetch(`/api/projects/${state.project_id}/sections/${content_path}?expand=authors,editors`, { credentials: 'include' });
                if (r.ok) {
                    const resp = await r.json();
                    const data = resp?.data || resp;
                    sectionData.metadata.authors = data?.metadata?.authors || [];
                    sectionData.metadata.editors = data?.metadata?.editors || [];
                    sectionData.metadata.authors_expanded = data?.metadata?.authors_expanded || [];
                    sectionData.metadata.editors_expanded = data?.metadata?.editors_expanded || [];
                }
                renderAuthorsEditors(type);
            });
        }
    };

    renderAuthorsEditors('authors');
    renderAuthorsEditors('editors');
    setupPersonSearch('authors');
    setupPersonSearch('editors');

    // Identifiers add/remove/update
    let identifiers: any[] = (sectionData && sectionData.metadata && sectionData.metadata.identifiers) ? [...sectionData.metadata.identifiers] : [];

    function renderIdentifiersList(list: any[]) {
        const listEl = document.getElementById('section_metadata_identifiers_list');
        if (!listEl) return;
        listEl.innerHTML = '';
        // @ts-ignore
        const tpl = (Handlebars.templates && (Handlebars.templates as any).editor_section_identifier_row) || (Handlebars.partials as any)?.editor_section_identifier_row;
        if (!tpl) return;
        list.forEach(item => {
            // Some backends may use `identifier_type` or `identifierType` casing; normalize
            const ctx = {
                id: item.id || null,
                name: item.name || '',
                value: item.value || '',
                identifier_type: item.identifier_type || item.identifierType || ''
            };
            // @ts-ignore
            listEl.insertAdjacentHTML('beforeend', Handlebars.templates.editor_section_identifier_row(ctx));
        });

        // Attach remove + change listeners
        listEl.querySelectorAll('.section_metadata_identifier_remove_btn').forEach(btn => {
            btn.addEventListener('click', async (e) => {
                e.preventDefault();
                const row = (e.currentTarget as HTMLElement).closest('.section_metadata_identifier_row') as HTMLElement | null;
                if (!row) return;
                const id = row.getAttribute('data-identifier-id');
                const type = row.getAttribute('data-identifier-type') || '';
                const nameEl = row.querySelector('.section_metadata_identifier_name') as HTMLInputElement | null;
                const valueEl = row.querySelector('.section_metadata_identifier_value') as HTMLInputElement | null;
                const nameVal = nameEl?.value || '';
                const valueVal = valueEl?.value || '';

                identifiers = identifiers.filter(it => {
                    if (id && it.id) return it.id !== id;
                    // fallback compare
                    return !(it.identifier_type === type && it.name === nameVal && it.value === valueVal);
                });
                await patchSectionMeta(content_path, { identifiers });
                // Refetch for fresh IDs
                try {
                    const r = await fetch(`/api/projects/${state.project_id}/sections/${content_path}?expand=authors,editors`, { credentials: 'include' });
                    if (r.ok) {
                        const resp = await r.json();
                        const data = resp?.data || resp;
                        identifiers = data?.metadata?.identifiers || [];
                    }
                } catch {}
                renderIdentifiersList(identifiers);
            });
        });

        // Handle inline changes to identifier name/value
        listEl.querySelectorAll('.section_metadata_identifier_row').forEach(row => {
            const id = (row as HTMLElement).getAttribute('data-identifier-id');
            const type = (row as HTMLElement).getAttribute('data-identifier-type') || '';
            const nameEl = row.querySelector('.section_metadata_identifier_name') as HTMLInputElement | null;
            const valueEl = row.querySelector('.section_metadata_identifier_value') as HTMLInputElement | null;
            const applyChange = debounce(async () => {
                const nameVal = nameEl?.value || '';
                const valueVal = valueEl?.value || '';
                identifiers = identifiers.map(it => {
                    const match = id ? (it.id === id) : (it.identifier_type === type && it.name === (row as any)._origName && it.value === (row as any)._origValue);
                    if (match) return { ...it, name: nameVal, value: valueVal };
                    return it;
                });
                await patchSectionMeta(content_path, { identifiers });
            }, 400);
            if (nameEl) nameEl.addEventListener('input', applyChange);
            if (valueEl) valueEl.addEventListener('input', applyChange);
            // store original for non-id rows
            (row as any)._origName = nameEl?.value || '';
            (row as any)._origValue = valueEl?.value || '';
        });
    }

    renderIdentifiersList(identifiers);

    const addBtn = document.getElementById('section_metadata_identifiers_add');
    if (addBtn) {
        addBtn.addEventListener('click', async (e) => {
            e.preventDefault();
            const typeEl = document.getElementById('section_metadata_identifiers_type') as HTMLSelectElement | null;
            const nameEl = document.getElementById('section_metadata_identifiers_name') as HTMLInputElement | null;
            const valueEl = document.getElementById('section_metadata_identifiers_value') as HTMLInputElement | null;
            const type = typeEl?.value || 'DOI';
            const name = nameEl?.value?.trim() || type;
            const value = valueEl?.value?.trim() || '';
            if (!value) return;
            identifiers = [...identifiers, { id: null, name, value, identifier_type: type }];
            await patchSectionMeta(content_path, { identifiers });
            // Clear inputs
            if (nameEl) nameEl.value = '';
            if (valueEl) valueEl.value = '';
            // Refetch to get server-assigned ids
            try {
                const r = await fetch(`/api/projects/${state.project_id}/sections/${content_path}?expand=authors,editors`, { credentials: 'include' });
                if (r.ok) {
                    const resp = await r.json();
                    const data = resp?.data || resp;
                    identifiers = data?.metadata?.identifiers || [];
                }
            } catch {}
            renderIdentifiersList(identifiers);
        });
    }
}

/**
 * Converts a Yjs block representation into the plain EditorJS data shape.
 *
 * This function is used both for initial render and for applying remote updates.
 */
function normalizeBlockData(block: any) {
    /**
     * Recursively converts Yjs types (`Y.Text`, `Y.Map`, `Y.Array`) into plain JS values.
     */
    function valueToJs(val: any): any {
        if (val == null) return val;
        if (val instanceof Y.Text) return val.toString();
        if (val instanceof Y.Array) return val.toArray().map(valueToJs);
        if (val instanceof Y.Map) {
            const obj: any = {};
            val.forEach((v: any, k: string) => {
                obj[k] = valueToJs(v);
            });
            return obj;
        }
        return val;
    }

    // Prefer explicit Yjs handling over `toJSON()`. Some values coming from observer deltas can
    // be non-integrated Yjs types; reading them via `toJSON()` triggers "Invalid access".
    let blockData: any = block;
    if (block instanceof Y.Map) {
        try {
            // Read directly from integrated Yjs types.
            blockData = {
                id: block.get('id'),
                type: block.get('type'),
                data: valueToJs(block.get('data')),
                tunes: valueToJs(block.get('tunes')),
            };
        } catch (e) {
            console.error('normalizeBlockData: Failed to read Yjs block map, falling back to plain access', e);
            // Avoid calling `toJSON()` here: observer deltas can include non-integrated Yjs types
            // and `toJSON()` can throw "Invalid access".
            blockData = { id: undefined, type: undefined, data: {}, tunes: {} };
        }
    } else {
        // Avoid `toJSON()` for the same reason as above.
        blockData = block;
    }

    // Normalize properties
    let type = blockData?.type || blockData?.block_type || blockData?.blockType || blockData?.kind || blockData?.tool;
    let data = blockData?.data;
    const uuid = blockData?.uuid || blockData?.id;

    // Normalize specific type names
    if (typeof type === 'string') {
        type = type.toLowerCase();
        if (type === 'heading') type = 'header';
    }

    // Avoid over-aggressive inference: our canonical schema should always provide `type`.
    if (!type) {
        console.warn("normalizeBlockData: Missing 'type' for block, falling back to 'paragraph'. Raw data:", blockData);
        type = 'paragraph';
    }

    // Ensure `data` is always a plain object for EditorJS.
    if (data == null || typeof data !== 'object') {
        data = {};
    }

    return {
        type,
        data,
        tunes: blockData?.tunes,
        id: uuid,
        uuid: uuid,
    };
}
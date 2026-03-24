import * as Editor from './Editor';
import {BibEntryOrFolder, BibliographyAPI} from '../api_requests';
import {state} from './Main';
import {show_alert} from "../tools";

// Ensure we don't accumulate multiple sidebar event listeners across re-renders
let sidebarListenersController: AbortController | null = null;

const bibAPI = BibliographyAPI();

let active_id: string | null = null;

// Centralized list of entry types (avoid duplication)
const ENTRY_TYPES = [
    "Anthology", "Article", "Audio", "Blog", "Book", "Booklet", "Conference",
    "CourtDecision", "Document", "Entry", "InBook", "InCollection",
    "InProceedings", "Legislation", "Manual", "Map", "MastersThesis", "Misc",
    "Patent", "Periodical", "PhdThesis", "Post", "Proceedings", "Reference",
    "Report", "Repository", "Software", "Speech", "Standard", "TechReport",
    "Thesis", "Unpublished", "Video", "Web", "Workshop"
];

export async function init(){
    await render_sidebar_only();

    // Hide preview column if visible
    Editor.hide_preview_column();

    // Clear main column or show a message
    const main_col = document.querySelector('.main-col') as HTMLElement;
    if (main_col) {
        if (active_id) {
            await show_bib_editor(active_id);
        } else {
            main_col.innerHTML = "Select an entry to edit.";
        }
    }
}

// Expose small helpers so other tools (e.g., CitationTool) can reuse
// the existing editor rendering and listeners without duplicating code.
// - openEntryEditor: opens the editor in the main column
// - openEntryEditorInPreview: opens the editor in the preview column
export async function openEntryEditor(id: string){
    await show_bib_editor(id);
}

export async function openEntryEditorInPreview(id: string){
    active_id = id;
    // Ensure preview column is visible
    Editor.show_preview_column();

    const preview_col = document.querySelector('.preview-col') as HTMLElement | null;
    if (!preview_col) return;

    // Ensure a stable wrapper inside the preview column so re-renders don't wipe controls
    let wrapper = preview_col.querySelector('#preview-bib-wrapper') as HTMLElement | null;
    if (!wrapper) {
        wrapper = document.createElement('div');
        wrapper.id = 'preview-bib-wrapper';
        // Minimal toolbar with a Close button
        const toolbar = document.createElement('div');
        toolbar.className = 'preview-bib-toolbar';
        const closeBtn = document.createElement('button');
        closeBtn.id = 'preview-bib-close';
        closeBtn.className = 'btn btn-sm btn-secondary';
        closeBtn.textContent = 'Close';
        toolbar.appendChild(closeBtn);

        const content = document.createElement('div');
        content.id = 'preview-bib-content';

        wrapper.appendChild(toolbar);
        wrapper.appendChild(content);
        // Clear and append wrapper (safe since preview is dedicated for this)
        preview_col.innerHTML = '';
        preview_col.appendChild(wrapper);

        // Wire close action
        closeBtn.addEventListener('click', () => {
            // Clear content and hide preview column
            content.innerHTML = '';
            Editor.hide_preview_column();
        });
    } else {
        // Re-bind close in case of lost listeners after hot reloads
        const closeBtn = wrapper.querySelector('#preview-bib-close') as HTMLButtonElement | null;
        const content = wrapper.querySelector('#preview-bib-content') as HTMLElement | null;
        if (closeBtn && content) {
            closeBtn.onclick = null;
            closeBtn.addEventListener('click', () => {
                content.innerHTML = '';
                Editor.hide_preview_column();
            });
        }
    }

    const contentDiv = wrapper.querySelector('#preview-bib-content') as HTMLElement | null;
    if (!contentDiv) return;

    try{
        await fetch_and_render_into(contentDiv, id);
    }catch(e){
        console.error(e);
        show_alert("Failed to load bibliography entry.");
    }
}

async function render_sidebar_only() {
    let contents_panel : HTMLElement = document.getElementsByClassName("sidebar-full-contents-panel")[0] as HTMLElement;

    try {
        const tree = await bibAPI.get_bibliography_tree(state.project_id);

        // @ts-ignore
        contents_panel.innerHTML = Handlebars.templates.editor_sidebar_bibliography({tree, active_id});

        add_sidebar_listeners();
        
        if (active_id) {
            const activeNode = contents_panel.querySelector(`.bibliography-node[data-id="${active_id}"]`);
            if (activeNode) {
                activeNode.scrollIntoView({ behavior: 'smooth', block: 'nearest' });
            }
        }
    } catch (e) {
        console.error(e);
        show_alert("Failed to load bibliography tree.");
    }
}

function add_sidebar_listeners() {
    // Abort and detach any previously attached listeners to avoid duplicates
    try {
        sidebarListenersController?.abort();
    } catch (_e) {
        // ignore
    }
    sidebarListenersController = new AbortController();
    const signal = sidebarListenersController.signal;

    const sidebar_panel = document.querySelector('.sidebar-full-contents-panel');
    if (!sidebar_panel) return;

    // Small helper to DRY creation flow for entry/folder
    const createAndOpen = async (payload: BibEntryOrFolder) => {
        try {
            const id = await bibAPI.post_bibliography_entry(state.project_id, payload);
            active_id = id;
            await render_sidebar_only();
            await show_bib_editor(id);
        } catch (e) {
            console.error(e);
            const what = 'BibEntry' in payload ? 'bibliography entry' : 'folder';
            show_alert(`Failed to create new ${what}.`);
        }
    };

    sidebar_panel.addEventListener('click', async (e) => {
        const target = e.target as HTMLElement;
        // Handle "New Entry" and "New Folder" via delegation to avoid rebinding on re-render
        if (target.closest('#sidebar-new-bib-entry')) {
            e.preventDefault();
            e.stopPropagation();
            const newEntry: BibEntryOrFolder = {
                BibEntry: {
                    key: '00000000-0000-0000-0000-000000000000', // Server will generate UUID
                    entry_type: 'Article',
                    authors: [],
                    editors: [],
                    affiliated: [],
                    parents: []
                }
            };
            await createAndOpen(newEntry);
            return;
        }

        if (target.closest('#sidebar-new-bib-folder')) {
            e.preventDefault();
            e.stopPropagation();
            const newFolder: BibEntryOrFolder = {
                BibFolder: {
                    name: 'New Folder',
                    parent: null
                }
            };
            await createAndOpen(newFolder);
            return;
        }

        const body = target.closest('.sidebar-contents-section-body');
        const node = body?.closest('.bibliography-node');
        
        if (node) {
            e.preventDefault();
            e.stopPropagation();
            const id = node.getAttribute('data-id');
            console.log("Bibliography node clicked", id);
            if (id) {
                active_id = id;
                // Remove active class from all nodes
                document.querySelectorAll('.bibliography-node .sidebar-contents-section-body').forEach(b => b.classList.remove('active'));
                // Add active class to clicked node
                body.classList.add('active');
                await show_bib_editor(id);
            }
        }
    }, { signal });

    // Drag and drop listeners
    sidebar_panel.addEventListener('dragstart', (e: DragEvent) => {
        const target = e.target as HTMLElement;
        const node = target.closest('.bibliography-node');
        if (node) {
            const id = node.getAttribute('data-id');
            if (id) {
                e.dataTransfer?.setData('text/plain', id);
                // Use setTimeout to avoid immediate dragend in Chromium-based browsers
                // caused by synchronous DOM/layout changes during dragstart
                setTimeout(() => {
                    node.classList.add('dragging');
                    // Mark the root as drag-active so the top-level dropzone becomes visible
                    const root = document.querySelector('#bibliography-tree-root');
                    root?.classList.add('drag-active');
                }, 0);
            }
        }
    }, { signal });

    sidebar_panel.addEventListener('dragend', (e: DragEvent) => {
        const target = e.target as HTMLElement;
        const node = target.closest('.bibliography-node');
        if (node) {
            node.classList.remove('dragging');
        }
        document.querySelectorAll('.drag-over').forEach(el => el.classList.remove('drag-over'));
        // Remove drag-active from root when drag operation ends
        const root = document.querySelector('#bibliography-tree-root');
        root?.classList.remove('drag-active');
    }, { signal });

    sidebar_panel.addEventListener('dragover', (e: DragEvent) => {
        e.preventDefault();
        const target = e.target as HTMLElement;
        const node = target.closest('.bibliography-node');
        const root = target.closest('#bibliography-tree-root');

        if (node && !node.classList.contains('dragging')) {
            node.classList.add('drag-over');
        } else if (root) {
            root.classList.add('drag-over');
        }
    }, { signal });

    sidebar_panel.addEventListener('dragleave', (e: DragEvent) => {
        const target = e.target as HTMLElement;
        const node = target.closest('.bibliography-node');
        const root = target.closest('#bibliography-tree-root');
        if (node) {
            node.classList.remove('drag-over');
        }
        if (root && !root.contains(e.relatedTarget as Node)) {
            root.classList.remove('drag-over');
        }
    }, { signal });

    sidebar_panel.addEventListener('drop', async (e: DragEvent) => {
        e.preventDefault();
        const draggedId = e.dataTransfer?.getData('text/plain');
        if (!draggedId) return;

        const target = e.target as HTMLElement;
        const node = target.closest('.bibliography-node');
        const root = target.closest('#bibliography-tree-root');

        let newParentId: string | null = null;
        if (node) {
            // Make the dragged item a CHILD of the drop target (folder OR entry)
            // This fixes the bug where dropping onto an entry did not set any parent in the patch
            newParentId = node.getAttribute('data-id');
        } else if (root) {
            newParentId = null;
        } else {
            return;
        }

        if (draggedId === newParentId) return;

        try {
            const entry = await bibAPI.get_bibliography_entry(state.project_id, draggedId);
            if ('BibEntry' in entry) {
                entry.BibEntry.parents = newParentId ? [newParentId] : [];
            } else {
                entry.BibFolder.parent = newParentId;
            }
            await bibAPI.patch_bibliography_entry(state.project_id, draggedId, entry);
            await render_sidebar_only();
        } catch (err) {
            console.error(err);
            show_alert("Failed to move bibliography item.");
        }
    }, { signal });
}

async function show_bib_editor(id: string) {
    active_id = id;
    const main_col = document.querySelector('.main-col') as HTMLElement;
    if (!main_col) return;

    try {
        await fetch_and_render_into(main_col, id);
    } catch (e) {
        console.error(e);
        show_alert("Failed to load bibliography entry.");
    }
}

function debounce(fn: Function, delay: number = 500) {
    let timeout: any;
    const debounced = (...args: any[]) => {
        clearTimeout(timeout);
        timeout = setTimeout(() => fn(...args), delay);
    };
    debounced.flush = async () => {
        clearTimeout(timeout);
    };
    return debounced;
}

function add_editor_listeners(rootEl: HTMLElement, id: string, entry: BibEntryOrFolder) {
    const main_col = rootEl;

    const rerenderIntoSameContainer = async () => {
        try {
            await fetch_and_render_into(main_col, id);
        } catch (e) {
            console.error(e);
            show_alert("Failed to refresh bibliography editor.");
        }
    };

    const save = debounce(async () => {
        try {
            await bibAPI.patch_bibliography_entry(state.project_id, id, entry);
            // Refresh sidebar to reflect changes (e.g. name change)
            await render_sidebar_only();
        } catch (e) {
            console.error(e);
            show_alert("Failed to save bibliography entry.");
        }
    }, 500);

    // Helper to persist current entry state immediately and re-render into same container
    const persistAndRerender = async () => {
        await save.flush();
        await bibAPI.patch_bibliography_entry(state.project_id, id, entry);
        await rerenderIntoSameContainer();
    };

    main_col.querySelectorAll('.bib-quickchange').forEach(input => {
        const handler = async (e: Event) => {
            const target = e.target as HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement;
            const path = target.getAttribute('data-path');
            if (path) {
                set_value_by_path(entry, path, target.value);
                await save();

                // Live update collapsed title if it's the title field
                if (path === 'BibEntry.title.value' || path === 'BibFolder.name') {
                    const collapsedTitle = main_col.querySelector('.editor_section_view_collapsed_metadata_inner h1');
                    if (collapsedTitle) {
                        collapsedTitle.textContent = target.value || (path === 'BibEntry.title.value' ? 'Untitled' : 'New Folder');
                    }
                }
            }
        };

        input.addEventListener('input', handler);
        if (input.tagName === 'SELECT') {
            input.addEventListener('change', handler);
        }
    });

    main_col.querySelectorAll('.bib-quickchange-bool').forEach(input => {
        input.addEventListener('change', async (e) => {
            const target = e.target as HTMLInputElement;
            const path = target.getAttribute('data-path');
            if (path) {
                set_value_by_path(entry, path, target.checked);
                await save();
            }
        });
    });

    main_col.querySelectorAll('.bib-person-change').forEach(input => {
        input.addEventListener('input', async (e) => {
            const target = e.target as HTMLInputElement;
            const type = target.getAttribute('data-type') as 'authors' | 'editors';
            const field = target.getAttribute('data-field') as 'name' | 'given_name' | 'prefix' | 'suffix' | 'alias';
            const index = parseInt(target.closest('.person_row')?.getAttribute('data-index') || '0');

            if ('BibEntry' in entry) {
                // @ts-ignore
                if (!entry.BibEntry[type][index]) {
                     // @ts-ignore
                     entry.BibEntry[type][index] = {name: ''};
                }
                // @ts-ignore
                entry.BibEntry[type][index][field] = target.value;
                await save();
            }
        });
    });

    main_col.querySelectorAll('.bib-person-add').forEach(btn => {
        btn.addEventListener('click', async (e) => {
            const type = (e.currentTarget as HTMLElement).getAttribute('data-type') as 'authors' | 'editors';
            console.log("Adding person", type);
            if ('BibEntry' in entry) {
                // @ts-ignore
                if (!entry.BibEntry[type]) entry.BibEntry[type] = [];
                // @ts-ignore
                entry.BibEntry[type].push({name: ''});
                await persistAndRerender(); // Re-render to show new row
            }
        });
    });

    main_col.querySelectorAll('.bib-person-remove').forEach(btn => {
        btn.addEventListener('click', async (e) => {
            const type = (e.currentTarget as HTMLElement).getAttribute('data-type') as 'authors' | 'editors';
            const index = parseInt((e.currentTarget as HTMLElement).getAttribute('data-index') || '0');
            console.log("Removing person", type, index);
            if ('BibEntry' in entry) {
                // @ts-ignore
                entry.BibEntry[type].splice(index, 1);
                await persistAndRerender(); // Re-render
            }
        });
    });

    main_col.querySelector('#bib_entry_delete')?.addEventListener('click', async () => {
        if (confirm("Are you sure you want to delete this?")) {
            try {
                await bibAPI.delete_bibliography_entry(state.project_id, id);
                active_id = null;
                main_col.innerHTML = "Select an entry to edit.";
                await render_sidebar_only();
            } catch (e) {
                console.error(e);
                show_alert("Failed to delete bibliography entry.");
            }
        }
    });
}

// Helper to render a bibliography entry or folder into a given container and attach listeners
function render_entry_into(container: HTMLElement, id: string, entry: BibEntryOrFolder){
    if ('BibEntry' in entry) {
        // @ts-ignore
        container.innerHTML = Handlebars.templates.editor_bibliography_entry_editor({entry, entry_types: ENTRY_TYPES});
    } else {
        // @ts-ignore
        container.innerHTML = Handlebars.templates.editor_bibliography_folder_editor({entry});
    }

    add_editor_listeners(container, id, entry);
}

// Helper to fetch an entry and render it into the provided container
async function fetch_and_render_into(container: HTMLElement, id: string){
    const entry = await bibAPI.get_bibliography_entry(state.project_id, id);
    render_entry_into(container, id, entry);
}

function set_value_by_path(obj: any, path: string, value: any) {
    const parts = path.split('.');
    let current = obj;

    for (let i = 0; i < parts.length - 1; i++) {
        const part = parts[i];
        if (!current[part]) {
            // Check if next part is a number (for arrays, though not used here yet) or just an object
            current[part] = {};
        }
        current = current[part];
    }
    
    const lastPart = parts[parts.length - 1];
    if (value === "" || value === null || value === undefined) {
        if (current.hasOwnProperty(lastPart)) {
            delete current[lastPart];
        }
        
        // Clean up parent if it's now empty (specifically for MyMaybeTyped)
        if (Object.keys(current).length === 0) {
            const parentParts = parts.slice(0, -1);
            if (parentParts.length > 0) {
                let parentObj = obj;
                for (let i = 0; i < parentParts.length - 1; i++) {
                    parentObj = parentObj[parentParts[i]];
                }
                delete parentObj[parentParts[parentParts.length - 1]];
            }
        }
    } else {
        current[lastPart] = value;
    }

    // After setting a sub-property of a MyMaybeTyped (like .String), 
    // we must ensure the other variant (like .Typed) is removed.
    if (lastPart === 'String') {
        if (current.hasOwnProperty('Typed')) delete current.Typed;
    } else if (lastPart === 'Typed') {
        if (current.hasOwnProperty('String')) delete current.String;
    }

    // Fix for BibEntryOrFolder enum structure if it got polluted by wrong paths in the past
    // or if the path was relative but should have been absolute within the object.
    // In this specific case, we want to make sure 'name' and 'parent' are not at the root
    // if 'BibFolder' exists.
    if (obj.BibFolder) {
        if (obj.hasOwnProperty('name')) delete obj.name;
        if (obj.hasOwnProperty('parent')) delete obj.parent;
    }
}
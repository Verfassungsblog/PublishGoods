import * as Editor from './Editor';
import {BibEntryOrFolder, BibliographyAPI} from '../api_requests';
import {state} from './Main';
import {show_alert} from "../tools";

const bibAPI = BibliographyAPI();

export async function init(){
    await render_sidebar_only();

    // Hide preview column if visible
    Editor.hide_preview_column();

    // Clear main column or show a message
    const main_col = document.querySelector('.main-col') as HTMLElement;
    if (main_col) {
        main_col.innerHTML = "Select an entry to edit.";
    }
}

async function render_sidebar_only() {
    let contents_panel : HTMLElement = document.getElementsByClassName("sidebar-full-contents-panel")[0] as HTMLElement;

    try {
        const tree = await bibAPI.get_bibliography_tree(state.project_id);

        // @ts-ignore
        contents_panel.innerHTML = Handlebars.templates.editor_sidebar_bibliography({tree});

        add_sidebar_listeners();
    } catch (e) {
        console.error(e);
        show_alert("Failed to load bibliography tree.");
    }
}

function add_sidebar_listeners() {
    const sidebar_panel = document.querySelector('.sidebar-full-contents-panel');
    if (!sidebar_panel) return;

    sidebar_panel.addEventListener('click', async (e) => {
        const target = e.target as HTMLElement;
        const body = target.closest('.sidebar-contents-section-body');
        const node = body?.closest('.bibliography-node');
        
        if (node) {
            e.preventDefault();
            e.stopPropagation();
            const id = node.getAttribute('data-id');
            console.log("Bibliography node clicked", id);
            if (id) {
                // Remove active class from all nodes
                document.querySelectorAll('.bibliography-node .sidebar-contents-section-body').forEach(b => b.classList.remove('active'));
                // Add active class to clicked node
                body.classList.add('active');
                await show_bib_editor(id);
            }
        }
    });

    document.getElementById('sidebar-new-bib-entry')?.addEventListener('click', async (e) => {
        e.preventDefault();
        e.stopPropagation();
        try {
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
            const id = await bibAPI.post_bibliography_entry(state.project_id, newEntry);
            await render_sidebar_only();
            await show_bib_editor(id);
        } catch (e) {
            console.error(e);
            show_alert("Failed to create new bibliography entry.");
        }
    });

    document.getElementById('sidebar-new-bib-folder')?.addEventListener('click', async (e) => {
        e.preventDefault();
        e.stopPropagation();
        try {
            const newFolder: BibEntryOrFolder = {
                BibFolder: {
                    name: 'New Folder',
                    parent: null
                }
            };
            await bibAPI.post_bibliography_entry(state.project_id, newFolder);
            await render_sidebar_only();
        } catch (e) {
            console.error(e);
            show_alert("Failed to create new folder.");
        }
    });
}

async function show_bib_editor(id: string) {
    const main_col = document.querySelector('.main-col') as HTMLElement;
    if (!main_col) return;

    try {
        const entry = await bibAPI.get_bibliography_entry(state.project_id, id);

        const entry_types = [
            "Anthology", "Article", "Audio", "Blog", "Book", "Booklet", "Conference",
            "CourtDecision", "Document", "Entry", "InBook", "InCollection",
            "InProceedings", "Legislation", "Manual", "Map", "MastersThesis", "Misc",
            "Patent", "Periodical", "PhdThesis", "Post", "Proceedings", "Reference",
            "Report", "Repository", "Software", "Speech", "Standard", "TechReport",
            "Thesis", "Unpublished", "Video", "Web", "Workshop"
        ];

        if ('BibEntry' in entry) {
            // @ts-ignore
            main_col.innerHTML = Handlebars.templates.editor_bibliography_entry_editor({entry, entry_types});
        } else {
            // @ts-ignore
            main_col.innerHTML = Handlebars.templates.editor_bibliography_folder_editor({entry});
        }

        add_editor_listeners(id, entry);
    } catch (e) {
        console.error(e);
        show_alert("Failed to load bibliography entry.");
    }
}

function add_editor_listeners(id: string, entry: BibEntryOrFolder) {
    const main_col = document.querySelector('.main-col') as HTMLElement;

    const save = async () => {
        try {
            await bibAPI.patch_bibliography_entry(state.project_id, id, entry);
            // Refresh sidebar to reflect changes (e.g. name change)
            await render_sidebar_only();
        } catch (e) {
            console.error(e);
            show_alert("Failed to save bibliography entry.");
        }
    };

    main_col.querySelectorAll('.bib-quickchange').forEach(input => {
        input.addEventListener('change', async (e) => {
            const target = e.target as HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement;
            const path = target.getAttribute('data-path');
            if (path) {
                set_value_by_path(entry, path, target.value);
                await save();
            }
        });
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
        input.addEventListener('change', async (e) => {
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
            if ('BibEntry' in entry) {
                // @ts-ignore
                if (!entry.BibEntry[type]) entry.BibEntry[type] = [];
                // @ts-ignore
                entry.BibEntry[type].push({name: ''});
                await save();
                await show_bib_editor(id); // Re-render to show new row
            }
        });
    });

    main_col.querySelectorAll('.bib-person-remove').forEach(btn => {
        btn.addEventListener('click', async (e) => {
            const type = (e.currentTarget as HTMLElement).getAttribute('data-type') as 'authors' | 'editors';
            const index = parseInt((e.currentTarget as HTMLElement).getAttribute('data-index') || '0');
            if ('BibEntry' in entry) {
                // @ts-ignore
                entry.BibEntry[type].splice(index, 1);
                await save();
                await show_bib_editor(id); // Re-render
            }
        });
    });

    document.getElementById('bib_parents_input')?.addEventListener('change', async (e) => {
        const target = e.target as HTMLInputElement;
        const uuids = target.value.split(',').map(s => s.trim()).filter(s => s.length > 0);
        if ('BibEntry' in entry) {
            // @ts-ignore
            entry.BibEntry.parents = uuids;
            await save();
        }
    });

    document.getElementById('bib_entry_delete')?.addEventListener('click', async () => {
        if (confirm("Are you sure you want to delete this?")) {
            try {
                await bibAPI.delete_bibliography_entry(state.project_id, id);
                main_col.innerHTML = "Select an entry to edit.";
                await render_sidebar_only();
            } catch (e) {
                console.error(e);
                show_alert("Failed to delete bibliography entry.");
            }
        }
    });
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
    } else {
        current[lastPart] = value;
    }
}
import {state} from "../Main";
import {BibEntryOrFolder, BibliographyAPI} from "../../api_requests";
import {openEntryEditorInPreview} from "../Bibliography";

export class CitationTool {
    private button: HTMLButtonElement | null;
    private state: boolean;
    private api: any;

    static get isInline() {
        return true;
    }

    // @ts-ignore
    constructor({data, api}) {
        this.button = null;
        this.state = false;
        this.api = api;

        CitationTool.add_all_show_note_settings_listeners();
    }

    static add_all_show_note_settings_listeners(){
        let notes = document.getElementsByTagName("citation");
        for(let i = 0; i < notes.length; i++){
            notes[i].removeEventListener('click', this.show_note_settings_editor);
            notes[i].addEventListener('click', this.show_note_settings_editor);
        }
    }

    /// Get's called when an existing citation is clicked.
    static show_note_settings_editor(e: Event){
        // Hide all other note-settings dialogs
        // @ts-ignore
        for(let settings of document.getElementsByClassName('citation-settings')){
            settings.remove();
        }

        let citation = e.target as HTMLElement;
        const key = citation.getAttribute("data-key") || "";

        let settings_dialog_html = "" +
            "<div class='citation-settings'>" +
            "<label>Modify Citation:</label><br>" +
            // Title placeholder, will be filled after fetch
            "<div id='citation-entry-title' style='font-weight: 600; margin-bottom: 6px;'>Loading…</div>"+
            // Prefix/Suffix inputs
            "<div class='mt-1'>"+
            "<label for='citation-prefix' style='display:block'>Prefix (optional)</label>"+
            "<input type='text' class='cdx-input' id='citation-prefix' placeholder='e.g., see also'/>"+
            "</div>"+
            "<div class='mt-1'>"+
            "<label for='citation-suffix' style='display:block'>Suffix (optional)</label>"+
            "<input type='text' class='cdx-input' id='citation-suffix' placeholder='e.g., p. 42'/>"+
            "</div>"+
            "<div style='display: flex; justify-content: space-between; gap: 8px'>"+
            "<button id='citation-edit-entry' class='btn btn-sm btn-primary mt-1'>Edit entry</button>"+
            "<button id='citation-save' class='btn btn-sm btn-success mt-1'>Save</button>"+
            "<button id='citation-delete' class='btn btn-sm btn-danger mt-1'>Delete Citation</button>"+
            "<button id='citation-abort' class='btn btn-sm btn-secondary mt-1'>Cancel</button>"+
            "</div>" +
            "</div>";
        document.body.insertAdjacentHTML('afterbegin', settings_dialog_html);

        let settings_dialog: HTMLElement = document.getElementsByClassName('citation-settings')[0] as HTMLElement;

        let dest = citation.getBoundingClientRect();

        // Check if the dialog would be outside the viewport
        let dest_left ;
        if(dest.left + 300 > window.innerWidth){
            dest_left =  window.innerWidth - 300;
        }else if (dest.left < 0) {
            dest_left = 0;
        }else{
            dest_left = dest.left;
        }

        settings_dialog.style.left = dest_left + 'px';
        // Add 10px to the bottom of the citation
        settings_dialog.style.top = (dest.bottom + window.scrollY + 10) + 'px';

        document.getElementById('citation-abort')!.addEventListener('click', () => {
            settings_dialog.remove();
        });

        document.getElementById('citation-delete')!.addEventListener('click', async () => {
            const parent = citation.parentElement;
            citation.remove();
            settings_dialog.remove();
            if (parent) {
                parent.dispatchEvent(new Event('input', { bubbles: true }));
            }
        });

        // Load and display entry title; wire the Edit button to open preview editor
        (async () => {
            try{
                const bibAPI = BibliographyAPI();
                const entry: BibEntryOrFolder = await bibAPI.get_bibliography_entry(state.project_id, key);
                const titleEl = document.getElementById('citation-entry-title')!;
                let displayTitle = key;
                if ('BibEntry' in entry) {
                    // @ts-ignore
                    displayTitle = entry.BibEntry.title?.value || key;
                }
                titleEl.textContent = displayTitle;

                const editBtn = document.getElementById('citation-edit-entry')!;
                editBtn.addEventListener('click', async () => {
                    await openEntryEditorInPreview(key);
                    settings_dialog.remove();
                });

                // Prefill prefix/suffix inputs from citation element
                const prefixInput = document.getElementById('citation-prefix') as HTMLInputElement;
                const suffixInput = document.getElementById('citation-suffix') as HTMLInputElement;
                if(prefixInput){ prefixInput.value = citation.getAttribute('data-prefix') || ''; }
                if(suffixInput){ suffixInput.value = citation.getAttribute('data-suffix') || ''; }

                // Save handler: persist attributes to element
                const saveBtn = document.getElementById('citation-save');
                if(saveBtn){
                    saveBtn.addEventListener('click', () => {
                        const parent = citation.parentElement;
                        const pref = prefixInput?.value?.trim() || '';
                        const suff = suffixInput?.value?.trim() || '';
                        if(pref){ citation.setAttribute('data-prefix', pref); } else { citation.removeAttribute('data-prefix'); }
                        if(suff){ citation.setAttribute('data-suffix', suff); } else { citation.removeAttribute('data-suffix'); }
                        settings_dialog.remove();
                        if (parent) {
                            parent.dispatchEvent(new Event('input', { bubbles: true }));
                        }
                    });
                }
            }catch(err){
                const titleEl = document.getElementById('citation-entry-title');
                if(titleEl){ titleEl.textContent = `Entry ${key}`; }
                console.error('Failed to load bibliography entry for citation', err);
            }
        })();
    }

    render(){
        this.button = document.createElement('button');
        this.button.type = 'button';
        this.button.textContent = 'Cite';
        this.button.classList.add("ce-inline-tool");

        return this.button;
    }

    show_note_settings(range: Range){
        if(document.getElementsByClassName('citation-settings').length > 0){
            return;
        }

        let settings_dialog_html = "" +
            "<div class='citation-settings'>" +
            "<label>Add new Citation:</label>" +
            "<div class='mt-1'>"+
            "<label for='citation-prefix' style='display:block'>Prefix (optional)</label>"+
            "<input type='text' class='cdx-input' id='citation-prefix' placeholder='e.g., see also'>"+
            "</div>"+
            "<div class='mt-1'>"+
            "<label for='citation-suffix' style='display:block'>Suffix (optional)</label>"+
            "<input type='text' class='cdx-input' id='citation-suffix' placeholder='e.g., p. 42'>"+
            "</div>"+
            "<input type='text' class='cdx-input' id='citation-search' placeholder='Search title, author or editor'>"+
            "<div id='citation-search-res' class='hide'><ul id='citation-search-res-ul'></ul></div>"+
            "<div style='display: flex; justify-content: space-between'><button id='citation-abort' class='btn btn-sm btn-secondary mt-1'>Cancel</button></div>" +
            "</div>";
        document.body.insertAdjacentHTML('afterbegin', settings_dialog_html);

        let settings_dialog: HTMLElement = document.getElementsByClassName('citation-settings')[0] as HTMLElement;
        
        let rect = range.getBoundingClientRect();

        // Check if the dialog would be outside the viewport
        let dest_left ;
        if(rect.left + 300 > window.innerWidth){
            dest_left =  window.innerWidth - 300;
        }else if (rect.left < 0) {
            dest_left = 0;
        }else{
            dest_left = rect.left;
        }

        settings_dialog.style.left = dest_left + 'px';
        // Add 10px to the bottom of the range
        settings_dialog.style.top = (rect.bottom + window.scrollY + 10) + 'px';

        let send_search = this.send_search;
        let search_input = <HTMLInputElement>document.getElementById('citation-search');

        let search_handler = async function(){
            // @ts-ignore
            let search_res = await send_search(search_input.value, state.project_id);
            console.log(search_res);
            let search_res_div = document.getElementById("citation-search-res")!;
            let search_res_ul = document.getElementById("citation-search-res-ul")!;

            search_res_ul.innerHTML = "";
            search_res_div.classList.remove("hide");
            // Limit client-side as a safeguard, even though backend also limits
            const items = (search_res.data || []).slice(0, 5);
            for(let entry of items){
                // entry has: id, entry_type, title
                let safeTitle = (entry.title || "").replace(/</g, "&lt;").replace(/>/g, "&gt;");
                let li = "<li data-key='"+entry.id+"' class='citation-search-res-li'>["+entry.entry_type+"] "+safeTitle+"</li>"
                search_res_ul.innerHTML += li;
            }

            // @ts-ignore
            for(let entry of document.getElementsByClassName("citation-search-res-li")){
                entry.addEventListener("click", function(e: Event){
                    let key = (<HTMLElement>e.target).getAttribute("data-key");
                    let citeentry = document.createElement("citation");
                    citeentry.innerText = "C";
                    citeentry.setAttribute("data-key", key || "");
                    // Read optional prefix/suffix from inputs
                    const prefixInput = document.getElementById('citation-prefix') as HTMLInputElement | null;
                    const suffixInput = document.getElementById('citation-suffix') as HTMLInputElement | null;
                    const pref = prefixInput?.value?.trim() || '';
                    const suff = suffixInput?.value?.trim() || '';
                    if(pref){ citeentry.setAttribute('data-prefix', pref); }
                    if(suff){ citeentry.setAttribute('data-suffix', suff); }
                    citeentry.addEventListener("click", CitationTool.show_note_settings_editor);
                    range.collapse(false);
                    range.insertNode(citeentry);
                    settings_dialog.remove();
                    
                    // Trigger EditorJS change
                    citeentry.dispatchEvent(new Event('input', { bubbles: true }));
                });
            }
        }

        search_input.addEventListener('input', search_handler);

        document.getElementById('citation-abort')!.addEventListener('click', () => {
            settings_dialog.remove();
        });
    }

    surround(range: Range){
        if (this.state) {
            return;
        }
        this.show_note_settings(range)
    }

    checkState(selection: any) {
        const text = selection.anchorNode;

        if (!text) {
            return;
        }

        const anchorElement = text instanceof Element ? text : text.parentElement;

        this.state = !!anchorElement.closest('citation');
    }

    static get sanitize() {
        return {
            citation: function(el : any){
                return true;
            }
        };
    }

    async send_search(query: string, project_id: string) {
        const url = `/api/project/${project_id}/bibliography/search?q=${encodeURIComponent(query)}`;
        const response = await fetch(url, {
            method: 'GET',
            headers: {
                'Content-Type': 'application/json'
            }

        });
        if (!response.ok) {
            throw new Error(`Failed to search for bib entries: ${response.status}`);
        } else {
            let response_data = await response.json();
            if (response_data.hasOwnProperty("error")) {
                throw new Error(`Failed to search for bib entries: ` + Object.keys(response_data["error"])[0] + " " + Object.values(response_data["error"])[0]);
            } else {
                return response_data;
            }
        }
    }
}

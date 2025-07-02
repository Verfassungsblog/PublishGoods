import {save_changes} from "./Editor";

export class CitationToolOld {
    private button: HTMLButtonElement;
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

        CitationToolOld.add_all_show_note_settings_listeners();
    }

    static add_all_show_note_settings_listeners(){
        let notes = document.getElementsByTagName("citation");
        for(let i = 0; i < notes.length; i++){
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
        let toolbar = document.getElementsByClassName('ce-inline-toolbar')[0] as HTMLElement;

        let settings_dialog_html = "" +
            "<div class='citation-settings'>" +
            "<label>Modify Citation:</label><br>" +
            "<span>Key: "+citation.getAttribute("data-key")+"</span><br>"+
            "<div style='display: flex; justify-content: space-between'><button id='citation-delete' class='btn btn-sm btn-danger mt-1'>Delete Citation</button><button id='citation-abort' class='btn btn-sm btn-secondary mt-1'>Cancel</button></div>" +
            "</div>";
        document.body.insertAdjacentHTML('afterbegin', settings_dialog_html);

        let settings_dialog: HTMLElement = document.getElementsByClassName('citation-settings')[0] as HTMLElement;

        let dest = citation.getBoundingClientRect();

        // Check if the dialog would be outside the viewport
        let dest_left ;
        if(dest.left + settings_dialog.offsetWidth > window.innerWidth){
            dest_left =  window.innerWidth - settings_dialog.offsetWidth;
        }else{
            dest_left = dest.left;
        }

        settings_dialog.style.left = dest_left + 'px';
        // Add the same position as the toolbar but add 40px to the top
        settings_dialog.style.top = (dest.bottom + 40) + 'px';

        document.getElementById('citation-abort').addEventListener('click', () => {
            settings_dialog.remove();
        });

        document.getElementById('citation-delete').addEventListener('click', async () => {
            citation.remove();
            settings_dialog.remove();
            save_changes().then();
        });
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
        let toolbar = document.getElementsByClassName('ce-inline-toolbar')[0] as HTMLElement;

        let settings_dialog_html = "" +
            "<div class='citation-settings'>" +
            "<label>Add new Citation:</label>" +
            "<input type='text' class='cdx-input' id='citation-search' placeholder='Search Citation Key'>"+
            "<div id='citation-search-res' class='hide'><ul id='citation-search-res-ul'></ul></div>"+
            "<div style='display: flex; justify-content: space-between'><button id='citation-abort' class='btn btn-sm btn-secondary mt-1'>Cancel</button></div>" +
            "</div>";
        toolbar.insertAdjacentHTML('afterend', settings_dialog_html);

        let settings_dialog: HTMLElement = toolbar.parentElement.querySelector('.citation-settings');
        settings_dialog.style.left = toolbar.style.left;
        // Add the same position as the toolbar but add 40px to the top
        let currentTop = parseInt(toolbar.style.top, 10);
        settings_dialog.style.top = (currentTop + 40) + 'px';

        let send_search = this.send_search;
        let search_input = <HTMLInputElement>document.getElementById('citation-search');

        let search_handler = async function(){
            // @ts-ignore
            let search_res = await send_search(search_input.value, globalThis.project_id);
            console.log(search_res);
            let search_res_div = document.getElementById("citation-search-res");
            let search_res_ul = document.getElementById("citation-search-res-ul");

            search_res_ul.innerHTML = "";
            search_res_div.classList.remove("hide");
            for(let entry of search_res.data){
                let li = "<li data-key='"+entry.key+"' class='citation-search-res-li'>["+entry.entry_type+"] "+entry.key+"</li>"
                search_res_ul.innerHTML += li;
            }

            // @ts-ignore
            for(let entry of document.getElementsByClassName("citation-search-res-li")){
                entry.addEventListener("click", function(e: Event){
                    let key = (<HTMLElement>e.target).getAttribute("data-key");
                    let citeentry = document.createElement("citation");
                    citeentry.innerText = "C";
                    citeentry.setAttribute("data-key", key);
                    citeentry.addEventListener("click", CitationToolOld.show_note_settings_editor);
                    range.collapse(false);
                    range.insertNode(citeentry);
                    settings_dialog.remove();
                    save_changes().then();
                });
            }
        }

        search_input.addEventListener('input', search_handler);

        document.getElementById('citation-abort').addEventListener('click', () => {
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
        const response = await fetch(`/api/projects/` + project_id + `/bibliography/search?query=` + query, {
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
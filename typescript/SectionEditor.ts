import * as Tools from "./tools";
import {
    PatchSection,
    PatchSectionMetadata,
    PersonsAPI,
    SectionAPI,
    PersonUuidOrString
} from "./api_requests"
import {show_editor} from "./Editor";

//TODO: Use a State Model which always reflects the current section data + dirty flags

let typing_timer : number|null;
let section_api = SectionAPI();
let person_api = PersonsAPI();
let dragged_element: HTMLElement | null;
let section_data: any;

export async function load_section_view(){
    // Get section data
    await update_section_data();

    show_section_view();

    await show_editor();
}

export function show_section_view(){
    // @ts-ignore
    document.getElementsByClassName("editor-details")[0].innerHTML = Handlebars.templates.editor_section_view(section_data);

    // Add event listeners:
    document.getElementById("section_show_metadata").addEventListener("click", expand_metadata);
    document.getElementById("section_hide_metadata").addEventListener("click", collapse_metadata);
    add_authors_editors_listeners();
    add_search_listeners();
    add_remove_identifier_listeners();
    document.getElementById("section_metadata_identifiers_add").addEventListener("click", add_identifier_listener);
    document.getElementById("section_delete").addEventListener("click", async function(){
        if(confirm("Do you really want to delete this section?")){
            try {
                Tools.start_loading_spinner();
                // @ts-ignore
                await section_api.delete_section(globalThis.project_id, globalThis.section_path);
                // @ts-ignore
                window.location.href = "/projects/" + globalThis.project_id;
            }catch(e){
                Tools.show_alert("Error deleting section: " + e, "danger");
                Tools.stop_loading_spinner();
            }
        }
    });

    let quickchange_elements = document.getElementsByClassName("quickchange");
    for(let element of Array.from(quickchange_elements)){
        element.addEventListener("input", quickchange_handler);
    }
}

let add_remove_identifier_listeners = function(){
    let remove_identifier_listener = function(e: Event){
        let target = e.target as HTMLElement;
        let identifier_row = target.closest(".section_metadata_identifier_row") as HTMLElement;

        identifier_row.remove();
        metadata_change_handler().then();
    };

    let remove_identifier_buttons = Array.from(document.getElementsByClassName("section_metadata_identifier_remove_btn")) as HTMLElement[];
    for(let button of remove_identifier_buttons){
        button.addEventListener("click", remove_identifier_listener);
    }
}

let add_identifier_listener = function(){
    let identifier_type_select = document.getElementById("section_metadata_identifiers_type") as HTMLSelectElement;
    let identifier_name_input = document.getElementById("section_metadata_identifiers_name") as HTMLInputElement;
    let identifier_value_input = document.getElementById("section_metadata_identifiers_value") as HTMLInputElement;


    let id_type : string|null = identifier_type_select.value || null;
    let id_name: string|null = identifier_name_input.value || null;
    let id_value : string|null = identifier_value_input.value || null;

    if(!id_type || !id_name || !id_value){
        Tools.show_alert("Please fill out all fields to add an identifier.", "warning");
        return;
    }

    let identifier_data = {
        identifier_type: id_type,
        name: id_name,
        value: id_value
    };

    let identifiers_div = document.getElementById("section_metadata_identifiers_list") as HTMLElement;
    // @ts-ignore
    identifiers_div.insertAdjacentHTML("beforeend", Handlebars.templates.editor_section_identifier_row(identifier_data));
    metadata_change_handler().then();

    // Clear input fields
    identifier_name_input.value = "";
    identifier_value_input.value = "";

    // Add remove listener to new identifier
    add_remove_identifier_listeners();
}

let add_search_listeners = function () {
    // Add search listeners:
    // Add search for authors
    let authors_searchbar = document.getElementById("section_metadata_search_authors") as HTMLInputElement;
    let search_result_overlay = document.getElementById("section_metadata_search_authors_results") as HTMLElement;

    let author_search_callback = async function (selected: HTMLElement) {
        let author_to_add: PersonUuidOrString = {PersonUuid: selected.getAttribute("data-person-id")};
        if (section_data.metadata.authors.includes(author_to_add)) {
            Tools.show_alert("Author already added.", "warning");
            return;
        }

        try {
            Tools.start_loading_spinner();
            let person_data = await person_api.send_get_person_request(author_to_add.PersonUuid);
            Tools.stop_loading_spinner();

            let authors_div = document.getElementById("section_metadata_authors_div") as HTMLElement;
            // @ts-ignore
            authors_div.insertAdjacentHTML("beforeend", Handlebars.templates.editor_section_authors_li({Person: person_data}));
            await metadata_change_handler();
            add_authors_editors_listeners();
        } catch (e) {
            Tools.show_alert("Error adding author: " + e, "danger");
            Tools.stop_loading_spinner();
        }
    }
    // @ts-ignore
    Tools.add_search(authors_searchbar, search_result_overlay, person_api.send_search_person_request, Handlebars.templates.search_person_li, author_search_callback);

    let search_input = document.getElementById("section_metadata_search_authors") as HTMLInputElement;

    // Add Person as NameString if Enter pressed in search bar
    search_input.addEventListener("keydown", async function (e) {
        if (e.key !== "Enter") {
            return;
        }

        let name_to_add: PersonUuidOrString = {NameString: (search_input.value)};
        search_input.value = "";

        let authors_div = document.getElementById("section_metadata_authors_div") as HTMLElement;
        // @ts-ignore
        authors_div.insertAdjacentHTML("beforeend", Handlebars.templates.editor_section_authors_li(name_to_add));

        try {
            Tools.start_loading_spinner();
            await metadata_change_handler();
            add_authors_editors_listeners();
        } catch (e) {
            Tools.show_alert("Error adding author name: " + e.toString(), "danger");
        }
        Tools.stop_loading_spinner();
    });

    // Add search for editors
    let editors_searchbar = document.getElementById("section_metadata_search_editors") as HTMLInputElement;
    let editors_search_result_overlay = document.getElementById("section_metadata_search_editors_results") as HTMLElement;

    let editor_search_callback = async function (selected: HTMLElement) {
        let editor_to_add: PersonUuidOrString = {PersonUuid: selected.getAttribute("data-person-id")};
        if (section_data.metadata.editors.includes(editor_to_add)) {
            Tools.show_alert("Editor already added.", "warning");
            return;
        }

        try {
            Tools.start_loading_spinner();
            let person_data = await person_api.send_get_person_request(editor_to_add.PersonUuid);
            Tools.stop_loading_spinner();

            let editors_div = document.getElementById("section_metadata_editors_div") as HTMLElement;
            // @ts-ignore
            editors_div.insertAdjacentHTML("beforeend", Handlebars.templates.editor_section_editors_li({Person: person_data}));
            await metadata_change_handler();
            add_authors_editors_listeners();
        } catch (e) {
            Tools.show_alert("Error adding editor: " + e, "danger");
            Tools.stop_loading_spinner();
        }
    }

    // @ts-ignore
    Tools.add_search(editors_searchbar, editors_search_result_overlay, person_api.send_search_person_request, Handlebars.templates.search_person_li, editor_search_callback);

    let editors_search_input = document.getElementById("section_metadata_search_editors") as HTMLInputElement;

    // Add Person as NameString if Enter pressed in search bar
    editors_search_input.addEventListener("keydown", async function (e) {
        if (e.key !== "Enter") {
            return;
        }

        let name_to_add: PersonUuidOrString = {NameString: (editors_search_input.value)};
        editors_search_input.value = "";

        let editors_div = document.getElementById("section_metadata_editors_div") as HTMLElement;
        // @ts-ignore
        editors_div.insertAdjacentHTML("beforeend", Handlebars.templates.editor_section_editors_li(name_to_add));

        try {
            Tools.start_loading_spinner();
            await metadata_change_handler();
            add_authors_editors_listeners();
        } catch (e) {
            Tools.show_alert("Error adding editor name: " + e.toString(), "danger");
        }
        Tools.stop_loading_spinner();
    });
}

let quickchange_handler = async function(){
    if (typing_timer) {
        clearTimeout(typing_timer);
    }

    // Set a timeout to wait for the user to stop typing
    // @ts-ignore
    typing_timer = setTimeout(async function(){
        await metadata_change_handler();
    }, 1000);
}

let metadata_change_handler = async function () {
    let metadata: PatchSectionMetadata = {};

    let identifiers: any[] = [];
    // Vermeide @ts-ignore und überprüfe Typen korrekt
    let identifierRows = Array.from(document.getElementsByClassName("section_metadata_identifier_row")) as HTMLElement[];

    for (let identifier_row of identifierRows) {
        let identifier_id = identifier_row.getAttribute("data-identifier-id") || null;
        let identifier_type = identifier_row.getAttribute("data-identifier-type");

        let identifier_name = (identifier_row.querySelector(".section_metadata_identifier_name") as HTMLInputElement)?.value;
        let identifier_value = (identifier_row.querySelector(".section_metadata_identifier_value") as HTMLInputElement)?.value;

        identifiers.push({
            id: identifier_id,
            identifier_type: identifier_type,
            name: identifier_name,
            value: identifier_value
        });
    }

    // Deep comparison to avoid direct array reference check
    if (!deepEqual(identifiers, section_data.metadata.identifiers)) {
        metadata.identifiers = identifiers;
    }

    // Get and compare other metadata fields
    let lang = (document.getElementById("section_metadata_lang") as HTMLInputElement)?.value || null;
    if (lang === "none") lang = null;
    if (lang !== section_data.metadata.lang) {
        metadata.lang = lang;
    }

    let published = (document.getElementById("section_metadata_published") as HTMLInputElement)?.value || null;
    if (published !== section_data.metadata.published) {
        metadata.published = published;
    }

    let title = document.getElementById("section_metadata_title")?.innerText || null;
    if (title !== section_data.metadata.title) {
        metadata.title = title;
    }

    let toc_title_subtitle_override = (document.getElementById("section_metadata_toc_title_subtitle_override") as HTMLInputElement)?.value || null;
    if (toc_title_subtitle_override !== section_data.metadata.toc_title_subtitle_override) {
        metadata.toc_title_subtitle_override = toc_title_subtitle_override;
    }

    let subtitle = document.getElementById("section_metadata_subtitle")?.innerText || null;
    if (subtitle !== section_data.metadata.subtitle) {
        metadata.subtitle = subtitle;
    }

    let web_url = (document.getElementById("section_metadata_web_url") as HTMLInputElement)?.value || null;
    if (web_url !== section_data.metadata.web_url) {
        metadata.web_url = web_url;
    }

    let authors : PersonUuidOrString[] = [];
    for(let author_div of Array.from(document.getElementsByClassName("section_metadata_authors_div"))){
        if(author_div.getAttribute("data-entry-type") === "Person"){
            const id = author_div.getAttribute("data-id");
            if(id !== null) {
                authors.push({PersonUuid: id});
            }
        }else if(author_div.getAttribute("data-entry-type") === "NameString"){
            let name = author_div.getAttribute("data-name");
            authors.push({NameString: name})
        }
    }
    if (authors !== section_data.metadata.authors){
        metadata.authors = authors;
    }

    let editors: PersonUuidOrString[] = [];
    for (let editor_div of Array.from(document.getElementsByClassName("section_metadata_editors_div"))) {
        if (editor_div.getAttribute("data-entry-type") === "Person") {
            const id = editor_div.getAttribute("data-id");
            if (id !== null) {
                editors.push({PersonUuid: id});
            }
        } else if (editor_div.getAttribute("data-entry-type") === "NameString") {
            let name = editor_div.getAttribute("data-name");
            editors.push({NameString: name})
        }
    }
    if (editors !== section_data.metadata.editors) {
        metadata.editors = editors;
    }

    let patch_data: PatchSection = { metadata: metadata };

    Tools.start_loading_spinner();
    try {
        // @ts-ignore
        await section_api.patch_section(globalThis.project_id, globalThis.section_path, patch_data);
        await update_section_data();
    } catch (error) {
        Tools.show_alert(error, "danger");
    } finally {
        Tools.stop_loading_spinner();
    }
};

async function update_section_data(){
    try {
        Tools.start_loading_spinner();

        // @ts-ignore
        section_data = await section_api.read_section(globalThis.project_id, globalThis.section_path, true, true, false);
        if (section_data["metadata"]["lang"] !== null) {
            section_data["metadata"]["langval"] = {};
            // Add the language to the langval object, so that the language is selected in the dropdown
            section_data["metadata"]["langval"][section_data["metadata"]["lang"]] = true;
        }
        Tools.stop_loading_spinner();
    }catch(e) {
        Tools.show_alert("Couldn't get section data: " + e, "danger");
        Tools.stop_loading_spinner();
    }
}

function deepEqual(obj1: any, obj2: any): boolean {
    // Prüfen, ob beide Werte primitive Datentypen sind (string, number, boolean, null, undefined)
    if (obj1 === obj2) return true;

    // Prüfen, ob beide Werte Objekte oder Arrays sind und nicht null
    if (typeof obj1 !== 'object' || obj1 === null || typeof obj2 !== 'object' || obj2 === null) {
        return false;
    }

    // Wenn es sich um Arrays handelt, vergleiche die Arrays rekursiv
    if (Array.isArray(obj1) && Array.isArray(obj2)) {
        if (obj1.length !== obj2.length) return false;
        for (let i = 0; i < obj1.length; i++) {
            if (!deepEqual(obj1[i], obj2[i])) return false;
        }
        return true;
    }

    // Wenn es sich um Objekte handelt, vergleiche die Objektschlüssel und deren Werte
    const keys1 = Object.keys(obj1);
    const keys2 = Object.keys(obj2);

    if (keys1.length !== keys2.length) return false;

    for (let key of keys1) {
        if (!keys2.includes(key) || !deepEqual(obj1[key], obj2[key])) {
            return false;
        }
    }

    return true;
}

let collapse_metadata = function(){
    document.getElementsByClassName("editor_section_view_metadata")[0].classList.add("hide");
    document.getElementsByClassName("editor_section_view_collapsed_metadata")[0].classList.remove("hide");
}

let expand_metadata = function(){
    document.getElementsByClassName("editor_section_view_collapsed_metadata")[0].classList.add("hide");
    document.getElementsByClassName("editor_section_view_metadata")[0].classList.remove("hide");
}

function addDragAndDropListeners(dragElements: HTMLElement[], dropZones: HTMLElement[], allowedGroup: string) {
    for (let element of dragElements) {
        element.addEventListener("dragstart", function (e) {
            dragged_element = e.target as HTMLElement;
        });

        element.addEventListener("dragend", function () {
            dragged_element = null;
        });
    }

    for (let dropzone of dropZones) {
        dropzone.addEventListener("dragenter", function (e) {
            let target = e.target as HTMLElement;

            // Dont show drop opportunity for first dropzone for first element
            if(target.classList.contains("first_dropzone")){
                let first_element = dragged_element.parentElement.children[1];
                if(first_element.getAttribute("data-id") === dragged_element.getAttribute("data-id")){
                    return;
                }
            }

            // Überprüfe, ob das gezogene Element zur richtigen Gruppe gehört
            if (dragged_element && dragged_element.getAttribute("data-group") === allowedGroup && dragged_element.getAttribute("data-id") !== dropzone.getAttribute("data-dropzone-after")) {
                target.classList.add("dragover");
            }
        });

        dropzone.addEventListener("dragleave", function (e) {
            let target = e.target as HTMLElement;
            target.classList.remove("dragover");
        });

        dropzone.addEventListener("dragover", function (e) {
            e.preventDefault();
        });

        dropzone.addEventListener("drop", function (e) {
            let target = e.target as HTMLElement;

            // Überprüfe, ob das gezogene Element zur richtigen Gruppe gehört
            if (dragged_element && dragged_element.getAttribute("data-group") === allowedGroup) {
                let dragged_element_id = dragged_element.getAttribute("data-id");
                let dropzone_id = target.getAttribute("data-dropzone-after");

                // Wenn das Element in seine eigene Dropzone verschoben wird, nichts tun
                if (dragged_element_id === dropzone_id) {
                    return;
                }

                if(target.classList.contains("first_dropzone")){
                    // Dont do anything if first element moved to first dropzone
                    let first_element = dragged_element.parentElement.children[1];
                    if(first_element.getAttribute("data-id") === dragged_element_id){
                        return;
                    }

                    target.classList.remove("dragover");
                    dragged_element.parentNode.removeChild(dragged_element);
                    target.insertAdjacentElement('afterend', dragged_element);
                }else{
                    target.classList.remove("dragover");
                    dragged_element.parentNode.removeChild(dragged_element);
                    target.parentElement.insertAdjacentElement('afterend', dragged_element);
                }

                metadata_change_handler().then();
            }
        });
    }
}

let add_authors_editors_listeners = function(){
    let authors_divs = Array.from(document.getElementsByClassName("section_metadata_authors_div")) as HTMLElement[];
    let authors_dropzones = Array.from(document.getElementsByClassName("section_metadata_authors_div_after")) as HTMLElement[];
    authors_dropzones.push(document.getElementById("section_metadata_authors_first_dropzone") as HTMLElement);

    let editors_divs = Array.from(document.getElementsByClassName("section_metadata_editors_div")) as HTMLElement[];
    let editors_dropzones = Array.from(document.getElementsByClassName("section_metadata_editors_div_after")) as HTMLElement[];
    editors_dropzones.push(document.getElementById("section_metadata_editors_first_dropzone") as HTMLElement);

    // Drag-and-Drop für Autoren
    addDragAndDropListeners(authors_divs, authors_dropzones, "authors");

    // Drag-and-Drop für Editoren
    addDragAndDropListeners(editors_divs, editors_dropzones, "editors");

    // Add remove listener for authors
    let author_remove_listener = function(e: Event){
        let target = e.target as HTMLElement;
        let author_div = target.closest(".section_metadata_authors_div") as HTMLElement;

        author_div.remove();
        metadata_change_handler().then();

    };
    let authors_rm_buttons = Array.from(document.getElementsByClassName("section_metadata_authors_remove")) as HTMLElement[];
    for(let button of authors_rm_buttons){
        button.addEventListener("click", author_remove_listener);
    }


    // Add remove listener for editors
    let editor_remove_listener = function(e: Event){
        let target = e.target as HTMLElement;
        let editor_div = target.closest(".section_metadata_editors_div") as HTMLElement;

        editor_div.remove();
        metadata_change_handler().then();
    };
    let editors_rm_buttons = Array.from(document.getElementsByClassName("section_metadata_editors_remove")) as HTMLElement[];
    for(let button of editors_rm_buttons){
        button.addEventListener("click", editor_remove_listener);
    }
}

window.addEventListener("load", async function(){
    // @ts-ignore
    window.show_section_view = () => {load_section_view()};
});
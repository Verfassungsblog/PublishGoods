import {APIProjectData, EditorAPI, PersonsAPI, PersonUuidOrString} from "../api_requests";
import {init, main_col} from "./Editor";
import {state} from "./Main";
import {add_search, show_alert} from "../tools";

/**
 * Represents the primary interface for interacting with the editor.
 * Provides methods and properties to manipulate editor content,
 * manage editor state, and interact with editor-specific events and configurations.
 */
const editorApi = EditorAPI();
const personsApi = PersonsAPI();

/**
 * Represents the data to be used for patching or updating a resource.
 * This object typically contains key-value pairs where the*/
let patch_data : any = {};
/**
 * Represents the date or version number of the last patch applied.
 * This variable can hold a numeric representation of the patch
 * version or `null` if no patch has been applied.
 *
 * @type {(number|null)}
 */
let last_patch : number|null = null;
/**
 * Represents the timeout identifier for a save operation.
 * This variable is used to store the reference to a NodeJS timeout.
 * It can either hold a NodeJS.Timeout object when a timeout is set
 * or null when no timeout is currently active.
 */
let save_timeout : NodeJS.Timeout | null = null;

let dragged_editor_element: HTMLElement | null = null;

function prepareIdentifiersForTemplate(identifiers: any[]){
    return identifiers.map(identifier => {
        const rawType = identifier.identifier_type;

        let typeKey: string | undefined = undefined;
        let otherValue = "";

        if(typeof rawType === "string"){
            typeKey = rawType;
        }else if(rawType && typeof rawType === "object"){
            const entries = Object.entries(rawType);
            if(entries.length){
                typeKey = entries[0][0];
                otherValue = typeKey === "Other" ? (entries[0][1] as string || "") : "";
            }
        }

        const typeFlag = typeKey
            ? {[typeKey]: typeKey === "Other" ? (otherValue || "") : true}
            : {};

        return {
            ...identifier,
            ...typeFlag,
            name: identifier.name ?? (typeKey === "Other" ? otherValue : typeKey || ""),
            value: identifier.value ?? ""
        };
    });
}

/**
 * Renders the project metadata settings view using the provided project data.
 *
 * @param {APIProjectData} data - The project data object containing metadata to be displayed.
 * @return {void} This function does not return a value.
 */
export async function show_project_metadata_settings(data: APIProjectData) {
    const preparedIdentifiers = prepareIdentifiersForTemplate(data.metadata?.identifiers || []);

    // @ts-ignore
    main_col.innerHTML = Handlebars.templates.editor_project_metadata_settings({
        ...data,
        metadata: {
            ...data.metadata,
            identifiers: preparedIdentifiers
        }
    });

    init_autopatch();
    // Add cover / backcover upload listeners
    add_cover_listeners();

    // Enable drag & drop + remove for project metadata authors & editors
    add_authors_listeners();
    add_editors_listeners();

    // Enable identifiers editing
    init_identifiers_listeners();

    // Enable search + add for project metadata authors & editors
    init_person_search();
}

function init_person_search(){
    const authors_searchbar = document.getElementById("project_metadata_search_authors") as HTMLInputElement | null;
    const authors_results = document.getElementById("project_metadata_search_authors_results") as HTMLElement | null;

    if(authors_searchbar && authors_results){
        const on_author_selected = async (selected: HTMLElement) => {
            const person_id = selected.getAttribute("data-person-id");
            if(!person_id){
                return;
            }

            const existing = document.querySelector<HTMLElement>(`.metadata_editors_div[data-group='authors'][data-id='${person_id}']`);
            if(existing){
                show_alert("Author already added.", "warning");
                return;
            }

            try{
                const person = await personsApi.send_get_person_request(person_id);
                const authors_div = document.getElementById("metadata_authors_div") as HTMLElement | null;
                if(!authors_div){
                    return;
                }

                // @ts-ignore
                authors_div.insertAdjacentHTML("beforeend", Handlebars.templates.editor_project_metadata_settings_person_li({
                    Person: person,
                    group: "authors"
                }));

                add_authors_listeners();
                await patch_authors_order();
            }catch (e){
                console.error("Failed to add author", e);
                show_alert("Couldn't add author.", "error");
            }
        };

        add_search(
            authors_searchbar,
            authors_results,
            personsApi.send_search_person_request,
            // @ts-ignore
            Handlebars.templates.search_person_li,
            on_author_selected
        );

        authors_searchbar.addEventListener("keydown", async function (e: KeyboardEvent){
            if(e.key !== "Enter"){
                return;
            }
            const value = authors_searchbar.value.trim();
            if(!value){
                return;
            }

            const authors_div = document.getElementById("metadata_authors_div") as HTMLElement | null;
            if(!authors_div){
                return;
            }

            authors_searchbar.value = "";

            // @ts-ignore
            authors_div.insertAdjacentHTML("beforeend", Handlebars.templates.editor_project_metadata_settings_person_li({
                NameString: value,
                group: "authors"
            }));

            add_authors_listeners();
            await patch_authors_order();
        });
    }

    const editors_searchbar = document.getElementById("section_metadata_search_editors") as HTMLInputElement | null;
    const editors_results = document.getElementById("section_metadata_search_editors_results") as HTMLElement | null;

    if(editors_searchbar && editors_results){
        const on_editor_selected = async (selected: HTMLElement) => {
            const person_id = selected.getAttribute("data-person-id");
            if(!person_id){
                return;
            }

            const existing = document.querySelector<HTMLElement>(`.metadata_editors_div[data-group='editors'][data-id='${person_id}']`);
            if(existing){
                show_alert("Editor already added.", "warning");
                return;
            }

            try{
                const person = await personsApi.send_get_person_request(person_id);
                const editors_div = document.getElementById("metadata_editors_div") as HTMLElement | null;
                if(!editors_div){
                    return;
                }

                // @ts-ignore
                editors_div.insertAdjacentHTML("beforeend", Handlebars.templates.editor_project_metadata_settings_person_li({
                    Person: person,
                    group: "editors"
                }));

                add_editors_listeners();
                await patch_editors_order();
            }catch (e){
                console.error("Failed to add editor", e);
                show_alert("Couldn't add editor.", "error");
            }
        };

        add_search(
            editors_searchbar,
            editors_results,
            personsApi.send_search_person_request,
            // @ts-ignore
            Handlebars.templates.search_person_li,
            on_editor_selected
        );

        editors_searchbar.addEventListener("keydown", async function (e: KeyboardEvent){
            if(e.key !== "Enter"){
                return;
            }
            const value = editors_searchbar.value.trim();
            if(!value){
                return;
            }

            const editors_div = document.getElementById("metadata_editors_div") as HTMLElement | null;
            if(!editors_div){
                return;
            }

            editors_searchbar.value = "";

            // @ts-ignore
            editors_div.insertAdjacentHTML("beforeend", Handlebars.templates.editor_project_metadata_settings_person_li({
                NameString: value,
                group: "editors"
            }));

            add_editors_listeners();
            await patch_editors_order();
        });
    }
}

function add_authors_listeners(){
    const authors_divs = Array.from(document.querySelectorAll<HTMLElement>(".metadata_editors_div[data-group='authors']"));
    const authors_dropzones = Array.from(document.getElementsByClassName("metadata_editors_div_after"))
        .filter(el => (el as HTMLElement).closest('#metadata_authors_div')) as HTMLElement[];

    const first_authors_dropzone = document.getElementById("metadata_authors_first_dropzone") as HTMLElement | null;
    if(first_authors_dropzone){
        authors_dropzones.push(first_authors_dropzone);
    }

    add_drag_and_drop_listeners(authors_divs, authors_dropzones, "authors");

    const authors_rm_buttons = Array.from(document.querySelectorAll<HTMLElement>("#metadata_authors_div .metadata_editors_remove"));
    const author_remove_listener = function(e: Event){
        const target = e.target as HTMLElement;
        const author_div = target.closest(".metadata_editors_div") as HTMLElement | null;
        if(!author_div){
            return;
        }
        author_div.remove();
        patch_authors_order().then();
    };

    for(const button of authors_rm_buttons){
        button.addEventListener("click", author_remove_listener);
    }
}

function add_editors_listeners(){
    const editors_divs = Array.from(document.querySelectorAll<HTMLElement>(".metadata_editors_div[data-group='editors']"));
    const editors_dropzones = Array.from(document.getElementsByClassName("metadata_editors_div_after"))
        .filter(el => (el as HTMLElement).closest('#metadata_editors_div')) as HTMLElement[];
    const first_dropzone = document.getElementById("metadata_editors_first_dropzone") as HTMLElement | null;
    if(first_dropzone){
        editors_dropzones.push(first_dropzone);
    }

    add_drag_and_drop_listeners(editors_divs, editors_dropzones, "editors");

    const editors_rm_buttons = Array.from(document.querySelectorAll<HTMLElement>("#metadata_editors_div .metadata_editors_remove"));
    const editor_remove_listener = function(e: Event){
        const target = e.target as HTMLElement;
        const editor_div = target.closest(".metadata_editors_div") as HTMLElement | null;
        if(!editor_div){
            return;
        }
        editor_div.remove();
        patch_editors_order().then();
    };

    for(const button of editors_rm_buttons){
        button.addEventListener("click", editor_remove_listener);
    }
}

function init_identifiers_listeners(){
    const typeSelect = document.getElementById("project_metadata_identifiers_type") as HTMLSelectElement | null;
    const otherInput = document.getElementById("project_metadata_identifiers_other") as HTMLInputElement | null;
    const nameInput = document.getElementById("project_metadata_identifiers_name") as HTMLInputElement | null;
    const valueInput = document.getElementById("project_metadata_identifiers_value") as HTMLInputElement | null;
    const addBtn = document.getElementById("project_metadata_identifiers_add") as HTMLButtonElement | null;
    const list = document.getElementById("project_metadata_identifiers_list") as HTMLElement | null;

    if(typeSelect && otherInput){
        typeSelect.addEventListener("change", function(){
            if(typeSelect.value === "Other"){
                otherInput.style.display = "block";
            }else{
                otherInput.style.display = "none";
                otherInput.value = "";
            }
        });
    }

    if(addBtn && list && typeSelect && nameInput && valueInput && otherInput){
        addBtn.addEventListener("click", async function(){
            const identifier_type = typeSelect.value;
            const name = nameInput.value.trim();
            const value = valueInput.value.trim();
            const other = otherInput.value.trim();

            if(identifier_type === "Other" && other === ""){
                show_alert("Please specify other identifier type.", "warning");
                return;
            }
            if(!value){
                show_alert("Identifier value is required.", "warning");
                return;
            }

            const identifierPayload: any = {
                id: null,
                name: name || (identifier_type === "Other" ? other : identifier_type),
                value: value,
                identifier_type: identifier_type === "Other" ? {Other: other} : {[identifier_type]: null}
            };

            const templateTypeFlag = identifier_type === "Other"
                ? {Other: other}
                : {[identifier_type]: true};

            // @ts-ignore
            list.insertAdjacentHTML("beforeend", Handlebars.templates.editor_project_metadata_identifier_row({
                id: identifierPayload.id,
                name: identifierPayload.name,
                value: identifierPayload.value,
                ...templateTypeFlag
            }));

            nameInput.value = "";
            valueInput.value = "";
            if(identifier_type === "Other"){
                otherInput.value = "";
                otherInput.style.display = "none";
                typeSelect.value = "DOI";
            }

            add_identifier_row_listeners(list.lastElementChild as HTMLElement);
            await patch_identifiers();
        });
    }

    const existingRows = Array.from(document.querySelectorAll<HTMLElement>(".project_metadata_identifier_row"));
    for(const row of existingRows){
        add_identifier_row_listeners(row);
    }
}

function add_identifier_row_listeners(row: HTMLElement){
    const removeBtn = row.querySelector<HTMLElement>(".project_metadata_identifier_remove_btn");
    const typeSelect = row.querySelector<HTMLSelectElement>(".project_metadata_identifier_type");
    const otherInput = row.querySelector<HTMLInputElement>(".project_metadata_identifier_other");
    const nameInput = row.querySelector<HTMLInputElement>(".project_metadata_identifier_name");
    const valueInput = row.querySelector<HTMLInputElement>(".project_metadata_identifier_value");

    if(typeSelect && otherInput){
        typeSelect.addEventListener("change", function(){
            if(typeSelect.value === "Other"){
                otherInput.classList.remove("hide");
            }else{
                otherInput.classList.add("hide");
                otherInput.value = "";
            }
            patch_identifiers().then();
        });
    }

    const inputs = [nameInput, valueInput, otherInput].filter(Boolean) as HTMLInputElement[];
    for(const input of inputs){
        input.addEventListener("input", function(){
            // debounce using existing patch batching
            patch_identifiers().then();
        });
    }

    if(removeBtn){
        removeBtn.addEventListener("click", function(){
            row.remove();
            patch_identifiers().then();
        });
    }
}

async function patch_identifiers(){
    const rows = Array.from(document.querySelectorAll<HTMLElement>(".project_metadata_identifier_row"));
    const identifiers: any[] = [];

    for(const row of rows){
        const idAttr = row.getAttribute("data-identifier-id");
        const typeSelect = row.querySelector<HTMLSelectElement>(".project_metadata_identifier_type");
        const otherInput = row.querySelector<HTMLInputElement>(".project_metadata_identifier_other");
        const nameInput = row.querySelector<HTMLInputElement>(".project_metadata_identifier_name");
        const valueInput = row.querySelector<HTMLInputElement>(".project_metadata_identifier_value");

        if(!typeSelect || !valueInput){
            continue;
        }

        const identifier_type_value = typeSelect.value;
        const other = otherInput?.value.trim() || "";
        const name = nameInput?.value.trim() || (identifier_type_value === "Other" ? other : identifier_type_value);
        const value = valueInput.value.trim();

        if(!value){
            continue; // skip empty rows
        }

        let identifier_type: any;
        if(identifier_type_value === "Other"){
            if(!other){
                continue;
            }
            identifier_type = {Other: other};
        }else{
            identifier_type = {[identifier_type_value]: null};
        }

        identifiers.push({
            id: idAttr && idAttr !== "null" && idAttr !== "undefined" ? idAttr : null,
            name,
            value,
            identifier_type
        });
    }

    try{
        await editorApi.patchProject(state.project_id, {metadata: {identifiers}});
    }catch (e){
        console.error("Failed to save identifiers", e);
        show_alert("Couldn't save identifiers.", "error");
    }
}

function add_drag_and_drop_listeners(dragElements: HTMLElement[], dropZones: HTMLElement[], allowedGroup: string){
    for (const element of dragElements) {
        element.addEventListener("dragstart", function (e) {
            // Always use the element the listener is attached to, not an inner SVG/button
            dragged_editor_element = e.currentTarget as HTMLElement;

            // Highlight eligible dropzones as soon as dragging starts
            // (but do NOT highlight the dragged element's own dropzone
            //  and avoid the first dropzone if the element is already first)
            const draggedId = dragged_editor_element.getAttribute("data-id");
            const parent = dragged_editor_element.parentElement;

            for (const dz of dropZones) {
                // Skip if we don't have an id for the dragged element
                if (!draggedId) {
                    dz.classList.add("dragactive");
                    continue;
                }

                // Don't highlight the first dropzone when dragging the first element
                if (dz.classList.contains("first_dropzone") && parent && parent.children.length > 1) {
                    const firstElement = parent.children[1] as HTMLElement;
                    if (firstElement.getAttribute("data-id") === draggedId) {
                        continue;
                    }
                }

                const afterId = dz.getAttribute("data-dropzone-after");

                // Don't highlight the dropzone that belongs to the dragged element itself
                if (afterId && afterId === draggedId) {
                    continue;
                }

                dz.classList.add("dragactive");
            }
        });

        element.addEventListener("dragend", function () {
            dragged_editor_element = null;

            // Remove drag highlight from all dropzones once dragging stops
            for (const dz of dropZones) {
                dz.classList.remove("dragactive");
            }
        });
    }

    for (const dropzone of dropZones) {
        dropzone.addEventListener("dragenter", function (e) {
            const target = e.target as HTMLElement;
            const zone = e.currentTarget as HTMLElement;

            if(!dragged_editor_element){
                return;
            }

            // Don't show drop opportunity for first dropzone for first element
            if(zone.classList.contains("first_dropzone")){
                const parent = dragged_editor_element.parentElement;
                if(parent && parent.children.length > 1){
                    const first_element = parent.children[1] as HTMLElement;
                    if(first_element.getAttribute("data-id") === dragged_editor_element.getAttribute("data-id")){
                        return;
                    }
                }
            }

            if (dragged_editor_element.getAttribute("data-group") === allowedGroup &&
                dragged_editor_element.getAttribute("data-id") !== zone.getAttribute("data-dropzone-after")) {
                zone.classList.add("dragover");
            }
        });

        dropzone.addEventListener("dragleave", function (e) {
            const zone = e.currentTarget as HTMLElement;
            zone.classList.remove("dragover");
        });

        dropzone.addEventListener("dragover", function (e) {
            e.preventDefault();
        });

        dropzone.addEventListener("drop", function (e) {
            const zone = e.currentTarget as HTMLElement;

            if (!dragged_editor_element || dragged_editor_element.getAttribute("data-group") !== allowedGroup) {
                return;
            }

            const dragged_element_id = dragged_editor_element.getAttribute("data-id");
            const dropzone_id = zone.getAttribute("data-dropzone-after");

            // If element is dropped into its own dropzone, nothing to do
            if (dragged_element_id === dropzone_id) {
                return;
            }

            if(zone.classList.contains("first_dropzone")){
                const parent = dragged_editor_element.parentElement;
                if(parent && parent.children.length > 1){
                    const first_element = parent.children[1] as HTMLElement;
                    if(first_element.getAttribute("data-id") === dragged_element_id){
                        return;
                    }
                }

                zone.classList.remove("dragover");
                dragged_editor_element.parentNode?.removeChild(dragged_editor_element);
                zone.insertAdjacentElement('afterend', dragged_editor_element);
            }else{
                zone.classList.remove("dragover");
                dragged_editor_element.parentNode?.removeChild(dragged_editor_element);
                zone.parentElement?.insertAdjacentElement('afterend', dragged_editor_element);
            }

            if(allowedGroup === 'editors'){
                patch_editors_order().then();
            }else if(allowedGroup === 'authors'){
                patch_authors_order().then();
            }
        });
    }
}

async function patch_editors_order(){
    const editors_divs = Array.from(document.querySelectorAll<HTMLElement>(".metadata_editors_div[data-group='editors']"));
    const editors: PersonUuidOrString[] = [];

    for(const editor_div of editors_divs){
        const entry_type = editor_div.getAttribute("data-entry-type");
        if(entry_type === "Person"){
            const id = editor_div.getAttribute("data-id");
            if(id !== null){
                editors.push({PersonUuid: id});
            }
        }else if(entry_type === "NameString"){
            const name = editor_div.getAttribute("data-name");
            if(name !== null){
                editors.push({NameString: name});
            }
        }
    }

    try{
        await editorApi.patchProject(state.project_id, {metadata: {editors}});
    }catch (e){
        console.error("Failed to save editors order", e);
        show_alert("Couldn't save editors order.", "error");
    }
}

async function patch_authors_order(){
    const authors_divs = Array.from(document.querySelectorAll<HTMLElement>(".metadata_editors_div[data-group='authors']"));
    const authors: PersonUuidOrString[] = [];

    for(const author_div of authors_divs){
        const entry_type = author_div.getAttribute("data-entry-type");
        if(entry_type === "Person"){
            const id = author_div.getAttribute("data-id");
            if(id !== null){
                authors.push({PersonUuid: id});
            }
        }else if(entry_type === "NameString"){
            const name = author_div.getAttribute("data-name");
            if(name !== null){
                authors.push({NameString: name});
            }
        }
    }

    try{
        await editorApi.patchProject(state.project_id, {metadata: {authors}});
    }catch (e){
        console.error("Failed to save authors order", e);
        show_alert("Couldn't save authors order.", "error");
    }
}

/**
 * Initializes autopatch functionality by attaching appropriate event listeners
 * to elements with the "autopatch" class. It processes elements to detect
 * the necessary attributes and configuration for automatic updates.
 *
 * Elements must include a "data-patch" attribute. Fails with an error message
 * in the console for elements missing this attribute.
 *
 * Event listeners are added based on element type and input type:
 * - For `input` elements with types such as `checkbox`, `radio`, `date`, or `datetime-local`,
 *   a "change" event listener is added.
 * - For other `input` elements, an "input" event listener is added.
 * - All other element types are currently unsupported (TODO: implement further support).
 *
 * @return {void} This function does not return a value.
 */
function init_autopatch(){
    let targets = document.getElementsByClassName("autopatch");
    for (let target of Array.from(targets)) {
        let patch_field = target.getAttribute("data-patch");
        if (!patch_field) {
            console.error("Element "+target.id+" has autopatch class but misses data-patch attribute.");
            continue;
        }

        // Get Element type
        if(target.tagName.toLowerCase() === "input"){
            let input_type = (target.getAttribute("type") || "text").toLowerCase();
            if(input_type === "checkbox" || input_type === "radio" || input_type === "date" || input_type === "datetime-local"){
                target.addEventListener("change", autopatch_listener);
            }else{
                target.addEventListener("input", autopatch_listener);
            }
        }else if (target.tagName.toLowerCase() === "select"){
            target.addEventListener("change", autopatch_listener);
        } else{
            //TODO: implement for textfields etc.
            console.error("Autopatch not implemented for tag "+target.tagName.toLowerCase());
        }
    }
}

/**
 * Listens for an event, extracts and processes data from the target element's "data-patch" attribute,
 * updates the global `patch_data` object, and triggers a request to send the updated data.
 *
 * @param {Event} e The event object triggered by the listener.
 * @return {Promise<void>} Returns a Promise that resolves when the patch request is initiated.
 */
async function autopatch_listener(e: Event){
    let target = e.target as HTMLElement;
    let patch_field = target.getAttribute("data-patch");
    let splitted = patch_field.split(".");
    let scope = splitted[0] || null;
    let field_name = splitted[1] || null;

    if(!scope || !field_name){
        console.error("Element "+target.id+" has invalid data-patch attribute.");
        return;
    }

    let value: any;
    if(target instanceof HTMLInputElement){
        let input_type = (target.getAttribute("type") || "text").toLowerCase();
        if(input_type === "checkbox"){ // Boolean
            value = target.checked;
        }else if(input_type === "number"){
            value = target.valueAsNumber
        }else{
            value = target.value;
        }
    }else if(target instanceof HTMLSelectElement){
        value = target.value;
    }
    else{
        value = target.innerHTML;
    }

    if (!patch_data[scope]) patch_data[scope] = {};

    patch_data[scope][field_name] = value;
    console.log(patch_data);
    request_patch().then();
}

/**
 * Handles a patch request by ensuring there is at least a 1-second interval
 * since the last request before invoking the `send_patch` method. If a
 * request is attempted within the cooldown period, it schedules the request
 * to be sent after the cooldown using a timeout.
 *
 * @return {Promise<void>} A promise that resolves after the patch request is sent
 *                         or a timeout is set. The promise waits for the `send_patch`
 *                         function to complete if it is invoked immediately.
 */
async function request_patch(){
    if (save_timeout) return;
    if(last_patch){
        if(Date.now() - last_patch < 1000){ // Do not set a new save timeout if there already is one waiting
            save_timeout = setTimeout(send_patch, 1000);
            return;
        }
    }

    // At least 1 second since last save or no save yet
    await send_patch();
}

/**
 * Sends a patch request to update project settings if there is data to send.
 * Ensures that the global patch data is cleared immediately after being moved to local scope.
 * Prevents empty objects from being sent. Logs an error if the request fails.
 *
 * @return {Promise<void>} Resolves when the patch request is successfully completed,
 * or immediately returns if there is no data to send.
 */
async function send_patch(){
    save_timeout = null;
    
    // Move data to local scope and clear global IMMEDIATELLY to prevent data loss
    // if user types while request is in flight.
    const data_to_send = patch_data;
    patch_data = {};

    // Don't send empty objects
    if (Object.keys(data_to_send).length === 0) return;

    try {
        await editorApi.patchProject(state.project_id, data_to_send);
        last_patch = Date.now();
    } catch (e) {
        console.error("Failed to save project settings", e);
        // Restore data in case of error to prevent data loss
        // Merge failed data back into patch_data, but allow current patch_data to take precedence (newer changes)
        for (const scope in data_to_send) {
            if (!patch_data[scope]) {
                patch_data[scope] = data_to_send[scope];
            } else {
                // Merge fields: keep current patch_data values if they exist (user kept typing),
                // otherwise restore the failed values.
                patch_data[scope] = { ...data_to_send[scope], ...patch_data[scope] };
            }
        }
        show_alert("Couldn't save changes. Trying again!", "warning");
        await request_patch();
    }
}

function add_cover_listeners(){
    document.getElementById("settings.backcover_image").addEventListener("change", cover_upload_listener);
    document.getElementById("settings.delete_backcover_image").addEventListener("click", delete_cover_listener);
    document.getElementById("settings.cover_image").addEventListener("change", cover_upload_listener);
    document.getElementById("settings.delete_cover_image").addEventListener("click", delete_cover_listener);
}

async function cover_upload_listener(e: Event){
    let target = e.target as HTMLInputElement;
    if(target.files && target.files.length > 0){
        // 1. upload new cover image
        try {
            let resp = await editorApi.uploadToProject(state.project_id, target.files[0]);
            console.log(resp);

            // 2. Patch settings
            let patch;
            if(target.id === "settings.backcover_image"){
                 patch = {
                     settings: {
                         backcover_image_path: resp.filename
                     }
                 }
            }else{
                patch = {
                    settings: {
                        cover_image_path: resp.filename
                    }
                }
            }

            await editorApi.patchProject(state.project_id, patch);
            // Reload application & data:
            await init();
        }catch(e){
            console.error("Failed to upload project settings", e);
            show_alert("Couldn't upload cover image.", "error");
        }
    }
}

async function delete_cover_listener(e: Event){
    let target = e.target as HTMLElement;

    // 1. Delete image
    let filename = target.getAttribute("data-cover-image-path");
    if(!filename){
        return;
    }

    try {
        await editorApi.deleteProjectUpload(state.project_id, filename);
        // 2. Delete from settings
        let patch;
        if (target.id === "settings.delete_cover_image") { // Delete cover
            patch = {
                settings: {
                    cover_image_path: null,
                }
            }
        } else {// Delete backcover
            patch = {
                settings: {
                    backcover_image_path: null,
                }
            }
        }

        await editorApi.patchProject(state.project_id, patch);
        // Reload application & data:
        await init();
    }catch(e){
        console.error("Failed to delete cover image", e);
        show_alert("Couldn't delete cover image.", "error");
    }
}
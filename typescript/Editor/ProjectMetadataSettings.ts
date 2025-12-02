import {APIProjectData, EditorAPI} from "../api_requests";
import {main_col} from "./Editor";
import {state} from "./Main";
import {show_alert} from "../tools";

/**
 * Represents the primary interface for interacting with the editor.
 * Provides methods and properties to manipulate editor content,
 * manage editor state, and interact with editor-specific events and configurations.
 */
const editorApi = EditorAPI();

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

/**
 * Renders the project metadata settings view using the provided project data.
 *
 * @param {APIProjectData} data - The project data object containing metadata to be displayed.
 * @return {void} This function does not return a value.
 */
export async function show_project_metadata_settings(data: APIProjectData) {
    // @ts-ignore
    main_col.innerHTML = Handlebars.templates.editor_project_metadata_settings(data);

    init_autopatch();
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
        }else{
            //TODO: implement for textfields etc.
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
    }else{
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
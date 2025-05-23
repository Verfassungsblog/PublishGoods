import * as Tools from "./tools";
import * as API from "./api_requests";
import {
    CategoryTree,
    ImportAPI,
    PreviewRequest,
    WordpressFilterData,
    WordpressImportData,
    WordpressImportRequest
} from "./api_requests";

/**
 * Handles the event when the Import button is clicked. This method initializes
 * the import wizard overlay and sets up event listeners for various wizard options
 * such as Pandoc and WordPress import workflows.
 *
 * @return {void} Does not return a value. It modifies the DOM to display the import wizard
 * and attaches event handlers for further interactions within the wizard.
 */
function import_btn_handler() {
    let overlay_wrapper = document.getElementById("overlay-wrapper");
    let overlay_content = document.getElementById("inner_overlay");
    overlay_wrapper.classList.remove("hide");

    // @ts-ignore
    overlay_content.innerHTML = Handlebars.templates.editor_import_wizard();

    Tools.add_event_listeners("#wizard-pandoc-btn", "click", function () {
        document.getElementById("wizard-start").classList.add("hide");
        document.getElementById("wizard-pandoc-1").classList.remove("hide");
    });
    //Tools.add_event_listeners("#wizard-pandoc-upload-btn", "click", upload_files_handler);

    Tools.add_event_listeners("#wizard-wp-btn", "click", function () {
        document.getElementById("wizard-start").classList.add("hide");
        document.getElementById("wizard-wordpress-1").classList.remove("hide");
    });
    Tools.add_event_listeners("#wizard-wp-filter-btn", "click", function () {
        document.getElementById("wizard-wordpress-1").classList.add("hide");
        document.getElementById("wizard-wordpress-by-filter-1").classList.remove("hide");
    });
    Tools.add_event_listeners("#wizard-wp-links-btn", "click", function () {
        document.getElementById("wizard-wordpress-1").classList.add("hide");
        document.getElementById("wizard-wordpress-by-links").classList.remove("hide");
    });
    Tools.add_event_listeners("#wizard-wordpress-host-next", "click", wordpress_filter_load_categories);
    //Tools.add_event_listeners("#wizard-wordpress-upload-btn", "click", wordpress_import_handler);
}

/**
 * Loads and displays the category tree for WordPress filtering based on the provided host.
 * It dynamically updates the UI to show or hide elements depending on the loading state.
 *
 * @return {Promise<void>} Returns a Promise that resolves when the category tree is successfully loaded and displayed, or displays an error alert if the process fails.
 */
async function wordpress_filter_load_categories(): Promise<void> {
    let host = (document.getElementById("wizard-wordpress-host") as HTMLInputElement).value || null;

    if (!host) {
        return;
    }

    document.getElementById("wizard-wordpress-by-filter-1").classList.add("hide");
    document.getElementById("wizard-wordpress-by-filter-2").classList.remove("hide");

    // Load categories
    let api = API.ImportAPI();

    try {
        let category_tree = await api.load_category_tree(host);
        await wordpress_filter_show_filter_mask(category_tree, host);
    } catch (e) {
        document.getElementById("overlay-wrapper").classList.add("hide");
        Tools.show_alert(e, "danger");
    }
}

/**
 * Initializes and displays the filter mask for the WordPress import wizard.
 *
 * This function performs the following actions:
 * 1. Displays the filter mask element and hides the previous wizard step.
 * 2. Populates the filter mask with category tree data using Handlebars templates.
 * 3. Attaches event listeners to handle filter adjustments and preview post counts.
 *
 * @param {CategoryTree} category_tree - The category tree data used to populate the filter mask.
 * @param {string} host - The base URL of the WordPress instance for API interactions.
 * @return {Promise<void>} Resolves once the filter mask is fully initialized and preview data is loaded.
 */
async function wordpress_filter_show_filter_mask(category_tree: CategoryTree, host: string): Promise<void> {
    let filter_mask = document.getElementById("wizard-wordpress-by-filter-3");

    document.getElementById("wizard-wordpress-by-filter-2").classList.add("hide");
    filter_mask.classList.remove("hide");

    // @ts-ignore
    filter_mask.innerHTML = Handlebars.templates.editor_import_wizard_filter_mask(category_tree);

    /**
     * Toggles the state of a wizard column based on the checkbox input event.
     *
     * This function handles enabling or disabling a wizard column's content
     * when the associated checkbox is checked or unchecked. When enabled,
     * the disabled class is removed from the column content and all input
     * fields within the column content are enabled. When disabled, the disabled
     * class is added, and all input fields are set to disabled.
     *
     * @param {Event} e - The event triggered by the input element, typically a checkbox.
     */
    let toggle_wizard_column = async function (e: Event) {
        let target = e.target as HTMLInputElement;

        let column_content = target.closest(".wizard-column").getElementsByClassName("wizard-column-content")[0];
        let column_content_inputs = column_content.querySelectorAll("input, button");

        if (target.checked) { // Enable column
            column_content.classList.remove("disabled"); //Remove disabled class from column content
            for (let input of Array.from(column_content_inputs)) {
                (input as HTMLInputElement | HTMLButtonElement).disabled = false;
            }
        } else { // Disable column
            column_content.classList.add("disabled"); //Add disabled class to column content
            for (let input of Array.from(column_content_inputs)) {
                (input as HTMLInputElement | HTMLButtonElement).disabled = true;
            }
        }
    }

    /**
     * Asynchronously fetches and displays a preview of posts based on user-configured filters.
     *
     * The `get_posts_preview` function retrieves a preview of posts from the server by constructing
     * a request object (`PreviewRequest`) which includes various filter options such as time frame,
     * included categories, and excluded categories. It sends the request to the backend API and
     * updates the relevant DOM element with the count of posts returned.
     *
     * Filters include:
     * - Time frame: Specified using "before" and "after" dates.
     * - Included categories: Posts belonging to the selected categories.
     * - Excluded categories: Posts outside the specified categories.
     *
     * In case of an error during the API request, it logs the error to the console and updates
     * the post count field with "N/A".
     *
     * @function
     * @async
     * @name get_posts_preview
     *
     * @throws Will log an error to the console if the API request fails.
     */
    let get_posts_preview = async function () {
        let posts_num_field = document.getElementById("wizard-wordpress-filter-mask-affected-posts-num");

        let preview_request: PreviewRequest = {
            base_url: host,
            page: 1,
            per_page: 1,
        };

        if ((document.getElementById("wizard-wordpress-filter-mask-time-frame") as HTMLInputElement).checked) {
            preview_request.before = (document.getElementById("wizard-wordpress-filter-mask-time-frame-before") as HTMLInputElement).value || null;
            preview_request.after = (document.getElementById("wizard-wordpress-filter-mask-time-frame-after") as HTMLInputElement).value || null;
        }

        if ((document.getElementById("wizard-wordpress-filter-mask-include-categories-check") as HTMLInputElement).checked) {
            // Collect checked categories
            let categories: number[] = [];

            let checkboxes = document.getElementById("wizard-wordpress-filter-mask-include-categories-list").querySelectorAll("input");
            for (let checkbox of Array.from(checkboxes)) {
                if (checkbox.checked) {
                    categories.push(Number.parseInt(checkbox.value));
                }
            }

            preview_request.include_categories = categories;
        }
        if ((document.getElementById("wizard-wordpress-filter-mask-exclude-categories-check") as HTMLInputElement).checked) {
            // Collect checked categories
            let categories: number[] = [];

            let checkboxes = document.getElementById("wizard-wordpress-filter-mask-exclude-categories-list").querySelectorAll("input");
            for (let checkbox of Array.from(checkboxes)) {
                if (checkbox.checked) {
                    categories.push(Number.parseInt(checkbox.value));
                }
            }

            preview_request.exclude_categories = categories;
        }

        let api = ImportAPI();

        try {
            let resp = await api.load_posts_preview(preview_request);
            console.log(resp);

            posts_num_field.innerText = resp.number_of_posts.toString();
        } catch (e) {
            console.error("Couldn't load posts preview: " + e);
            posts_num_field.innerText = "N/A"
        }
    }

    Tools.add_event_listeners("#wizard-filter-mask-wrapper input", "change", get_posts_preview);
    Tools.add_event_listeners(".wizard-column-enabler", "change", toggle_wizard_column);

    // Add listeners for recursive category check and uncheck buttons
    Tools.add_event_listeners(".wizard-wp-category-check-recursive-btn", "click", function(e: Event){
        let category = (e.target as HTMLElement).closest(".wizard-wp-category") as HTMLElement;
        let children = category.querySelectorAll(".wizard-wp-subcategories input");

        children.forEach(function(child){
            (child as HTMLInputElement).checked = true;
        });

        // Toggle recursive check to recursive uncheck buttons
        let buttons = category.querySelectorAll(".wizard-wp-category-check-recursive-btn");
        buttons.forEach(function(button){button.classList.add("hide")});
        buttons = category.querySelectorAll(".wizard-wp-category-uncheck-recursive-btn");
        buttons.forEach(function(button){button.classList.remove("hide")});
    });
    Tools.add_event_listeners(".wizard-wp-category-uncheck-recursive-btn", "click", function(e: Event){
        let category = (e.target as HTMLElement).closest(".wizard-wp-category") as HTMLElement;
        let children = category.querySelectorAll(".wizard-wp-subcategories input");

        children.forEach(function(child){
            (child as HTMLInputElement).checked = false;
        });

        // Toggle recursive check to recursive uncheck buttons
        let buttons = category.querySelectorAll(".wizard-wp-category-check-recursive-btn");
        buttons.forEach(function(button){button.classList.remove("hide")});
        buttons = category.querySelectorAll(".wizard-wp-category-uncheck-recursive-btn");
        buttons.forEach(function(button){button.classList.add("hide")});
    });


    get_posts_preview().then(_ => {});

    document.getElementById("wizard-wordpress-filter-mask-next").addEventListener("click", async function(){
        let wordpress_import_data : WordpressImportData = {
            WordpressFilter: get_filter_settings(host),
        };

        document.getElementById("wizard-wordpress-by-filter-3").classList.add("hide");
        await wordpress_show_settings(wordpress_import_data);
    });
}

function get_filter_settings(host: string): WordpressFilterData{
    let filter_data : WordpressFilterData = {
        wp_host: host
    };

    if ((document.getElementById("wizard-wordpress-filter-mask-time-frame") as HTMLInputElement).checked){
        filter_data.before = (document.getElementById("wizard-wordpress-filter-mask-time-frame-before") as HTMLInputElement).value || null;
        filter_data.after = (document.getElementById("wizard-wordpress-filter-mask-time-frame-after") as HTMLInputElement).value || null;
    }

    if ((document.getElementById("wizard-wordpress-filter-mask-include-categories-check") as HTMLInputElement).checked){
        let categories: number[] = [];
        let checked_inputs = document.getElementById("wizard-wordpress-filter-mask-include-categories-list").querySelectorAll("input:checked") as NodeListOf<HTMLInputElement>;
        checked_inputs.forEach((element) => {
            categories.push(parseInt(element.value));
        });
        filter_data.include_categories = categories;
    }

    if ((document.getElementById("wizard-wordpress-filter-mask-exclude-categories-check") as HTMLInputElement).checked){
        let categories: number[] = [];
        let checked_inputs = document.getElementById("wizard-wordpress-filter-mask-exclude-categories-list").querySelectorAll("input:checked") as NodeListOf<HTMLInputElement>;
        checked_inputs.forEach((element) => {
            categories.push(parseInt(element.value));
        });
        filter_data.exclude_categories = categories;
    }

    return filter_data
}

async function wordpress_show_settings(import_data: WordpressImportData){
    document.getElementById("wizard-wordpress-settings").classList.remove("hide");

    let start_import_btn = document.getElementById("wizard-wordpress-start-import-btn") as HTMLInputElement;

    start_import_btn.addEventListener("click", async function(){
        let import_request : WordpressImportRequest = {
            convert_links: (document.getElementById("wizard-wordpress-settings-convert-links") as HTMLInputElement).checked,
            data: import_data,
            endnotes: (document.getElementById("wizard-wordpress-settings-convert-to-endnotes") as HTMLInputElement).checked,
            // @ts-ignore
            project_id: globalThis.project_id,
            shift_headings: (document.getElementById("wizard-wordpress-settings-shift-levels-up") as HTMLInputElement).checked
        };

        let api = API.ImportAPI();

        try {
            start_import_btn.disabled = true;
            let job_id = await api.add_wp_import_job(import_request);
            document.getElementById("wizard-wordpress-settings").classList.add("hide");
            await show_import_status(job_id);
        }catch (e) {
            console.error(e);
            Tools.show_alert("Couldn't add import job!", "danger");
            start_import_btn.disabled = false;
        }
    })
}

async function show_import_status(import_job_id: string){
    document.getElementById("wizard-progress").classList.remove("hide");

    let status_text = document.getElementById("wizard-upload-progress-status");
    let status_bar = document.getElementById("wizard-upload-progress") as HTMLProgressElement;
    let overlay_wrapper = document.getElementById("overlay-wrapper");

    let api = API.ImportAPI();

    let error_counter = 0;
    let update_status = setInterval(async function(){
        try{
            let res = await api.poll_import_status(import_job_id);
            console.log(res);
            if(typeof res === "string"){
                switch (res){
                    case "Pending":
                        status_text.innerText = "Pending in Queue";
                        break
                    case "RequestWPPosts":
                        status_text.innerText = "Requesting Posts from WordPress host";
                        break
                    case "Complete":
                        status_text.innerText = "Import completed!";
                        clearInterval(update_status);
                        overlay_wrapper.classList.add("hide");
                        Tools.show_alert("Import completed!", "success");
                }
            }else if ("Processing" in res){
                let details = res.Processing;

                if(details.total){
                    status_bar.value = Math.round((details.current / details.total) * 100);
                }

                status_text.innerText = "Processing post "+details.current+" of "+details.total;
            }else if ("Failed" in res) {
                const error = res.Failed;
                console.error(error);

                let error_msg = "";

                if(typeof error === "string"){
                    switch (error){
                        case "UnsupportedFileType":
                            error_msg ="The file type of the file is not supported."
                            break;
                        case "InvalidFile":
                            error_msg = "The uploaded file is corrupted."
                            break;
                        case "BibFileInvalid":
                            error_msg = "The uploaded bibliography file is invalid."
                            break;
                        case "PandocError":
                            error_msg = "Couldn't convert the uploaded file to HTML due to an Pandoc error. Contact your administrator."
                            break;
                        case "HtmlConversionFailed":
                            error_msg = "Couldn't convert the uploaded file to HTML."
                            break;
                        case "ProjectNotFound":
                            error_msg = "Couldn't find the project to import into. Was it deleted in the meantime?";
                            break;
                    }
                }else if ("WordpressApiError" in error){
                    let details = error.WordPressApiError as string;
                    switch (details){
                        case "SerdeParsingError":
                            error_msg = "Invalid Import Request."
                            break;
                        case "ReqwestError":
                            error_msg = "Couldn't connect to WordPress host."
                            break;
                        case "InvalidURL":
                            error_msg = "URL to WordPress host seems invalid."
                            break;
                        case "NotFound":
                            error_msg = "No posts found."
                            break;
                        case "UnexpectedResponse":
                            error_msg = "Got an unexpected response from the WordPress host.";
                            break;
                    }
                }

                status_text.innerText = "Import failed :(";
                clearInterval(update_status);
                overlay_wrapper.classList.add("hide");
                Tools.show_alert("Import failed: "+error_msg, "danger");
            }
        }catch(e){
            error_counter += 1;
            console.error(e);
            Tools.show_alert("Couldn't fetch import job status!", "danger");
            // Cancel Interval after 3 failures
            if(error_counter >= 3){
                clearInterval(update_status);
            }
        }
    }, 500);
}

/**
 * Handles file upload functionality. This method processes the selected files from specific HTML input elements, prepares the FormData object,
 * and sends it to a server via an API call. It monitors the upload and processing status and updates the progress bar and status messages accordingly.
 * It also manages reloading the page upon successful processing or handles an error state gracefully.
 *
 * @return {Promise<void>} A promise that resolves when the file upload handling is completed, or rejects if any errors occur.
 */
/*async function upload_files_handler() {
    let files = (<HTMLInputElement>document.getElementById("wizard-pandoc-upload-input")).files;

    let formData = new FormData();
    for (let i = 0; i < files.length; i++) {
        formData.append("files", files[i]);
    }

    let bib_file = (<HTMLInputElement>document.getElementById("wizard-pandoc-upload-bib-input")).files;
    if (bib_file.length > 0) {
        formData.append("bib_file", bib_file[0]);
    }

    let convert_to_endnotes = (<HTMLInputElement>document.getElementById("wizard-pandoc-settings-convert-to-endnotes")).checked;
    formData.append("convert_footnotes_to_endnotes", convert_to_endnotes.toString());

    // @ts-ignore
    formData.append("project_id", globalThis.project_id);

    document.getElementById("wizard-pandoc-1").classList.add("hide");
    document.getElementById("wizard-progress").classList.remove("hide");
    let status_text = document.getElementById("wizard-upload-progress-status");
    let progress_bar = document.getElementById("wizard-upload-progress");

    try {
        let import_id = (await API.send_import_from_upload(formData))["data"];
        let poller = setInterval(async function () {
            let res = (await API.send_poll_import_status(import_id))["data"];
            let status = res["status"];
            progress_bar.setAttribute("max", res["length"]);
            progress_bar.setAttribute("value", res["processed"]);
            if (status == "Pending") {
                status_text.innerHTML = "Waiting for files to be processed...";
            }
            if (status == "Processing") {
                status_text.innerHTML = "Processing file " + res["processed"] + " of " + res["length"] + "...";
            }
            if (status == "Complete") {
                status_text.innerHTML = "Files processed successfully!";
                clearInterval(poller);
                // Reload page:
                location.reload();
            }
            if (status == "Failed") {
                status_text.innerHTML = "Failed to process files!";
                clearInterval(poller);
            }
        }, 250);

    } catch (e) {
        console.error(e);
        Tools.show_alert("Failed to upload files", "error");
    }
} */

/**
 * Handles the import process for WordPress settings by gathering input data,
 * sending it to the server for processing, and monitoring the progress of the import operation.
 * Updates the UI to reflect the current status of the process.
 *
 * @return {Promise<void>} A promise that resolves once the import process is completed and the UI is updated accordingly.
 */
/*async function wordpress_import_handler() {
    let data: any = {};
    data["links"] = [];

    let links_field = document.getElementById("wizard-wordpress-settings-links") as HTMLTextAreaElement;
    console.log(links_field.value);
    for (let link of links_field.value.trim().split("\n")) {
        data["links"].push(link);
    }
    data["endnotes"] = (<HTMLInputElement>document.getElementById("wizard-wordpress-settings-convert-to-endnotes")).checked;
    data["shift_headings"] = (<HTMLInputElement>document.getElementById("wizard-wordpress-settings-convert-to-endnotes")).checked;
    data["convert_links"] = (<HTMLInputElement>document.getElementById("wizard-wordpress-settings-convert-links")).checked;
    // @ts-ignore
    data["project_id"] = globalThis.project_id;

    console.log(data);

    document.getElementById("wizard-wordpress-1").classList.add("hide");
    document.getElementById("wizard-progress").classList.remove("hide");
    let status_text = document.getElementById("wizard-upload-progress-status");
    let progress_bar = document.getElementById("wizard-upload-progress");

    try {
        let import_id = (await API.send_import_from_wordpress(data))["data"];
        let poller = setInterval(async function () {
            let res = (await API.send_poll_import_status(import_id))["data"];
            let status = res["status"];
            console.log(res);
            progress_bar.setAttribute("max", res["length"]);
            progress_bar.setAttribute("value", res["processed"]);
            if (status == "Pending") {
                status_text.innerHTML = "Waiting for files to be processed...";
            }
            if (status == "Processing") {
                status_text.innerHTML = "Processing file " + res["processed"] + " of " + res["length"] + "...";
            }
            if (status == "Complete") {
                status_text.innerHTML = "Files processed successfully!";
                clearInterval(poller);
                // Reload page:
                location.reload();
            }
            if (status == "Failed") {
                status_text.innerHTML = "Failed to process files!";
                clearInterval(poller);
            }
        }, 250);

    } catch (e) {
        console.error(e);
        status_text.innerHTML = "Failed :(";
        Tools.show_alert("Failed to upload files", "error");
    }
}*/

window.addEventListener("load", async function () {
    // @ts-ignore
    window.add_import_listeners = () => {
        Tools.add_event_listeners("#editor_sidebar_import", "click", import_btn_handler)
    }
});
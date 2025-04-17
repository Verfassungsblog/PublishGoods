// Import API functions
import {PdfVariant, ProjectTemplateV2, TemplateAPI} from "./api_requests";
import * as Tools from "./tools";

import {basicSetup, EditorView} from "codemirror"
import {html} from "@codemirror/lang-html"

let typing_timer: any | null = null;
let template_api = TemplateAPI();
let template_data: ProjectTemplateV2 | null = null;
let current_export_format: string | null = null;
let editor: EditorView | null = null;
let collapse_status: Record<string, boolean> = {
    metadata: true,
    assets: false,
    export_steps: false
};
let export_steps_to_save: string[] = [];
let focused_before_saved: string | null = null;
let selectionEnd_before_saved: number | null = null;
let selectionStart_before_saved: number | null = null;

async function tstart() {
    // Get template_id from URL (last part of the URL)
    let url = new URL(window.location.href);
    let template_id = url.pathname.split("/").pop();
    if (template_id == null) {
        console.error("Template ID not found in URL, cannot continue.");
        return;
    }

    // Load the current template data

    try {
        template_data = await template_api.read_template(template_id);
        console.log(template_data);

        // Add Sidebar:
        let sidebar = document.getElementById("template_editor_sidebar");
        // @ts-ignore
        sidebar.innerHTML = Handlebars.templates.template_editor_sidebar(template_data);

        // Add Main:
        show_metadata();
    } catch (e) {
        console.error("Error loading template data: ", e);
        // Show error message
        Tools.show_alert("Error loading Template Editor :( ", "danger");
    }

    // Add event listeners
    document.getElementById("template_editor_sidebar_new_export_format_btn").addEventListener("click", add_export_format_handler);
    document.getElementById("template_editor_global_assets_btn").addEventListener("click", show_global_assets);
    let export_format_entries = Array.from(document.getElementsByClassName("template_editor_sidebar_export_format"));
    for (let entry of export_format_entries) {
        entry.addEventListener("click", export_format_list_click_lstn)
    }

    // Event listeners for context menu:
    document.addEventListener('click', function (event) {
        let contextmenu = document.getElementById("template_editor_assets_contextmenu");
        if (!contextmenu || !event.target) {
            return;
        }
        let isClickInside = contextmenu.contains(event.target as Node);

        if (!isClickInside) {
            // The click was outside the contextmenu, hide it.
            contextmenu.classList.add("hide");
        }
    });
}

async function export_format_list_click_lstn(e: Event) {
    let target = e.target as HTMLElement;
    let slug = target.getAttribute("data-slug") as string;
    if (!slug) {
        console.error("show_export_format called but slug missing")
        return
    }
    current_export_format = slug;
    await show_export_format();
}

async function show_export_format() {
    export_steps_to_save = [];
    template_data = await template_api.read_template(template_data.id);
    let data: any = {};
    data.collapse_status = collapse_status;
    data.export_format_data = template_data.export_formats[current_export_format];
    if (!data.export_format_data) {
        console.error("Couldn't find export format data for slug " + current_export_format);
    }

    // Modify export step data for handlebars:
    for (let export_step of data.export_format_data.export_steps) {
        if (export_step.data.Pandoc) {
            let output_format = export_step.data.Pandoc.output_format;
            export_step.data.Pandoc["output_format_extra"] = {};
            export_step.data.Pandoc["output_format_extra"][output_format] = true;
            let input_format = export_step.data.Pandoc.input_format;
            export_step.data.Pandoc["input_format_extra"] = {};
            export_step.data.Pandoc["input_format_extra"][input_format] = true;
        }
    }

    try {
        let assets = await template_api.list_export_format_assets(template_data.id, current_export_format);

        Tools.start_loading_spinner();
        await Promise.all([assets]).then(async function (values) {
            Tools.stop_loading_spinner();
            data.assets = values[0].assets;
        });

    } catch (e) {
        console.error(e);
        Tools.show_alert(e, "danger");
        return;
    }
    console.log(data);


    let main = document.getElementById("template_editor_main_panel");
    // @ts-ignore
    main.innerHTML = Handlebars.templates.template_editor_export_format(data);

    // Restore user focus & cursor position if we saved previously
    if(focused_before_saved){
        console.log("Focussing on previously focused element:");
        console.log(focused_before_saved);
        let previously_focused = document.getElementById(focused_before_saved);
        if(previously_focused) { // Is element still there?
            if(selectionStart_before_saved){
                (previously_focused as HTMLInputElement | HTMLTextAreaElement).selectionStart = selectionStart_before_saved;
            }
            if(selectionEnd_before_saved){
                (previously_focused as HTMLInputElement | HTMLTextAreaElement).selectionEnd = selectionEnd_before_saved;
            }
            previously_focused.focus();
        }
    }

    // Handler to collapse / expand cards
    let collapse_handler = function (e: Event) {
        let target = e.target as HTMLElement;
        let card = target.closest(".card");
        let card_body = card.getElementsByClassName("card-body")[0];
        let sign = target.getElementsByClassName("card-collapse-or-expand-sign")[0];
        let state = target.getAttribute("data-state");
        let card_id = target.getAttribute("data-card") as string;

        if (state === "collapsed") {
            card_body.classList.remove("hide");
            sign.innerHTML = "-";
            target.setAttribute("data-state", "extended")
            collapse_status[card_id] = true;
        } else {
            card_body.classList.add("hide");
            sign.innerHTML = "+";
            target.setAttribute("data-state", "collapsed");
            collapse_status[card_id] = false;
        }
    }

    // Add collapse/expand listeners
    for (let card of Array.from(document.getElementsByClassName("card-collapse-or-expand"))) {
        card.addEventListener("click", collapse_handler);
    }

    // Add delete listener
    document.getElementById("delete_export_format").addEventListener("click", async function () {
        // Ask user if sure
        if (confirm("You are going to delete the export format " + data.export_format_data.name + ", are you sure?") == true) {
            console.log("Deleting export format with slug " + data.export_format_data.slug)
            try {
                await template_api.delete_export_format(template_data.id, current_export_format);
                await tstart();
            } catch (e) {
                console.error(e);
                Tools.show_alert(e, "danger");
            }
        }
    });

    // Add metadata change listeners
    document.getElementById("export_format_name").addEventListener("input", save_export_format_metadata_listener);
    document.getElementById("export_format_preview_path").addEventListener("input", save_export_format_metadata_listener);

    // --- Assets ---

    // Add file click listeners (open/download)
    let file_rows = Array.from(document.getElementsByClassName("file_row_icon+name"));
    for (let file_row of file_rows) {
        file_row.addEventListener("click", export_format_file_row_click_handler);
    }
    // Add upload listener
    document.getElementById("template_editor_assets_menu_upload_btn").addEventListener("click", export_format_upload_asset_handler);
    //Add delete listener
    document.getElementById("template_editor_assets_menu_delete_btn").addEventListener("click", export_formats_delete_selected_handler);
    // Add new folder listener
    document.getElementById("template_editor_assets_menu_new_folder_btn").addEventListener("click", new_folder_handler);
    document.getElementById("template_editor_assets_menu_new_folder_dialog_create").addEventListener("click", export_formats_create_folder_handler);
    // Add moving listener
    // Add dropzones to folder rows
    let dropzones = Array.from(document.getElementsByClassName("folder_row_top"));
    for (let dropzone of dropzones) {
        dropzone.addEventListener("drop", drop_handler_for_export_formats);
        dropzone.addEventListener("dragover", drag_over_handler);
    }
    // Add global dropzone
    var dropzone_before = document.getElementById("template_editor_asset_rows_before");
    var dropzone_after = document.getElementById("template_editor_asset_rows_after");
    dropzone_before.addEventListener("drop", drop_handler_for_export_formats);
    dropzone_before.addEventListener("dragover", drag_over_handler);
    dropzone_after.addEventListener("drop", drop_handler_for_export_formats);
    dropzone_after.addEventListener("dragover", drag_over_handler);

    document.getElementById("template_editor_assets_menu_select_all_btn").addEventListener("click", select_deselect_all_handler);

    let asset_rows = Array.from(document.getElementsByClassName("asset_row"));
    for (let row of asset_rows) {
        // Add drag listeners
        row.addEventListener("dragstart", drag_start_handler);
        // Add contextmenu listener
        row.addEventListener("contextmenu", show_asset_context_menu);
    }
    // Add contextmenu listener
    document.getElementById("template_editor_assets_contextmenu_rename").addEventListener("click", show_contextmenu_rename_dialog);
    document.getElementById("template_editor_assets_contextmenu_rename_dialog_btn").addEventListener("click", save_new_asset_name_for_export_formats);

    // --- Export Steps ---

    // Add export step listeners
    add_export_step_listeners();

}

function add_export_step_listeners() {
    // Add listener to create new export step button
    document.getElementById("add_export_step_btn").addEventListener("click", add_new_export_step);

    // Add listeners to delete buttons
    let delete_btns = document.getElementsByClassName("export_step_delete_btn");
    for (let btn of Array.from(delete_btns)) {
        btn.addEventListener("click", delete_export_step);
    }

    // Add listeners to move buttons
    let move_up_btns = document.getElementsByClassName("export_step_move_up_btn");
    for (let btn of Array.from(move_up_btns)){
        btn.addEventListener("click", move_export_step_up);
    }

    // Add listeners to move buttons
    let move_down_btns = document.getElementsByClassName("export_step_move_down_btn");
    for (let btn of Array.from(move_down_btns)){
        btn.addEventListener("click", move_export_step_down);
    }

    //Add change listener to all autosave properties
    let elements = Array.from(document.getElementsByClassName("autosave")) as HTMLElement[];
    for (let element of elements) {
        element.addEventListener("input", save_export_step_listener)
    }
}

async function move_export_step_up(e: Event){
    let target = e.target as HTMLElement;

    let export_step_entry = target.closest(".export_step");
    let id = export_step_entry.getAttribute("data-export-step-id") || null;

    // @ts-ignore
    let export_steps = template_data.export_formats[current_export_format].export_steps;
    let export_steps_ids = [];

    // Get current position:
    for(let entry of export_steps){
        export_steps_ids.push(entry.id);
    }
    let pos = export_steps_ids.indexOf(id);
    if(pos == 0){
       return;
    }else{
        let new_pos = pos-2;

        let move_after;
        if(new_pos < 0){
            move_after = null;
        }else{
            move_after = export_steps_ids[new_pos];
        }
        await template_api.move_export_step_after(template_data.id, current_export_format, id, move_after);
        await show_export_format();
    }
}

async function move_export_step_down(e: Event){
    let target = e.target as HTMLElement;

    let export_step_entry = target.closest(".export_step");
    let id = export_step_entry.getAttribute("data-export-step-id") || null;

    // @ts-ignore
    let export_steps = template_data.export_formats[current_export_format].export_steps;
    let export_steps_ids = [];

    // Get current position:
    for(let entry of export_steps){
        export_steps_ids.push(entry.id);
    }
    let pos = export_steps_ids.indexOf(id);
    if(pos == export_steps_ids.length-1){
        return;
    }else{
        let new_pos = pos+1;

        let move_after = export_steps_ids[new_pos];
        await template_api.move_export_step_after(template_data.id, current_export_format, id, move_after);
        await show_export_format();
    }
}

async function delete_export_step(e: Event) {
    let target = e.target as HTMLElement;
    let export_step = target.closest(".export_step");
    let id = export_step.getAttribute("data-export-step-id");

    if (!confirm('Are you sure you want to delete this export step?')) {
        return;
    }

    try {
        await template_api.delete_export_step(template_data.id, current_export_format, id);
        await show_export_format();
    } catch (e) {
        console.error(e);
        Tools.show_alert("danger", e);
    }
}

function add_new_export_step() {
    let export_step_type_select = document.getElementById("add_export_step_type") as HTMLSelectElement;
    let export_step_type = export_step_type_select.value;
    let export_step_list = document.getElementById("export_steps_list");

    if (export_step_type == "raw") {
        // @ts-ignore
        export_step_list.innerHTML += Handlebars.templates.template_editor_export_format_raw();
    } else if (export_step_type == "pandoc") {
        // @ts-ignore
        export_step_list.innerHTML += Handlebars.templates.template_editor_export_format_pandoc();
    } else if (export_step_type == "vivliostyle") {
        // @ts-ignore
        export_step_list.innerHTML += Handlebars.templates.template_editor_export_format_vivliostyle();
    }else if (export_step_type == "weasyprint") {
        // @ts-ignore
        export_step_list.innerHTML += Handlebars.templates.template_editor_export_format_weasyprint();
    }
    add_export_step_listeners();
}

function save_export_step_listener(e: Event) {
    let target = e.target as HTMLElement;
    let export_step_id = target.closest(".export_step").getAttribute("data-export-step-id");
    if (export_step_id) {
        if (!export_steps_to_save.includes(export_step_id)) {
            export_steps_to_save.push(export_step_id);
        }
    }

    // Wait for 1 second after the last keypress before saving
    if (typing_timer != null) {
        clearTimeout(typing_timer);
    }
    typing_timer = setInterval(save_export_steps, 1000)
}

function save_export_format_metadata_listener(e: Event) {
     // Wait for 1 second after the last keypress before saving
    if (typing_timer != null) {
        clearTimeout(typing_timer);
    }
    typing_timer = setInterval(save_export_format_metadata, 1000)
}

async function save_export_format_metadata(){
    clearTimeout(typing_timer);
    let name = document.getElementById("export_format_name").innerText.trim();
    let preview_path = (document.getElementById("export_format_preview_path") as HTMLInputElement).value;

    if(preview_path.trim() === ""){
        preview_path = null;
    }
    await template_api.change_export_format_metadata(template_data.id, current_export_format, name, preview_path);
    await tstart();
}

async function save_export_steps() {
    clearTimeout(typing_timer);
    console.log("Saving changes!")
    console.log(`Pending changes for ${export_steps_to_save} and possible new export steps`);

    let requests = []; //Request holder except for new export steps since we need to assign the id

    let export_steps = Array.from(document.getElementsByClassName("export_step") as HTMLCollectionOf<HTMLElement>);
    for (let export_step of export_steps) {
        let id = export_step.getAttribute("data-export-step-id") || null;

        if(id && !export_steps_to_save.includes(id)){ //Only save steps which are marked as to save (edited recently)
            continue;
        }

        //Check type
        let type = export_step.getAttribute("data-export-step-type");

        if (type === "raw") {
            let name = (export_step.getElementsByClassName("export_step_raw_name")[0] as HTMLInputElement).value || null;
            let entry_point = (export_step.getElementsByClassName("export_step_raw_entry_point")[0] as HTMLInputElement).value || null;
            let output_file = (export_step.getElementsByClassName("export_step_raw_output_file")[0] as HTMLInputElement).value || null;

            //Check if all required filled out
            if (!name || !entry_point || !output_file) {
                console.log("Export step missing fields, not submitting (yet)");
                if(id){ //If we are updating an entry, notify user:
                    Tools.show_alert("Export Step can't be saved until you fill out all required fields!", "warning")
                }

                return;
            }

            let export_step_data = {
                id,
                name,
                data: {
                    "Raw": {
                        entry_point,
                        output_file,
                    }
                },
                files_to_keep: [output_file]
            }
            if(id){
                requests.push(template_api.update_export_step(template_data.id, current_export_format, export_step_data))
            }else {
                requests.push(template_api.create_export_step(template_data.id, current_export_format, export_step_data))
            }
        } else if (type === "vivliostyle") {
            let name = (export_step.getElementsByClassName("export_step_vivliostyle_name")[0] as HTMLInputElement).value || null;
            let input_file = (export_step.getElementsByClassName("export_step_vivliostyle_input_file")[0] as HTMLInputElement).value || null;
            let output_file = (export_step.getElementsByClassName("export_step_vivliostyle_output_file")[0] as HTMLInputElement).value || null;
            let press_ready: boolean = (export_step.getElementsByClassName("export_step_vivliostyle_press_ready")[0] as HTMLInputElement).checked;

            //Check if all required filled out
            if (!name || !input_file || !output_file) {
                console.log("Export step missing fields, not submitting (yet)");

                if(id){ //If we are updating an entry, notify user:
                    Tools.show_alert("Export Step can't be saved until you fill out all required fields!", "warning")
                }

                return;
            }
            let export_step_data = {
                id,
                name,
                data: {
                    "Vivliostyle": {
                        input_file,
                        output_file,
                        press_ready
                    }
                },
                files_to_keep: [output_file]
            }
            if(id){
                requests.push(template_api.update_export_step(template_data.id, current_export_format, export_step_data))
            }else {
                requests.push(template_api.create_export_step(template_data.id, current_export_format, export_step_data))
            }
        } else if (type === "pandoc") {
            let name = (export_step.getElementsByClassName("export_step_pandoc_name")[0] as HTMLInputElement).value || null;
            let input_file = (export_step.getElementsByClassName("export_step_pandoc_input_file")[0] as HTMLInputElement).value || null;
            let input_format = (export_step.getElementsByClassName("export_step_pandoc_input_format")[0] as HTMLSelectElement).value || null;
            let output_file = (export_step.getElementsByClassName("export_step_pandoc_output_file")[0] as HTMLInputElement).value || null;
            let output_format = (export_step.getElementsByClassName("export_step_pandoc_output_format")[0] as HTMLSelectElement).value || null;
            let shift_heading_level_by = parseInt((export_step.getElementsByClassName("export_step_pandoc_shift_heading_level_by")[0] as HTMLInputElement).value) || null;
            let metadata_file = (export_step.getElementsByClassName("export_step_pandoc_metadata_file")[0] as HTMLInputElement).value || null;
            let epub_cover_image_path = (export_step.getElementsByClassName("export_step_pandoc_epub_cover_image_path")[0] as HTMLInputElement).value || null;
            let epub_metadata_file = (export_step.getElementsByClassName("export_step_pandoc_epub_metadata_file")[0] as HTMLInputElement).value || null;
            let epub_title_page = (export_step.getElementsByClassName("export_step_pandoc_epub_title_page")[0] as HTMLInputElement).checked;
            let epub_embed_fonts_raw = (export_step.getElementsByClassName("export_step_pandoc_embed_fonts")[0] as HTMLTextAreaElement).value || null;

            let epub_embed_fonts: string[] | null = epub_embed_fonts_raw && epub_embed_fonts_raw !== "" ? epub_embed_fonts_raw.split("\n") : null;

            if(epub_embed_fonts) {
                // Trim each entry
                epub_embed_fonts = epub_embed_fonts.map(entry => entry.trim());
            }

            //Check if all required filled out
            if (!name || !input_file || !input_format || !output_file || !output_format) {
                console.log("Export step missing fields, not submitting (yet)");
                if(id){ //If we are updating an entry, notify user:
                    Tools.show_alert("Export Step can't be saved until you fill out all required fields!", "warning")
                }
                return;
            }
            let export_step_data = {
                id,
                name,
                data: {
                    "Pandoc": {
                        input_file,
                        output_file,
                        input_format,
                        output_format,
                        shift_heading_level_by,
                        metadata_file,
                        epub_cover_image_path,
                        epub_metadata_file,
                        epub_title_page,
                        epub_embed_fonts
                    }
                },
                files_to_keep: [output_file]
            }
            if(id){
                requests.push(template_api.update_export_step(template_data.id, current_export_format, export_step_data))
            }else {
                requests.push(template_api.create_export_step(template_data.id, current_export_format, export_step_data))
            }

        }else if (type === "weasyprint") {
            let name = (export_step.getElementsByClassName("export_step_weasyprint_name")[0] as HTMLInputElement).value || null;
            let input_file = (export_step.getElementsByClassName("export_step_weasyprint_input_file")[0] as HTMLInputElement).value || null;
            let output_file = (export_step.getElementsByClassName("export_step_weasyprint_output_file")[0] as HTMLInputElement).value || null;
            let pdf_variant = (export_step.getElementsByClassName("export_step_weasyprint_pdf_variant")[0] as HTMLInputElement).value || null;

            //Check if all required filled out
            if (!name || !input_file || !output_file) {
                console.log("Export step missing fields, not submitting (yet)");

                if(id){ //If we are updating an entry, notify user:
                    Tools.show_alert("Export Step can't be saved until you fill out all required fields!", "warning")
                }

                return;
            }

            console.log("PDF Variant: " + pdf_variant);
            // Match pdf_variant
            let parsed_pdf_variant = Object.values(PdfVariant).includes(pdf_variant as PdfVariant) ? (pdf_variant as PdfVariant) : PdfVariant.PDF;

            console.log("Parsed PDF Variant: " + parsed_pdf_variant);

            let export_step_data = {
                id,
                name,
                data: {
                    "Weasyprint": {
                        input_file,
                        output_file,
                        pdf_variant: parsed_pdf_variant
                    }
                },
                files_to_keep: [output_file]
            }
            if(id){
                requests.push(template_api.update_export_step(template_data.id, current_export_format, export_step_data))
            }else {
                requests.push(template_api.create_export_step(template_data.id, current_export_format, export_step_data))
            }
        }

        try {
            await Promise.all(requests).then((res) => {
                console.log(res);
            });
            focused_before_saved = document.activeElement.id;
            if (document.activeElement.tagName === 'INPUT' || document.activeElement.tagName === 'TEXTAREA') {
                // @ts-ignore
                selectionStart_before_saved = document.activeElement.selectionStart;
                // @ts-ignore
                selectionEnd_before_saved = document.activeElement.selectionEnd;

            }

            await show_export_format();
        } catch (e) {
            Tools.show_alert(e, "danger");
        }
    }

}

function show_metadata() {
    current_export_format = null;
    let main = document.getElementById("template_editor_main_panel");
    // @ts-ignore
    main.innerHTML = Handlebars.templates.template_editor_metadata(template_data);
    let elements = Array.from(document.getElementsByClassName("quickchange"));
    for (let element of elements) {
        element.addEventListener("input", change_metadata_handler);
    }
}

async function show_global_assets() {
    current_export_format = null;
    console.log("Loading global asset list");
    let main = document.getElementById("template_editor_main_panel");
    let assets = await template_api.list_global_assets(template_data.id);

    // @ts-ignore
    main.innerHTML = Handlebars.templates.template_editor_assets(assets);
    console.log(assets);

    // Get a list of all asset rows
    let asset_rows = Array.from(document.getElementsByClassName("asset_row"));

    for (let row of asset_rows) {
        // Add drag listeners
        row.addEventListener("dragstart", drag_start_handler);
        // Add contextmenu listener
        row.addEventListener("contextmenu", show_asset_context_menu);
    }

    // Add dropzones to folder rows
    let dropzones = Array.from(document.getElementsByClassName("folder_row_top"));
    for (let dropzone of dropzones) {
        dropzone.addEventListener("drop", drop_handler);
        dropzone.addEventListener("dragover", drag_over_handler);
    }
    // Add global dropzone
    var dropzone_before = document.getElementById("template_editor_asset_rows_before");
    var dropzone_after = document.getElementById("template_editor_asset_rows_after");
    dropzone_before.addEventListener("drop", drop_handler);
    dropzone_before.addEventListener("dragover", drag_over_handler);
    dropzone_after.addEventListener("drop", drop_handler);
    dropzone_after.addEventListener("dragover", drag_over_handler);

    // Add file click listeners (open/download)
    let file_rows = Array.from(document.getElementsByClassName("file_row_icon+name"));
    for (let file_row of file_rows) {
        file_row.addEventListener("click", file_row_click_handler);
    }

    // Add menu listeners
    document.getElementById("template_editor_assets_menu_new_folder_btn").addEventListener("click", new_folder_handler);
    document.getElementById("template_editor_assets_menu_new_folder_dialog_create").addEventListener("click", create_folder_handler);
    document.getElementById("template_editor_assets_menu_select_all_btn").addEventListener("click", select_deselect_all_handler);
    document.getElementById("template_editor_assets_menu_delete_btn").addEventListener("click", delete_selected_handler);
    document.getElementById("template_editor_assets_menu_upload_btn").addEventListener("click", upload_asset_handler);

    // Add contextmenu listener
    document.getElementById("template_editor_assets_contextmenu_rename").addEventListener("click", show_contextmenu_rename_dialog);
    document.getElementById("template_editor_assets_contextmenu_rename_dialog_btn").addEventListener("click", save_new_asset_name);
}

function show_contextmenu_rename_dialog() {
    let context = document.getElementById("template_editor_assets_contextmenu").getAttribute("data-context-path");
    if (!context) {
        return;
    }

    let dialog = document.getElementById("template_editor_assets_contextmenu_rename_dialog") as HTMLElement;
    let name_input = document.getElementById("template_editor_assets_contextmenu_rename_dialog_input") as HTMLInputElement;
    name_input.value = document.getElementById("Path-" + context).getAttribute("data-name");
    dialog.setAttribute("data-path", context);
    dialog.classList.remove("hide");
}

async function save_new_asset_name() {
    let input = document.getElementById("template_editor_assets_contextmenu_rename_dialog_input") as HTMLInputElement;
    let dialog = document.getElementById("template_editor_assets_contextmenu_rename_dialog") as HTMLDivElement;

    let path = dialog.getAttribute("data-path");
    let new_name = input.value;
    let path_parts = path.split("/");
    // Change last part of path to new name
    path_parts[path_parts.length - 1] = new_name;
    let new_path = path_parts.join("/");

    console.log("Moving " + path + " to " + new_path);
    try {
        await template_api.move_global_asset(template_data.id, path, new_path, false);
        show_global_assets();
    } catch (e) {
        Tools.show_alert(e, "danger");
    }
    dialog.classList.add("hide");

}

async function save_new_asset_name_for_export_formats() {
    let input = document.getElementById("template_editor_assets_contextmenu_rename_dialog_input") as HTMLInputElement;
    let dialog = document.getElementById("template_editor_assets_contextmenu_rename_dialog") as HTMLDivElement;

    let path = dialog.getAttribute("data-path");
    let new_name = input.value;
    let path_parts = path.split("/");
    // Change last part of path to new name
    path_parts[path_parts.length - 1] = new_name;
    let new_path = path_parts.join("/");

    console.log("Moving " + path + " to " + new_path);
    try {
        await template_api.move_asset_for_export_format(template_data.id, path, new_path, current_export_format, false);
        await show_export_format();
    } catch (e) {
        Tools.show_alert(e, "danger");
    }
    dialog.classList.add("hide");

}

function show_asset_context_menu(e: MouseEvent) {
    e.preventDefault();
    let target = e.target as HTMLElement;
    let asset_row;

    if (target.classList.contains("asset_row")) {
        asset_row = target;
    } else {
        asset_row = target.closest(".asset_row");
    }
    let contextmenu = document.getElementById("template_editor_assets_contextmenu");
    contextmenu.setAttribute("data-context-path", asset_row.getAttribute("data-path"));
    contextmenu.style.left = e.x + "px";
    contextmenu.style.top = e.y + "px";
    contextmenu.classList.remove("hide");
}

async function file_row_click_handler() {
    let row = this.closest(".file_row");
    try {
        let result = await template_api.get_asset_file(template_data.id, row.getAttribute("data-path"));
        if (result.type == "blob") {
            const url = URL.createObjectURL(result.data as Blob);
            const a = document.createElement('a');
            a.href = url;
            a.download = row.getAttribute("data-name");
            a.target = "_blank";
            a.click();
        } else if (result.type == "text") {
            // Show text edit
            show_text_file_editor(row.getAttribute("data-name"), row.getAttribute("data-path"), result.data as string);
        }
        console.log(result);
    } catch (e) {
        Tools.show_alert(e, "danger");
        return;
    }
}

async function export_format_file_row_click_handler() {
    let row = this.closest(".file_row");
    try {
        let result = await template_api.get_asset_file_for_export_format(template_data.id, current_export_format, row.getAttribute("data-path"));
        if (result.type == "blob") {
            const url = URL.createObjectURL(result.data as Blob);
            const a = document.createElement('a');
            a.href = url;
            a.download = row.getAttribute("data-name");
            a.target = "_blank";
            a.click();
        } else if (result.type == "text") {
            // Show text edit
            await show_text_file_editor(row.getAttribute("data-name"), row.getAttribute("data-path"), result.data as string);
        }
        console.log(result);
    } catch (e) {
        Tools.show_alert(e, "danger");
        return;
    }
}

async function show_text_file_editor(name: string, path: string, content: string) {
    let main = document.getElementById("template_editor_asset_edit");

    // @ts-ignore
    main.innerHTML = Handlebars.templates.template_editor_asset_edit({name: name, path: path, content: content});

    let save_btn = document.getElementById("template_editor_asset_edit_save_btn");
    // Enable save button if name is changed
    document.getElementById("template_editor_asset_edit_name").addEventListener("input", function () {
        save_btn.removeAttribute("disabled");
    });
    save_btn.addEventListener("click", update_text_asset);

    editor = new EditorView({
        doc: content,
        extensions: [basicSetup, html(), EditorView.updateListener.of(function (e) {
            if (e.docChanged) { // Enable save button if content is changed
                let save_btn = document.getElementById("template_editor_asset_edit_save_btn");
                save_btn.removeAttribute("disabled");
            }
        })],
        parent: document.getElementById("template_editor_assset_edit_editor"),
    });
}

async function update_text_asset(e: Event) {
    let target = e.target as HTMLElement;
    let path = target.getAttribute("data-path");
    let old_name = (document.getElementById("template_editor_asset_edit_name") as HTMLElement).getAttribute("data-old-name");
    let name = (document.getElementById("template_editor_asset_edit_name") as HTMLElement).innerText;
    let content = editor.state.doc.toString();

    try {
        if (current_export_format) {
            await template_api.update_asset_text_file_for_export_format(template_data.id, path, current_export_format, content);
        } else {
            await template_api.update_asset_text_file(template_data.id, path, content);
        }


        //Update name if changed
        if (name !== old_name) {
            let path_parts = path.split("/");
            // Change last part of path to new name
            path_parts[path_parts.length - 1] = name;
            let new_path = path_parts.join("/");

            if (current_export_format) {
                await template_api.move_asset_for_export_format(template_data.id, path, new_path, current_export_format, false);
            } else {
                await template_api.move_global_asset(template_data.id, path, new_path, false);
            }
        }
        // Disable save button after saving
        target.setAttribute("disabled", "true");

        if (current_export_format) {
            await show_export_format();
        } else {
            await show_global_assets();
        }
    } catch (e) {
        Tools.show_alert(e, "danger");
        return;
    }
}

function upload_asset_handler() {
    var input = document.createElement("input");
    input.type = "file";

    input.addEventListener("change", async function () {
        let file = (input.files as FileList)[0];
        try {
            await template_api.upload_file(template_data.id, file);
            show_global_assets();
        } catch (e) {
            Tools.show_alert(e, "danger");
            return;
        }
    });
    input.click();
}


function export_format_upload_asset_handler() {
    var input = document.createElement("input");
    input.type = "file";

    input.addEventListener("change", async function () {
        let file = (input.files as FileList)[0];
        try {
            await template_api.upload_file_for_export_format(template_data.id, current_export_format, file);
            await show_export_format();
        } catch (e) {
            Tools.show_alert(e, "danger");
            return;
        }
    });
    input.click();
}

async function delete_selected_handler() {

    let checkboxes = Array.from(document.getElementsByClassName("asset_row_check")) as HTMLInputElement[];
    let checkedCheckboxes = checkboxes.filter(checkbox => checkbox.checked);
    let selectedPaths = checkedCheckboxes.map(checkbox => checkbox.closest(".asset_row").getAttribute("data-path"));
    console.log(selectedPaths);
    // Perform delete operation using selectedPaths
    try {
        await template_api.delete_assets(template_data.id, selectedPaths);
        show_global_assets();
    } catch (e) {
        Tools.show_alert(e, "danger");
        return;
    }
}

async function export_formats_delete_selected_handler() {

    let checkboxes = Array.from(document.getElementsByClassName("asset_row_check")) as HTMLInputElement[];
    let checkedCheckboxes = checkboxes.filter(checkbox => checkbox.checked);
    let selectedPaths = checkedCheckboxes.map(checkbox => checkbox.closest(".asset_row").getAttribute("data-path"));
    console.log(selectedPaths);
    // Perform delete operation using selectedPaths
    try {
        await template_api.delete_assets_for_export_formats(template_data.id, current_export_format, selectedPaths);
        await show_export_format();
    } catch (e) {
        Tools.show_alert(e, "danger");
        return;
    }
}

function new_folder_handler() {
    let dialog = document.getElementById("template_editor_assets_menu_new_folder_dialog");
    dialog.classList.toggle("hide");
}

async function create_folder_handler() {
    let folder_name = (document.getElementById("template_editor_assets_menu_new_folder_dialog_name") as HTMLInputElement).value;

    if (folder_name == "") {
        Tools.show_alert("Folder name cannot be empty", "danger");
        return;
    }

    try {
        await template_api.create_folder(template_data.id, folder_name);
        await show_global_assets();
    } catch (e) {
        Tools.show_alert(e, "danger");
        return;
    }
}

async function export_formats_create_folder_handler() {
    let folder_name = (document.getElementById("template_editor_assets_menu_new_folder_dialog_name") as HTMLInputElement).value;

    if (folder_name == "") {
        Tools.show_alert("Folder name cannot be empty", "danger");
        return;
    }

    try {
        await template_api.create_folder_for_export_format(template_data.id, folder_name, current_export_format);
        await show_export_format()
    } catch (e) {
        Tools.show_alert(e, "danger");
        return;
    }
}

async function drop_handler(event: DragEvent) {
    event.preventDefault();
    // Old path before being moved
    var old_path = event.dataTransfer.getData("text");
    var dropped_element = document.getElementById("Path-" + old_path);

    let new_path;
    if ((event.target as HTMLElement).classList.contains("template_editor_global_drop_zone")) {
        // New Path after being moved: Just the filename
        new_path = dropped_element.getAttribute("data-name");
    } else {
        var current_folder = (event.target as HTMLElement).closest(".folder_row");
        var folder_content = current_folder.getElementsByClassName("folder_contents")[0];
        //folder_content.appendChild(dropped_element); no longer needed since we will update the whole list

        // New Path after being moved: Current Folder Path + Filename
        new_path = current_folder.getAttribute("data-path") + "/" + dropped_element.getAttribute("data-name");
    }

    if (old_path != new_path) {
        let overwrite_option = false;
        // Check if new_path already exists
        let existing_element = document.getElementById("Path-" + new_path);
        if (existing_element) {
            let confirm = window.confirm("An element with the same name already exists in this folder. Do you want to replace it?");
            if (confirm) {
                overwrite_option = true;
            } else {
                return;
            }
        }

        console.log("Moving ", old_path, " to ", new_path);
        try {
            await template_api.move_global_asset(template_data.id, old_path, new_path, overwrite_option);
            show_global_assets();
        } catch (e) {
            Tools.show_alert(e, "danger");
            return;
        }
    }
}

async function drop_handler_for_export_formats(event: DragEvent) {
    event.preventDefault();
    // Old path before being moved
    var old_path = event.dataTransfer.getData("text");
    console.log("Old Path:" + old_path);
    var dropped_element = document.getElementById("Path-" + old_path);

    let new_path;
    if ((event.target as HTMLElement).classList.contains("template_editor_global_drop_zone")) {
        // New Path after being moved: Just the filename
        new_path = dropped_element.getAttribute("data-name");
    } else {
        var current_folder = (event.target as HTMLElement).closest(".folder_row");
        console.log(dropped_element);
        console.log(dropped_element.getAttribute("data-name"));

        // New Path after being moved: Current Folder Path + Filename
        new_path = current_folder.getAttribute("data-path") + "/" + dropped_element.getAttribute("data-name");
    }

    if (old_path != new_path) {
        let overwrite_option = false;
        // Check if new_path already exists
        let existing_element = document.getElementById("Path-" + new_path);
        if (existing_element) {
            let confirm = window.confirm("An element with the same name already exists in this folder. Do you want to replace it?");
            if (confirm) {
                overwrite_option = true;
            } else {
                return;
            }
        }

        console.log("Moving ", old_path, " to ", new_path);
        try {
            await template_api.move_asset_for_export_format(template_data.id, old_path, new_path, current_export_format, overwrite_option);
            await show_export_format();
        } catch (e) {
            Tools.show_alert(e, "danger");
            return;
        }
    }
}

function drag_over_handler(event: DragEvent) {
    event.preventDefault();
}

function drag_start_handler(event: DragEvent) {
    event.dataTransfer.setData("text", (event.target as HTMLElement).getAttribute("data-path"));
}

async function add_export_format_handler() {
    let name = (document.getElementById("template_editor_sidebar_new_export_format_name") as HTMLInputElement).value;

    try {
        let res = await template_api.create_export_format(template_data.id, name)
        await tstart();
    } catch (e) {
        Tools.show_alert(e, "danger");
    }
}

function change_metadata_handler() {
    // Wait for 1 second after the last keypress before saving
    if (typing_timer != null) {
        clearTimeout(typing_timer);
    }
    typing_timer = setInterval(save_metadata, 2000)
}

function select_deselect_all_handler() {
    let checkboxes = Array.from(document.getElementsByClassName("asset_row_check"));

    // Check if all are checked
    let all_checked = true;
    for (let checkbox of checkboxes) {
        if (!(checkbox as HTMLInputElement).checked) {
            all_checked = false;
            break;
        }
    }

    if (all_checked) {
        for (let checkbox of checkboxes) {
            (checkbox as HTMLInputElement).checked = false;
        }
    } else {
        for (let checkbox of checkboxes) {
            (checkbox as HTMLInputElement).checked = true;
        }
    }
}

async function save_metadata() {
    clearTimeout(typing_timer);
    let template_name = document.getElementById("template_name").innerText.trim() || null;
    let template_description = document.getElementById("template_description").innerHTML.trim() || null;

    if (!template_name || !template_description) {
        return;
    }
    template_data.name = template_name;
    template_data.description = template_description;
    console.log(template_data);

    // Save the metadata
    await template_api.update_template(template_data);
}

window.addEventListener("load", async function () {
    tstart();
});
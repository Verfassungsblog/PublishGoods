import * as Tools from "./tools";
import * as API from "./api_requests";

function import_btn_handler(){
    let overlay_wrapper = document.getElementById("overlay-wrapper");
    let overlay_content = document.getElementById("inner_overlay");
    overlay_wrapper.classList.remove("hide");

    // @ts-ignore
    overlay_content.innerHTML = Handlebars.templates.editor_import_wizard();
    document.getElementById("wizard-pandoc-btn").addEventListener("click", function(){
        document.getElementById("wizard-start").classList.add("hide");
        document.getElementById("wizard-pandoc-1").classList.remove("hide");
    });
    document.getElementById("wizard-pandoc-upload-btn").addEventListener("click", upload_files_handler);

    document.getElementById("wizard-wp-btn").addEventListener("click", function(){
        document.getElementById("wizard-start").classList.add("hide");
        document.getElementById("wizard-wordpress-1").classList.remove("hide");
    });
    document.getElementById("wizard-wordpress-upload-btn").addEventListener("click", wordpress_import_handler);
}

async function upload_files_handler(){
    let files = (<HTMLInputElement>document.getElementById("wizard-pandoc-upload-input")).files;

    let formData = new FormData();
    for(let i = 0; i < files.length; i++){
        formData.append("files", files[i]);
    }

    let bib_file = (<HTMLInputElement>document.getElementById("wizard-pandoc-upload-bib-input")).files;
    if(bib_file.length > 0){
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
        let poller = setInterval(async function(){
            let res = (await API.send_poll_import_status(import_id))["data"];
            let status = res["status"];
            progress_bar.setAttribute("max", res["length"]);
            progress_bar.setAttribute("value", res["processed"]);
            if(status == "Pending"){
                status_text.innerHTML = "Waiting for files to be processed...";
            }
            if(status == "Processing"){
                status_text.innerHTML = "Processing file "+res["processed"]+" of "+res["length"]+"...";
            }
            if(status == "Complete"){
                status_text.innerHTML = "Files processed successfully!";
                clearInterval(poller);
                // Reload page:
                location.reload();
            }
            if(status == "Failed"){
                status_text.innerHTML = "Failed to process files!";
                clearInterval(poller);
            }
        }, 250);

    }catch (e) {
        console.error(e);
        Tools.show_alert("Failed to upload files", "error");
    }
}

async function wordpress_import_handler(){
    let data : any = {
    };
    data["links"] = [];

    let links_field = document.getElementById("wizard-wordpress-settings-links") as HTMLTextAreaElement;
    console.log(links_field.value);
    for(let link of links_field.value.trim().split("\n")){
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
        let poller = setInterval(async function(){
            let res = (await API.send_poll_import_status(import_id))["data"];
            let status = res["status"];
            console.log(res);
            progress_bar.setAttribute("max", res["length"]);
            progress_bar.setAttribute("value", res["processed"]);
            if(status == "Pending"){
                status_text.innerHTML = "Waiting for files to be processed...";
            }
            if(status == "Processing"){
                status_text.innerHTML = "Processing file "+res["processed"]+" of "+res["length"]+"...";
            }
            if(status == "Complete"){
                status_text.innerHTML = "Files processed successfully!";
                clearInterval(poller);
                // Reload page:
                location.reload();
            }
            if(status == "Failed"){
                status_text.innerHTML = "Failed to process files!";
                clearInterval(poller);
            }
        }, 250);

    }catch (e) {
        console.error(e);
        status_text.innerHTML = "Failed :(";
        Tools.show_alert("Failed to upload files", "error");
    }
}

window.addEventListener("load", async function(){
    // @ts-ignore
    window.add_import_listeners = () => {document.getElementById("editor_sidebar_import").addEventListener("click", import_btn_handler)}
});
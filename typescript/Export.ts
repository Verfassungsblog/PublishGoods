import {
    ProjectAPI,
    send_get_template_id_for_project,
    TemplateAPI,
    NewLocalRenderingRequest,
    ProjectTemplateV2,
    SectionOrToc,
    ExportAPI
} from "./api_requests";
import * as Tools from "./tools";
import * as pdfjs from 'pdfjs-dist';
pdfjs.GlobalWorkerOptions.workerSrc =
    '/js/pdf.worker.mjs';

let template_api = TemplateAPI();
let project_api = ProjectAPI();
let export_api = ExportAPI();
let checker_timer: string | number | NodeJS.Timeout = null;

let close_overlay = function(){
    let overlay_wrapper = document.getElementById("overlay-wrapper");
    let overlay_content = document.getElementById("inner_overlay");
    overlay_wrapper.classList.add("hide");
    overlay_content.innerHTML = "";

}
export async function add_listeners(){
    document.getElementById("overlay_close_btn").addEventListener("click", close_overlay);
    document.getElementById("editor_render_project_btn").addEventListener("click", preview_project_listener);
    document.getElementById("editor_export_project_btn").addEventListener("click", export_project_listener);
    addEventListener("keydown", function(e){
        if(e.key === "Escape" && !document.getElementById("overlay-wrapper").classList.contains("hide")){
            close_overlay();
        }
    });
}

async function preview_project_listener(){
    // Get template id for project
    // @ts-ignore
    let template_id = await send_get_template_id_for_project(globalThis.project_id);
    let template = await template_api.read_template(template_id);

    let found_export_format = null;
    let preview_pdf_path = null;

    for(let export_format of Object.values(template.export_formats)){
        if(export_format.preview_pdf_path){
            console.log("Found export_format with preview pdf path: "+export_format.slug);
            found_export_format = export_format.slug;
            preview_pdf_path = export_format.preview_pdf_path;
            break;
        }
    }
    if(!found_export_format){
        Tools.show_alert("Can't render preview if no export format has a preview pdf path set!", "danger");
        return;
    }
    let local_rendering_request : NewLocalRenderingRequest = {
        // @ts-ignore
        project_id: globalThis.project_id,
        export_formats: [found_export_format],
        sections: null,
    }

    console.log(local_rendering_request);

    let request_id;
    try {
        request_id = await export_api.send_new_rendering_request(local_rendering_request);
        console.log("Request has id "+request_id+".");
        Tools.show_alert("Started preview rendering.", "success");
    }catch(e){
        console.error(e);
        Tools.show_alert("Couldn't send rendering request to server :(", "danger");
        return;
    }

    let waiter = setInterval(async function () {
        try {
            let status = await export_api.get_request_status(request_id);
            console.log(status);

            if(typeof status === "string"){
                if(status === "SavedOnLocal"){
                    clearInterval(waiter)
                    console.log("/export/"+request_id+"/"+preview_pdf_path);
                    await show_pdf("/export/" + request_id + "/" + preview_pdf_path);
                }
            }else if("Failed" in status){
                clearInterval(waiter)
                console.error("Couldn't render preview: "+status.Failed);
                Tools.show_alert("Couldn't render preview. Check export log.", "danger");
            }
        } catch (e) {
            console.error(e);
            Tools.show_alert("Couldn't check rendering status :( Check network connection.", "danger");
            clearInterval(waiter);
        }
    }, 500)
}

async function show_pdf(uri: string) {
    let scale = 1;
    let viewer = document.getElementById("test");

    if (!viewer) {
        console.error("Viewer element not found.");
        return;
    }

    viewer.innerHTML = "";

    let loadingTask = pdfjs.getDocument(uri);
    let pdf = await loadingTask.promise;

    for (let page_num = 1; page_num <= pdf.numPages; page_num++) {
        let canvas = document.createElement("canvas");
        canvas.classList.add("pdf-page");
        viewer.appendChild(canvas);
        await renderPage(page_num, canvas);
    }

    async function renderPage(pageNumber: number, canvas: HTMLCanvasElement) {
        let page = await pdf.getPage(pageNumber);

        let viewport = page.getViewport({ scale: scale });
        canvas.height = viewport.height;
        canvas.width = viewport.width;
        let context = canvas.getContext('2d');
        if (context) {
            await page.render({
                canvasContext: context,
                viewport: viewport
            }).promise;
        } else {
            console.error("Canvas context not found.");
        }
    }

    // Lesezeichen (Outline) abrufen
    let outline = await pdf.getOutline();

    // Zu einem Lesezeichen springen
    async function jumpToBookmark(title: string) {
        if (!outline) {
            console.error('Keine Lesezeichen gefunden.');
            return;
        }

        // Lesezeichen durchsuchen
        for (let item of outline) {
            if (item.title === title) {
                // @ts-ignore
                let destination = await pdf.getDestination(item.dest);
                if (destination && Array.isArray(destination)) {
                    const pageNumber = await pdf.getPageIndex(destination[0]);  // Korrekte Seitennummer herausfinden
                    const x = destination[2];
                    const y = destination[3];

                    console.log("Navigating to page:", pageNumber + 1);
                    let canvas = viewer.querySelectorAll<HTMLCanvasElement>('.pdf-page')[pageNumber - 1];
                    let page = await pdf.getPage(pageNumber + 1);
                    let viewport = page.getViewport({ scale: scale });

                    if (canvas) {
                        // Berechne die exakte Y-Position für das Scrolling
                        const scrollY = canvas.offsetTop + (viewport.height - y * scale);
                        console.log("ScrollY:", scrollY);
                        viewer.scrollTo({
                            top: scrollY,
                            behavior: 'smooth'
                        });
                    } else {
                        console.error("Canvas for page not found.");
                    }
                } else {
                    console.error("Destination for bookmark not found or invalid format.");
                }
                break;
            }
        }
    }

    //await jumpToBookmark("Test3");

    show_rendering_col();
}

function show_rendering_col(){
    document.getElementById("editor-render-preview").classList.remove("hide");
}

export async function export_project_listener(){
    // Get template id for project
    // @ts-ignore
    let template_id = await send_get_template_id_for_project(globalThis.project_id);
    console.log("template: "+template_id);

    // Get template and project
    let get_template_req = template_api.read_template(template_id);
    // @ts-ignore
    let get_project_req = project_api.read_project_contents(globalThis.project_id);
    let data = {
        template: null as ProjectTemplateV2 | null,
        sections: null as SectionOrToc[] | null,
    };

    try {
        await Promise.all([get_template_req, get_project_req]).then((values) => {
            data.template = values[0];
            data.sections = values[1];
        })
    }catch(e){
        console.error(e);
        Tools.show_alert("Couldn't load required resources.", "danger");
        return;
    }

    console.log(data);

    let overlay_wrapper = document.getElementById("overlay-wrapper");
    let overlay_content = document.getElementById("inner_overlay");
    overlay_wrapper.classList.remove("hide");

    document.getElementById("overlay_close_btn").addEventListener("click", function(){
        overlay_wrapper.classList.add("hide");
        overlay_content.innerHTML = "";
    });
    // @ts-ignore
    overlay_content.innerHTML = Handlebars.templates.editor_export_wizard(data);

    // Add listener to export selection:
    document.getElementById("export-wizard-select-all").addEventListener("change", function(){
       document.getElementById("export-wizard-selected-chapters").classList.add("hide");
    });
    document.getElementById("export-wizard-select-selected-chapters").addEventListener("change", function(){
        document.getElementById("export-wizard-selected-chapters").classList.remove("hide");
    });

    document.getElementById("export-wizard-start-export").addEventListener("click", start_export);
}

async function start_export(){
    console.log("Starting export.");

    // Get export_formats
    let export_format_inputs = Array.from(document.querySelectorAll('input.export-wizard-format:checked'));

    if(export_format_inputs.length === 0){
        console.log("no export formats checked.");
        Tools.show_alert("Please check at least one export format.", "warning");
        return;
    }
    let export_formats = [];
    for(let export_format of export_format_inputs){
        let checkbox = export_format as HTMLInputElement;
        export_formats.push(checkbox.value)
    }

    // Check if all or only selected sections should be rendered
    let sections_to_export;
    if((document.getElementById("export-wizard-select-all") as HTMLInputElement).checked){
        sections_to_export = null
    }else{
        sections_to_export = [];
        let selected_sections = Array.from(document.querySelectorAll('input.export-wizard-selected-chapter:checked'));
        if(selected_sections.length === 0){
            console.log("No sections to export selected.");
            Tools.show_alert("Please select at least one section to export.", "warning");
            return;
        }
        for(let section of Array.from(selected_sections)){
            let section_checkbox = section as HTMLInputElement;
            sections_to_export.push(section_checkbox.value);
        }
    }

    let local_rendering_request : NewLocalRenderingRequest = {
        // @ts-ignore
        project_id: globalThis.project_id,
        export_formats,
        sections: sections_to_export,
    }

    console.log(local_rendering_request);

    try {
        let request_id = await export_api.send_new_rendering_request(local_rendering_request);
        console.log("Request has id "+request_id+".");
        show_export_progress(request_id);
    }catch(e){
        console.error(e);
        Tools.show_alert("Couldn't send rendering request to server :(", "danger")
    }
}

function show_export_progress(request_id: string){
    document.getElementById("wizard-start").classList.add("hide");
    document.getElementById("wizard-working").classList.remove("hide");

    async function refresh_rendering_status() {
        try {
            let status = await export_api.get_request_status(request_id);
            let txt = "";

            console.log(status);
            if("QueuedOnLocal" === status){
                txt = "Job queued for preparation...";
            }else if("PreparingOnLocal" === status){
                txt = "Project data gets prepared...";
            }else if("PreparedOnLocal" === status){
                txt = "Project data got prepared...";
            }else if("SendToRenderingServer" === status){
                txt = "Job sent to rendering server...";
            }else if("RequestingTemplate" === status){
                txt = "Rendering server requested template data...";
            }else if("TransmittingTemplate" === status){
                txt = "Sending template data to rendering server...";
            }else if("QueuedOnRendering" === status){
                txt = "Job queued for rendering...";
            }else if("Running" === status){
                txt = "Rendering is running...";
            }else if("SavedOnLocal" === status){
                txt = "Done!"
                clearInterval(checker_timer)
                show_download(request_id);
            }else if("Failed" in status){
                clearInterval(checker_timer)
                show_rendering_error(status.Failed);
            }else{
                console.error("Unknown Status "+status+"!");
            }

            document.getElementById("wizard-working-current-step").innerText = txt;
        } catch (e) {
            console.error("Couldn't get status: "+e);
            clearInterval(checker_timer)
        }
    }

    checker_timer = setInterval(refresh_rendering_status, 500);
}

function show_download(request_id: string){
    document.getElementById("wizard-working").classList.add("hide");
    document.getElementById("wizard-download").classList.remove("hide");

    document.getElementById("wizard-download-link").setAttribute("href", "/export/"+request_id);
}

function show_rendering_error(log: string){
    document.getElementById("wizard-working").classList.add("hide");
    document.getElementById("wizard-show-error").classList.remove("hide");

    document.getElementById("wizard-show-error-log-pane").innerHTML = log;
}

window.addEventListener("load", async function(){
    // @ts-ignore
    window.add_export_listeners = () => {add_listeners()};
});

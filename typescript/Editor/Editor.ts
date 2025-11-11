import {state} from './Main';
import {EditorAPI} from '../api_requests';
import {show_alert} from "../tools";

export async function init() {
    let contents_panel : HTMLElement = document.getElementsByClassName("sidebar-full-contents-panel")[0] as HTMLElement;

    show_preview_column(); //TODO: only show after rendering
    add_divider_drag_listeners();
    add_sidebar_collapse_listeners();

    // Initial fetch of project data
    let editorAPI = EditorAPI();
    try{
        let data = await editorAPI.getProject(state.project_id, {extend: ["metadata", "settings", "template", "sections"]});
        console.log(data);

        // @ts-ignore
        contents_panel.innerHTML = Handlebars.templates.editor_sidebar_editor(data);
    }catch(e){
        show_alert("Couldn't load project data. Reload page and try again.");
        console.error(e);
    }

}

const preview_col = document.getElementsByClassName("preview-col")[0] as HTMLElement;
const main_col = document.getElementsByClassName("main-col")[0] as HTMLElement;
let divider = document.getElementsByClassName("divider")[0] as HTMLElement;

export function show_preview_column(){
    if(state.preferred_main_row_width){
        main_col.style.width = state.preferred_main_row_width + 'px';
    }else{
        main_col.style.width = "calc((100vw - 305px)/2)";
    }
    preview_col.classList.remove("hide");
    divider.classList.remove("hide");
}

export function hide_preview_column(){
    main_col.style.width = "100%";
    preview_col.classList.add("hide");
    divider.classList.add("hide");
}

const min_main_col_size = 300;
const min_preview_col_size = 200;

function add_divider_drag_listeners(){
    let start_x: number;
    let start_width: number;

    let move_listener = function(e: PointerEvent){
        let x_offset = e.clientX-start_x;
        let new_width = start_width+x_offset;

        if(new_width < min_main_col_size){
            new_width = min_main_col_size;
        }
        if(preview_col.offsetWidth<min_preview_col_size && new_width > start_width){
            return;
        }
        main_col.style.width = new_width + "px";
        state.preferred_main_row_width = new_width; // Save new width as preferred with
    }

    divider.addEventListener("pointerdown", function(e){
        e.preventDefault();
        start_x = e.clientX;
        start_width = main_col.offsetWidth;
        document.addEventListener("pointermove", move_listener);
    });
    document.addEventListener("pointerup", function(e){
        document.removeEventListener("pointermove", move_listener);
    });
}

function add_sidebar_collapse_listeners(){
    let sidebar_full = document.getElementsByClassName("sidebar-full")[0] as HTMLElement;
    let sidebar_collapse_btn = document.getElementsByClassName("sidebar-full-header-collapse")[0] as HTMLElement;
    let sidebar_small = document.getElementsByClassName("sidebar-small")[0] as HTMLElement;
    let sidebar_extend_btn = document.getElementsByClassName("sidebar-small-extend")[0] as HTMLElement;

    sidebar_collapse_btn.addEventListener("click", function(){
        sidebar_full.classList.add("hide");
        sidebar_small.classList.remove("hide");
    });
    sidebar_extend_btn.addEventListener("click", function(){
        sidebar_full.classList.remove("hide");
        sidebar_small.classList.add("hide");
    })
}
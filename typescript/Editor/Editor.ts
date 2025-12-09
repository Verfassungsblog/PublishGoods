import {state} from './Main';
import {EditorAPI, SectionAPI} from '../api_requests';
import {show_alert} from "../tools";
import {show_project_metadata_settings} from "./ProjectMetadataSettings";

export async function init() {
    let contents_panel : HTMLElement = document.getElementsByClassName("sidebar-full-contents-panel")[0] as HTMLElement;
    let project_name : HTMLElement = document.getElementById("sidebar-full-header-project-name") as HTMLElement;

    show_preview_column(); //TODO: only show after rendering
    add_divider_drag_listeners();
    add_sidebar_collapse_listeners();

    // Initial fetch of project data
    let editorAPI = EditorAPI();
    try{
        let data = await editorAPI.getProject(state.project_id, {extend: ["metadata", "metadata.editors", "metadata.authors", "settings", "template", "sections", "available_csl_locales", "available_csl_styles"]});
        console.log(data);

        // @ts-ignore
        contents_panel.innerHTML = Handlebars.templates.editor_sidebar_content_editor(data);
        // Attach drag and drop listeners after render
        add_dnd_listeners();
        // Set project name
        project_name.innerText = data.metadata.title;
        project_name.addEventListener("click", init);
        // Show project metadata & settings
        await show_project_metadata_settings(data);
    }catch(e){
        show_alert("Couldn't load project data. Reload page and try again.");
        console.error(e);
    }

}

function add_dnd_listeners(){
    const PROXIMITY_PX = 24; // how close the pointer must be to reveal a dropzone
    const sectionAPI = SectionAPI();
    const container = document.getElementsByClassName("sidebar-full-contents-panel-contents")[0] as HTMLElement;
    if(!container){ return; }

    let draggedSectionEl: HTMLElement | null = null;
    let isDragging = false;

    // Add draggable listeners to each section body
    const bodies = Array.from(container.querySelectorAll('.sidebar-contents-section-body')) as HTMLElement[];
    bodies.forEach(body => {
        body.addEventListener('dragstart', (e: DragEvent) => {
            isDragging = true;
            const sectionEl = body.closest('.sidebar-contents-section') as HTMLElement;
            if(!sectionEl) return;
            draggedSectionEl = sectionEl;
            body.classList.add('drag-source');
            if(e.dataTransfer){
                const id = sectionEl.getAttribute('data-section-id') || '';
                e.dataTransfer.setData('text/plain', id);
                e.dataTransfer.effectAllowed = 'move';
                // Create a ghost image using the section title
                const img = document.createElement('div');
                img.className = 'drag-ghost';
                img.textContent = (body.querySelector('.section-title')?.textContent || 'Section');
                document.body.appendChild(img);
                e.dataTransfer.setDragImage(img, 10, 10);
                // Remove after a tick
                setTimeout(() => document.body.removeChild(img), 0);
            }
        });

        // While hovering over a section body during drag, explicitly reveal its dropzones
        body.addEventListener('dragover', (e: DragEvent) => {
            if(!isDragging || !draggedSectionEl) return;
            const currentSection = body.closest('.sidebar-contents-section') as HTMLElement | null;
            if(!currentSection) return;
            // Do not reveal zones inside the dragged section itself
            if(currentSection === draggedSectionEl || isDescendant(draggedSectionEl, currentSection)){
                return;
            }
            const asChildZone = currentSection.querySelector('[data-dropzone="as-child"]') as HTMLElement | null;
            const afterZone = currentSection.querySelector('[data-dropzone="after"]') as HTMLElement | null;
            if(asChildZone){ asChildZone.classList.add('dz-visible'); }
            if(afterZone){ afterZone.classList.add('dz-visible'); }
        });

        body.addEventListener('dragend', () => {
            body.classList.remove('drag-source');
            isDragging = false;
            draggedSectionEl = null;
            // cleanup highlights
            Array.from(container.querySelectorAll('.drop-target')).forEach(el => el.classList.remove('drop-target'));
            Array.from(container.querySelectorAll('[data-dropzone].dz-visible')).forEach(el => el.classList.remove('dz-visible'));
        });
    });

    function isDescendant(parent: Element, child: Element): boolean{
        let node: Element | null = child as Element;
        while (node) {
            if (node === parent) return true;
            node = node.parentElement;
        }
        return false;
    }

    // Dropzones: either as-child or after
    const dropzones = Array.from(container.querySelectorAll('[data-dropzone]')) as HTMLElement[];

    // Helper to test proximity of pointer to a zone rect
    function isNearZone(zone: HTMLElement, x: number, y: number): boolean{
        const r = zone.getBoundingClientRect();
        // treat near as within expanded rectangle by PROXIMITY_PX
        const withinX = x >= (r.left - PROXIMITY_PX) && x <= (r.right + PROXIMITY_PX);
        const withinY = y >= (r.top - PROXIMITY_PX) && y <= (r.bottom + PROXIMITY_PX);
        return withinX && withinY;
    }

    // While dragging, reveal only nearby dropzones so they take space
    container.addEventListener('dragover', (e: DragEvent) => {
        if(!isDragging || !draggedSectionEl){ return; }
        const x = e.clientX;
        const y = e.clientY;
        dropzones.forEach(zone => {
            // Do not show dropzones inside the dragged element (prevent self moves)
            if(isDescendant(draggedSectionEl as Element, zone)){
                zone.classList.remove('dz-visible');
                return;
            }
            if(isNearZone(zone, x, y)){
                zone.classList.add('dz-visible');
            }else{
                zone.classList.remove('dz-visible');
            }
        });
    });

    dropzones.forEach(zone => {
        zone.addEventListener('dragover', (e: DragEvent) => {
            if(!draggedSectionEl) return;
            // Prevent dropping into itself or its descendants when zone is inside dragged element
            if(isDescendant(draggedSectionEl, zone)){
                return;
            }
            e.preventDefault();
            e.stopPropagation();
            e.dataTransfer && (e.dataTransfer.dropEffect = 'move');
            zone.classList.add('dz-visible');
            zone.classList.add('drop-target');
        });
        zone.addEventListener('dragleave', (e: DragEvent) => {
            e.stopPropagation();
            zone.classList.remove('drop-target');
            zone.classList.remove('dz-visible');
        });
        zone.addEventListener('drop', async (e: DragEvent) => {
            if(!draggedSectionEl) return;
            e.preventDefault();
            e.stopPropagation();
            zone.classList.remove('drop-target');
            zone.classList.remove('dz-visible');
            Array.from(container.querySelectorAll('[data-dropzone].dz-visible')).forEach(el => el.classList.remove('dz-visible'));

            // Do not allow drop into itself
            if(isDescendant(draggedSectionEl, zone)){
                return;
            }

            const zoneType = zone.getAttribute('data-dropzone');

            const movedEl = draggedSectionEl; // keep a reference
            const originalParent = movedEl.parentElement as Node;
            const originalNext = movedEl.nextElementSibling as Element | null;

            const movedId = movedEl.getAttribute('data-section-id');
            if(!movedId){ return; }

            try{
                if(zoneType === 'as-child'){
                    // target parent is the section of the zone
                    const parentSection = zone.closest('.sidebar-contents-section') as HTMLElement | null;
                    const parentId = parentSection?.getAttribute('data-section-id');
                    if(!parentId){ return; }

                    // Optimistically update UI: insert as FIRST child into the parent's children container
                    const childrenContainer = parentSection.querySelector('.sidebar-contents-section-children') as HTMLElement | null;
                    if(childrenContainer){
                        childrenContainer.insertBefore(movedEl, childrenContainer.firstElementChild as Element | null);
                    }else{
                        // Fallback: insert after parentSection body
                        parentSection.appendChild(movedEl);
                    }

                    // Persist
                    await sectionAPI.move_section_child_of(state.project_id, movedId, parentId);
                }else if(zoneType === 'after'){
                    const targetSection = zone.closest('.sidebar-contents-section') as HTMLElement | null;
                    const afterId = targetSection?.getAttribute('data-section-id');
                    if(!afterId){ return; }

                    // Prevent no-op moves
                    if(afterId === movedId){ return; }

                    // Optimistically update UI
                    if(targetSection && targetSection.parentElement){
                        targetSection.parentElement.insertBefore(movedEl, targetSection.nextSibling);
                    }

                    // Persist
                    await sectionAPI.move_section_after(state.project_id, movedId, afterId);
                }
            }catch(err){
                // Rollback UI on error
                if(originalParent){
                    if(originalNext){
                        (originalParent as Element).insertBefore(movedEl, originalNext);
                    }else{
                        (originalParent as Element).appendChild(movedEl);
                    }
                }
                console.error('Failed to persist section move', err);
                show_alert('Failed to save new section order. Please try again.');
            }
        });
    });
}

const preview_col = document.getElementsByClassName("preview-col")[0] as HTMLElement;
export const main_col = document.getElementsByClassName("main-col")[0] as HTMLElement;
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
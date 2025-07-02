import * as Editor from './Editor';

export async function init(){
    let contents_panel : HTMLElement = document.getElementsByClassName("sidebar-full-contents-panel")[0] as HTMLElement;

    contents_panel.innerHTML = "";

    // Hide preview column if visible
    Editor.hide_preview_column();
}
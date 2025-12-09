import * as Tools from "../tools";
import * as Editor from "./Editor";
import * as Bibliography from "./Bibliography";

export type EditorState = {
    project_id: string;
    preferred_main_row_width: number | null;
}

export let state: EditorState;

async function main() {
    let project_id = window.location.href.split('/').pop();

    state = {
        project_id: project_id,
        preferred_main_row_width: null,
    };

    Tools.add_event_listeners(".sidebar-full-bibliography-editor-switcher > span", "click", editor_bibliography_switch_listener);

    // By default load Editor
    await Editor.init();
}

async function editor_bibliography_switch_listener(e: Event) {
    let target = e.target as HTMLElement;

    let switch_editor_btn: HTMLElement = document.getElementById("sidebar-full-bibliography-editor-switcher-editor");
    let switch_bibliography_btn: HTMLElement = document.getElementById("sidebar-full-bibliography-editor-switcher-bibliography");

    if (target === switch_editor_btn) {
        switch_editor_btn.classList.add("active");
        switch_bibliography_btn.classList.remove("active");

        await Editor.init();
    } else {
        switch_bibliography_btn.classList.add("active");
        switch_editor_btn.classList.remove("active");

        await Bibliography.init();
    }
}

window.addEventListener('load', async () => {
    console.log('Loading Editor!');
    // Register Handlebars helpers
    // @ts-ignore
    Handlebars.registerHelper("is", function (arg1: unknown, arg2: unknown, options: unknown) {
        // @ts-ignore
        return (arg1 == arg2) ? options.fn(this) : options.inverse(this);
    });

    /**
     * Creates a base64 string from any utf-8 string
     */
// @ts-ignore
    Handlebars.registerHelper("base64", function (arg1: string) {
        let bytes = new TextEncoder().encode(arg1);
        // @ts-ignore
        let binstring = Array.from(bytes, (byte) => // @ts-ignore
            String.fromCodePoint(byte),
        ).join("");
        return btoa(binstring);
    });

    await main();
});
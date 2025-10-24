import {save_changes} from "./Editor";

export class CustomStyleTool{
    private button: HTMLButtonElement;
    private state: boolean;
    private api: any;

    static get isInline() {
        return true;
    }


    // @ts-ignore
    constructor({api}) {
        this.button = null;
        this.state = false;
        this.api = api;
    }

    render(){
        this.button = document.createElement('button');
        this.button.type = 'button';
        this.button.textContent = 'CSS';
        this.button.classList.add("ce-inline-tool");

        return this.button;
    }

    show_create_dialog(range: Range){
        if(document.getElementsByClassName('custom-style-tool-settings').length > 0){
            document.getElementsByClassName('custom-style-tool-settings')[0].remove();
        }
        let toolbar = document.getElementsByClassName('ce-inline-toolbar')[0] as HTMLElement;

        let settings_dialog_html = "" +
            "<div class='custom-style-tool-settings'>" +
            "<label>Classes:</label>" +
            "<input class='cdx-input' id='custom-style-tool-settings-classes' type='text' placeholder='example-class1 my-class2'>"+
            "<label>Inline Style (CSS):</label>" +
            "<textarea class='cdx-input' id='custom-style-tool-settings-inline-style' placeholder='background-color: gray;'></textarea>" +
            "<div style='display: flex; justify-content: space-between'><button id='custom-style-abort' class='btn btn-sm btn-secondary mt-1'>Cancel</button><button id='custom-style-save' class='btn btn-sm btn-primary mt-1'>Save</button></div>" +
            "</div>";
        toolbar.insertAdjacentHTML('afterend', settings_dialog_html);

        let settings_dialog: HTMLElement = toolbar.parentElement.querySelector('.custom-style-tool-settings') as HTMLElement;
        settings_dialog.style.left = toolbar.style.left;
        // Add the same position as the toolbar but add 40px to the top
        let currentTop = parseInt(toolbar.style.top, 10);
        settings_dialog.style.top = (currentTop + 40) + 'px';

        document.getElementById("custom-style-abort").addEventListener('click', () => {
            settings_dialog.remove();
        });

        document.getElementById("custom-style-save").addEventListener('click', () => {
            let classes = (document.getElementById('custom-style-tool-settings-classes') as HTMLInputElement).value;
            let inline_style = (document.getElementById('custom-style-tool-settings-inline-style') as HTMLTextAreaElement).value;

            let custom_style = document.createElement('customstyle');
            custom_style.setAttribute('inline-style', inline_style);
            custom_style.setAttribute('classes', classes);

            let selectedText = range.extractContents();
            custom_style.appendChild(selectedText);
            range.insertNode(custom_style);
            settings_dialog.remove();

            this.api.selection.expandToTag(custom_style);
            save_changes().then();
        });
    }

    show_change_dialog(range: Range){
        if(document.getElementsByClassName('custom-style-tool-settings').length > 0){
            document.getElementsByClassName('custom-style-tool-settings')[0].remove();
        }
        let element = this.api.selection.findParentTag('CUSTOMSTYLE');

        let toolbar = document.getElementsByClassName('ce-inline-toolbar')[0] as HTMLElement;

        let settings_dialog_html = "" +
            "<div class='custom-style-tool-settings'>" +
            "<label>Classes:</label>" +
            "<input class='cdx-input' id='custom-style-tool-settings-classes' type='text' placeholder='example-class1 my-class2' value="+element.getAttribute("classes")+">"+
            "<label>Inline Style (CSS):</label>" +
            "<textarea class='cdx-input' id='custom-style-tool-settings-inline-style' placeholder='background-color: gray;'>"+element.getAttribute("inline-style")+"</textarea>" +
            "<div style='display: flex; justify-content: space-between'><button id='custom-style-abort' class='btn btn-sm btn-secondary mt-1'>Cancel</button><button id='custom-style-delete' class='btn btn-sm btn-danger'>Delete</button><button id='custom-style-save' class='btn btn-sm btn-primary mt-1'>Save</button></div>" +
            "</div>";
        toolbar.insertAdjacentHTML('afterend', settings_dialog_html);

        let settings_dialog: HTMLElement = toolbar.parentElement.querySelector('.custom-style-tool-settings') as HTMLElement;
        settings_dialog.style.left = toolbar.style.left;
        // Add the same position as the toolbar but add 40px to the top
        let currentTop = parseInt(toolbar.style.top, 10);
        settings_dialog.style.top = (currentTop + 40) + 'px';

        document.getElementById("custom-style-abort").addEventListener('click', () => {
            settings_dialog.remove();
        });

        document.getElementById("custom-style-save").addEventListener('click', () => {
            element.setAttribute("classes", (document.getElementById('custom-style-tool-settings-classes') as HTMLInputElement).value);
            element.setAttribute("inline-style", (document.getElementById('custom-style-tool-settings-inline-style') as HTMLTextAreaElement).value);
            settings_dialog.remove();
        });

        document.getElementById("custom-style-delete").addEventListener('click', () => {
            let text = range.extractContents();
            element.remove();
            range.insertNode(text);
            settings_dialog.remove();
        });
    }

    surround(range: Range){
        if (this.state) {
            this.show_change_dialog(range);
        }else {
            this.show_create_dialog(range);
        }
    }

    checkState(selection: any) {
        const mark = this.api.selection.findParentTag('CUSTOMSTYLE');

        this.state = !!mark;
    }

    static get sanitize() {
        return {
            customstyle: function(el : any){
                return el.getAttribute("inline-style").trim().length > 0 || el.getAttribute("classes").trim().length > 0;
            }
        };
    }

    static get title(){
        return 'Custom Style';
    }
}
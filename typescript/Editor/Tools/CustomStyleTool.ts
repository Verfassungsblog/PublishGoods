export class CustomStyleTool{
    private button: HTMLButtonElement | null;
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

        let settings_dialog_html = "" +
            "<div class='custom-style-tool-settings'>" +
            "<label>Classes:</label>" +
            "<input class='cdx-input' id='custom-style-tool-settings-classes' type='text' placeholder='example-class1 my-class2'>"+
            "<label>Inline Style (CSS):</label>" +
            "<textarea class='cdx-input' id='custom-style-tool-settings-inline-style' placeholder='background-color: gray;'></textarea>" +
            "<div style='display: flex; justify-content: space-between'><button id='custom-style-abort' class='btn btn-sm btn-secondary mt-1'>Cancel</button><button id='custom-style-save' class='btn btn-sm btn-primary mt-1'>Save</button></div>" +
            "</div>";
        document.body.insertAdjacentHTML('afterbegin', settings_dialog_html);

        let settings_dialog: HTMLElement = document.getElementsByClassName('custom-style-tool-settings')[0] as HTMLElement;
        
        let rect = range.getBoundingClientRect();

        // Check if the dialog would be outside the viewport
        let dest_left ;
        if(rect.left + 300 > window.innerWidth){
            dest_left =  window.innerWidth - 300;
        }else if (rect.left < 0) {
            dest_left = 0;
        }else{
            dest_left = rect.left;
        }

        settings_dialog.style.left = dest_left + 'px';
        // Add 10px to the bottom of the range
        settings_dialog.style.top = (rect.bottom + window.scrollY + 10) + 'px';

        document.getElementById("custom-style-abort")!.addEventListener('click', () => {
            settings_dialog.remove();
        });

        document.getElementById("custom-style-save")!.addEventListener('click', () => {
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
            
            // Trigger EditorJS change
            custom_style.dispatchEvent(new Event('input', { bubbles: true }));
        });
    }

    show_change_dialog(range: Range){
        if(document.getElementsByClassName('custom-style-tool-settings').length > 0){
            document.getElementsByClassName('custom-style-tool-settings')[0].remove();
        }
        let element = this.api.selection.findParentTag('CUSTOMSTYLE');

        let settings_dialog_html = "" +
            "<div class='custom-style-tool-settings'>" +
            "<label>Classes:</label>" +
            "<input class='cdx-input' id='custom-style-tool-settings-classes' type='text' placeholder='example-class1 my-class2' value='"+(element.getAttribute("classes") || "")+"'>"+
            "<label>Inline Style (CSS):</label>" +
            "<textarea class='cdx-input' id='custom-style-tool-settings-inline-style' placeholder='background-color: gray;'>"+(element.getAttribute("inline-style") || "")+"</textarea>" +
            "<div style='display: flex; justify-content: space-between'><button id='custom-style-abort' class='btn btn-sm btn-secondary mt-1'>Cancel</button><button id='custom-style-delete' class='btn btn-sm btn-danger'>Delete</button><button id='custom-style-save' class='btn btn-sm btn-primary mt-1'>Save</button></div>" +
            "</div>";
        document.body.insertAdjacentHTML('afterbegin', settings_dialog_html);

        let settings_dialog: HTMLElement = document.getElementsByClassName('custom-style-tool-settings')[0] as HTMLElement;
        
        let rect = element.getBoundingClientRect();

        // Check if the dialog would be outside the viewport
        let dest_left ;
        if(rect.left + 300 > window.innerWidth){
            dest_left =  window.innerWidth - 300;
        }else if (rect.left < 0) {
            dest_left = 0;
        }else{
            dest_left = rect.left;
        }

        settings_dialog.style.left = dest_left + 'px';
        // Add 10px to the bottom of the element
        settings_dialog.style.top = (rect.bottom + window.scrollY + 10) + 'px';

        document.getElementById("custom-style-abort")!.addEventListener('click', () => {
            settings_dialog.remove();
        });

        document.getElementById("custom-style-save")!.addEventListener('click', () => {
            element.setAttribute("classes", (document.getElementById('custom-style-tool-settings-classes') as HTMLInputElement).value);
            element.setAttribute("inline-style", (document.getElementById('custom-style-tool-settings-inline-style') as HTMLTextAreaElement).value);
            settings_dialog.remove();
            
            // Trigger EditorJS change
            element.dispatchEvent(new Event('input', { bubbles: true }));
        });

        document.getElementById("custom-style-delete")!.addEventListener('click', () => {
            const parent = element.parentElement;
            let text = range.extractContents();
            element.remove();
            range.insertNode(text);
            settings_dialog.remove();
            
            if (parent) {
                parent.dispatchEvent(new Event('input', { bubbles: true }));
            }
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
                return (el.getAttribute("inline-style") || "").trim().length > 0 || (el.getAttribute("classes") || "").trim().length > 0;
            }
        };
    }

    static get title(){
        return 'Custom Style';
    }
}

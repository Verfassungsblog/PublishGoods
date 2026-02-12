export class FixedLinkTool{
    private button: HTMLButtonElement;
    private _state: boolean;
    private api: any;
    private readonly tag: any;
    private readonly class: any;

    get state() {
        return this._state;
    }

    set state(state) {
        this._state = state;
        this.button.classList.toggle(this.api.styles.inlineToolButtonActive, state);
    }

    static get isInline() {
        return true;
    }

    // @ts-ignore
    constructor({api}) {
        this.button = null as unknown as HTMLButtonElement;
        this._state = false;
        this.api = api;
        this.tag = "A";
        this.class = 'cdx-link'
    }

    render(){
        this.button = document.createElement('button');
        this.button.type = 'button';
        this.button.textContent = 'Link';
        this.button.classList.add("ce-inline-tool");
        this.button.classList.add("ce-inline-tool-fixed-link");

        return this.button;
    }

    show_create_dialog(range: Range){
        if(document.getElementsByClassName('fixed-link-tool-settings').length > 0){
            (document.getElementsByClassName('fixed-link-tool-settings')[0] as HTMLElement).remove();
        }
        const toolbar = document.getElementsByClassName('ce-inline-toolbar')[0] as HTMLElement;

        const settings_dialog_html = ""
            + "<div class='fixed-link-tool-settings'>"
            + "<label>Link Destination: </label>"
            + "<input class='cdx-input' id='fixed-link-tool-settings-href' type='text' placeholder='https://verfassungsblog.de'>"
            + "<label>Rel: (optional)</label>"
            + "<input class='cdx-input' id='fixed-link-tool-settings-rel' type='text' placeholder='noreferrer'>"
            + "<label>Target:</label>"
            + "<select class='cdx-input' id='fixed-link-tool-settings-target'>"
            + "<option value='_self' selected>_self (open link in same tab)</option>"
            + "<option value='_blank'>_blank (open link in new tab)</option>"
            + "<option value='_parent'>_parent</option>"
            + "<option value='_top'>_top</option>"
            + "<option value='_unfencedTop'>_unfencedTop</option>"
            + "</select>"
            + "<div style='display: flex; justify-content: space-between'><button id='fixed-link-tool-abort' class='btn btn-sm btn-secondary mt-1'>Cancel</button><button id='fixed-link-tool-save' class='btn btn-sm btn-primary mt-1'>Save</button></div>"
            + "</div>";
        toolbar.insertAdjacentHTML('afterend', settings_dialog_html);

        const settings_dialog: HTMLElement = toolbar.parentElement!.querySelector('.fixed-link-tool-settings') as HTMLElement;
        settings_dialog.style.left = toolbar.style.left;
        const currentTop = parseInt(toolbar.style.top, 10);
        settings_dialog.style.top = (currentTop + 40) + 'px';

        document.getElementById("fixed-link-tool-abort")!.addEventListener('click', () => {
            settings_dialog.remove();
        });

        document.getElementById("fixed-link-tool-save")!.addEventListener('click', () => {
            const href = (document.getElementById("fixed-link-tool-settings-href") as HTMLInputElement).value;
            const rel = (document.getElementById("fixed-link-tool-settings-rel") as HTMLInputElement).value;
            const target = (document.getElementById("fixed-link-tool-settings-target") as HTMLSelectElement).value;

            if(!href || !target){
                alert("Please enter the link target!");
                return;
            }

            settings_dialog.remove();

            const selectedText = range.extractContents();
            const link : HTMLAnchorElement = document.createElement(this.tag);
            link.href = href.trim();
            if(rel && rel.trim()){
                link.rel = rel.trim();
            }
            link.target = target;

            link.classList.add(this.class);
            link.appendChild(selectedText);
            range.insertNode(link);

            this.api.selection.expandToTag(link);
        });
    }

    show_change_dialog(range: Range){
        if(document.getElementsByClassName('fixed-link-tool-settings').length > 0){
            (document.getElementsByClassName('fixed-link-tool-settings')[0] as HTMLElement).remove();
        }
        const link = this.api.selection.findParentTag(this.tag);

        this.api.selection.expandToTag(link);

        const toolbar = document.getElementsByClassName('ce-inline-toolbar')[0] as HTMLElement;

        const settings_dialog_html = ""
            + "<div class='fixed-link-tool-settings'>"
            + "<label>Link Destination: </label>"
            + `<input class='cdx-input' id='fixed-link-tool-settings-href' value='${link.href}' type='text' placeholder='https://verfassungsblog.de'>`
            + "<label>Rel: (optional)</label>"
            + `<input class='cdx-input' id='fixed-link-tool-settings-rel' value='${link.rel}' type='text' placeholder='noreferrer'>`
            + "<label>Target:</label>"
            + `<select class='cdx-input' id='fixed-link-tool-settings-target'>`
            + `<option value="_self" ${link.target === "_self" ? "selected" : ""}>_self (open link in same tab)</option>`
            + `<option value="_blank" ${link.target === "_blank" ? "selected" : ""}>_blank (open link in new tab)</option>`
            + `<option value="_parent" ${link.target === "_parent" ? "selected" : ""}>_parent</option>`
            + `<option value="_top" ${link.target === "_top" ? "selected" : ""}>_top</option>`
            + `<option value="_unfencedTop" ${link.target === "_unfencedTop" ? "selected" : ""}>_unfencedTop</option>`
            + "</select>"
            + "<div style='display: flex; justify-content: space-between'><button id='fixed-link-tool-abort' class='btn btn-sm btn-secondary mt-1'>Cancel</button><button id='fixed-link-tool-save' class='btn btn-sm btn-primary mt-1'>Save</button><button id='fixed-link-tool-delete' class='btn btn-sm btn-danger mt-1'>Delete</button></div>"
            + "</div>";
        toolbar.insertAdjacentHTML('afterend', settings_dialog_html);

        const settings_dialog: HTMLElement = toolbar.parentElement!.querySelector('.fixed-link-tool-settings') as HTMLElement;
        settings_dialog.style.left = toolbar.style.left;
        const currentTop = parseInt(toolbar.style.top, 10);
        settings_dialog.style.top = (currentTop + 40) + 'px';

        document.getElementById("fixed-link-tool-abort")!.addEventListener('click', () => {
            settings_dialog.remove();
        });

        document.getElementById("fixed-link-tool-save")!.addEventListener('click', () => {
            const href = (document.getElementById("fixed-link-tool-settings-href") as HTMLInputElement).value;
            const rel = (document.getElementById("fixed-link-tool-settings-rel") as HTMLInputElement).value;
            const target = (document.getElementById("fixed-link-tool-settings-target") as HTMLSelectElement).value;

            if(!href || !target){
                alert("Please enter the link target!");
                return;
            }

            settings_dialog.remove();
            link.href = href.trim();
            if(rel && rel.trim()){
                link.rel = rel.trim();
            }
            link.target = target;
        });
        document.getElementById("fixed-link-tool-delete")!.addEventListener('click', () => {
            const inner_nodes : ChildNode[] = Array.from(link.childNodes);
            link.replaceWith(...inner_nodes);
            settings_dialog.remove();
        });
    }

    surround(range: Range){
        if (this.state) { // Already a link
            this.show_change_dialog(range);
        }else { // Not a link
            this.show_create_dialog(range);
        }
    }

    checkState(selection: any) {
        const link = this.api.selection.findParentTag(this.tag);
        this.state = !!link;
    }

    static get sanitize() {
        return {
            a: true
        };
    }

    static get title(){
        return 'Link';
    }
}

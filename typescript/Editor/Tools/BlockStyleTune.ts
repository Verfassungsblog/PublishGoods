export class BlockStyleTune{
    private api: any;
    private data: any;

    private block: any;

    static get isTune() {
        return true;
    }

    // @ts-ignore
    constructor({api, data, block}){
        this.api = api;
        this.block = block;
        this.data = {
            css_classes: undefined
        };

        if(data && data.css_classes){
            this.data.css_classes = data.css_classes;
        }
    }

    get css_classes() {
        return this.data.css_classes || '';
    }

    set css_classes(classes) {
        if (classes.length > 0) {
            this.data.css_classes = classes;
        } else {
            this.data.css_classes = undefined;
        }
    }

    render():HTMLDivElement{
        let wrapper = document.createElement('div');
        let wrapper_input = document.createElement('input');
        wrapper_input.type = 'text';
        wrapper_input.placeholder = 'css-class-one css-class-two';
        wrapper_input.classList.add('cdx-input');
        wrapper_input.value = this.css_classes;

        wrapper_input.addEventListener('input', (event) => {
            this.css_classes = (<HTMLInputElement>event.target).value;
            
            if (this.block && typeof this.block.dispatchChange === 'function') {
                this.block.dispatchChange();
            } else {
               console.error("Couldn't dispatch change event for block: " + this.block);
            }
        });
        wrapper.appendChild(wrapper_input);
        return wrapper;
    }

    save() : any|undefined{
        if(!this.data.css_classes){
            return undefined;
        }
        return this.data;
    }
}
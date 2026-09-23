export function hide_all(class_name: string){
    // @ts-ignore
    for(let element of document.getElementsByClassName(class_name)){
        element.classList.add("hide");
    }
}
export function start_loading_spinner(){
    let loading_spinner = document.getElementById("loading_spinner");
    if(loading_spinner !== null){
        loading_spinner.style.display = "block";
    }
}

export function stop_loading_spinner(){
    let loading_spinner = document.getElementById("loading_spinner");
    if(loading_spinner !== null){
        loading_spinner.style.display = "none";
    }
}

export function show_alert(message: string, type: string = "danger|warning|success|info|primary|secondary|light|dark"){
    let id = Math.floor(Math.random() * 100000000);

    // @ts-ignore
    let alert_html = Handlebars.templates.alert_tmpl({"message": message, "type": type, "id": id});

    //Insert alert as first element of body
    document.body.insertAdjacentHTML("afterbegin", alert_html);

    let alert = document.getElementById("alert_" + id);
    alert.getElementsByClassName("alert-close")[0].addEventListener("click", function(){
        alert.remove();
    });

    setTimeout(() => {
        if (alert !== null) {
            alert.remove();
        }
    }, 5000);
}

let overlay_keydown_handler: ((e: KeyboardEvent) => void) | null = null;
let overlay_on_close: (() => void) | null = null;

/**
 * Shows the page's faded-out-background overlay (#overlay-wrapper / #inner_overlay,
 * see editor.html.hbs / bibliography.html.hbs) filled with the given content.
 * Closes on Escape or via the overlay's close button; call hide_overlay() to close
 * it programmatically (e.g. after a wizard step finishes).
 *
 * @param content - HTML string or element to render inside the overlay
 * @param on_close - Optional callback invoked once, whenever the overlay closes
 */
export function show_overlay(content: string | HTMLElement, on_close?: () => void){
    let overlay_wrapper = document.getElementById("overlay-wrapper");
    let overlay_content = document.getElementById("inner_overlay");
    let overlay_close_btn = document.getElementById("overlay_close_btn");

    if(overlay_wrapper === null || overlay_content === null){
        console.error("show_overlay: #overlay-wrapper / #inner_overlay not found on this page");
        return;
    }

    if(typeof content === "string"){
        overlay_content.innerHTML = content;
    } else {
        overlay_content.innerHTML = "";
        overlay_content.appendChild(content);
    }

    overlay_on_close = on_close || null;
    overlay_wrapper.classList.remove("hide");

    if(overlay_close_btn){
        overlay_close_btn.onclick = hide_overlay;
    }

    if(overlay_keydown_handler){
        document.removeEventListener("keydown", overlay_keydown_handler);
    }
    overlay_keydown_handler = function(e: KeyboardEvent){
        if(e.key === "Escape"){
            hide_overlay();
        }
    };
    document.addEventListener("keydown", overlay_keydown_handler);
}

/**
 * Hides the overlay opened by show_overlay(), clears its content and fires the
 * on_close callback (if one was given to show_overlay()).
 */
export function hide_overlay(){
    let overlay_wrapper = document.getElementById("overlay-wrapper");
    let overlay_content = document.getElementById("inner_overlay");

    if(overlay_wrapper !== null){
        overlay_wrapper.classList.add("hide");
    }
    if(overlay_content !== null){
        overlay_content.innerHTML = "";
    }

    if(overlay_keydown_handler){
        document.removeEventListener("keydown", overlay_keydown_handler);
        overlay_keydown_handler = null;
    }

    let on_close = overlay_on_close;
    overlay_on_close = null;
    if(on_close){
        on_close();
    }
}

interface SearchAPIHandler{
    (search_query: string): Promise<any[]>;
}

interface SearchResultClickHandler{
    (entry: HTMLElement): void;
}

/**
 * Add a new searchbar
 * @param searchbar - The searchbar as an HTMLInputElement
 * @param search_result_overlay - A div/ul/etc that will be filled with the rendered search results
 * @param search_api_handler - A function that retrieves the search results and returns them as an array of objects
 * @param search_result_entry_template - A handlebars template function that parses a single search result object
 * @param on_search_result_click - A function that is called when a search result is clicked
 */
export function add_search(searchbar: HTMLInputElement, search_result_overlay: HTMLElement, search_api_handler: SearchAPIHandler, search_result_entry_template: Function, on_search_result_click: SearchResultClickHandler){
    let hide_result_overlay = function(e: Event){
        let target = e.target as HTMLElement;

        if(target !== search_result_overlay && target !== searchbar){
            if(target != null){
                if(target.parentElement === search_result_overlay){
                    return;
                }
            }
            search_result_overlay.classList.add("hide");
            window.removeEventListener("click", hide_result_overlay);
            window.removeEventListener("focusin", hide_result_overlay);
        }
    }

    let show_result_overlay = function(){
        search_result_overlay.classList.remove("hide");
        window.addEventListener("click", hide_result_overlay);
        window.addEventListener("focusin", hide_result_overlay);
    }

    searchbar.addEventListener("click", show_result_overlay);
    searchbar.addEventListener("focus", show_result_overlay);

    searchbar.addEventListener("input", async function () {
        try {
            let res: any[] = await search_api_handler(searchbar.value);
            search_result_overlay.innerHTML = "";
            show_result_overlay();

            window.addEventListener("click", hide_result_overlay);
            window.addEventListener("focusin", hide_result_overlay);

            for(let entry of res){
                let rendered : string = search_result_entry_template(entry);
                search_result_overlay.insertAdjacentHTML("beforeend", rendered);
                let added_entry = search_result_overlay.lastElementChild as HTMLElement;
                added_entry.addEventListener("click", function(){
                    search_result_overlay.classList.add("hide");
                    searchbar.value = "";
                    on_search_result_click(added_entry);
                });
            }
        } catch (e) {
            console.error(e);
            show_alert("Couldn't fetch search results: " + e);
        }

    });
}

/**
 * Adds event listeners to all elements matching the specified query selector.
 *
 * @param {string} query_selector - A DOMString containing one or more selectors to match.
 * @param {string} event - The name of the event to listen for (e.g., 'click', 'mouseover').
 * @param {EventListener} listener - The callback function that will be invoked when the event is triggered on the target elements.
 * @return {void} Does not return a value.
 */
export function add_event_listeners(query_selector: string, event: string, listener: EventListener){
    let targets = document.querySelectorAll(query_selector);
    targets.forEach(target => {
        target.addEventListener(event, listener);
    })
}
import {Person, PersonsAPI} from "../api_requests";
import {hide_overlay, show_alert, show_overlay} from "../tools";

const personsApi = PersonsAPI();

function identifier_value(person_identifier: {value?: string} | null | undefined): string {
    return person_identifier?.value ?? "";
}

function input_value(id: string): string {
    return (document.getElementById(id) as HTMLInputElement | null)?.value.trim() ?? "";
}

/** Builds the identifier object the backend expects, or null for an empty input. */
function to_identifier(type: "ORCID" | "GND" | "ROR", value: string): any {
    return value ? {name: type, value: value, identifier_type: type} : null;
}

function bio_textareas(): HTMLTextAreaElement[] {
    return Array.from(document.querySelectorAll<HTMLTextAreaElement>("#overlay_person_biographies .person_biography"));
}

function display_name(person: Person): string {
    return [person.first_names, person.last_names].filter(Boolean).join(" ");
}

interface PersonOverlayOptions {
    /** The project the person is added to (create) or belongs to (edit). */
    project_id: string;
    /** Edit this person instead of creating a new one. */
    person?: Person | null;
    /** Called after the person was saved, with the stored person. */
    on_saved: (person: Person) => void | Promise<void>;
}

/**
 * Opens the overlay to create a person (by default only in the given project, optionally in the
 * project's team library) or, if `options.person` is given, to edit that person.
 */
export async function open_person_overlay(options: PersonOverlayOptions){
    const is_new = !options.person;
    // Editing replaces the stored biographies with whatever we send, so start from the full record.
    let existing: Person | null = null;
    if(options.person && options.person.id){
        try{
            existing = await personsApi.send_get_person_request(options.person.id);
        }catch(e){
            console.error("Failed to load person", e);
            show_alert("Couldn't load the person.", "danger");
            return;
        }
    }

    // @ts-ignore
    show_overlay(Handlebars.templates.editor_person_overlay({
        is_new: is_new,
        first_names: existing?.first_names ?? "",
        last_names: existing?.last_names ?? "",
        orcid: identifier_value(existing?.orcid),
        gnd: identifier_value(existing?.gnd),
        ror: identifier_value(existing?.ror),
        bios: (existing?.bios ?? []).map(bio => ({language_code: bio.lang, content: bio.content}))
    }));

    const save_btn = document.getElementById("overlay_person_save") as HTMLButtonElement | null;
    if(!save_btn){
        return;
    }

    document.getElementById("overlay_person_add_bio_btn")?.addEventListener("click", function(){
        const select = document.getElementById("overlay_person_add_bio_lang") as HTMLSelectElement;
        const language_code = select.value;
        if(!language_code || language_code === "none"){
            return;
        }
        if(bio_textareas().some(textarea => textarea.dataset.language === language_code)){
            show_alert("There is already a biography for this language.", "warning");
            return;
        }
        // @ts-ignore
        document.getElementById("overlay_person_biographies")?.insertAdjacentHTML("beforeend", Handlebars.templates.persons_bio_edit({language_code: language_code}));
        select.selectedIndex = 0;
    });

    save_btn.addEventListener("click", async function(){
        const last_names = input_value("overlay_person_last_names");
        if(!last_names){
            show_alert("Please input at least a last name.", "danger");
            return;
        }

        const fields = {
            first_names: input_value("overlay_person_first_names") || null,
            last_names: last_names,
            orcid: to_identifier("ORCID", input_value("overlay_person_orcid")),
            gnd: to_identifier("GND", input_value("overlay_person_gnd")),
            ror: to_identifier("ROR", input_value("overlay_person_ror")),
            // Empty textareas are dropped, so clearing a biography removes it.
            bios: bio_textareas()
                .filter(textarea => textarea.value.trim() !== "")
                .map(textarea => ({lang: textarea.dataset.language, content: textarea.value}))
        };

        save_btn.disabled = true;
        try{
            let saved: Person;
            if(existing){
                saved = await personsApi.send_update_person_request({...existing, ...fields} as Person);
            }else{
                const to_team = (document.getElementById("overlay_person_scope_team") as HTMLInputElement | null)?.checked;
                saved = await personsApi.send_create_person_request({
                    ...fields,
                    scope: {Project: options.project_id}
                } as Person);
                if(to_team && saved.id){
                    // Created for the project first; the team library is a promotion of it.
                    try{
                        saved = await personsApi.send_promote_person_request(saved.id);
                    }catch(e){
                        console.error("Failed to move person to the team library", e);
                        show_alert("The person was created, but only for this project. Use \"Move to team persons\" to share it.", "warning");
                    }
                }
            }
            hide_overlay();
            await options.on_saved(saved);
        }catch(e){
            console.error("Failed to save person", e);
            show_alert("Couldn't save the person.", "danger");
            save_btn.disabled = false;
        }
    });
}

/**
 * Asks for confirmation and deletes the person. If it is still used in projects, the user sees
 * where first; its entries then remain in those projects as plain names.
 *
 * @returns true if the person was deleted
 */
export async function confirm_and_delete_person(person: Person): Promise<boolean> {
    if(!person.id){
        return false;
    }
    const person_id = person.id;

    let usages;
    try{
        usages = await personsApi.send_person_usage_request(person_id);
    }catch(e){
        console.error("Failed to load person usage", e);
        show_alert("Couldn't check where this person is used.", "danger");
        return false;
    }

    return new Promise<boolean>((resolve) => {
        // @ts-ignore
        show_overlay(Handlebars.templates.editor_person_delete_overlay({
            name: display_name(person),
            used: usages.length > 0,
            usages: usages
        }), () => resolve(false));

        document.getElementById("overlay_person_cancel_delete")?.addEventListener("click", () => hide_overlay());
        document.getElementById("overlay_person_confirm_delete")?.addEventListener("click", async () => {
            try{
                await personsApi.send_delete_person_request(person_id, usages.length > 0);
            }catch(e){
                console.error("Failed to delete person", e);
                show_alert("Couldn't delete the person.", "danger");
                return;
            }
            // Settle before closing: closing fires the on_close callback, which resolves false.
            resolve(true);
            hide_overlay();
        });
    });
}

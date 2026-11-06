///<reference path="./General.ts"/>
namespace Persons {
    /** The team whose person library is being worked on: the selector on the list page, or the hidden field on the create page. */
    function current_team_id(): string | null {
        let select = document.getElementById("persons_team_select") as HTMLSelectElement | null;
        if(select !== null){
            return select.value;
        }
        let hidden = document.getElementById("person_team_id") as HTMLInputElement | null;
        return hidden !== null ? hidden.value : null;
    }

    function persons_list_url(): string {
        let team_id = current_team_id();
        return team_id === null ? "/persons" : `/persons?team_id=${encodeURIComponent(team_id)}`;
    }

    // @ts-ignore
    export function init_person_form(){
        // Disable bio languages which are already present
        let bio_languages = document.getElementsByClassName("person_biography");
        // @ts-ignore
        for(let bio of bio_languages){
            let language = bio.getAttribute("data-language");
            let language_option = document.getElementById("add_biography_lang_" + language) as HTMLOptionElement;
            if(language_option !== null){
                language_option.disabled = true;
            }
        }

        let add_biography_button = document.getElementById("add_bio_btn");
        if(add_biography_button !== null){
            add_biography_button.addEventListener("click", add_biography_handler);
        }
        let create_project_button = document.getElementById("create_project_btn");
        if(create_project_button !== null){
            create_project_button.addEventListener("click", create_project_handler);
        }
        let delete_button = document.getElementById("delete_person_btn") as HTMLButtonElement | null;
        if(delete_button !== null){
            delete_button.addEventListener("click", delete_person_handler);
        }
    }

    // @ts-ignore
    async function delete_person_handler(){
        let person_id = (document.getElementById("person_id") as HTMLInputElement).value;
        if(person_id === ""){
            Tools.show_alert("Failed to delete person: no ID.", "danger");
            return;
        }
        try{
            // Persons that are still used in projects are only deleted after the user confirmed, seeing where.
            let usages = await load_person_usage(person_id);
            let confirmed: boolean;
            if(usages.length === 0){
                confirmed = confirm("Are you sure you want to delete this person?");
            }else{
                confirmed = confirm(usage_warning(usages));
            }
            if(!confirmed){
                return;
            }
            await send_delete_person(person_id, usages.length > 0);
            Tools.show_alert("Person deleted.", "success");
            window.location.href = persons_list_url();
        }catch (e) {
            console.log(e);
            Tools.show_alert("Failed to delete person.", "danger");
        }
    }

    /** Warning text listing every project (and section) that uses the person. */
    function usage_warning(usages: any[]): string {
        let lines = [];
        for(let usage of usages){
            let where = usage["project_title"];
            if(usage["section_title"]){
                where += " (section: " + usage["section_title"] + ")";
            }
            lines.push("- " + where + " [" + usage["role"] + "]");
        }
        return "This person is still used in the following projects:\n\n" + lines.join("\n")
            + "\n\nIf you delete them, they will remain in these projects as plain names. Delete anyway?";
    }

    async function load_person_usage(id: string): Promise<any[]> {
        Tools.start_loading_spinner();
        let response = await fetch(`/api/persons/${id}/usage`);
        Tools.stop_loading_spinner();
        if(!response.ok){
            throw new Error("Failed to load person usage.");
        }
        let response_data = await response.json();
        if(response_data.hasOwnProperty("error")){
            throw new Error(`Failed to load person usage: ${response_data["error"]}`);
        }
        return response_data["data"];
    }

    // @ts-ignore
    export async function add_biography_handler(){
        let language_select = document.getElementById("add_biography_lang") as HTMLSelectElement;
        if(language_select.value && language_select.value !== "none" && !language_select.options[language_select.selectedIndex].disabled){
            let language = {};
            language["language_label"] = language_select.options[language_select.selectedIndex].text;
            language["language_code"] = language_select.value;
            let biographies = document.getElementById("biographies") as HTMLDivElement;
            // @ts-ignore
            let biography = Handlebars.templates.persons_bio_edit(language);
            biographies.insertAdjacentHTML("beforeend", biography);
            // @ts-ignore
            for(let element of document.getElementsByClassName("persons_edit_field")){
                element.addEventListener("change", edit_person_data_handler)
            }

            // Disable language option
            let language_option = document.getElementById("add_biography_lang_" + language_select.value) as HTMLOptionElement;
            if(language_option !== null){
                language_option.disabled = true;
            }
        }
    }

    // @ts-ignore
    export async function create_project_handler(){
        let data = {};
        data["first_names"] = (document.getElementById("person_first_names") as HTMLInputElement).value || null;

        data["last_names"] = (document.getElementById("person_last_names") as HTMLInputElement).value || null;
        if(data["last_names"] === null){
            Tools.show_alert("Please input at least a last name.", "danger");
            return;
        }

        data["orcid"] = (document.getElementById("person_orcid") as HTMLInputElement).value || null;

        if(data["orcid"] !== null){
            data["orcid"] = {
                    "name": "ORCID",
                    "value": data["orcid"],
                    "identifier_type": "ORCID"
            }
        }
        data["gnd"] = (document.getElementById("person_gnd") as HTMLInputElement).value || null;

        if(data["gnd"] !== null){
            data["gnd"] = {
                "name": "GND",
                "value": data["gnd"],
                "identifier_type": "GND"
            }
        }
        data["ror"] = (document.getElementById("person_ror") as HTMLInputElement).value || null;

        if(data["ror"] !== null){
            data["ror"] = {
                "name": "ROR",
                "value": data["ror"],
                "identifier_type": "ROR"
            }
        }

        let bios = document.getElementsByClassName("person_biography");
        let bios_res = [];

        // @ts-ignore
        for(let bio of bios){
            let bio_data = {};
            bio_data["lang"] = bio.getAttribute("data-language");
            bio_data["content"] = (bio as HTMLTextAreaElement).value;
            if (bio_data["content"] !== ""){
                bios_res.push(bio_data);
            }
        }
        data["bios"] = bios_res;

        // New persons created here go into the library of the selected team
        let team_id = current_team_id();
        if(team_id === null || team_id === ""){
            Tools.show_alert("No team selected.", "danger");
            return;
        }
        data["scope"] = {"Team": team_id};

        console.log(data);
        try{
            let person_id = await send_person_data(data);
            console.log(person_id);
            //TODO: redirect to person page
            window.location.href = persons_list_url();
        }catch (e) {
            console.log(e);
            Tools.show_alert("Failed to create person.", "danger");
        }
    }

    async function send_person_data(data){
        Tools.start_loading_spinner();
        let response = await fetch("/api/persons", {
            method: "POST",
            headers: {
                "Content-Type": "application/json"
            },
            body: JSON.stringify(data)
        });

        Tools.stop_loading_spinner();
        if(response.ok){
            let response_data = await response.json();
            console.log(response_data);
            if(response_data.hasOwnProperty("error")){
                throw new Error(`Failed to create person: ${response_data["error"]}`);
            }else{
                if (!response_data["data"].hasOwnProperty("id")) {
                    throw new Error(`Failed to create person: ${response_data}`);
                }else{
                    return response_data["data"]["id"];
                }
            }
        }else{
            throw new Error(`Failed to create person.`);
        }
    }

    async function send_delete_person(id, confirm_in_use: boolean){
        Tools.start_loading_spinner();
        let response = await fetch("/api/persons/" + id + (confirm_in_use ? "?confirm=true" : ""), {
            method: "DELETE",
            headers: {
                "Content-Type": "application/json"
            }
        });

        Tools.stop_loading_spinner();
        if(response.ok){
            let response_data = await response.json();
            if(response_data.hasOwnProperty("error")){
                throw new Error(`Failed to delete person: ${response_data["error"]}`);
            }else{
                return true;
            }
        }else{
            throw new Error(`Failed to create person.`);
        }
    }

    async function update_person_data(data){
        Tools.start_loading_spinner();
        let response = await fetch(`/api/persons/${data.id}`, {
            method: "PUT",
            headers: {
                "Content-Type": "application/json"
            },
            body: JSON.stringify(data)
        });

        Tools.stop_loading_spinner();
        if(response.ok){
            let response_data = await response.json();
            console.log(response_data);
            if(response_data.hasOwnProperty("error")){
                throw new Error(`Failed to create person: ${response_data["error"]}`);
            }else{
                if (!response_data["data"].hasOwnProperty("id")) {
                    throw new Error(`Failed to create person: ${response_data}`);
                }else{
                    return response_data["data"]["id"];
                }
            }
        }else{
            throw new Error(`Failed to create person.`);
        }
    }

    // @ts-ignore
    export async function init_list(){
        let team_select = document.getElementById("persons_team_select") as HTMLSelectElement | null;
        if(team_select !== null){
            team_select.addEventListener("change", function(){
                window.location.href = `/persons?team_id=${encodeURIComponent(team_select.value)}`;
            });
        }
        // @ts-ignore
        for(let row of document.getElementsByClassName("persons_list_row")){
            row.addEventListener("click", persons_list_row_click_handler);
        }
    }

    // @ts-ignore
    async function persons_list_row_click_handler(row){
        let id = row.currentTarget.getAttribute("data-person-id");
        let data = await load_person_data(id);
        // @ts-ignore
        document.getElementById("persons_view_panel_inner").innerHTML = Handlebars.templates.persons_edit(data);
        init_person_form();
        // Add listener for all changes to any field
        // @ts-ignore
        for(let element of document.getElementsByClassName("persons_edit_field")){
            element.addEventListener("change", edit_person_data_handler)
        }
    }

    // @ts-ignore
    async function edit_person_data_handler(){
        let data = {};
        data["id"] = (document.getElementById("person_id") as HTMLInputElement).value || null;
        if(data["id"] === null){
            Tools.show_alert("Failed to update person: no ID.", "danger");
            return;
        }
        data["first_names"] = (document.getElementById("person_first_names") as HTMLInputElement).value || null;

        data["last_names"] = (document.getElementById("person_last_names") as HTMLInputElement).value || null;
        if(data["last_names"] === null){
            Tools.show_alert("Please input at least a last name.", "danger");
            return;
        }

        data["orcid"] = (document.getElementById("person_orcid") as HTMLInputElement).value || null;

        if(data["orcid"] !== null){
            data["orcid"] = {
                "name": "ORCID",
                "value": data["orcid"],
                "identifier_type": "ORCID"
            }
        }
        data["gnd"] = (document.getElementById("person_gnd") as HTMLInputElement).value || null;

        if(data["gnd"] !== null){
            data["gnd"] = {
                "name": "GND",
                "value": data["gnd"],
                "identifier_type": "GND"
            }
        }
        data["ror"] = (document.getElementById("person_ror") as HTMLInputElement).value || null;

        if(data["ror"] !== null){
            data["ror"] = {
                "name": "ROR",
                "value": data["ror"],
                "identifier_type": "ROR"
            }
        }

        let bios = document.getElementsByClassName("person_biography");
        let bios_res = [];

        // @ts-ignore
        for(let bio of bios){
            let bio_data = {};
            bio_data["lang"] = bio.getAttribute("data-language");
            bio_data["content"] = (bio as HTMLTextAreaElement).value;
            if (bio_data["content"] !== ""){
                bios_res.push(bio_data);
            }
        }
        data["bios"] = bios_res;

        console.log(data);
        try{
            await update_person_data(data);
            Tools.show_alert("Person updated.", "success");
        }catch (e) {
            console.log(e);
            Tools.show_alert("Failed to update person.", "danger");
        }
    }

    // @ts-ignore
    async function load_person_data(id: string) {
        Tools.start_loading_spinner();
        let response = await fetch(`/api/persons/${id}`);
        Tools.stop_loading_spinner();
        if (response.ok) {
            let response_data = await response.json();
            if (response_data.hasOwnProperty("error")) {
                throw new Error(`Failed to load person: ${response_data["error"]}`);
            } else {
                if (!response_data["data"]) {
                    throw new Error(`Failed to load person: ${response_data}`);
                } else {
                    return response_data["data"];
                }
            }
        } else {
            throw new Error(`Failed to load person.`);
        }
    }
}

// @ts-ignore
window.addEventListener("load", async function(){
    if(window.location.pathname === "/persons/create"){
        await Persons.init_person_form();
    }else{
        await Persons.init_list();
    }
});

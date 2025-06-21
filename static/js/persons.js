var __awaiter = (this && this.__awaiter) || function (thisArg, _arguments, P, generator) {
    function adopt(value) { return value instanceof P ? value : new P(function (resolve) { resolve(value); }); }
    return new (P || (P = Promise))(function (resolve, reject) {
        function fulfilled(value) { try { step(generator.next(value)); } catch (e) { reject(e); } }
        function rejected(value) { try { step(generator["throw"](value)); } catch (e) { reject(e); } }
        function step(result) { result.done ? resolve(result.value) : adopt(result.value).then(fulfilled, rejected); }
        step((generator = generator.apply(thisArg, _arguments || [])).next());
    });
};
var _a;
var Tools;
(function (Tools) {
    function hide_all(class_name) {
        // @ts-ignore
        for (let element of document.getElementsByClassName(class_name)) {
            element.classList.add("hide");
        }
    }
    Tools.hide_all = hide_all;
    function start_loading_spinner() {
        let loading_spinner = document.getElementById("loading_spinner");
        if (loading_spinner !== null) {
            loading_spinner.style.display = "block";
        }
    }
    Tools.start_loading_spinner = start_loading_spinner;
    function stop_loading_spinner() {
        let loading_spinner = document.getElementById("loading_spinner");
        if (loading_spinner !== null) {
            loading_spinner.style.display = "none";
        }
    }
    Tools.stop_loading_spinner = stop_loading_spinner;
    function show_alert(message, type = "danger|warning|success|info|primary|secondary|light|dark") {
        let id = Math.floor(Math.random() * 100000000);
        // @ts-ignore
        let alert_html = Handlebars.templates.alert_tmpl({ "message": message, "type": type, "id": id });
        //Insert alert as first element of body
        document.body.insertAdjacentHTML("afterbegin", alert_html);
        let alert = document.getElementById("alert_" + id);
        alert.getElementsByClassName("alert-close")[0].addEventListener("click", function () {
            alert.remove();
        });
        setTimeout(() => {
            if (alert !== null) {
                alert.remove();
            }
        }, 5000);
    }
    Tools.show_alert = show_alert;
})(Tools || (Tools = {}));
// Add click listener for mobile navbar
(_a = document.getElementById("show_mobile_navbar")) === null || _a === void 0 ? void 0 : _a.addEventListener("click", function () {
    console.log("Adding click listener for mobile navbar");
    let navbar = document.getElementById("mobile_navbar");
    if (navbar !== null) {
        navbar.classList.toggle("hide");
    }
    else {
        console.log("Navbar is null");
    }
});
// Register Handlebars helpers
// @ts-ignore
Handlebars.registerHelper("is", function (arg1, arg2, options) {
    // @ts-ignore
    return (arg1 == arg2) ? options.fn(this) : options.inverse(this);
});
/**
 * Creates a base64 string from any utf-8 string
 */
// @ts-ignore
Handlebars.registerHelper("base64", function (arg1) {
    let bytes = new TextEncoder().encode(arg1);
    // @ts-ignore
    let binstring = Array.from(bytes, (byte) => // @ts-ignore
     String.fromCodePoint(byte)).join("");
    return btoa(binstring);
});
///<reference path="./General.ts"/>
var Persons;
///<reference path="./General.ts"/>
(function (Persons) {
    // @ts-ignore
    function init_person_form() {
        // Disable bio languages which are already present
        let bio_languages = document.getElementsByClassName("person_biography");
        // @ts-ignore
        for (let bio of bio_languages) {
            let language = bio.getAttribute("data-language");
            let language_option = document.getElementById("add_biography_lang_" + language);
            if (language_option !== null) {
                language_option.disabled = true;
            }
        }
        let add_biography_button = document.getElementById("add_bio_btn");
        if (add_biography_button !== null) {
            add_biography_button.addEventListener("click", add_biography_handler);
        }
        let create_project_button = document.getElementById("create_project_btn");
        if (create_project_button !== null) {
            create_project_button.addEventListener("click", create_project_handler);
        }
        let delete_button = document.getElementById("delete_person_btn");
        delete_button.addEventListener("click", delete_person_handler);
    }
    Persons.init_person_form = init_person_form;
    // @ts-ignore
    function delete_person_handler() {
        return __awaiter(this, void 0, void 0, function* () {
            let person_id = document.getElementById("person_id").value;
            if (person_id === "") {
                Tools.show_alert("Failed to delete person: no ID.", "danger");
                return;
            }
            let confirmed = confirm("Are you sure you want to delete this person?");
            if (confirmed) {
                try {
                    yield send_delete_person(person_id);
                    Tools.show_alert("Person deleted.", "success");
                    window.location.href = "/persons/";
                }
                catch (e) {
                    console.log(e);
                    Tools.show_alert("Failed to delete person.", "danger");
                }
            }
        });
    }
    // @ts-ignore
    function add_biography_handler() {
        return __awaiter(this, void 0, void 0, function* () {
            let language_select = document.getElementById("add_biography_lang");
            if (language_select.value && language_select.value !== "none" && !language_select.options[language_select.selectedIndex].disabled) {
                let language = {};
                language["language_label"] = language_select.options[language_select.selectedIndex].text;
                language["language_code"] = language_select.value;
                let biographies = document.getElementById("biographies");
                // @ts-ignore
                let biography = Handlebars.templates.persons_bio_edit(language);
                biographies.insertAdjacentHTML("beforeend", biography);
                // @ts-ignore
                for (let element of document.getElementsByClassName("persons_edit_field")) {
                    element.addEventListener("change", edit_person_data_handler);
                }
                // Disable language option
                let language_option = document.getElementById("add_biography_lang_" + language_select.value);
                if (language_option !== null) {
                    language_option.disabled = true;
                }
            }
        });
    }
    Persons.add_biography_handler = add_biography_handler;
    // @ts-ignore
    function create_project_handler() {
        return __awaiter(this, void 0, void 0, function* () {
            let data = {};
            data["first_names"] = document.getElementById("person_first_names").value || null;
            data["last_names"] = document.getElementById("person_last_names").value || null;
            if (data["last_names"] === null) {
                Tools.show_alert("Please input at least a last name.", "danger");
                return;
            }
            data["orcid"] = document.getElementById("person_orcid").value || null;
            if (data["orcid"] !== null) {
                data["orcid"] = {
                    "name": "ORCID",
                    "value": data["orcid"],
                    "identifier_type": "ORCID"
                };
            }
            data["gnd"] = document.getElementById("person_gnd").value || null;
            if (data["gnd"] !== null) {
                data["gnd"] = {
                    "name": "GND",
                    "value": data["gnd"],
                    "identifier_type": "GND"
                };
            }
            data["ror"] = document.getElementById("person_ror").value || null;
            if (data["ror"] !== null) {
                data["ror"] = {
                    "name": "ROR",
                    "value": data["ror"],
                    "identifier_type": "ROR"
                };
            }
            let bios = document.getElementsByClassName("person_biography");
            let bios_res = [];
            // @ts-ignore
            for (let bio of bios) {
                let bio_data = {};
                bio_data["lang"] = bio.getAttribute("data-language");
                bio_data["content"] = bio.value;
                if (bio_data["content"] !== "") {
                    bios_res.push(bio_data);
                }
            }
            data["bios"] = bios_res;
            console.log(data);
            try {
                let person_id = yield send_person_data(data);
                console.log(person_id);
                //TODO: redirect to person page
                window.location.href = `/persons/`;
            }
            catch (e) {
                console.log(e);
                Tools.show_alert("Failed to create person.", "danger");
            }
        });
    }
    Persons.create_project_handler = create_project_handler;
    function send_person_data(data) {
        return __awaiter(this, void 0, void 0, function* () {
            Tools.start_loading_spinner();
            let response = yield fetch("/api/persons", {
                method: "POST",
                headers: {
                    "Content-Type": "application/json"
                },
                body: JSON.stringify(data)
            });
            Tools.stop_loading_spinner();
            if (response.ok) {
                let response_data = yield response.json();
                console.log(response_data);
                if (response_data.hasOwnProperty("error")) {
                    throw new Error(`Failed to create person: ${response_data["error"]}`);
                }
                else {
                    if (!response_data["data"].hasOwnProperty("id")) {
                        throw new Error(`Failed to create person: ${response_data}`);
                    }
                    else {
                        return response_data["data"]["id"];
                    }
                }
            }
            else {
                throw new Error(`Failed to create person.`);
            }
        });
    }
    function send_delete_person(id) {
        return __awaiter(this, void 0, void 0, function* () {
            Tools.start_loading_spinner();
            let response = yield fetch("/api/persons/" + id, {
                method: "DELETE",
                headers: {
                    "Content-Type": "application/json"
                }
            });
            Tools.stop_loading_spinner();
            if (response.ok) {
                let response_data = yield response.json();
                if (response_data.hasOwnProperty("error")) {
                    throw new Error(`Failed to delete person: ${response_data["error"]}`);
                }
                else {
                    return true;
                }
            }
            else {
                throw new Error(`Failed to create person.`);
            }
        });
    }
    function update_person_data(data) {
        return __awaiter(this, void 0, void 0, function* () {
            Tools.start_loading_spinner();
            let response = yield fetch(`/api/persons/${data.id}`, {
                method: "PUT",
                headers: {
                    "Content-Type": "application/json"
                },
                body: JSON.stringify(data)
            });
            Tools.stop_loading_spinner();
            if (response.ok) {
                let response_data = yield response.json();
                console.log(response_data);
                if (response_data.hasOwnProperty("error")) {
                    throw new Error(`Failed to create person: ${response_data["error"]}`);
                }
                else {
                    if (!response_data["data"].hasOwnProperty("id")) {
                        throw new Error(`Failed to create person: ${response_data}`);
                    }
                    else {
                        return response_data["data"]["id"];
                    }
                }
            }
            else {
                throw new Error(`Failed to create person.`);
            }
        });
    }
    // @ts-ignore
    function init_list() {
        return __awaiter(this, void 0, void 0, function* () {
            // @ts-ignore
            for (let row of document.getElementsByClassName("persons_list_row")) {
                row.addEventListener("click", persons_list_row_click_handler);
            }
        });
    }
    Persons.init_list = init_list;
    // @ts-ignore
    function persons_list_row_click_handler(row) {
        return __awaiter(this, void 0, void 0, function* () {
            let id = row.currentTarget.getAttribute("data-person-id");
            let data = yield load_person_data(id);
            // @ts-ignore
            document.getElementById("persons_view_panel_inner").innerHTML = Handlebars.templates.persons_edit(data);
            init_person_form();
            // Add listener for all changes to any field
            // @ts-ignore
            for (let element of document.getElementsByClassName("persons_edit_field")) {
                element.addEventListener("change", edit_person_data_handler);
            }
        });
    }
    // @ts-ignore
    function edit_person_data_handler() {
        return __awaiter(this, void 0, void 0, function* () {
            let data = {};
            data["id"] = document.getElementById("person_id").value || null;
            if (data["id"] === null) {
                Tools.show_alert("Failed to update person: no ID.", "danger");
                return;
            }
            data["first_names"] = document.getElementById("person_first_names").value || null;
            data["last_names"] = document.getElementById("person_last_names").value || null;
            if (data["last_names"] === null) {
                Tools.show_alert("Please input at least a last name.", "danger");
                return;
            }
            data["orcid"] = document.getElementById("person_orcid").value || null;
            if (data["orcid"] !== null) {
                data["orcid"] = {
                    "name": "ORCID",
                    "value": data["orcid"],
                    "identifier_type": "ORCID"
                };
            }
            data["gnd"] = document.getElementById("person_gnd").value || null;
            if (data["gnd"] !== null) {
                data["gnd"] = {
                    "name": "GND",
                    "value": data["gnd"],
                    "identifier_type": "GND"
                };
            }
            data["ror"] = document.getElementById("person_ror").value || null;
            if (data["ror"] !== null) {
                data["ror"] = {
                    "name": "ROR",
                    "value": data["ror"],
                    "identifier_type": "ROR"
                };
            }
            let bios = document.getElementsByClassName("person_biography");
            let bios_res = [];
            // @ts-ignore
            for (let bio of bios) {
                let bio_data = {};
                bio_data["lang"] = bio.getAttribute("data-language");
                bio_data["content"] = bio.value;
                if (bio_data["content"] !== "") {
                    bios_res.push(bio_data);
                }
            }
            data["bios"] = bios_res;
            console.log(data);
            try {
                yield update_person_data(data);
                Tools.show_alert("Person updated.", "success");
            }
            catch (e) {
                console.log(e);
                Tools.show_alert("Failed to update person.", "danger");
            }
        });
    }
    // @ts-ignore
    function load_person_data(id) {
        return __awaiter(this, void 0, void 0, function* () {
            Tools.start_loading_spinner();
            let response = yield fetch(`/api/persons/${id}`);
            Tools.stop_loading_spinner();
            if (response.ok) {
                let response_data = yield response.json();
                if (response_data.hasOwnProperty("error")) {
                    throw new Error(`Failed to load person: ${response_data["error"]}`);
                }
                else {
                    if (!response_data["data"]) {
                        throw new Error(`Failed to load person: ${response_data}`);
                    }
                    else {
                        return response_data["data"];
                    }
                }
            }
            else {
                throw new Error(`Failed to load person.`);
            }
        });
    }
})(Persons || (Persons = {}));
// @ts-ignore
window.addEventListener("load", function () {
    return __awaiter(this, void 0, void 0, function* () {
        if (window.location.pathname === "/persons/create") {
            yield Persons.init_person_form();
        }
        else {
            yield Persons.init_list();
        }
    });
});

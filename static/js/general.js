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

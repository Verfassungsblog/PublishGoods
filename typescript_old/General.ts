namespace Tools{
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
}

// Add click listener for mobile navbar
document.getElementById("show_mobile_navbar")?.addEventListener("click", function(){
    console.log("Adding click listener for mobile navbar")
    let navbar = document.getElementById("mobile_navbar");
    if(navbar !== null){
        navbar.classList.toggle("hide");
    }else{
        console.log("Navbar is null")
    }
});

// Register Handlebars helpers
// @ts-ignore
Handlebars.registerHelper("is", function(arg1: unknown, arg2: unknown, options: unknown){
    // @ts-ignore
    return (arg1 == arg2) ? options.fn(this) : options.inverse(this);
});

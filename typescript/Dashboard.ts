import * as API from "./api_requests";

window.addEventListener("load", async function () {
    await show_pending_invitations_banner();
});

async function show_pending_invitations_banner(){
    const container = document.getElementById("dashboard_banner_container");
    if (container === null){
        return;
    }

    try {
        const invitations = await API.TeamsAPI().list_my_invitations();
        if (invitations.length === 0){
            return;
        }

        // @ts-ignore
        container.innerHTML = Handlebars.templates.dashboard_invitations_banner({
            count: invitations.length,
            plural: invitations.length !== 1
        });
    } catch (e) {
        console.error(e);
    }
}

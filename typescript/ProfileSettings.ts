import * as Tools from "./tools";
import * as API from "./api_requests";

window.addEventListener("load", async function () {
    await init();
});

const profile_settings_nav_account = document.getElementById("profile_settings_nav_account");
const profile_settings_nav_teams = document.getElementById("profile_settings_nav_teams");
const profile_settings_nav_api_keys = document.getElementById("profile_settings_nav_api_keys");
const tab_content = document.getElementById("tab-content");

const teams_api = API.TeamsAPI();
const api_keys_api = API.ApiKeysAPI();

/**
 * Marks exactly one of the three tab nav links as active, clearing the others. A plain
 * `classList.add`/`classList.remove` pair per tab-switch function (as used to work for
 * just Account/Teams) doesn't scale past two tabs: switching to a third tab would leave a
 * previous tab's "active" class in place unless every switch function also clears it.
 */
function set_active_tab(tab: "account" | "teams" | "api_keys"){
    profile_settings_nav_account.classList.toggle("active", tab === "account");
    profile_settings_nav_teams.classList.toggle("active", tab === "teams");
    profile_settings_nav_api_keys.classList.toggle("active", tab === "api_keys");
}

let current_user_id: string | null = null;
let current_user_email: string | null = null;

// Teams the current user belongs to, as last fetched from the server. Kept around so
// role-change/removal handlers can build the full replacement member list that
// PATCH /api/teams/<id> expects, without re-fetching.
let current_teams: API.Team[] = [];

function capitalize(s: string): string {
    return s.length === 0 ? s : s.charAt(0).toUpperCase() + s.slice(1);
}

async function init(){
    // Show account settings first (this is also what populates current_user_id/
    // current_user_email), THEN wire up tab navigation — so a click on the Teams tab
    // can never race ahead of current_user_id being set and see itself as a plain
    // "member" of its own teams.
    await show_account_settings();

    // Deep-link support (e.g. the pending-invitations banner on the home page links here
    // with #teams) — open the Teams tab right away if that's where we were sent.
    if (window.location.hash === "#teams"){
        await show_teams_settings();
    }

    profile_settings_nav_account.addEventListener("click", show_account_settings);
    profile_settings_nav_teams.addEventListener("click", show_teams_settings)
    profile_settings_nav_api_keys.addEventListener("click", show_api_keys_settings)
}

async function show_account_settings(){
    set_active_tab("account");

    try {
        let user = await API.send_get_current_user();
        current_user_id = user.id;
        current_user_email = user.email;

        // @ts-ignore
        tab_content.innerHTML = Handlebars.templates.profile_settings_account_tab({
            name: user.name,
            email: user.email
        });

        add_account_settings_listeners();
    } catch (e) {
        Tools.show_alert("Failed to load account settings.", "danger");
        console.error(e);
    }
}

function add_account_settings_listeners(){
    let save_button = tab_content.querySelector(".btn-primary");
    if (save_button !== null){
        save_button.addEventListener("click", save_account_settings);
    }

    let delete_button = document.getElementById("delete_account");
    delete_button.addEventListener("click", start_account_deletion);
}

async function start_account_deletion(){
    if (current_user_id === null || current_user_email === null){
        Tools.show_alert("Failed to start account deletion.", "danger");
        return;
    }

    // @ts-ignore
    Tools.show_overlay(Handlebars.templates.profile_settings_account_tab_delete_user_overlay({email: current_user_email}));

    let overlay_content = document.getElementById("inner_overlay");
    let confirm_input = <HTMLInputElement>overlay_content.querySelector("#delete_account_confirm_email");
    let confirm_button = <HTMLButtonElement>overlay_content.querySelector("#confirm_delete_account");

    confirm_button.addEventListener("click", async function (){
        let typed_email = confirm_input.value.trim();
        if (typed_email.toLowerCase() !== current_user_email.toLowerCase()){
            Tools.show_alert("The email address doesn't match.", "danger");
            return;
        }

        try {
            await API.send_delete_own_account(typed_email);
            Tools.hide_overlay();
            window.location.href = "/login";
        } catch (e) {
            Tools.show_alert("Failed to delete account.", "danger");
            console.error(e);
        }
    });
}

async function save_account_settings(){
    if (current_user_id === null){
        Tools.show_alert("Failed to save account settings.", "danger");
        return;
    }

    let name = (<HTMLInputElement>tab_content.querySelector("#name")).value;
    let email = (<HTMLInputElement>tab_content.querySelector("#email")).value;
    let new_password = (<HTMLInputElement>tab_content.querySelector("#new_password")).value;
    let new_password2 = (<HTMLInputElement>tab_content.querySelector("#new_password2")).value;

    if (!name || !email){
        Tools.show_alert("Name and email can't be empty.", "danger");
        return;
    }

    if (new_password || new_password2){
        if (new_password !== new_password2){
            Tools.show_alert("Passwords don't match.", "danger");
            return;
        }
    }

    let patch_data: any = {
        id: current_user_id,
        name: name,
        email: email
    };
    if (new_password){
        patch_data.password = new_password;
    }

    try {
        await API.send_update_user(current_user_id, patch_data);
        Tools.show_alert("Account settings saved.", "success");

        (<HTMLInputElement>tab_content.querySelector("#new_password")).value = "";
        (<HTMLInputElement>tab_content.querySelector("#new_password2")).value = "";
    } catch (e) {
        Tools.show_alert("Failed to save account settings.", "danger");
        console.error(e);
    }
}

async function show_teams_settings(){
    set_active_tab("teams");

    try {
        const [teams, pending_invitations] = await Promise.all([
            teams_api.list_teams(),
            teams_api.list_my_invitations()
        ]);
        current_teams = teams;

        // @ts-ignore
        tab_content.innerHTML = Handlebars.templates.profile_settings_teams_tab({
            teams: teams.map(build_team_view),
            pending_invitations: pending_invitations.map(invitation => ({
                id: invitation.id,
                team_name: invitation.team_name,
                role: capitalize(invitation.role)
            }))
        });

        add_teams_settings_listeners();
    } catch (e) {
        Tools.show_alert("Failed to load teams.", "danger");
        console.error(e);
    }
}

function build_team_view(team: API.Team){
    const me = team.members.find(member => member.user.id === current_user_id);
    const my_role: API.TeamRole = me ? me.role : "member";
    // The shared "Default" team (every account is automatically an "owner" of it, see
    // ensure_default_team on the backend) can't actually be managed through this API —
    // the backend rejects delete/patch on it outright — so don't show controls that
    // would only ever fail with a confusing error.
    const is_default_team = team.name === "Default";
    const can_manage = !is_default_team && (my_role === "owner" || my_role === "admin");

    return {
        id: team.id,
        name: team.name,
        my_role: capitalize(my_role),
        can_manage: can_manage,
        can_delete: can_manage && my_role === "owner",
        members: team.members.map(member => {
            const is_owner_row = member.role === "owner";
            const is_last_owner = is_owner_row
                && team.members.filter(m => m.role === "owner").length === 1;
            // The server (patch_team) only lets an "owner" touch who owns the team — an
            // "admin" may freely reassign admin/member roles but can't promote anyone to
            // owner or change an existing owner's role at all. Mirror that here so the UI
            // never offers a control that's guaranteed to be rejected.
            const can_edit_role = can_manage && (my_role === "owner" || !is_owner_row);
            // The server also rejects a patch that would leave the team without an owner
            // (see replace_members) — the last owner must first hand ownership to someone
            // else via the role dropdown before they (or anyone) can be removed.
            const can_remove = can_edit_role && !is_last_owner;
            const selectable_roles: API.TeamRole[] =
                my_role === "owner" ? ["owner", "admin", "member"] : ["admin", "member"];

            return {
                user_id: member.user.id,
                name: member.user.name,
                is_self: member.user.id === current_user_id,
                can_edit_role: can_edit_role,
                can_remove: can_remove,
                // Leaving the Default team isn't supported either (the leave_team endpoint
                // rejects it outright), so don't offer a "Leave" button that would fail.
                show_leave: !is_default_team && !is_last_owner && member.user.id === current_user_id,
                role_label: capitalize(member.role),
                roles: selectable_roles.map(role => ({
                    value: role,
                    label: capitalize(role),
                    selected: role === member.role
                }))
            };
        })
    };
}

function add_teams_settings_listeners(){
    document.getElementById("create_team_btn").addEventListener("click", start_create_team);

    tab_content.querySelectorAll(".accept_invitation_btn").forEach(btn => {
        btn.addEventListener("click", async function (){
            const invitation_id = (btn.closest("[data-invitation-id]") as HTMLElement).dataset.invitationId;
            try {
                await teams_api.accept_invitation(invitation_id);
                Tools.show_alert("Invitation accepted.", "success");
                await show_teams_settings();
            } catch (e) {
                Tools.show_alert("Failed to accept invitation.", "danger");
                console.error(e);
            }
        });
    });

    tab_content.querySelectorAll(".decline_invitation_btn").forEach(btn => {
        btn.addEventListener("click", async function (){
            const invitation_id = (btn.closest("[data-invitation-id]") as HTMLElement).dataset.invitationId;
            try {
                await teams_api.decline_my_invitation(invitation_id);
                Tools.show_alert("Invitation declined.", "success");
                await show_teams_settings();
            } catch (e) {
                Tools.show_alert("Failed to decline invitation.", "danger");
                console.error(e);
            }
        });
    });

    tab_content.querySelectorAll(".card[data-team-id]").forEach(card => {
        const team_id = (card as HTMLElement).dataset.teamId;

        const save_name_btn = card.querySelector(".save_team_name_btn");
        if (save_name_btn !== null){
            save_name_btn.addEventListener("click", () => save_team_name(team_id, card as HTMLElement));
        }

        const invite_btn = card.querySelector(".invite_member_btn");
        if (invite_btn !== null){
            invite_btn.addEventListener("click", () => start_invite_member(team_id));
        }

        const delete_btn = card.querySelector(".delete_team_btn");
        if (delete_btn !== null){
            delete_btn.addEventListener("click", () => {
                const team = current_teams.find(t => t.id === team_id);
                if (team){
                    start_delete_team(team_id, team.name);
                }
            });
        }

        card.querySelectorAll(".member_role_select").forEach(select => {
            select.addEventListener("change", function (){
                const user_id = (select.closest("[data-user-id]") as HTMLElement).dataset.userId;
                update_member_role(team_id, user_id, (select as HTMLSelectElement).value as API.TeamRole);
            });
        });

        card.querySelectorAll(".remove_member_btn").forEach(btn => {
            btn.addEventListener("click", function (){
                const user_id = (btn.closest("[data-user-id]") as HTMLElement).dataset.userId;
                if (window.confirm("Remove this member from the team?")){
                    remove_member(team_id, user_id);
                }
            });
        });

        card.querySelectorAll(".leave_team_btn").forEach(btn => {
            btn.addEventListener("click", function (){
                if (current_user_id !== null && window.confirm("Are you sure you want to leave this team?")){
                    leave_team(team_id);
                }
            });
        });
    });
}

async function save_team_name(team_id: string, card: HTMLElement){
    const name = (<HTMLInputElement>card.querySelector(".team_name_input")).value.trim();
    if (!name){
        Tools.show_alert("Team name can't be empty.", "danger");
        return;
    }

    try {
        await teams_api.patch_team(team_id, {name: name});
        Tools.show_alert("Team renamed.", "success");
        await show_teams_settings();
    } catch (e) {
        Tools.show_alert("Failed to rename team.", "danger");
        console.error(e);
    }
}

/**
 * Builds the full replacement member list expected by PATCH /api/teams/<id> from the
 * team's currently-known members, applying `role_by_user_id` overrides and dropping any
 * user id present in `remove_user_id`.
 */
function build_members_patch(team: API.Team, role_by_user_id: Record<string, API.TeamRole>, remove_user_id?: string): API.PatchTeamMember[] {
    return team.members
        .filter(member => member.user.id !== remove_user_id)
        .map(member => ({
            user_id: member.user.id,
            role: role_by_user_id[member.user.id] ?? member.role
        }));
}

async function update_member_role(team_id: string, user_id: string, new_role: API.TeamRole){
    const team = current_teams.find(t => t.id === team_id);
    if (!team){
        return;
    }

    try {
        await teams_api.patch_team(team_id, {members: build_members_patch(team, {[user_id]: new_role})});
        Tools.show_alert("Role updated.", "success");
        await show_teams_settings();
    } catch (e) {
        Tools.show_alert("Failed to update role.", "danger");
        console.error(e);
        await show_teams_settings();
    }
}

async function leave_team(team_id: string){
    try {
        await teams_api.leave_team(team_id);
        Tools.show_alert("You left the team.", "success");
        await show_teams_settings();
    } catch (e) {
        Tools.show_alert("Failed to leave team.", "danger");
        console.error(e);
    }
}

async function remove_member(team_id: string, user_id: string){
    const team = current_teams.find(t => t.id === team_id);
    if (!team){
        return;
    }

    try {
        await teams_api.patch_team(team_id, {members: build_members_patch(team, {}, user_id)});
        Tools.show_alert("Member removed.", "success");
        await show_teams_settings();
    } catch (e) {
        Tools.show_alert("Failed to remove member.", "danger");
        console.error(e);
    }
}

function start_create_team(){
    // @ts-ignore
    Tools.show_overlay(Handlebars.templates.profile_settings_teams_tab_create_team_overlay({}));

    const overlay_content = document.getElementById("inner_overlay");
    const name_input = <HTMLInputElement>overlay_content.querySelector("#new_team_name");
    const confirm_btn = overlay_content.querySelector("#confirm_create_team");

    confirm_btn.addEventListener("click", async function (){
        const name = name_input.value.trim();
        if (!name){
            Tools.show_alert("Please enter a team name.", "danger");
            return;
        }

        try {
            await teams_api.create_team(name);
            Tools.hide_overlay();
            Tools.show_alert("Team created.", "success");
            await show_teams_settings();
        } catch (e) {
            Tools.show_alert("Failed to create team.", "danger");
            console.error(e);
        }
    });
}

function start_delete_team(team_id: string, team_name: string){
    // @ts-ignore
    Tools.show_overlay(Handlebars.templates.profile_settings_teams_tab_delete_team_overlay({name: team_name}));

    const overlay_content = document.getElementById("inner_overlay");
    const confirm_input = <HTMLInputElement>overlay_content.querySelector("#delete_team_confirm_name");
    const confirm_btn = overlay_content.querySelector("#confirm_delete_team");

    confirm_btn.addEventListener("click", async function (){
        if (confirm_input.value.trim() !== team_name){
            Tools.show_alert("The team name doesn't match.", "danger");
            return;
        }

        try {
            await teams_api.delete_team(team_id);
            Tools.hide_overlay();
            Tools.show_alert("Team deleted.", "success");
            await show_teams_settings();
        } catch (e) {
            Tools.show_alert("Failed to delete team.", "danger");
            console.error(e);
        }
    });
}

async function start_invite_member(team_id: string){
    const team = current_teams.find(t => t.id === team_id);
    if (!team){
        return;
    }

    try {
        const invitations = await teams_api.list_invitations(team_id);
        show_invite_overlay(team, invitations);
    } catch (e) {
        Tools.show_alert("Failed to load pending invitations.", "danger");
        console.error(e);
    }
}

function show_invite_overlay(team: API.Team, invitations: API.Invitation[]){
    // @ts-ignore
    Tools.show_overlay(Handlebars.templates.profile_settings_teams_tab_invite_overlay({
        team_name: team.name,
        invitations: invitations.map(invitation => ({
            id: invitation.id,
            label: invitation.email !== null ? invitation.email : "Existing user",
            role: capitalize(invitation.role)
        }))
    }));

    const overlay_content = document.getElementById("inner_overlay");
    const email_input = <HTMLInputElement>overlay_content.querySelector("#invite_email");
    const role_select = <HTMLSelectElement>overlay_content.querySelector("#invite_role");
    const send_btn = overlay_content.querySelector("#send_invite_btn");

    send_btn.addEventListener("click", async function (){
        const email = email_input.value.trim();
        if (!email){
            Tools.show_alert("Please enter an email address.", "danger");
            return;
        }

        try {
            await teams_api.create_invitation(team.id, role_select.value as API.TeamRole, email);
            Tools.hide_overlay();
            Tools.show_alert("Invitation sent.", "success");
        } catch (e) {
            Tools.show_alert("Failed to send invitation.", "danger");
            console.error(e);
        }
    });

    overlay_content.querySelectorAll(".revoke_invitation_btn").forEach(btn => {
        btn.addEventListener("click", async function (){
            const row = btn.closest("[data-invitation-id]") as HTMLElement;
            try {
                await teams_api.revoke_invitation(team.id, row.dataset.invitationId);
                row.remove();
                Tools.show_alert("Invitation revoked.", "success");
            } catch (e) {
                Tools.show_alert("Failed to revoke invitation.", "danger");
                console.error(e);
            }
        });
    });
}

async function show_api_keys_settings(){
    set_active_tab("api_keys");

    try {
        const api_keys = await api_keys_api.list_api_keys();

        // @ts-ignore
        tab_content.innerHTML = Handlebars.templates.profile_settings_api_keys_tab({
            api_keys: api_keys.map(key => ({
                id: key.id,
                name: key.name,
                key_prefix: key.key_prefix,
                created_at: new Date(key.created_at).toLocaleString()
            }))
        });

        add_api_keys_settings_listeners();
    } catch (e) {
        Tools.show_alert("Failed to load API keys.", "danger");
        console.error(e);
    }
}

function add_api_keys_settings_listeners(){
    document.getElementById("create_api_key_btn").addEventListener("click", start_create_api_key);

    tab_content.querySelectorAll(".delete_api_key_btn").forEach(btn => {
        btn.addEventListener("click", function (){
            const row = btn.closest("[data-api-key-id]") as HTMLElement;
            if (window.confirm("Delete this API key? Anything using it will stop working immediately.")){
                delete_api_key(row.dataset.apiKeyId);
            }
        });
    });
}

function start_create_api_key(){
    // @ts-ignore
    Tools.show_overlay(Handlebars.templates.profile_settings_api_keys_tab_create_overlay({}));

    const overlay_content = document.getElementById("inner_overlay");
    const name_input = <HTMLInputElement>overlay_content.querySelector("#new_api_key_name");
    const confirm_btn = overlay_content.querySelector("#confirm_create_api_key");

    confirm_btn.addEventListener("click", async function (){
        const name = name_input.value.trim();
        if (!name){
            Tools.show_alert("Please enter a name.", "danger");
            return;
        }

        try {
            const created = await api_keys_api.create_api_key(name);
            // Refresh the list behind the overlay first, so the new key is already there
            // however the "key created" overlay that follows gets dismissed (Done button,
            // the X button, or Escape all just close it without any extra handling).
            await show_api_keys_settings();
            show_created_api_key_overlay(created.key);
        } catch (e) {
            Tools.show_alert("Failed to create API key.", "danger");
            console.error(e);
        }
    });
}

function show_created_api_key_overlay(key: string){
    // @ts-ignore
    Tools.show_overlay(Handlebars.templates.profile_settings_api_keys_tab_created_overlay({key: key}));

    const overlay_content = document.getElementById("inner_overlay");
    const key_input = <HTMLInputElement>overlay_content.querySelector("#created_api_key_value");
    const copy_btn = overlay_content.querySelector("#copy_api_key_btn");
    const close_btn = overlay_content.querySelector("#close_created_api_key_btn");

    copy_btn.addEventListener("click", async function (){
        try {
            // @ts-ignore - navigator.clipboard is undefined on plain-HTTP non-localhost origins.
            if (navigator.clipboard && navigator.clipboard.writeText){
                await navigator.clipboard.writeText(key_input.value);
                Tools.show_alert("API key copied to clipboard.", "success");
                return;
            }
        } catch (e) {
            console.error(e);
        }
        key_input.select();
        Tools.show_alert("Couldn't copy automatically — the key is selected, press Ctrl+C.", "warning");
    });

    close_btn.addEventListener("click", Tools.hide_overlay);
}

async function delete_api_key(key_id: string){
    try {
        await api_keys_api.delete_api_key(key_id);
        Tools.show_alert("API key deleted.", "success");
        await show_api_keys_settings();
    } catch (e) {
        Tools.show_alert("Failed to delete API key.", "danger");
        console.error(e);
    }
}
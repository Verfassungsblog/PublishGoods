window.addEventListener("load", function () {
    init_template_filter();
});

/**
 * A project can only use templates of its own team: offers only the templates of the
 * selected team in the template select and keeps the hint below it in sync.
 */
function init_template_filter() {
    const team_select = document.getElementById("team_id") as HTMLSelectElement | null;
    const template_select = document.getElementById("template_id") as HTMLSelectElement | null;
    const hint = document.getElementById("template_id_hint");
    if (team_select === null || template_select === null || hint === null) {
        return;
    }

    function filter_templates() {
        let first_visible: HTMLOptionElement | null = null;
        for (let i = 0; i < template_select.options.length; i++) {
            const option = template_select.options[i];
            const visible = option.dataset.teamId === team_select.value;
            option.hidden = !visible;
            option.disabled = !visible;
            if (visible && first_visible === null) {
                first_visible = option;
            }
        }
        if (template_select.selectedOptions.length === 0 || template_select.selectedOptions[0].disabled) {
            template_select.value = first_visible !== null ? first_visible.value : "";
        }
        hint.textContent = first_visible !== null
            ? "Only templates of the selected team can be used."
            : "The selected team has no templates yet. Create one under Templates first.";
    }

    team_select.addEventListener("change", filter_templates);
    filter_templates();
}

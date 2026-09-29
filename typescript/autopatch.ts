import {show_alert} from "./tools";

/**
 * Sends the accumulated patch data to the backend. Rejects to signal
 * a failed save, in which case the data is queued for retry.
 */
export interface PatchSender {
    (data: any): Promise<void>;
}

/**
 * Watches elements with the "autopatch" class and debounces + batches
 * changes into a single patch request, retrying on failure without
 * losing data the user entered in the meantime.
 *
 * Elements must have a "data-patch" attribute of the form "scope.field",
 * e.g. data-patch="metadata.title" results in {metadata: {title: value}}.
 */
export class AutoPatch {
    private patch_data: any = {};
    private last_patch: number | null = null;
    private save_timeout: ReturnType<typeof setTimeout> | null = null;

    constructor(private readonly send: PatchSender, private readonly error_message: string = "Couldn't save changes. Trying again!") {
    }

    /**
     * Attaches change/input listeners to all ".autopatch" elements within scope.
     */
    init(scope: Document | Element = document) {
        const targets = scope.getElementsByClassName("autopatch");
        for (const target of Array.from(targets)) {
            const patch_field = target.getAttribute("data-patch");
            if (!patch_field) {
                console.error("Element " + target.id + " has autopatch class but misses data-patch attribute.");
                continue;
            }

            if (target.tagName.toLowerCase() === "input") {
                const input_type = (target.getAttribute("type") || "text").toLowerCase();
                if (input_type === "checkbox" || input_type === "radio" || input_type === "date" || input_type === "datetime-local") {
                    target.addEventListener("change", this.autopatch_listener);
                } else {
                    target.addEventListener("input", this.autopatch_listener);
                }
            } else if (target.tagName.toLowerCase() === "select") {
                target.addEventListener("change", this.autopatch_listener);
            } else if (target.tagName.toLowerCase() === "textarea") {
                target.addEventListener("input", this.autopatch_listener);
            } else {
                console.error("Autopatch not implemented for tag " + target.tagName.toLowerCase());
            }
        }
    }

    private autopatch_listener = (e: Event) => {
        const target = e.target as HTMLElement;
        const patch_field = target.getAttribute("data-patch");
        const splitted = patch_field.split(".");
        const scope = splitted[0] || null;
        const field_name = splitted[1] || null;

        if (!scope || !field_name) {
            console.error("Element " + target.id + " has invalid data-patch attribute.");
            return;
        }

        let value: any;
        if (target instanceof HTMLInputElement) {
            const input_type = (target.getAttribute("type") || "text").toLowerCase();
            if (input_type === "checkbox") { // Boolean
                value = target.checked;
            } else if (input_type === "number") {
                value = target.valueAsNumber;
            } else {
                value = target.value;
            }
        } else if (target instanceof HTMLSelectElement || target instanceof HTMLTextAreaElement) {
            value = target.value;
        } else {
            value = target.innerHTML;
        }

        this.set_field(scope, field_name, value);
    };

    /**
     * Queues a single field for the next patch request, e.g. for values
     * that aren't driven by an ".autopatch" element (custom widgets etc.).
     */
    set_field(scope: string, field_name: string, value: any) {
        if (!this.patch_data[scope]) this.patch_data[scope] = {};
        this.patch_data[scope][field_name] = value;
        this.request_patch();
    }

    /**
     * Ensures there is at least a 1-second interval since the last request
     * before invoking send(). If called within the cooldown, schedules the
     * request instead of sending immediately.
     */
    async request_patch() {
        if (this.save_timeout) return;
        if (this.last_patch) {
            if (Date.now() - this.last_patch < 1000) { // Do not set a new save timeout if there already is one waiting
                this.save_timeout = setTimeout(() => this.send_patch(), 1000);
                return;
            }
        }

        // At least 1 second since last save or no save yet
        await this.send_patch();
    }

    private async send_patch() {
        this.save_timeout = null;

        // Move data to local scope and clear immediately to prevent data loss
        // if the user keeps typing while the request is in flight.
        const data_to_send = this.patch_data;
        this.patch_data = {};

        // Don't send empty objects
        if (Object.keys(data_to_send).length === 0) return;

        try {
            await this.send(data_to_send);
            this.last_patch = Date.now();
        } catch (e) {
            console.error("Failed to save patch data", e);
            // Merge failed data back into patch_data, but allow current patch_data to take precedence (newer changes)
            for (const scope in data_to_send) {
                if (!this.patch_data[scope]) {
                    this.patch_data[scope] = data_to_send[scope];
                } else {
                    this.patch_data[scope] = {...data_to_send[scope], ...this.patch_data[scope]};
                }
            }
            show_alert(this.error_message, "warning");
            await this.request_patch();
        }
    }
}

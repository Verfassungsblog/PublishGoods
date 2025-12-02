import {APIProjectData, EditorAPI, License, ProjectSettingsV5} from "../api_requests";
import {main_col} from "./Editor";
import {state} from "./Main";

type IdentifierType =
    | { DOI: null }
    | { ISBN: null }
    | { ISSN: null }
    | { URL: null }
    | { URN: null }
    | { ORCID: null }
    | { ROR: null }
    | { GND: null }
    | { Other: string };

type Identifier = {
    id: string | null,
    name: string,
    value: string,
    identifier_type: IdentifierType,
};

type Keyword = { title: string, gnd: Identifier | null };

// Type for additional field definitions
type AdditionalFieldDef = {
    key: string;
    label: string;
    type: 'text' | 'number' | 'date' | 'textarea';
};

// Helper to format field values for display
function formatFieldValue(value: any, type: string): string {
    if (value === null || value === undefined) return '';
    if (Array.isArray(value)) {
        // For arrays like authors/editors, join with comma
        return value.map((item: any) => {
            if (typeof item === 'string') return item;
            if (item?.NameString) return item.NameString;
            if (item?.PersonUuid) return `[Person: ${item.PersonUuid}]`;
            return String(item);
        }).join(', ');
    }
    return String(value);
}

const editorApi = EditorAPI();

export async function show_project_metadata_settings(data: APIProjectData){
    const md = data.metadata || {
        title: "",
        subtitle: null,
        authors: null,
        editors: null,
        web_url: null,
        identifiers: [],
        published: null,
        languages: null,
        number_of_pages: null,
        short_abstract: null,
        long_abstract: null,
        keywords: [],
        ddc: null,
        license: null as License | null,
        series: null,
        volume: null,
        edition: null,
        publisher: null,
        custom_fields: {} as Record<string, string>,
    };
    const st: ProjectSettingsV5 = data.settings || {
        toc_enabled: true,
        csl_style: null,
        csl_language_code: null,
        metadata_page_additional_html: null,
        cover_image_path: null,
        backcover_image_path: null,
        add_soft_hyphens: false,
    };
    // Load available CSL styles
    let cslStyles: string[] = [];
    try{
        cslStyles = await editorApi.getCslStyles();
    }catch(e){
        console.warn('Failed to load CSL styles', e);
        cslStyles = [];
    }

    // Build options for CSL select (None + styles)
    const csl_options = [{ value: 'None', label: 'None', selected: st.csl_style == null }]
        .concat(
            (cslStyles || []).map(s => ({ value: s, label: s, selected: st.csl_style === s }))
        );

    // Define standard additional fields that can be added
    const standardAdditionalFields: AdditionalFieldDef[] = [
        { key: 'authors', label: 'Authors', type: 'text' },
        { key: 'editors', label: 'Editors', type: 'text' },
        { key: 'web_url', label: 'Web URL', type: 'text' },
        { key: 'published', label: 'Published Date', type: 'date' },
        { key: 'languages', label: 'Languages', type: 'text' },
        { key: 'number_of_pages', label: 'Number of Pages', type: 'number' },
        { key: 'short_abstract', label: 'Short Abstract', type: 'textarea' },
        { key: 'long_abstract', label: 'Long Abstract', type: 'textarea' },
        { key: 'ddc', label: 'DDC', type: 'text' },
        { key: 'series', label: 'Series', type: 'text' },
        { key: 'volume', label: 'Volume', type: 'text' },
        { key: 'edition', label: 'Edition', type: 'text' },
        { key: 'publisher', label: 'Publisher', type: 'text' },
    ];

    // Determine which additional fields are currently active (have non-null values)
    const activeAdditionalFields = standardAdditionalFields.filter(f => {
        const val = (md as any)[f.key];
        return val !== null && val !== undefined && val !== '';
    });

    // Available fields are those not yet active
    const availableAdditionalFields = standardAdditionalFields.filter(f => {
        const val = (md as any)[f.key];
        return val === null || val === undefined || val === '';
    });

    // Build custom fields data
    const customFields = md.custom_fields || {};
    const customFieldsList = Object.entries(customFields).map(([key, value]) => ({ key, value }));

    // Prepare template data
    const licenseOpts = buildLicenseOptions(md.license);
    const identifiersVm = buildIdentifierVM(md.identifiers ?? []);
    const tplData = {
        settings: st,
        metadata: { title: md.title, subtitle: md.subtitle ?? "" },
        license_options: licenseOpts.options,
        license_is_other: licenseOpts.isOther,
        license_other_value: licenseOpts.otherValue,
        identifiers_ctx: { identifiers: identifiersVm },
        // tags UI handles keywords; keep legacy csv out
        csl_options,
        // Additional fields data
        active_additional_fields: activeAdditionalFields.map(f => ({
            ...f,
            value: formatFieldValue((md as any)[f.key], f.type)
        })),
        available_additional_fields: availableAdditionalFields,
        custom_fields: customFieldsList,
    };

    // Render via Handlebars template
    // @ts-ignore
    main_col.innerHTML = Handlebars.templates.editor_project_metadata_settings(tplData);

    // Initial render: show only existing identifiers (no empty placeholder by default)
    renderIdentifiers(md.identifiers ?? []);
    // Generic delegated handlers for data-patch inputs
    const $ = <T extends HTMLElement>(sel: string) => document.querySelector(sel) as T;
    const root = document.querySelector('.editor-metadata-settings') as HTMLElement;

    function coerceFromDataset(el: HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement){
        const ds = (el as HTMLElement).dataset as any;
        let value: any;
        if(el instanceof HTMLInputElement && el.type === 'checkbox') {
            value = el.checked;
        } else {
            value = (el as any).value;
        }

        // Always trim string inputs and set to null if empty
        if(typeof value === 'string'){
            value = value.trim();
            if(value.length === 0) value = null;
        }

        // Map specific literal to null when requested (e.g., select "None")
        if(ds.nullWhen && value === ds.nullWhen) value = null;
        return value;
    }

    function applyPatch(scope: 'settings' | 'metadata', key: string, value: any){
        const payload: any = { [key]: value };
        return scope === 'settings' ? patchSettings(payload) : patchMetadata(payload);
    }

    function handlePatchedInput(target: HTMLElement){
        const ds: any = (target as HTMLElement).dataset || {};
        const patch: string | undefined = ds.patch;
        if(!patch) return;
        // License special handling
        if(ds.license === 'select'){
            const sel = target as HTMLSelectElement;
            const otherEl = $('#md-license-other') as HTMLInputElement;
            const val = sel.value;
            if(val === 'None'){
                otherEl.classList.add('hide');
                return applyPatch('metadata', 'license', null);
            }
            if(val === 'Other'){
                otherEl.classList.remove('hide');
                return; // Wait for other input blur
            }
            otherEl.classList.add('hide');
            return applyPatch('metadata', 'license', val as License);
        }
        if(ds.license === 'other'){
            const txt = (target as HTMLInputElement).value.trim();
            return applyPatch('metadata', 'license', txt ? ({ Other: txt } as License) : null);
        }

        const [scope, key] = patch.split('.') as ['settings' | 'metadata', string];
        const value = coerceFromDataset(target as any);
        return applyPatch(scope, key, value);
    }

    // Avoid double patches: for text-like inputs, browsers fire `change` on blur.
    // Strategy: handle selects/checkboxes on `change`; handle text inputs and textareas on `blur`.
    root.addEventListener('change', (e) => {
        const t = e.target as HTMLElement;
        if(!t) return;
        // Only handle selects and checkbox/radio in `change` phase
        if(t instanceof HTMLSelectElement) return void handlePatchedInput(t);
        if(t instanceof HTMLInputElement){
            const type = t.type?.toLowerCase();
            if(type === 'checkbox' || type === 'radio') return void handlePatchedInput(t);
        }
        // Ignore other inputs here; they will be handled on blur
    });
    root.addEventListener('blur', (e) => {
        const t = e.target as HTMLElement;
        if(!t) return;
        // Only handle text-like inputs and textareas on blur
        if(t instanceof HTMLTextAreaElement) return void handlePatchedInput(t);
        if(t instanceof HTMLInputElement){
            const type = t.type?.toLowerCase();
            // Skip checkbox/radio; they are handled on change
            if(type === 'checkbox' || type === 'radio') return;
            return void handlePatchedInput(t);
        }
        // Skip selects here; they are handled on change
    }, true);

    (document.getElementById('id-add') as HTMLButtonElement).addEventListener('click', async () => {
        // If an empty row already exists, just focus its value input; otherwise append one and render.
        const rows = Array.from(document.querySelectorAll('#id-list .id-row')) as HTMLElement[];
        const existingEmpty = rows.some(row => {
            const val = (row.querySelector('.id-value') as HTMLInputElement | null)?.value ?? '';
            return (val.trim().length === 0);
        });
        if(!existingEmpty){
            const current = collectIdentifiersFromUI();
            current.push({ id: null, name: '', value: '', identifier_type: { DOI: null } as any });
            renderIdentifiers(current);
        }
        // Focus the value input of the last row (the empty row we just appended or the existing one)
        const allRows = document.querySelectorAll('#id-list .id-row');
        const last = allRows[allRows.length - 1] as HTMLElement | undefined;
        const valueInp = last?.querySelector('.id-value') as HTMLInputElement | null;
        valueInp?.focus();
    });

    // Identifiers: delegated wiring on the list container
    const idList = document.getElementById('id-list') as HTMLElement;
    const persistIdentifiers = async () => {
        const all = collectIdentifiersFromUI();
        await patchMetadata({ identifiers: all });
    };
    idList.addEventListener('change', async (e) => {
        const t = e.target as HTMLElement;
        if(t.classList.contains('id-type')){ await persistIdentifiers(); }
    });
    idList.addEventListener('blur', async (e) => {
        const t = e.target as HTMLElement;
        if(t.classList.contains('id-name') || t.classList.contains('id-value')){ await persistIdentifiers(); }
    }, true);
    idList.addEventListener('click', async (e) => {
        const btn = (e.target as HTMLElement).closest('.id-remove') as HTMLButtonElement | null;
        if(!btn) return;
        const row = btn.closest('.id-row');
        row?.remove();
        await persistIdentifiers();
    });

    // Tags (free + GND)
    const tagsListEl = document.getElementById('md-tags-list') as HTMLElement;
    const freeInput = document.getElementById('md-tags-free') as HTMLInputElement;
    const gndInput = document.getElementById('md-tags-gnd') as HTMLInputElement;
    const gndSuggest = document.getElementById('md-tags-gnd-suggest') as HTMLElement;

    type UITag = { kind: 'free'; label: string } | { kind: 'gnd'; label: string; gndValue: string };
    let uiTags: UITag[] = toUITags(md.keywords as any);
    renderTags();

    freeInput.addEventListener('keydown', async (e) => {
        if(e.key === 'Enter'){
            e.preventDefault();
            const val = freeInput.value.trim();
            if(!val) return;
            uiTags.push({ kind: 'free', label: val });
            freeInput.value = '';
            renderTags();
            await persistTags();
        }
    });

    let gndDebounce: number | undefined;
    gndInput.addEventListener('input', () => {
        const q = gndInput.value.trim();
        window.clearTimeout(gndDebounce);
        if(q.length < 2){ gndSuggest.classList.add('hide'); gndSuggest.innerHTML=''; return; }
        gndDebounce = window.setTimeout(async () => {
            try{
                const results: any[] = await editorApi.searchGnd(q);
                renderGndSuggestions(results);
            }catch(err){ console.warn('GND search failed', err); }
        }, 250);
    });

    function renderGndSuggestions(items: any[]){
        gndSuggest.innerHTML = '';
        if(!items || items.length === 0){ gndSuggest.classList.add('hide'); return; }
        gndSuggest.classList.remove('hide');
        items.slice(0, 10).forEach((it) => {
            // lobid preferredName and id are typical
            const label = it.preferredName || it.name || it.label || String(it.id || it.gndIdentifier || 'GND');
            const id = it.id || it.gndIdentifier || label;
            const el = document.createElement('div');
            el.className = 'eds-suggest__item';
            el.textContent = label;
            el.addEventListener('mousedown', async (ev) => {
                ev.preventDefault();
                uiTags.push({ kind: 'gnd', label, gndValue: String(id) });
                gndInput.value = '';
                gndSuggest.classList.add('hide');
                gndSuggest.innerHTML = '';
                renderTags();
                await persistTags();
            });
            gndSuggest.appendChild(el);
        });
    }

    async function persistTags(){
        const payload: Keyword[] = uiTags.map((t): Keyword => {
            if(t.kind === 'free') return { title: t.label, gnd: null };
            return {
                title: t.label,
                gnd: {
                    id: null,
                    name: 'GND',
                    value: t.gndValue,
                    identifier_type: { GND: null }
                }
            };
        });
        await patchMetadata({ keywords: payload });
    }

    function renderTags(){
        // Render via Handlebars template instead of building DOM here
        // @ts-ignore
        tagsListEl.innerHTML = Handlebars.templates.editor_project_metadata_settings_tags({ tags: uiTags });
    }

    // Delegated remove handler for tag chips
    tagsListEl.addEventListener('click', async (e) => {
        const target = e.target as HTMLElement;
        const btn = target.closest('.eds-chip__remove') as HTMLButtonElement | null;
        if(!btn) return;
        const idx = parseInt(btn.getAttribute('data-idx') || '-1', 10);
        if(Number.isFinite(idx) && idx >= 0 && idx < uiTags.length){
            uiTags.splice(idx, 1);
            renderTags();
            await persistTags();
        }
    });

    // === Additional Fields UI ===
    const addFieldSelect = document.getElementById('add-field-select') as HTMLSelectElement | null;
    const addFieldSearch = document.getElementById('add-field-search') as HTMLInputElement | null;
    const addFieldSuggest = document.getElementById('add-field-suggest') as HTMLElement | null;
    const activeFieldsContainer = document.getElementById('active-additional-fields') as HTMLElement | null;
    const customFieldsContainer = document.getElementById('custom-fields-list') as HTMLElement | null;
    const addCustomFieldBtn = document.getElementById('add-custom-field') as HTMLButtonElement | null;

    // Track current state of available fields
    let currentAvailableFields = [...availableAdditionalFields];
    let currentCustomFields = { ...customFields };

    // Filter and display suggestions based on search input
    function filterFieldSuggestions(searchTerm: string) {
        if (!addFieldSuggest) return;
        const term = searchTerm.toLowerCase().trim();
        
        if (term.length === 0) {
            addFieldSuggest.classList.add('hide');
            addFieldSuggest.innerHTML = '';
            return;
        }

        const filtered = currentAvailableFields.filter(f => 
            f.label.toLowerCase().includes(term) || f.key.toLowerCase().includes(term)
        );

        if (filtered.length === 0) {
            // Show option to create a custom field
            addFieldSuggest.classList.remove('hide');
            addFieldSuggest.innerHTML = `<div class="eds-suggest__item eds-suggest__item--create" data-create-custom="${searchTerm}">+ Create custom field "${searchTerm}"</div>`;
        } else {
            addFieldSuggest.classList.remove('hide');
            addFieldSuggest.innerHTML = filtered.map(f => 
                `<div class="eds-suggest__item" data-add-field="${f.key}">${f.label}</div>`
            ).join('') + `<div class="eds-suggest__item eds-suggest__item--create" data-create-custom="${searchTerm}">+ Create custom field "${searchTerm}"</div>`;
        }
    }

    // Add a standard additional field
    async function addStandardField(fieldKey: string) {
        const fieldDef = standardAdditionalFields.find(f => f.key === fieldKey);
        if (!fieldDef) return;

        // Set default value based on type
        let defaultValue: any = '';
        if (fieldDef.type === 'number') defaultValue = null;
        
        // Patch metadata to add the field
        await patchMetadata({ [fieldKey]: defaultValue });

        // Remove from available, add to active
        currentAvailableFields = currentAvailableFields.filter(f => f.key !== fieldKey);
        
        // Re-render active fields
        renderActiveFields();
        
        // Clear search
        if (addFieldSearch) {
            addFieldSearch.value = '';
        }
        if (addFieldSuggest) {
            addFieldSuggest.classList.add('hide');
            addFieldSuggest.innerHTML = '';
        }
    }

    // Remove a standard additional field
    async function removeStandardField(fieldKey: string) {
        const fieldDef = standardAdditionalFields.find(f => f.key === fieldKey);
        if (!fieldDef) return;

        // Patch metadata to clear the field
        await patchMetadata({ [fieldKey]: null });

        // Add back to available, remove from active
        currentAvailableFields.push(fieldDef);
        
        // Re-render active fields
        renderActiveFields();
    }

    // Add a custom field
    async function addCustomField(fieldName: string) {
        const key = fieldName.trim();
        if (!key || key in currentCustomFields) return;

        currentCustomFields[key] = '';
        await patchMetadata({ custom_fields: currentCustomFields });
        
        renderCustomFields();
        
        // Clear search
        if (addFieldSearch) {
            addFieldSearch.value = '';
        }
        if (addFieldSuggest) {
            addFieldSuggest.classList.add('hide');
            addFieldSuggest.innerHTML = '';
        }
    }

    // Update a custom field value
    async function updateCustomField(key: string, value: string) {
        currentCustomFields[key] = value;
        await patchMetadata({ custom_fields: currentCustomFields });
    }

    // Remove a custom field
    async function removeCustomField(key: string) {
        delete currentCustomFields[key];
        await patchMetadata({ custom_fields: currentCustomFields });
        renderCustomFields();
    }

    // Render active additional fields
    function renderActiveFields() {
        if (!activeFieldsContainer) return;
        
        const activeFields = standardAdditionalFields.filter(f => !currentAvailableFields.find(a => a.key === f.key));
        
        // @ts-ignore
        activeFieldsContainer.innerHTML = Handlebars.templates.editor_project_metadata_settings_additional_fields({
            fields: activeFields.map(f => ({
                ...f,
                value: '',  // Will be populated by data-patch
            }))
        });
    }

    // Render custom fields
    function renderCustomFields() {
        if (!customFieldsContainer) return;
        
        const fieldsList = Object.entries(currentCustomFields).map(([key, value]) => ({ key, value }));
        
        // @ts-ignore
        customFieldsContainer.innerHTML = Handlebars.templates.editor_project_metadata_settings_custom_fields({
            fields: fieldsList
        });
    }

    // Event handler for search input
    if (addFieldSearch) {
        addFieldSearch.addEventListener('input', () => {
            filterFieldSuggestions(addFieldSearch.value);
        });

        addFieldSearch.addEventListener('keydown', async (e) => {
            if (e.key === 'Enter') {
                e.preventDefault();
                const term = addFieldSearch.value.trim();
                
                // Check if it matches an available field
                const matchedField = currentAvailableFields.find(f => 
                    f.label.toLowerCase() === term.toLowerCase() || f.key.toLowerCase() === term.toLowerCase()
                );
                
                if (matchedField) {
                    await addStandardField(matchedField.key);
                } else if (term) {
                    // Create custom field
                    await addCustomField(term);
                }
            }
        });

        // Hide suggestions when clicking outside
        document.addEventListener('click', (e) => {
            if (!addFieldSuggest?.contains(e.target as Node) && e.target !== addFieldSearch) {
                addFieldSuggest?.classList.add('hide');
            }
        });
    }

    // Event handler for suggestion clicks
    if (addFieldSuggest) {
        addFieldSuggest.addEventListener('mousedown', async (e) => {
            e.preventDefault();
            const target = e.target as HTMLElement;
            
            const addFieldKey = target.getAttribute('data-add-field');
            if (addFieldKey) {
                await addStandardField(addFieldKey);
                return;
            }
            
            const createCustom = target.getAttribute('data-create-custom');
            if (createCustom) {
                await addCustomField(createCustom);
            }
        });
    }

    // Event delegation for active fields container
    if (activeFieldsContainer) {
        activeFieldsContainer.addEventListener('click', async (e) => {
            const target = e.target as HTMLElement;
            const removeBtn = target.closest('.eds-field__remove') as HTMLButtonElement | null;
            if (removeBtn) {
                const fieldKey = removeBtn.getAttribute('data-remove-field');
                if (fieldKey) {
                    await removeStandardField(fieldKey);
                }
            }
        });

        activeFieldsContainer.addEventListener('blur', async (e) => {
            const target = e.target as HTMLElement;
            if (target.classList.contains('eds-input') || target.classList.contains('eds-textarea')) {
                const inp = target as HTMLInputElement | HTMLTextAreaElement;
                const fieldKey = inp.getAttribute('data-additional-field');
                if (fieldKey) {
                    let value: any = inp.value.trim();
                    if (value === '') value = null;
                    
                    // Handle number type
                    const fieldDef = standardAdditionalFields.find(f => f.key === fieldKey);
                    if (fieldDef?.type === 'number' && value !== null) {
                        const parsed = parseInt(value, 10);
                        value = isNaN(parsed) ? null : parsed;
                    }
                    
                    await patchMetadata({ [fieldKey]: value });
                }
            }
        }, true);
    }

    // Event delegation for custom fields container
    if (customFieldsContainer) {
        customFieldsContainer.addEventListener('click', async (e) => {
            const target = e.target as HTMLElement;
            const removeBtn = target.closest('.eds-field__remove') as HTMLButtonElement | null;
            if (removeBtn) {
                const fieldKey = removeBtn.getAttribute('data-remove-custom');
                if (fieldKey) {
                    await removeCustomField(fieldKey);
                }
            }
        });

        customFieldsContainer.addEventListener('blur', async (e) => {
            const target = e.target as HTMLElement;
            if (target.classList.contains('eds-input')) {
                const inp = target as HTMLInputElement;
                const fieldKey = inp.getAttribute('data-custom-field');
                if (fieldKey) {
                    await updateCustomField(fieldKey, inp.value);
                }
            }
        }, true);
    }

    // Add custom field button handler - focuses the search input
    if (addCustomFieldBtn && addFieldSearch) {
        addCustomFieldBtn.addEventListener('click', () => {
            addFieldSearch.focus();
            addFieldSearch.placeholder = 'Type custom field name and press Enter...';
        });
        
        // Reset placeholder when losing focus
        addFieldSearch.addEventListener('blur', () => {
            setTimeout(() => {
                addFieldSearch.placeholder = 'Search or type to create custom field...';
            }, 200);
        });
    }

    // Click handler for available field chips
    const availableFieldsRoot = document.querySelector('.eds-available-fields__list');
    if (availableFieldsRoot) {
        availableFieldsRoot.addEventListener('click', async (e) => {
            const target = e.target as HTMLElement;
            const addAvailable = target.getAttribute('data-add-available');
            if (addAvailable) {
                await addStandardField(addAvailable);
                // Also remove the chip from UI
                target.remove();
            }
        });
    }
}

function normalizeNullableText(v: string): string | null{
    const t = (v ?? '').trim();
    return t.length === 0 ? null : t;
}

async function patchSettings(patch: Partial<ProjectSettingsV5>){
    // Double-option fields are handled by sending null to clear, omitting to keep.
    await editorApi.patchProject(state.project_id, { settings: patch });
}

async function patchMetadata(patch: any){
    await editorApi.patchProject(state.project_id, { metadata: patch });
}

function buildLicenseOptions(value: License | null){
    const optionsBase: {value: string, label: string}[] = [
        { value: 'None', label: 'No license set' },
        { value: 'CC0', label: 'CC0' },
        { value: 'CC_BY_4', label: 'CC BY 4.0' },
        { value: 'CC_BY_SA_4', label: 'CC BY-SA 4.0' },
        { value: 'CC_BY_ND_4', label: 'CC BY-ND 4.0' },
        { value: 'CC_BY_NC_4', label: 'CC BY-NC 4.0' },
        { value: 'CC_BY_NC_SA_4', label: 'CC BY-NC-SA 4.0' },
        { value: 'CC_BY_NC_ND_4', label: 'CC BY-NC-ND 4.0' },
        { value: 'Other', label: 'Other (custom)' },
    ];
    let selectedVal = 'None';
    let isOther = false;
    let otherValue: string | null = null;
    if(value === null){
        selectedVal = 'None';
    }else if(typeof value === 'string'){
        selectedVal = value;
    }else{
        selectedVal = 'Other';
        isOther = true;
        // @ts-ignore
        otherValue = value.Other as string;
    }
    const options = optionsBase.map(o => ({...o, selected: o.value === selectedVal}));
    return { options, isOther, otherValue };
}

function renderIdentifiers(ids: Identifier[]){
    const container = document.getElementById('id-list') as HTMLElement;
    const vm = { identifiers: buildIdentifierVM(ids) };
    // @ts-ignore
    container.innerHTML = Handlebars.templates.editor_project_metadata_settings_identifiers(vm);
}

function buildIdentifierVM(ids: Identifier[]){
    const list = (ids || []);
    return list.map((id, idx) => {
        const typeStr = identifierTypeToString(id.identifier_type);
        const options = ['DOI','ISBN','ISSN','URL','URN','ORCID','ROR','GND','Other'].map(v => ({ value: v, selected: v === typeStr }));
        return {
            idx,
            name: id.name || '',
            value: id.value || '',
            options
        };
    });
}

function identifierTypeToString(t: IdentifierType | string | undefined | null): string{
    // Be defensive: backend might return a plain string (e.g., "DOI")
    // or even null/undefined for legacy/empty identifiers
    if(typeof t === 'string') return t;
    if(t && typeof t === 'object'){
        // In tagged form, detect Other or the single key
        // eslint-disable-next-line no-prototype-builtins
        if((t as any).hasOwnProperty('Other')) return 'Other';
        const keys = Object.keys(t as object);
        return keys.length > 0 ? keys[0] : 'DOI';
    }
    return 'DOI';
}

function stringToIdentifierType(s: string): IdentifierType{
    switch(s){
        case 'DOI': return { DOI: null };
        case 'ISBN': return { ISBN: null };
        case 'ISSN': return { ISSN: null };
        case 'URL': return { URL: null };
        case 'URN': return { URN: null };
        case 'ORCID': return { ORCID: null };
        case 'ROR': return { ROR: null };
        case 'GND': return { GND: null };
        case 'Other': default: return { Other: '' };
    }
}

function collectIdentifiersFromUI(): Identifier[]{
    const rows = Array.from(document.querySelectorAll('#id-list .id-row')) as HTMLElement[];
    return rows.map(row => {
        const typeSel = row.querySelector('.id-type') as HTMLSelectElement;
        const nameInp = row.querySelector('.id-name') as HTMLInputElement;
        const valueInp = row.querySelector('.id-value') as HTMLInputElement;
        const type = stringToIdentifierType(typeSel.value);
        // fill Other with name if empty to keep something meaningful
        if('Other' in type && !type.Other){ type.Other = 'Other'; }
        return {
            id: null,
            name: nameInp.value || (('Other' in type) ? type.Other as string : typeSel.value),
            value: valueInp.value,
            identifier_type: type
        } as Identifier;
    }).filter(i => i.value && i.value.trim().length > 0);
}

function toUITags(keywords: Keyword[] | null | undefined){
    const res: Array<{ kind: 'free'; label: string } | { kind: 'gnd'; label: string; gndValue: string }> = [];
    if(!keywords) return res;
    for(const k of keywords){
        if(k && k.gnd && typeof k.gnd === 'object'){
            const idType: any = (k.gnd as any).identifier_type;
            let isGnd = false;
            if(typeof idType === 'string'){
                isGnd = idType === 'GND';
            }else if(idType && typeof idType === 'object'){
                isGnd = Object.prototype.hasOwnProperty.call(idType, 'GND');
            }
            if(isGnd){
                const gndValue = String((k.gnd as any).value ?? (k.gnd as any).id ?? k.title);
                res.push({ kind: 'gnd', label: k.title, gndValue });
                continue;
            }
        }
        if(k && typeof k.title === 'string'){
            res.push({ kind: 'free', label: k.title });
        }
    }
    return res;
}
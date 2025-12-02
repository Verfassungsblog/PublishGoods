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
        csl_options
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
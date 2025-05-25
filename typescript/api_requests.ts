export async function send_get_template_id_for_project(project_id: string) {
    const response = await fetch(`/api/projects/${project_id}/template`, {
        method: 'GET',
        headers: {
            'Content-Type': 'application/json'
        }
    });
    if (!response.ok) {
        throw new Error(`Failed to get project template: ${response.status}`);
    } else {
        let response_data = await response.json();
        if (response_data.hasOwnProperty("error")) {
            console.error(response_data["error"]);
            throw new Error(`Failed to save content blocks: ${response_data["error"]}`);
        } else {
            return response_data.data;
        }
    }
}

export async function send_update_content_blocks(project_id: string, section_path: string, data: any) {
    const response = await fetch(`/api/projects/` + project_id + `/sections/` + section_path + "/content_blocks/", {
        method: 'PUT',
        headers: {
            'Content-Type': 'application/json'
        },
        body: JSON.stringify(data)
    });
    if (!response.ok) {
        throw new Error(`Failed to update content block: ${response.status}`);
    } else {
        let response_data = await response.json();
        if (response_data.hasOwnProperty("error")) {
            console.error(response_data["error"]);
            throw new Error(`Failed to save content blocks: ${response_data["error"]}`);
        } else {
            return response_data;
        }
    }
}

export async function send_get_content_blocks(project_id: string, section_path: string) {
    const response = await fetch(`/api/projects/` + project_id + `/sections/` + section_path + "/content_blocks/", {
        method: 'GET',
        headers: {
            'Content-Type': 'application/json'
        }
    });
    if (!response.ok) {
        throw new Error(`Failed to get content blocks: ${response.status}`);
    } else {
        let response_data = await response.json();
        if (response_data.hasOwnProperty("error")) {
            throw new Error(`Failed to get content blocks: ${response_data["error"]}`);
        } else {
            return response_data;
        }
    }
}

export async function send_add_user(user_data: object) {
    const response = await fetch(`/api/users/`, {
        method: 'POST',
        headers: {
            'Content-Type': 'application/json'
        },
        body: JSON.stringify(user_data)
    });
    if (!response.ok) {
        throw new Error(`Failed to add user: ${response.status}`);
    } else {
        let response_data = await response.json();
        if (response_data.hasOwnProperty("error")) {
            throw new Error(`Failed to add user: ${response_data["error"]}`);
        } else {
            return response_data;
        }
    }
}

export async function send_update_user(user_id: string, patch_data: object) {
    const response = await fetch(`/api/users/` + user_id, {
        method: 'PATCH',
        headers: {
            'Content-Type': 'application/json'
        },
        body: JSON.stringify(patch_data)
    });
    if (!response.ok) {
        throw new Error(`Failed to update user: ${response.status}`);
    } else {
        let response_data = await response.json();
        if (response_data.hasOwnProperty("error")) {
            throw new Error(`Failed to update user: ${response_data["error"]}`);
        } else {
            return response_data;
        }
    }
}

export async function send_delete_user(user_id: string) {
    const response = await fetch(`/api/users/` + user_id, {
        method: 'DELETE',
        headers: {
            'Content-Type': 'application/json'
        }
    });
    if (!response.ok) {
        throw new Error(`Failed to delete user: ${response.status}`);
    } else {
        let response_data = await response.json();
        if (response_data.hasOwnProperty("error")) {
            throw new Error(`Failed to delete user: ${response_data["error"]}`);
        } else {
            return response_data;
        }
    }
}

export async function send_add_new_bib_entry(data: any, project_id: string) {
    const response = await fetch(`/api/projects/` + project_id + `/bibliography`, {
        method: 'POST',
        body: JSON.stringify(data),
        headers: {
            'Content-Type': 'application/json'
        }

    });
    if (!response.ok) {
        throw new Error(`Failed to add new bib entry: ${response.status}`);
    } else {
        let response_data = await response.json();
        if (response_data.hasOwnProperty("error")) {
            throw new Error(`Failed to add new bib entry: ` + Object.keys(response_data["error"])[0] + " " + Object.values(response_data["error"])[0]);
        } else {
            return response_data;
        }
    }
}

export async function send_get_bib_list(project_id: string) {
    const response = await fetch(`/api/projects/` + project_id + `/bibliography`, {
        method: 'GET',
        headers: {
            'Content-Type': 'application/json'
        }

    });
    if (!response.ok) {
        throw new Error(`Failed to get bib entries: ${response.status}`);
    } else {
        let response_data = await response.json();
        if (response_data.hasOwnProperty("error")) {
            throw new Error(`Failed to get bib entries: ` + Object.keys(response_data["error"])[0] + " " + Object.values(response_data["error"])[0]);
        } else {
            return response_data;
        }
    }
}

export async function send_get_bib_entry(key: string, project_id: string) {
    const response = await fetch(`/api/projects/` + project_id + `/bibliography/` + key, {
        method: 'GET',
        headers: {
            'Content-Type': 'application/json'
        }

    });
    if (!response.ok) {
        throw new Error(`Failed to get bib entry: ${response.status}`);
    } else {
        let response_data = await response.json();
        if (response_data.hasOwnProperty("error")) {
            throw new Error(`Failed to get bib entry: ` + Object.keys(response_data["error"])[0] + " " + Object.values(response_data["error"])[0]);
        } else {
            return response_data;
        }
    }
}

export async function update_bib_entry(data: any, key: string, project_id: string) {
    const response = await fetch(`/api/projects/` + project_id + `/bibliography/` + key, {
        method: 'PUT',
        body: JSON.stringify(data),
        headers: {
            'Content-Type': 'application/json'
        }

    });
    if (!response.ok) {
        throw new Error(`Failed to update bib entry: ${response.status}`);
    } else {
        let response_data = await response.json();
        if (response_data.hasOwnProperty("error")) {
            throw new Error(`Failed to update bib entry: ` + Object.keys(response_data["error"])[0] + " " + Object.values(response_data["error"])[0]);
        } else {
            return response_data;
        }
    }
}

export async function send_poll_import_status(id: string) {
    const response = await fetch(`/api/import/status/` + id, {
        method: 'GET',
        headers: {
            'Content-Type': 'application/json'
        }

    });
    if (!response.ok) {
        throw new Error(`Failed to get import status: ${response.status}`);
    } else {
        let response_data = await response.json();
        if (response_data.hasOwnProperty("error")) {
            console.error(response_data["error"]);
            throw new Error(`Failed to get import status: ` + Object.keys(response_data["error"])[0] + " " + Object.values(response_data["error"])[0]);
        } else {
            return response_data;
        }
    }
}


export async function send_import_from_wordpress(data: any) {
    const response = await fetch(`/api/import/wordpress`, {
        method: 'POST',
        body: JSON.stringify(data),
    });
    if (!response.ok) {
        throw new Error(`Failed to upload: ${response.status}`);
    } else {
        let response_data = await response.json();
        if (response_data.hasOwnProperty("error")) {
            throw new Error(`Failed to upload: ${response_data["error"]}`);
        } else {
            return response_data;
        }
    }
}

export type ExportStepData =
    | { Raw: RawExportStep }
    | { Vivliostyle: VivliostyleExportStep }
    | { Pandoc: PandocExportStep }
    | { Weasyprint: WeasyprintExportStep};

export interface ExportStep {
    id: string;
    name: string;
    data: ExportStepData;
    files_to_keep: string[];
}

export interface RawExportStep {
    entry_point: string;
    output_file: string;
}

export interface VivliostyleExportStep {
    press_ready: boolean;
    input_file: string;
    output_file: string;
}

export interface WeasyprintExportStep{
    input_file: string;
    output_file: string;
    pdf_variant: PdfVariant;
}

export enum PdfVariant{
    PDF = "PDF",
    PDFA1B = "PDFA1B",
    PDFA2B = "PDFA2B",
    PDFA3B = "PDFA3B",
    PDFA4B = "PDFA4B",
    PDFA2U = "PDFA2U",
    PDFA3U = "PDFA3U",
    PDFA4U = "PDFA4U",
    PDFUA1 = "PDFUA1",
    DEBUG = "DEBUG",
}

export interface PandocExportStep {
    input_file: string;
    input_format: string;
    output_file: string;
    output_format: string;
    shift_heading_level_by?: number;
    metadata_file?: string;
    epub_cover_image_path?: string;
    epub_title_page?: boolean;
    epub_metadata_file?: string;
    epub_embed_fonts?: string[];
}

export type ApiResult<T> = {
    error?: ApiError;
    data?: T;
}

export type ApiError =
    | { NotFound?: never }
    | { BadRequest?: string }
    | { Unauthorized?: never }
    | { InternalServerError?: never }
    | { Conflict?: string }
    | { Other?: string };

export function apiErrorToString(error: ApiError): string {
    let errorType = Object.keys(error)[0] as keyof ApiError;
    let errorMessage = error[errorType];
    return errorMessage ? `${errorMessage}` : errorType;
}

interface ExportFormat {
    slug: string;
    name: string;
    export_steps: ExportStep[];
    output_files: string[];
    preview_pdf_path: string | null;
}

export interface ProjectTemplateV2 {
    id: string;
    name: string;
    description: string;
    export_formats: Record<string, ExportFormat>;
}

export interface AssetList {
    assets: Asset[];
}

export interface AssetFolder {
    name: string;
    assets: Asset[];
}

export interface AssetFile {
    name: string;
    mime_type?: string;
}

export type Asset = AssetFolder | AssetFile;

export type SectionOrToc =
    | { Section: Section }
    | { Toc: never };

export interface Section {
    id?: string; // UUID represented as a string in JavaScript/TypeScript
    css_classes: string[];
    sub_sections: Section[];
    children: NewContentBlock[];
    visible_in_toc: boolean;
    metadata: SectionMetadata;
}

export interface APISectionResult {
    id: string; // UUID represented as a string in JavaScript/TypeScript
    css_classes: string[];
    sub_sections?: Section[];
    children: NewContentBlock[];
    visible_in_toc: boolean;
    metadata: APISectionMetadataResult;
}

export interface PatchSection{
    id?: string|null;
    css_classes?: string[];
    visible_in_toc?: boolean;
    metadata?: PatchSectionMetadata;
}

export interface PatchSectionMetadata{
    title?: string;
    toc_title_override?: string|null;
    subtitle?: string|null;
    toc_subtitle_override?: string|null;
    authors?: string[];
    editors?: string[];
    web_url?: string|null;
    identifiers?: Identifier[];
    published?: string|null;
    last_changed?: string|null;
    lang?: string|null;
}

export interface Person {
    id?: string; // UUID represented as a string in JavaScript/TypeScript
    first_names?: string | null;
    last_names: string;
    orcid?: Identifier | null;
    gnd?: Identifier | null;
    bios?: Biography[] | null;
    ror?: Identifier | null;
}

export interface Biography {
    content: string;
    lang?: Language | null;
}

enum Language {
    DE,
    EN
}

type APISectionMetadataResult = {
    title: string,
    subtitle: string | null,
    authors: string[],
    authors_expanded?: Person[],
    editors: string[],
    editors_expanded?: Person[],
    web_url: string | null,
    identifiers: Identifier[],
    published: Date | null,
    last_changed: Date | null,
    lang: string | null,
};

export type NewContentBlock = {
    id: string;
    block_type: BlockType;
    data: BlockData;
    css_classes: string[];
    revision_id?: string;
};

enum BlockType {
    Paragraph,
    Heading,
    Raw,
    List,
    Quote,
    Image,
}

type SectionMetadata = {
    title: string,
    toc_title_override: string|null,
    subtitle: string | null,
    toc_subtitle_override: string|null,
    authors: string[],
    editors: string[],
    web_url: string | null,
    identifiers: Identifier[],
    published: Date | null,
    last_changed: Date | null,
    lang: string | null,
};

type Identifier = {
    id: string | null,
    name: string,
    value: string,
    identifier_type: IdentifierType,
};

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

interface UploadedImage {
    url: string;
    filename: string;
}

type BlockData =
    | { Paragraph: {text: string} }
    | { Heading: {text: string, level: number} }
    | { Raw: {html: string} }
    | { List: {style: string, items: string[]} }
    | { Quote: {text: string, caption: string, alignment: string} }
    | { Image: {file: UploadedImage, caption?: string, with_border: boolean, with_background: boolean, stretched: boolean}};

export function ProjectAPI() {
    async function read_project_contents(project_id: string) {
        const response = await fetch(`/api/projects/${project_id}/contents`, {
            method: 'GET',
            headers: {
                'Content-Type': 'application/json'
            }
        });

        if (!response.ok) {
            throw new Error(`Failed to get template: ${response.status}`);
        }

        const response_data: ApiResult<SectionOrToc[]> = await response.json();

        if (response_data.error) {
            throw new Error(`Failed to get project contents: ${apiErrorToString(response_data.error)}`);
        }
        if (!response_data.data) {
            throw new Error('No data received');
        }

        return response_data.data;
    }

    return {
        read_project_contents
    }
}

export type NewLocalRenderingRequest = {
    // id of the project to render
    project_id: string,
    // list of export formats slugs that should be rendered
    export_formats: string[],
    // list of section ids to be rendered or null if all should be rendered
    sections: string[] | null
}

export type APIRenderingStatus =
    | "QueuedOnLocal"
    | "PreparingOnLocal"
    | "PreparedOnLocal"
    | "SendToRenderingServer"
    | "RequestingTemplate"
    | "TransmittingTemplate"
    | "QueuedOnRendering"
    | "Running"
    | "SavedOnLocal"
    | { Failed: string};

export function PersonsAPI(){
    async function send_search_person_request(search_term: string){
        const response = await fetch(`/api/persons?query=${search_term}`, {
            method: 'GET',
            headers: {
                'Content-Type': 'application/json'
            }
        });
        if(!response.ok){
            throw new Error(`Failed to search for persons: ${response.status}`);
        }
        const response_data: ApiResult<Person[]> = await response.json();

        if(response_data.error){
            throw new Error(`Failed to search for persons: ${apiErrorToString(response_data.error)}`);
        }

        if(!response_data.data){
            throw new Error('No data received');
        }
        return response_data.data;
    }
    async function send_get_person_request(person_id: string){
        const response = await fetch(`/api/persons/${person_id}`, {
            method: 'GET',
            headers: {
                'Content-Type': 'application/json'
            }
        });
        if(!response.ok){
            throw new Error(`Failed to get person: ${response.status}`);
        }
        const response_data: ApiResult<Person> = await response.json();

        if(response_data.error){
            throw new Error(`Failed to get person: ${apiErrorToString(response_data.error)}`);
        }

        if(!response_data.data){
            throw new Error('No data received');
        }
        return response_data.data;
    }
    async function send_delete_person_request(person_id: string){
        const response = await fetch(`/api/persons/${person_id}`, {
            method: 'DELETE',
            headers: {
                'Content-Type': 'application/json'
            }
        });
        if(!response.ok){
            throw new Error(`Failed to delete person: ${response.status}`);
        }
        const response_data: ApiResult<null> = await response.json();

        if(response_data.error){
            throw new Error(`Failed to delete person: ${apiErrorToString(response_data.error)}`);
        }

        return response_data.data;
    }
    return{
        send_search_person_request,
        send_get_person_request,
        send_delete_person_request
    }
}

export function ExportAPI(){
    async function send_new_rendering_request(rendering_request: NewLocalRenderingRequest){
        const response = await fetch(`/api/export/request`, {
            method: 'POST',
            headers: {
                'Content-Type': 'application/json'
            },
            body: JSON.stringify(rendering_request)
        });

        if (!response.ok) {
            throw new Error(`Failed to send rendering request: ${response.status}`);
        }

        const response_data: ApiResult<string> = await response.json();

        if (response_data.error) {
            throw new Error(`Failed to send rendering request: ${apiErrorToString(response_data.error)}`);
        }
        if (!response_data.data) {
            throw new Error('No data received');
        }

        return response_data.data;
    }
    async function get_request_status(rendering_request_id: string){
        const response = await fetch(`/api/export/request/${rendering_request_id}/status`, {
            method: 'GET',
            headers: {
                'Content-Type': 'application/json'
            }
        });

        if (!response.ok) {
            throw new Error(`Failed to fetch rendering request status: ${response.status}`);
        }

        const response_data: ApiResult<APIRenderingStatus> = await response.json();

        if (response_data.error) {
            throw new Error(`Failed to fetch rendering request status: ${apiErrorToString(response_data.error)}`);
        }
        if (!response_data.data) {
            throw new Error('No data received');
        }

        return response_data.data;
    }
    return{send_new_rendering_request, get_request_status}
}

export function TemplateAPI() {

    async function read_template(template_id: string) {
        const response = await fetch(`/api/templates/${template_id}`, {
            method: 'GET',
            headers: {
                'Content-Type': 'application/json'
            }
        });

        if (!response.ok) {
            throw new Error(`Failed to get template: ${response.status}`);
        }

        const response_data: ApiResult<ProjectTemplateV2> = await response.json();

        if (response_data.error) {
            throw new Error(`Failed to get template: ${apiErrorToString(response_data.error)}`);
        }
        if (!response_data.data) {
            throw new Error('No data received');
        }

        return response_data.data;
    }

    async function update_template(template: ProjectTemplateV2) {
        const response = await fetch(`/api/templates/${template.id}`, {
            method: 'POST',
            headers: {
                'Content-Type': 'application/json'
            },
            body: JSON.stringify(template)
        });

        if (!response.ok) {
            throw new Error(`Failed to update template: ${response.status}`);
        }

        const response_data: ApiResult<ProjectTemplateV2> = await response.json();

        if (response_data.error) {
            throw new Error(`Failed to update template: ${apiErrorToString(response_data.error)}`);
        }

        return response_data.data;
    }

    async function list_global_assets(template_id: string) {
        const response = await fetch(`/api/templates/${template_id}/assets`, {
            method: 'GET',
            headers: {
                'Content-Type': 'application/json'
            }
        });

        if (!response.ok) {
            throw new Error(`Failed to list global assets: ${response.status}`);
        }

        const response_data: ApiResult<AssetList> = await response.json();

        if (response_data.error) {
            throw new Error(`Failed to list global assets: ${apiErrorToString(response_data.error)}`);
        }

        return response_data.data;
    }

    async function create_folder(template_id: string, name: string) {
        const response = await fetch(`/api/templates/${template_id}/assets/folder`, {
            method: 'POST',
            headers: {
                'Content-Type': 'application/json'
            },
            body: JSON.stringify({
                name: name
            })
        });

        if (!response.ok) {
            throw new Error(`Failed to create folder: ${response.status}`);
        }

        const response_data: ApiResult<null> = await response.json();

        console.log(response_data);
        if (response_data.error) {
            throw new Error(`Failed to create folder: ${apiErrorToString(response_data.error)}`);
        }

        return response_data.data;
    }

    async function upload_file(template_id: string, file: File) {
        const formData = new FormData();
        formData.append('file', file);

        const response = await fetch(`/api/templates/${template_id}/assets/file`, {
            method: 'POST',
            body: formData
        });

        if (!response.ok) {
            throw new Error(`Failed to upload file: ${response.status}`);
        }

        const response_data: ApiResult<null> = await response.json();

        if (response_data.error) {
            throw new Error(`Failed to upload file: ${apiErrorToString(response_data.error)}`);
        }

        return response_data.data;
    }

    async function move_global_asset(template_id: string, old_path: string, new_path: string, overwrite_option: boolean) {
        const response = await fetch(`/api/templates/${template_id}/assets/move`, {
            method: 'POST',
            headers: {
                'Content-Type': 'application/json'
            },
            body: JSON.stringify({
                old_path: old_path,
                new_path: new_path,
                overwrite: overwrite_option
            })
        });

        if (!response.ok) {
            throw new Error(`Failed to move asset: ${response.status}`);
        }

        const response_data: ApiResult<null> = await response.json();

        if (response_data.error) {
            throw new Error(`Failed to move asset: ${apiErrorToString(response_data.error)}`);
        }

        return response_data.data;
    }

    async function delete_assets(template_id: string, paths: string[]) {
        const response = await fetch(`/api/templates/${template_id}/assets/`, {
            method: 'DELETE',
            headers: {
                'Content-Type': 'application/json'
            },
            body: JSON.stringify({
                paths: paths
            })
        });

        if (!response.ok) {
            throw new Error(`Failed to delete assets: ${response.status}`);
        }

        const response_data: ApiResult<null> = await response.json();

        if (response_data.error) {
            throw new Error(`${apiErrorToString(response_data.error)}`);
        }

        return response_data.data;
    }

    async function delete_assets_for_export_formats(template_id: string, slug: string, paths: string[]) {
        const response = await fetch(`/api/templates/${template_id}/export_formats/${slug}/assets/`, {
            method: 'DELETE',
            headers: {
                'Content-Type': 'application/json'
            },
            body: JSON.stringify({
                paths: paths
            })
        });

        if (!response.ok) {
            throw new Error(`Failed to delete assets: ${response.status}`);
        }

        const response_data: ApiResult<null> = await response.json();

        if (response_data.error) {
            console.log(response_data.error);
            throw new Error(`${apiErrorToString(response_data.error)}`);
        }

        return response_data.data;
    }

    async function get_asset_file(template_id: string, path: string) {
        const response = await fetch(`/api/templates/${template_id}/assets/files/${path}`, {
            method: 'GET',
            headers: {
                'Content-Type': 'application/json'
            },
        });

        if (!response.ok) {
            throw new Error(`Failed to get asset file: ${response.status}`);
        }
        const contentType = response.headers.get('Content-Type');
        console.log("Content type: " + contentType)

        let result;
        if (contentType && contentType.startsWith('text/')) {
            // If it's a text file, read it as text
            result = {
                type: 'text',
                data: await response.text(),
            };
        } else {
            // If it's not a text file, get it as a blob
            result = {
                type: 'blob',
                data: await response.blob(),
            };
        }
        return result;
    }

    async function get_asset_file_for_export_format(template_id: string, slug: string, path: string) {
        const response = await fetch(`/api/templates/${template_id}/export_formats/${slug}/assets/files/${path}`, {
            method: 'GET',
            headers: {
                'Content-Type': 'application/json'
            },
        });

        if (!response.ok) {
            throw new Error(`Failed to get asset file: ${response.status}`);
        }
        const contentType = response.headers.get('Content-Type');
        console.log("Content type: " + contentType)

        let result;
        if (contentType && contentType.startsWith('text/')) {
            // If it's a text file, read it as text
            result = {
                type: 'text',
                data: await response.text(),
            };
        } else {
            // If it's not a text file, get it as a blob
            result = {
                type: 'blob',
                data: await response.blob(),
            };
        }
        return result;
    }

    async function update_asset_text_file(template_id: string, path: string, content: string) {
        const response = await fetch(`/api/templates/${template_id}/assets/files/${path}`, {
            method: 'PUT',
            headers: {
                'Content-Type': 'application/json'
            },
            body: JSON.stringify({
                content: content
            })
        });

        if (!response.ok) {
            throw new Error(`Failed to update asset: ${response.status}`);
        }

        const response_data: ApiResult<null> = await response.json();

        if (response_data.error) {
            throw new Error(`${apiErrorToString(response_data.error)}`);
        }

        return response_data.data;
    }

    async function update_asset_text_file_for_export_format(template_id: string, path: string, slug: string, content: string) {
        const response = await fetch(`/api/templates/${template_id}/export_formats/${slug}/assets/files/${path}`, {
            method: 'PUT',
            headers: {
                'Content-Type': 'application/json'
            },
            body: JSON.stringify({
                content: content
            })
        });

        if (!response.ok) {
            throw new Error(`Failed to update asset: ${response.status}`);
        }

        const response_data: ApiResult<null> = await response.json();

        if (response_data.error) {
            throw new Error(`${apiErrorToString(response_data.error)}`);
        }

        return response_data.data;
    }

    async function create_export_format(template_id: string, name: string): Promise<any> {
        let data = {
            name: name,
            export_steps: [] as any[],
            output_files: [] as any[],
            slug: slugify(name)
        }

        const response = await fetch(`/api/templates/${template_id}/export_formats/`, {
            method: 'POST',
            headers: {
                'Content-Type': 'application/json'
            },
            body: JSON.stringify(data)
        });

        if (!response.ok) {
            throw new Error(`Failed to create export format: ${response.status}`);
        }

        const response_data: ApiResult<null> = await response.json();

        if (response_data.error) {
            throw new Error(`${apiErrorToString(response_data.error)}`);
        }

        return response_data.data;
    }

    async function change_export_format_metadata(template_id: string, slug: string, new_name: string, new_preview_pdf_path: string | null): Promise<any> {
        let data = {
            name: new_name,
            slug: slugify(new_name),
            preview_pdf_path: new_preview_pdf_path
        }

        const response = await fetch(`/api/templates/${template_id}/export_formats/${slug}/metadata`, {
            method: 'POST',
            headers: {
                'Content-Type': 'application/json'
            },
            body: JSON.stringify(data)
        });

        if (!response.ok) {
            throw new Error(`Failed to create export format: ${response.status}`);
        }

        const response_data: ApiResult<null> = await response.json();

        if (response_data.error) {
            throw new Error(`${apiErrorToString(response_data.error)}`);
        }

        return response_data.data;
    }

    async function delete_export_format(template_id: string, slug: string): Promise<any> {

        const response = await fetch(`/api/templates/${template_id}/export_formats/${slug}`, {
            method: 'DELETE',
            headers: {
                'Content-Type': 'application/json'
            },
        });

        if (!response.ok) {
            throw new Error(`Failed to delete export format: ${response.status}`);
        }

        const response_data: ApiResult<null> = await response.json();

        if (response_data.error) {
            throw new Error(`${apiErrorToString(response_data.error)}`);
        }

        return response_data.data;
    }

    async function list_export_format_assets(template_id: string, slug: string) {
        const response = await fetch(`/api/templates/${template_id}/export_formats/${slug}/assets`, {
            method: 'GET',
            headers: {
                'Content-Type': 'application/json'
            }
        });

        if (!response.ok) {
            throw new Error(`Failed to list global assets: ${response.status}`);
        }

        const response_data: ApiResult<AssetList> = await response.json();

        if (response_data.error) {
            throw new Error(`Failed to list global assets: ${apiErrorToString(response_data.error)}`);
        }

        return response_data.data;
    }

    async function upload_file_for_export_format(template_id: string, slug: string, file: File) {
        const formData = new FormData();
        formData.append('file', file);

        const response = await fetch(`/api/templates/${template_id}/export_formats/${slug}/assets/file`, {
            method: 'POST',
            body: formData
        });

        if (!response.ok) {
            throw new Error(`Failed to upload file: ${response.status}`);
        }

        const response_data: ApiResult<null> = await response.json();

        if (response_data.error) {
            throw new Error(`Failed to upload file: ${apiErrorToString(response_data.error)}`);
        }

        return response_data.data;
    }

    async function create_folder_for_export_format(template_id: string, name: string, slug: string) {
        const response = await fetch(`/api/templates/${template_id}/export_formats/${slug}/assets/folder`, {
            method: 'POST',
            headers: {
                'Content-Type': 'application/json'
            },
            body: JSON.stringify({
                name: name
            })
        });

        if (!response.ok) {
            throw new Error(`Failed to create folder: ${response.status}`);
        }

        const response_data: ApiResult<null> = await response.json();

        console.log(response_data);
        if (response_data.error) {
            throw new Error(`Failed to create folder: ${apiErrorToString(response_data.error)}`);
        }

        return response_data.data;
    }

    async function move_asset_for_export_format(template_id: string, old_path: string, new_path: string, slug: string, overwrite_option: boolean) {
        const response = await fetch(`/api/templates/${template_id}/export_formats/${slug}/assets/move`, {
            method: 'POST',
            headers: {
                'Content-Type': 'application/json'
            },
            body: JSON.stringify({
                old_path: old_path,
                new_path: new_path,
                overwrite: overwrite_option
            })
        });

        if (!response.ok) {
            throw new Error(`Failed to move asset: ${response.status}`);
        }

        const response_data: ApiResult<null> = await response.json();

        if (response_data.error) {
            throw new Error(`Failed to move asset: ${apiErrorToString(response_data.error)}`);
        }

        return response_data.data;
    }

    async function create_export_step(template_id: string, export_format_slug: string, export_step: ExportStep): Promise<any> {
        const response = await fetch(`/api/templates/${template_id}/export_formats/${export_format_slug}/export_steps`, {
            method: 'POST',
            headers: {
                'Content-Type': 'application/json'
            },
            body: JSON.stringify(export_step)
        });

        if (!response.ok) {
            throw new Error(`Failed to create export step: ${response.status}`);
        }

        const response_data: ApiResult<null> = await response.json();

        if (response_data.error) {
            throw new Error(`${apiErrorToString(response_data.error)}`);
        }

        return response_data.data;
    }

    async function delete_export_step(template_id: string, slug: string, step_id: string): Promise<any> {

        const response = await fetch(`/api/templates/${template_id}/export_formats/${slug}/export_steps/${step_id}`, {
            method: 'DELETE',
            headers: {
                'Content-Type': 'application/json'
            },
        });

        if (!response.ok) {
            throw new Error(`Failed to delete export step: ${response.status}`);
        }

        const response_data: ApiResult<null> = await response.json();

        if (response_data.error) {
            throw new Error(`${apiErrorToString(response_data.error)}`);
        }

        return response_data.data;
    }

    async function update_export_step(template_id: string, export_format_slug: string, export_step: ExportStep): Promise<any> {
        const response = await fetch(`/api/templates/${template_id}/export_formats/${export_format_slug}/export_steps/${export_step.id}`, {
            method: 'POST',
            headers: {
                'Content-Type': 'application/json'
            },
            body: JSON.stringify(export_step)
        });

        if (!response.ok) {
            throw new Error(`Failed to update export step: ${response.status}`);
        }

        const response_data: ApiResult<null> = await response.json();

        if (response_data.error) {
            throw new Error(`${apiErrorToString(response_data.error)}`);
        }

        return response_data.data;
    }

    async function move_export_step_after(template_id: string, export_format_slug: string, export_step_id: string, move_after: string | null): Promise<any> {
        let data = {
            move_after: move_after
        };
        const response = await fetch(`/api/templates/${template_id}/export_formats/${export_format_slug}/export_steps/${export_step_id}/move`, {
            method: 'POST',
            headers: {
                'Content-Type': 'application/json'
            },
            body: JSON.stringify(data)
        });

        if (!response.ok) {
            throw new Error(`Failed to move export step: ${response.status}`);
        }

        const response_data: ApiResult<null> = await response.json();

        if (response_data.error) {
            throw new Error(`${apiErrorToString(response_data.error)}`);
        }

        return response_data.data;
    }

    async function list_export_steps(template_id: string, slug: string) {
        const response = await fetch(`/api/templates/${template_id}/export_formats/${slug}/export_steps`, {
            method: 'GET',
            headers: {
                'Content-Type': 'application/json'
            }
        });

        if (!response.ok) {
            throw new Error(`Failed to list export steps: ${response.status}`);
        }

        const response_data: ApiResult<AssetList> = await response.json();

        if (response_data.error) {
            throw new Error(`Failed to list export steps: ${apiErrorToString(response_data.error)}`);
        }

        return response_data.data;
    }

    return {
        read_template,
        update_template,
        create_folder,
        upload_file,
        list_global_assets,
        move_global_asset,
        delete_assets,
        get_asset_file,
        update_asset_text_file,
        create_export_format,
        delete_export_format,
        list_export_format_assets,
        get_asset_file_for_export_format,
        upload_file_for_export_format,
        delete_assets_for_export_formats,
        create_folder_for_export_format,
        move_asset_for_export_format,
        update_asset_text_file_for_export_format,
        create_export_step,
        delete_export_step,
        update_export_step,
        move_export_step_after,
        list_export_steps,
        change_export_format_metadata
    }
}

export function SectionAPI(){
    /**
     * Requests the data for a section
     *
     * @param project_id
     * @param section_path
     * @param expand_authors boolean, if true also adds details for authors
     * @param expand_editors boolean, if true also adds details for editors
     * @param expand_subsections boolean, if true also adds subsections
     */
    async function read_section(project_id: string, section_path: string, expand_authors: boolean, expand_editors: boolean, expand_subsections: boolean) {
        let expand_query = "expand=";

        if(expand_authors){
            expand_query += "authors,"
        }
        if(expand_editors){
            expand_query += "editors,"
        }
        if(expand_subsections){
            expand_query += "subsections,"
        }
        expand_query = expand_query.substring(0, expand_query.length -1);

        const response = await fetch(`/api/projects/${project_id}/sections/${section_path}?${expand_query}`, {
            method: 'GET',
            headers: {
                'Content-Type': 'application/json'
            }
        });

        if (!response.ok) {
            throw new Error(`Failed to get sections: ${response.status}`);
        }

        const response_data: ApiResult<APISectionResult> = await response.json();

        if (response_data.error) {
            throw new Error(`Failed to get template: ${apiErrorToString(response_data.error)}`);
        }
        if (!response_data.data) {
            throw new Error('No data received');
        }

        return response_data.data;
    }
    async function patch_section(project_id: string, section_path: string, data: PatchSection){
        const response = await fetch(`/api/projects/${project_id}/sections/${section_path}`, {
            method: 'PATCH',
            headers: {
                'Content-Type': 'application/json'
            },
            body: JSON.stringify(data)
        });

        if (!response.ok) {
            throw new Error(`Failed to update section: ${response.status}`);
        }

        const response_data: ApiResult<any> = await response.json();

        if (response_data.error) {
            throw new Error(`Failed to update section: ${apiErrorToString(response_data.error)}`);
        }

        return response_data.data;
    }
    async function delete_section(project_id: string, section_path: string){
        const response = await fetch(`/api/projects/${project_id}/sections/${section_path}`, {
            method: 'DELETE',
            headers: {
                'Content-Type': 'application/json'
            }
        });

        if (!response.ok) {
            throw new Error(`Failed to delete section: ${response.status}`);
        }

        const response_data: ApiResult<any> = await response.json();

        if (response_data.error) {
            throw new Error(`Failed to delete section: ${apiErrorToString(response_data.error)}`);
        }

        return response_data.data;
    }
    return{
        read_section,
        patch_section,
        delete_section
    }
}

function slugify(text: string): string {
    return text
        .trim() // trim leading and trailing spaces
        .toLowerCase() // convert text to lowercase
        .replace(/\s+/g, '-') // replace spaces with -
        .normalize('NFD') // decompose accented characters
        .replace(/[\u0300-\u036f]/g, '') // remove diacritics
        .replace(/[^a-z0-9 -]/g, '') // remove invalid characters
        .replace(/-+/g, '-'); // collapse multiple -'s
}

export interface HierarchicalCategory {
    id: number;
    count: number;
    description: string;
    name: string;
    slug: string;
    parent: number;
    children: HierarchicalCategory[];
}

export interface CategoryTree {
    categories: HierarchicalCategory[];
}

export interface PreviewRequest {
    base_url: string;
    include_categories?: number[];
    exclude_categories?: number[];
    before?: string; // ISO-8601 Datumsformat
    modified_before?: string; // ISO-8601 Datumsformat
    after?: string; // ISO-8601 Datumsformat
    modified_after?: string; // ISO-8601 Datumsformat
    per_page?: number;
    page?: number;
}

export interface PostPreviewResponse {
    number_of_posts: number;
    previews: PostPreview[];
}

export interface PostPreview {
    id: number;
    date: string; // ISO-8601 Datumsformat
    link: string;
    slug: string;
    post_type: string; // Umbenannt von "type"
    title: RenderedContent;
    author: number;
    excerpt: RenderedContent;
}

export interface RenderedContent {
    rendered: string;
}

export interface WordpressFilterData{
    wp_host: string;
    before?: string; // ISO-8601 Date Format
    after?: string; // ISO-8601 Date Format
    include_categories?: number[]; // Array of ids
    exclude_categories?: number[];
}

export type WordpressImportData = {
    WordpressLinks: string[];
} | {
    WordpressFilter: WordpressFilterData;
};

export interface WordpressImportRequest{
    project_id: string;
    data: WordpressImportData;
    endnotes: boolean;
    shift_headings: boolean;
    convert_links: boolean;
}

export type WordpressAPIError =
    | "SerdeParsingError"
    | "ReqwestError"
    | "InvalidURL"
    | "NotFound"
    | "UnexpectedResponse"
    | {Unsupported: string};

export type ImportError =
    | "UnsupportedFileType"
    | "InvalidFile"
    | "BibFileInvalid"
    | "PandocError"
    | "HtmlConversionFailed"
    | "ProjectNotFound"
    | { WordPressApiError: WordpressAPIError };

export interface ProcessingDetails{
    current: number;
    total?: number;
}

export type ImportStatus =
    | "Pending"
    | "RequestWPPosts"
    | "Complete"
    | {Processing: ProcessingDetails}
    | {Failed: ImportError};

export function ImportAPI(){
    /**
     * Loads the category tree from the specified host.
     *
     * @param {string} host - The base URL of the host to fetch the category tree from without protocol.
     * @return {Promise<CategoryTree>} A promise that resolves to the category tree data.
     * @throws {Error} If the request fails or if the response contains an error.
     */
    async function load_category_tree(host: string){
        const response = await fetch(`/api/import/wordpress/categories?base_url=${host}`, {
            method: 'GET',
            headers: {
                'Content-Type': 'application/json'
            }
        });

        if (!response.ok) {
            throw new Error(`Failed to retrieve category tree: ${response.status}`);
        }

        const response_data: ApiResult<CategoryTree> = await response.json();

        if (response_data.error) {
            throw new Error(`Failed to retrieve category tree: ${apiErrorToString(response_data.error)}`);
        }

        return response_data.data;
    }
    /**
     * Sends a request to retrieve a preview of posts based on the provided preview request data.
     *
     * @param {PreviewRequest} preview_request - The preview request data to be sent to the API, including any necessary parameters for filtering or customizing the posts preview.
     * @return {Promise<PostPreviewResponse>} A promise that resolves to the preview of posts, or throws an error if the API request fails or returns an error.
     */
    async function load_posts_preview(preview_request: PreviewRequest){
        const response = await fetch(`/api/import/wordpress/posts-preview`, {
            method: 'POST',
            headers: {
                'Content-Type': 'application/json'
            },
            body: JSON.stringify(preview_request)
        });

        if (!response.ok) {
            throw new Error(`Failed to retrieve posts preview: ${response.status}`);
        }

        const response_data: ApiResult<PostPreviewResponse> = await response.json();

        if (response_data.error) {
            throw new Error(`Failed to retrieve posts preview: ${apiErrorToString(response_data.error)}`);
        }

        return response_data.data;
    }
    /**
     * Adds a WordPress import job by sending the provided import data to the server.
     *
     * @param {WordpressImportRequest} import_data - The data required to initiate the WordPress import process.
     * @return {Promise<string>} A promise that resolves to a job identifier string if the import job is successfully created.
     * @throws {Error} If the request fails or if an error is returned by the server.
     */
    async function add_wp_import_job(import_data: WordpressImportRequest): Promise<string>{
        const response = await fetch(`/api/import/wordpress`, {
            method: 'POST',
            headers: {
                'Content-Type': 'application/json'
            },
            body: JSON.stringify(import_data)
        });

        if (!response.ok) {
            throw new Error(`Failed to start import from wordpress: ${response.status}`);
        }

        const response_data: ApiResult<string> = await response.json();

        if (response_data.error) {
            throw new Error(`Failed to start import from wordpress: ${apiErrorToString(response_data.error)}`);
        }

        return response_data.data;
    }
    async function poll_import_status(job_id: string){
        const response = await fetch(`/api/import/status/`+job_id, {
            method: 'GET',
            headers: {
                'Content-Type': 'application/json'
            },
        });

        if (!response.ok) {
            throw new Error(`Failed to poll import status: ${response.status}`);
        }

        const response_data: ApiResult<ImportStatus> = await response.json();

        if (response_data.error) {
            throw new Error(`Failed to poll import status: ${apiErrorToString(response_data.error)}`);
        }

        return response_data.data;
    }
    async function add_file_import_job(data: FormData) {
        const response = await fetch(`/api/import/upload`, {
            method: 'POST',
            body: data,
        });
        if (!response.ok) {
            throw new Error(`Failed to add file import job: ${response.status}`);
        }

        const response_data: ApiResult<string> = await response.json();

        if (response_data.error) {
            throw new Error(`Failed to add file import job: ${apiErrorToString(response_data.error)}`);
        }

        return response_data.data;
    }
    return{
        load_category_tree,
        load_posts_preview,
        add_wp_import_job,
        poll_import_status,
        add_file_import_job,
    }
}
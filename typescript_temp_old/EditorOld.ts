import EditorJS from "@editorjs/editorjs";
import Header from '@editorjs/header';
// @ts-ignore
import RawTool from '@editorjs/raw';
import {NoteTool} from "./NoteTool";
const Quote:any = require('@editorjs/quote');
const Undo: any = require('editorjs-undo');
const ImageTool: any = require('@editorjs/image');
const List: any = require("@editorjs/list");
const Strikethrough: any = require("@sotaproject/strikethrough");
import * as API from "./api_requests";
import * as Tools from "./tools";
import {CustomStyleTool} from "./CustomStyleTool";
import {CitationTool} from "./CitationTool";
import {BlockStyleTune} from "./BlockStyleTune";

let typing_timer: number | null = null;
let editor: EditorJS | null = null;

export async function show_editor(){
    let first_change = true;
    try {
        // @ts-ignore
        let data = (await API.send_get_content_blocks(globalThis.project_id, globalThis.section_path)).data;
        console.log(data);

        // @ts-ignore
        let by_file_upload_endpoint = '/api/projects/'+globalThis.project_id+'/uploads';

        editor = new EditorJS({
            holder: "section_content_blocks_inner",
            tools: {
                header: {
                    // @ts-ignore
                    class: Header,
                    inlineToolbar: true,
                },
                raw: RawTool,
                list: {
                    class: List,
                    inlineToolbar: true,
                    config: {
                        defaultStyle: 'unordered'
                    }
                },
                note: NoteTool,
                quote: {
                    class: Quote,
                    inlineToolbar: true,
                },
                strikethrough: Strikethrough,
                custom_style_tool: CustomStyleTool,
                citation: CitationTool,
                image: {
                    class: ImageTool,
                    config: {
                        endpoints: {
                            byFile: by_file_upload_endpoint,
                            byUrl: '/api/fetch_image', //TODO: implement endpoint
                        }
                    }
                },
                block_style_tune: BlockStyleTune
            },
            tunes: ['block_style_tune'],
            data: {blocks: data},
            onChange: (api, event) => {
                if(!first_change){ // Don't save the first change, as it's just the initial load
                    save_changes();
                }else{
                    first_change = false;
                }
            },
            onReady: () => {
                const undo = new Undo({ editor });
                undo.initialize({blocks: data});
            },
        });

        await editor.isReady;

        document.getElementById("section_content_blocks_inner").addEventListener("input", typing_handler);

        // Make all existing notes and citations clickable
        NoteTool.add_all_show_note_settings_listeners();
        CitationTool.add_all_show_note_settings_listeners();
    }catch(e){
        console.error(e);
        Tools.show_alert("Couldn't load content.", "danger");
    }
}

export async function save_changes(){
    let data = await editor.save();
    console.log(data);

    //TODO: only update content blocks that changed

    try {
        // @ts-ignore
        await API.send_update_content_blocks(globalThis.project_id, globalThis.section_path, data.blocks);
        //Tools.show_alert("Saved Changes.", "success");
    }catch(e){
        console.error(e);
        Tools.show_alert("Couldn't save content.", "danger");
    }
}

function typing_handler(){
    if (typing_timer) {
        clearTimeout(typing_timer);
    }

    // Set a timeout to wait for the user to stop typing
    // @ts-ignore
    typing_timer = setTimeout(async function(){
        await save_changes();
    }, 500);
}
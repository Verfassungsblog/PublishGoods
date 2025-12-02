import {APIProjectData} from "../api_requests";
import {main_col} from "./Editor";

export async function show_project_metadata_settings(data: APIProjectData){
    main_col.innerHTML = "Test123!";
}
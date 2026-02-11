use crate::session::session_guard::Session;
use crate::settings::Settings;
use crate::storage::project_storage::current::{BibEntryOrFolder, Bibliography};
use crate::storage::project_storage::ProjectStorage;
use crate::utils::api_helpers::{APIResult, ApiError, ApiErrorType};
use hayagriva::types::EntryType;
use rocket::serde::json::Json;
use rocket::serde::{Deserialize, Serialize};
use rocket::State;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ShortBibEntryOrFolder {
    pub id: Uuid,
    pub is_folder: bool,
    pub bib_entry_type: Option<EntryType>,
    pub name: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BibTreeEntry {
    #[serde(flatten)]
    pub short: ShortBibEntryOrFolder,
    pub children: Vec<BibTreeEntry>,
}

#[get("/project/<project_id>/bibliography")]
pub async fn get_bibliography_tree(
    project_id: &str,
    _session: Session,
    settings: &State<Settings>,
    project_storage: &State<Arc<ProjectStorage>>,
) -> APIResult<Vec<BibTreeEntry>> {
    let project_uuid = Uuid::parse_str(project_id)
        .map_err(|_| ApiError::from(ApiErrorType::UnparsableParameter("project_id".to_string())))?;
    let project_lock = project_storage
        .get_project(&project_uuid, settings)
        .await
        .map_err(|e| ApiError::from(e))?;
    let project = project_lock
        .read()
        .map_err(|_| ApiError::from(ApiErrorType::InternalServerError))?;

    let bibliography = &project.bibliography;

    // Build the tree
    let mut tree = Vec::new();
    let mut by_parent: HashMap<Option<Uuid>, Vec<Uuid>> = HashMap::new();

    for (id, entry) in &bibliography.entries {
        let parent = match entry {
            BibEntryOrFolder::BibEntry(e) => e.parents.first().cloned(), // Assuming first parent for tree view if multiple exist
            BibEntryOrFolder::BibFolder(f) => f.parent,
        };
        by_parent.entry(parent).or_default().push(*id);
    }

    if let Some(root_ids) = by_parent.get(&None) {
        for id in root_ids {
            tree.push(build_tree_node(*id, &bibliography, &by_parent));
        }
    }

    Ok(tree.into())
}

fn build_tree_node(
    id: Uuid,
    bib: &Bibliography,
    by_parent: &HashMap<Option<Uuid>, Vec<Uuid>>,
) -> BibTreeEntry {
    let entry = bib.entries.get(&id).unwrap();
    let (is_folder, bib_entry_type, name) = match entry {
        BibEntryOrFolder::BibEntry(e) => (
            false,
            Some(e.entry_type),
            e.title
                .as_ref()
                .map(|t| t.value.clone())
                .unwrap_or_else(|| e.key.to_string()),
        ),
        BibEntryOrFolder::BibFolder(f) => (true, None, f.name.clone()),
    };

    let mut children = Vec::new();
    if let Some(child_ids) = by_parent.get(&Some(id)) {
        for child_id in child_ids {
            children.push(build_tree_node(*child_id, bib, by_parent));
        }
    }

    BibTreeEntry {
        short: ShortBibEntryOrFolder {
            id,
            is_folder,
            bib_entry_type,
            name,
        },
        children,
    }
}

#[get("/project/<project_id>/bibliography/<entry_id>")]
pub async fn get_bibliography_entry(
    project_id: &str,
    entry_id: &str,
    _session: Session,
    settings: &State<Settings>,
    project_storage: &State<Arc<ProjectStorage>>,
) -> APIResult<BibEntryOrFolder> {
    let project_uuid = Uuid::parse_str(project_id)
        .map_err(|_| ApiError::from(ApiErrorType::UnparsableParameter("project_id".to_string())))?;
    let entry_uuid = Uuid::parse_str(entry_id)
        .map_err(|_| ApiError::from(ApiErrorType::UnparsableParameter("entry_id".to_string())))?;
    let project_lock = project_storage
        .get_project(&project_uuid, settings)
        .await
        .map_err(|e| ApiError::from(e))?;
    let project = project_lock
        .read()
        .map_err(|_| ApiError::from(ApiErrorType::InternalServerError))?;

    let entry = project
        .bibliography
        .entries
        .get(&entry_uuid)
        .ok_or_else(|| {
            ApiError::new(
                ApiErrorType::ResourceNotFound("Bibliography entry not found".to_string()),
                None,
            )
        })?;

    Ok(entry.clone().into())
}

#[post("/project/<project_id>/bibliography", data = "<entry>")]
pub async fn post_bibliography_entry(
    project_id: &str,
    entry: Json<BibEntryOrFolder>,
    _session: Session,
    settings: &State<Settings>,
    project_storage: &State<Arc<ProjectStorage>>,
) -> APIResult<Uuid> {
    let project_uuid = Uuid::parse_str(project_id)
        .map_err(|_| ApiError::from(ApiErrorType::UnparsableParameter("project_id".to_string())))?;
    let project_lock = project_storage
        .get_project(&project_uuid, settings)
        .await
        .map_err(|e| ApiError::from(e))?;

    let id = {
        let mut project = project_lock
            .write()
            .map_err(|_| ApiError::from(ApiErrorType::InternalServerError))?;

        let mut new_entry = entry.into_inner();
        let id = match &mut new_entry {
            BibEntryOrFolder::BibEntry(e) => {
                if e.key == Uuid::nil() {
                    e.key = Uuid::new_v4();
                }
                e.key
            }
            BibEntryOrFolder::BibFolder(_) => Uuid::new_v4(),
        };

        project.bibliography.entries.insert(id, new_entry);
        id
    };

    project_storage
        .save_project_to_disk(&project_uuid, settings)
        .await
        .map_err(|_| ApiError::from(ApiErrorType::InternalServerError))?;

    Ok(id.into())
}

#[patch("/project/<project_id>/bibliography/<entry_id>", data = "<patch>")]
pub async fn patch_bibliography_entry(
    project_id: &str,
    entry_id: &str,
    patch: Json<BibEntryOrFolder>,
    _session: Session,
    settings: &State<Settings>,
    project_storage: &State<Arc<ProjectStorage>>,
) -> APIResult<()> {
    let project_uuid = Uuid::parse_str(project_id)
        .map_err(|_| ApiError::from(ApiErrorType::UnparsableParameter("project_id".to_string())))?;
    let entry_uuid = Uuid::parse_str(entry_id)
        .map_err(|_| ApiError::from(ApiErrorType::UnparsableParameter("entry_id".to_string())))?;
    let project_lock = project_storage
        .get_project(&project_uuid, settings)
        .await
        .map_err(|e| ApiError::from(e))?;

    {
        let mut project = project_lock
            .write()
            .map_err(|_| ApiError::from(ApiErrorType::InternalServerError))?;

        if !project.bibliography.entries.contains_key(&entry_uuid) {
            return Err(ApiError::new(
                ApiErrorType::ResourceNotFound("Bibliography entry not found".to_string()),
                None,
            ));
        }

        project
            .bibliography
            .entries
            .insert(entry_uuid, patch.into_inner());
    }

    project_storage
        .save_project_to_disk(&project_uuid, settings)
        .await
        .map_err(|_| ApiError::from(ApiErrorType::InternalServerError))?;

    Ok(().into())
}

#[delete("/project/<project_id>/bibliography/<entry_id>")]
pub async fn delete_bibliography_entry(
    project_id: &str,
    entry_id: &str,
    _session: Session,
    settings: &State<Settings>,
    project_storage: &State<Arc<ProjectStorage>>,
) -> APIResult<()> {
    let project_uuid = Uuid::parse_str(project_id)
        .map_err(|_| ApiError::from(ApiErrorType::UnparsableParameter("project_id".to_string())))?;
    let entry_uuid = Uuid::parse_str(entry_id)
        .map_err(|_| ApiError::from(ApiErrorType::UnparsableParameter("entry_id".to_string())))?;
    let project_lock = project_storage
        .get_project(&project_uuid, settings)
        .await
        .map_err(|e| ApiError::from(e))?;

    {
        let mut project = project_lock
            .write()
            .map_err(|_| ApiError::from(ApiErrorType::InternalServerError))?;

        if project.bibliography.entries.remove(&entry_uuid).is_none() {
            return Err(ApiError::new(
                ApiErrorType::ResourceNotFound("Bibliography entry not found".to_string()),
                None,
            ));
        }

        // Clean up references in other entries
        for entry in project.bibliography.entries.values_mut() {
            match entry {
                BibEntryOrFolder::BibEntry(e) => {
                    e.parents.retain(|&id| id != entry_uuid);
                }
                BibEntryOrFolder::BibFolder(f) => {
                    if f.parent == Some(entry_uuid) {
                        f.parent = None;
                    }
                }
            }
        }
    }

    project_storage
        .save_project_to_disk(&project_uuid, settings)
        .await
        .map_err(|_| ApiError::from(ApiErrorType::InternalServerError))?;

    Ok(().into())
}

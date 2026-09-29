use crate::db::repositories::bibliography;
use crate::session::session_guard::Session;
use crate::storage::project_storage::current::{BibEntryOrFolder, Bibliography};
use crate::utils::api_helpers::{APIResult, ApiError, ApiErrorType};
use hayagriva::types::EntryType;
use rocket::State;
use rocket::serde::json::Json;
use rocket::serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::cmp::Ordering;
use std::collections::HashMap;
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

/// Returns the project's bibliography as a tree of folders and entries, sorted with
/// folders first and then alphabetically (case-insensitive) by name at each level.
#[get("/api/project/<project_id>/bibliography")]
pub async fn get_bibliography_tree(
    project_id: &str,
    _session: Session,
    pool: &State<PgPool>,
) -> APIResult<Vec<BibTreeEntry>> {
    let project_uuid = Uuid::parse_str(project_id)
        .map_err(|_| ApiError::from(ApiErrorType::UnparsableParameter("project_id".to_string())))?;
    let bibliography = bibliography::get_all_for_project(pool.inner(), project_uuid).await?;

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
        // Sort: folders first, then entries; both alphabetically by name (case-insensitive)
        let mut root_ids_sorted = root_ids.clone();
        sort_ids_by_folder_then_name(&mut root_ids_sorted, &bibliography);
        for id in root_ids_sorted {
            tree.push(build_tree_node(id, &bibliography, &by_parent));
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
        BibEntryOrFolder::BibEntry(e) => {
            let name = if let Some(title) = &e.title {
                title.value.clone()
            } else {
                e.key.to_string()
            };
            (false, Some(e.entry_type), name)
        }
        BibEntryOrFolder::BibFolder(f) => (true, None, f.name.clone()),
    };

    let mut children = Vec::new();
    if let Some(child_ids) = by_parent.get(&Some(id)) {
        let mut child_ids_sorted = child_ids.clone();
        sort_ids_by_folder_then_name(&mut child_ids_sorted, bib);
        for child_id in child_ids_sorted {
            children.push(build_tree_node(child_id, bib, by_parent));
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

/// Sorts IDs in place so that folders come first, then entries; both groups sorted
/// alphabetically (case-insensitive) by name.
fn sort_ids_by_folder_then_name(ids: &mut [Uuid], bib: &Bibliography) {
    ids.sort_by(|a, b| {
        let (ka, na) = entry_sort_key(*a, bib);
        let (kb, nb) = entry_sort_key(*b, bib);
        match ka.cmp(&kb) {
            Ordering::Equal => na.cmp(&nb),
            other => other,
        }
    });
}

// Returns (group_key, name) where group_key ensures folders (0) sort before entries (1)
fn entry_sort_key(id: Uuid, bib: &Bibliography) -> (u8, String) {
    let entry = bib.entries.get(&id).unwrap();
    match entry {
        BibEntryOrFolder::BibFolder(f) => (0u8, f.name.to_lowercase()),
        BibEntryOrFolder::BibEntry(e) => {
            let name = if let Some(title) = &e.title {
                title.value.clone()
            } else {
                e.key.to_string()
            };
            (1u8, name.to_lowercase())
        }
    }
}

/// Returns a single bibliography entry or folder by id, looked up from the project's
/// full bibliography.
#[get("/api/project/<project_id>/bibliography/<entry_id>")]
pub async fn get_bibliography_entry(
    project_id: &str,
    entry_id: &str,
    _session: Session,
    pool: &State<PgPool>,
) -> APIResult<BibEntryOrFolder> {
    let project_uuid = Uuid::parse_str(project_id)
        .map_err(|_| ApiError::from(ApiErrorType::UnparsableParameter("project_id".to_string())))?;
    let entry_uuid = Uuid::parse_str(entry_id)
        .map_err(|_| ApiError::from(ApiErrorType::UnparsableParameter("entry_id".to_string())))?;

    let bibliography = bibliography::get_all_for_project(pool.inner(), project_uuid).await?;
    let entry = bibliography.entries.get(&entry_uuid).ok_or_else(|| {
        ApiError::new(
            ApiErrorType::ResourceNotFound("Bibliography entry not found".to_string()),
            None,
        )
    })?;

    Ok(entry.clone().into())
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BibSearchResult {
    pub id: Uuid,
    pub entry_type: EntryType,
    pub title: String,
}

/// Search bibliography entries (entries only, folders excluded) by title, UUID or contributor names
#[get("/api/project/<project_id>/bibliography/search?<q>")]
pub async fn search_bibliography_entries(
    project_id: &str,
    q: &str,
    _session: Session,
    pool: &State<PgPool>,
) -> APIResult<Vec<BibSearchResult>> {
    let project_uuid = Uuid::parse_str(project_id)
        .map_err(|_| ApiError::from(ApiErrorType::UnparsableParameter("project_id".to_string())))?;
    let bibliography = bibliography::get_all_for_project(pool.inner(), project_uuid).await?;
    let needle = q.to_lowercase();

    let mut results: Vec<BibSearchResult> = Vec::new();
    for (id, entry) in &bibliography.entries {
        if let BibEntryOrFolder::BibEntry(e) = entry {
            // Title or UUID string match
            let mut haystacks: Vec<String> = vec![e.key.to_string().to_lowercase()];

            if let Some(title) = &e.title {
                haystacks.push(title.value.to_lowercase());
            }

            // Search in contributors (authors/editors) if present
            for p in e.authors.iter().chain(e.editors.iter()) {
                // `name` is a mandatory `String` in MyPerson
                haystacks.push(p.name.to_lowercase());
                if let Some(n) = &p.given_name {
                    haystacks.push(n.to_lowercase());
                }
                if let Some(n) = &p.prefix {
                    haystacks.push(n.to_lowercase());
                }
                if let Some(n) = &p.suffix {
                    haystacks.push(n.to_lowercase());
                }
                if let Some(n) = &p.alias {
                    haystacks.push(n.to_lowercase());
                }
            }

            if haystacks.iter().any(|h| h.contains(&needle)) {
                let title = e
                    .title
                    .as_ref()
                    .map(|t| t.value.clone())
                    .unwrap_or_else(|| e.key.to_string());
                results.push(BibSearchResult {
                    id: *id,
                    entry_type: e.entry_type,
                    title,
                });
            }
        }
    }

    // Basic ordering: by title
    results.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));

    // Limit the number of results to avoid overly long suggestion lists
    const MAX_RESULTS: usize = 5;
    if results.len() > MAX_RESULTS {
        results.truncate(MAX_RESULTS);
    }

    Ok(results.into())
}

/// Inserts a new bibliography entry or folder into the project, generating a fresh
/// id for it if one wasn't supplied, and returns the id of the created row.
#[post("/api/project/<project_id>/bibliography", data = "<entry>")]
pub async fn post_bibliography_entry(
    project_id: &str,
    entry: Json<BibEntryOrFolder>,
    _session: Session,
    pool: &State<PgPool>,
) -> APIResult<Uuid> {
    let project_uuid = Uuid::parse_str(project_id)
        .map_err(|_| ApiError::from(ApiErrorType::UnparsableParameter("project_id".to_string())))?;

    let id = bibliography::insert(pool.inner(), project_uuid, &entry.into_inner()).await?;
    Ok(id.into())
}

/// Replaces an existing bibliography entry or folder with the given patch data.
/// Returns a not-found error if the entry doesn't exist in the project.
#[patch("/api/project/<project_id>/bibliography/<entry_id>", data = "<patch>")]
pub async fn patch_bibliography_entry(
    project_id: &str,
    entry_id: &str,
    patch: Json<BibEntryOrFolder>,
    _session: Session,
    pool: &State<PgPool>,
) -> APIResult<()> {
    let project_uuid = Uuid::parse_str(project_id)
        .map_err(|_| ApiError::from(ApiErrorType::UnparsableParameter("project_id".to_string())))?;
    let entry_uuid = Uuid::parse_str(entry_id)
        .map_err(|_| ApiError::from(ApiErrorType::UnparsableParameter("entry_id".to_string())))?;

    bibliography::update(pool.inner(), project_uuid, entry_uuid, &patch.into_inner()).await?;
    Ok(().into())
}

/// Deletes a bibliography entry or folder from the project, cleaning up any
/// references to it from other entries' parent links.
#[delete("/api/project/<project_id>/bibliography/<entry_id>")]
pub async fn delete_bibliography_entry(
    project_id: &str,
    entry_id: &str,
    _session: Session,
    pool: &State<PgPool>,
) -> APIResult<()> {
    let project_uuid = Uuid::parse_str(project_id)
        .map_err(|_| ApiError::from(ApiErrorType::UnparsableParameter("project_id".to_string())))?;
    let entry_uuid = Uuid::parse_str(entry_id)
        .map_err(|_| ApiError::from(ApiErrorType::UnparsableParameter("entry_id".to_string())))?;

    bibliography::delete(pool.inner(), project_uuid, entry_uuid).await?;
    Ok(().into())
}

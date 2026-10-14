pub mod api;
pub mod create;
pub mod list;

use crate::db::repositories::persons as persons_repo;
use crate::storage::project_storage::current::PersonUuidOrString;
use crate::utils::api_helpers::{ApiError, ApiErrorType};
use sqlx::PgPool;
use uuid::Uuid;

/// Fails with `ResourceNotFound` unless every [`PersonUuidOrString::PersonUuid`] in `links` may
/// be linked as author/editor of `project_id` (see [`persons_repo::linkable_ids`]):
/// it must belong to the project itself or to the library of the team owning the project.
/// Free-text names are always allowed, they live inside the project.
pub async fn ensure_linkable(
    pool: &PgPool,
    project_id: Uuid,
    links: &[PersonUuidOrString],
    role: &str,
) -> Result<(), ApiError> {
    let ids: Vec<Uuid> = links
        .iter()
        .filter_map(|link| match link {
            PersonUuidOrString::PersonUuid(id) => Some(*id),
            PersonUuidOrString::NameString(_) => None,
        })
        .collect();
    if ids.is_empty() {
        return Ok(());
    }
    let linkable = persons_repo::linkable_ids(pool, &ids, project_id).await?;
    match ids.iter().find(|id| !linkable.contains(id)) {
        Some(id) => Err(ApiErrorType::ResourceNotFound(format!("{} with id {}", role, id)).into()),
        None => Ok(()),
    }
}

//! Filesystem storage for a section's yrs/Yjs CRDT body content.
//!
//! Deliberately kept out of Postgres (see the schema and `db::data_migration::migrate_section`)
//! — one flat file per section at `<data_path>/sections/<section_id>`, independent of the
//! `sections` table row, which only holds metadata.

use crate::settings::Settings;
use uuid::Uuid;

/// Path to the on-disk CRDT content file for a section.
fn section_path(settings: &Settings, section_id: Uuid) -> String {
    format!("{}/sections/{}", settings.data_path, section_id)
}

/// Reads a section's CRDT bytes. Returns an empty `Vec` (not an error) if no file exists yet
/// — a section with no content is a normal, new/empty document.
pub async fn read(settings: &Settings, section_id: Uuid) -> std::io::Result<Vec<u8>> {
    match tokio::fs::read(section_path(settings, section_id)).await {
        Ok(bytes) => Ok(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}

/// Writes a section's CRDT bytes to disk, creating the sections directory if needed.
pub async fn write(settings: &Settings, section_id: Uuid, bytes: &[u8]) -> std::io::Result<()> {
    let dir = format!("{}/sections", settings.data_path);
    tokio::fs::create_dir_all(&dir).await?;
    tokio::fs::write(section_path(settings, section_id), bytes).await
}

/// Deletes a section's CRDT file. Not finding one is not an error (nothing was ever written,
/// e.g. an empty section that was deleted before its first edit).
pub async fn delete(settings: &Settings, section_id: Uuid) -> std::io::Result<()> {
    match tokio::fs::remove_file(section_path(settings, section_id)).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

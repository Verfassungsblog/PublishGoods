use crate::storage::project_storage::sections::Section;
use uuid::Uuid;

/// Generates a block id as a UUID v4 string
pub fn generate_id(section: &Section) -> String {
    let _ = section;
    Uuid::new_v4().to_string()
}

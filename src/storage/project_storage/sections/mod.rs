use crate::storage::project_storage::sections::current::{SectionMetadataV6, SectionV6};

pub mod content;
pub mod current;
pub mod migration;

pub type Section = SectionV6;
pub type SectionMetadata = SectionMetadataV6;

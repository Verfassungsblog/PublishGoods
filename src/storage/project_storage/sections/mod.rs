use crate::storage::project_storage::sections::current::{SectionMetadataV5, SectionV5};

pub mod content;
pub mod current;
pub mod migration;

pub type Section = SectionV5;
pub type SectionMetadata = SectionMetadataV5;

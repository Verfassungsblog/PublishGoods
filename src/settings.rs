use config::{Config, ConfigError, Environment, File};
use serde::Deserialize;
use std::env;

/// Stores settings read from config files.
#[derive(Debug, Deserialize, Clone)]
#[allow(unused)]
pub struct Settings {
    /// Full app title shown in navbar
    pub app_title: String,
    /// How long should a project be kept in memory after it was last accessed in seconds
    pub project_cache_time: u64,
    /// Where should the app store the data
    pub data_path: String,
    /// How long should the app wait for a file lock in ms
    pub file_lock_timeout: u64,
    pub backup_to_file_interval: u64,
    pub max_connections_to_rendering_server: u64,
    pub max_import_threads: u64,
    pub zotero_translation_server: String,
    pub export_servers: Vec<ExportServer>,
    pub ca_cert_path: String,
    pub client_cert_path: String,
    pub client_key_path: String,
    pub revocation_list_path: String,
    pub version: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ExportServer {
    pub hostname: String,
    pub port: u32,
    pub domain_name: String,
}

impl Settings {
    pub fn builder() -> Result<Self, ConfigError> {
        let run_mode = env::var("RUN_MODE").unwrap_or_else(|_| "development".into());
        // Read version String from version.txt
        let version = std::fs::read_to_string("version.txt").unwrap_or_else(|_| "unknown".into());

        let s = Config::builder()
            .add_source(File::with_name("config/default"))
            .add_source(File::with_name(&format!("config/{}", run_mode)).required(false))
            .add_source(File::with_name("config/local").required(false))
            .add_source(Environment::with_prefix("app"))
            .set_override("version", version)?
            .build()?;

        s.try_deserialize()
    }
}

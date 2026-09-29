use config::{Config, ConfigError, Environment, File};
use serde::Deserialize;
use std::env;

/// Stores settings read from config files.
#[derive(Debug, Deserialize, Clone)]
#[allow(unused)]
pub struct Settings {
    /// Full app title shown in navbar
    pub app_title: String,
    /// Publicly accessible URL of this instance
    pub instance_url: String,
    /// How long should a project be kept in memory after it was last accessed in seconds
    pub project_cache_time: u64,
    /// Where should the app store the data
    pub data_path: String,
    /// PostgreSQL connection string (e.g. postgresql://user@host:5432/db)
    pub database_url: String,
    /// Max connections to postgresql db
    pub database_max_connections: u32,
    /// How long should the app wait for a file lock in ms
    pub file_lock_timeout: u64,
    pub backup_to_file_interval: u64,
    pub max_connections_to_rendering_server: u64,
    pub max_import_threads: u64,
    pub max_external_rendering_jobs: u64,
    /// Seconds a finished external rendering job's results stay downloadable before the job and
    /// its result files are removed.
    pub external_rendering_result_validity: u64,
    /// Seconds a rendering may take from the moment its request is sent to a rendering server
    /// until the result arrived (including template transfer and the server's own queue) before
    /// it is aborted. 0 disables the timeout.
    pub rendering_server_timeout: u64,
    /// Seconds connecting to a single rendering server may take before the next one is tried.
    /// 0 disables the timeout.
    pub rendering_server_connect_timeout: u64,
    /// Seconds preparing a project for rendering (before it is sent to a rendering server) may
    /// take before it is aborted. 0 disables the timeout.
    pub rendering_preprocessing_timeout: u64,
    /// Seconds an import job may take before it is aborted. 0 disables the timeout.
    pub import_timeout: u64,
    pub zotero_translation_server: String,
    pub export_servers: Vec<ExportServer>,
    pub ca_cert_path: String,
    pub client_cert_path: String,
    pub client_key_path: String,
    pub revocation_list_path: String,
    pub version: String,
    /// How many failed login attempts within `lockout_window_minutes` trigger a lockout.
    pub max_login_attempts: i64,
    /// Rolling window (in minutes) used both to count recent failed login attempts and to
    /// lock the account.
    pub lockout_window_minutes: i64,
    /// Connection string for smtp server
    pub smtp_connection_url: String,
    /// From Adress used for outgoing mails
    pub mail_from_address: String,
    /// SMTP minimum number of idle connections
    pub smtp_pool_min_idle: u32,
    /// SMTP maximum number of pooled connections
    pub smtp_pool_max_size: u32,
    /// SMTP connection idle timeout in seconds
    pub smtp_pool_idle_timeout: u64,
    /// Maximum number of times a failed mail job is retried before it is dropped
    pub mail_max_retries: u8,
    /// Base delay in seconds before the first retry attempt, multiplied by the retry attempt number for each subsequent retry
    pub mail_base_retry_delay_seconds: u64,
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
        let version = env!("CARGO_PKG_VERSION");

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

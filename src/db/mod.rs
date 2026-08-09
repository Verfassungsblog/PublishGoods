//! PostgreSQL connectivity for the ongoing migration away from the bincode-based
//! [`DataStorage`](crate::storage::data_storage::DataStorage) /
//! [`ProjectStorage`](crate::storage::project_storage::ProjectStorage).
//!
//! This module owns the [`sqlx::PgPool`], applies the SQL schema migrations in
//! `migrations/`, and (via [`data_migration`]) performs the one-shot import of the
//! existing on-disk bincode data into the database.

use crate::settings::Settings;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

pub mod data_migration;

/// Creates the PostgreSQL connection pool from [`Settings::database_url`].
///
/// Panics on failure — a missing/broken database connection is fatal at startup.
pub async fn init_pool(settings: &Settings) -> PgPool {
    PgPoolOptions::new()
        .max_connections(settings.database_max_connections)
        .connect(&settings.database_url)
        .await
        .expect("Failed to connect to PostgreSQL. Check the database_url setting.")
}

/// Applies all pending SQL schema migrations from the `migrations/` directory.
///
/// sqlx records applied migrations in `_sqlx_migrations`, so already-applied ones
/// are skipped on subsequent runs.
pub async fn run_migrations(pool: &PgPool) {
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .expect("Failed to run database schema migrations.");
}

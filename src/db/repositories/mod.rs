//! sqlx/PostgreSQL-backed repository layer.
//!
//! One module per schema domain (see `migrations/20260809074848_initial.sql`). Every
//! function takes `impl sqlx::PgExecutor<'e>` so callers can pass either `&PgPool` for a
//! standalone read, or `&mut *tx` for multi-statement atomic writes.

pub mod bibliography;
pub mod folders;
pub mod persons;
pub mod projects;
pub mod sections;
pub mod templates;
pub mod users;

/// Unified error type for the repository layer, convertible into [`crate::utils::api_helpers::ApiError`].
#[derive(Debug)]
pub enum DbError {
    /// No row found for the given lookup. Carries a resource name for the API error message.
    NotFound(&'static str),
    /// A check/unique constraint was violated (e.g. "exactly one owner", duplicate email).
    Conflict(String),
    /// Any other database error.
    Sqlx(sqlx::Error),
    /// CRDT content file I/O (section content lives on the filesystem, not in Postgres).
    Io(std::io::Error),
}

/// Wraps a content-file I/O failure as [`DbError::Io`].
impl From<std::io::Error> for DbError {
    fn from(e: std::io::Error) -> Self {
        DbError::Io(e)
    }
}

/// Classifies a raw `sqlx::Error` into the repository layer's error variants: a missing row
/// becomes [`DbError::NotFound`], a known constraint violation (or any unique violation)
/// becomes [`DbError::Conflict`], and everything else passes through as [`DbError::Sqlx`].
impl From<sqlx::Error> for DbError {
    fn from(e: sqlx::Error) -> Self {
        match &e {
            sqlx::Error::RowNotFound => DbError::NotFound("resource"),
            sqlx::Error::Database(db_err) => match db_err.constraint() {
                Some("owner") | Some("person_id_or_name") => {
                    DbError::Conflict(db_err.message().to_string())
                }
                Some(c) if db_err.is_unique_violation() => {
                    DbError::Conflict(format!("{} already in use", c))
                }
                _ => DbError::Sqlx(e),
            },
            _ => DbError::Sqlx(e),
        }
    }
}

/// Human-readable rendering used by [`crate::utils::api_helpers::ApiError`].
impl std::fmt::Display for DbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DbError::NotFound(resource) => write!(f, "{} not found", resource),
            DbError::Conflict(msg) => write!(f, "conflict: {}", msg),
            DbError::Sqlx(e) => write!(f, "database error: {}", e),
            DbError::Io(e) => write!(f, "content file error: {}", e),
        }
    }
}

impl std::error::Error for DbError {}

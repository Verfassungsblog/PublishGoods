pub mod api;
#[cfg(feature = "language_detection")]
pub mod language_detection;
pub mod link_converter;
pub mod processing;
pub mod wordpress;

/// Returns a process-wide shared HTTP client for requests made while importing (media
/// downloads, Zotero translation server), so the connection pool is reused. Timeouts are set
/// per request.
pub(crate) fn http_client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new)
}

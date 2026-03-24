use crate::settings::Settings;
use crate::utils::api_helpers::ApiError;
use hayagriva::citationberg::{IndependentStyle, Locale, LocaleFile};
use std::collections::HashMap;

pub struct CslData {
    pub locales: Vec<Locale>,
    pub styles: HashMap<String, IndependentStyle>,
}

impl CslData {
    pub fn new(settings: &Settings) -> CslData {
        CslData {
            locales: load_locales(settings),
            styles: load_styles(settings),
        }
    }
}

pub fn load_locales(settings: &Settings) -> Vec<Locale> {
    let files = std::fs::read_dir(format!("{}/csl_locales", settings.data_path)).unwrap();
    let mut locales = Vec::new();
    for file in files {
        let file = file.unwrap();
        let content = std::fs::read_to_string(file.path()).unwrap();
        let locale = LocaleFile::from_xml(&content).unwrap().into();
        locales.push(locale);
    }

    locales
}

pub fn load_styles(settings: &Settings) -> HashMap<String, IndependentStyle> {
    let mut styles = HashMap::new();

    let files = std::fs::read_dir(format!("{}/csl_styles", settings.data_path)).unwrap();
    for file in files {
        let file = file.unwrap();
        let content = std::fs::read_to_string(file.path()).unwrap();
        let fname = file.file_name().clone().to_string_lossy().to_string();
        styles.insert(
            fname.as_str()[..fname.len() - 4].to_string(),
            IndependentStyle::from_xml(&content).unwrap(),
        );
    }

    styles
}

/// Asynchronously retrieves a list of available CSL (Citation Style Language) styles.
///
/// This function scans the directory specified by `settings.data_path` under the
/// folder `csl_styles` for files with a `.csl` extension. It extracts the filenames
/// without the `.csl` extension and returns them as a list of strings.
///
/// # Arguments
///
/// * `settings` - A reference to a `Settings` struct that contains the path to the data directory.
///
/// # Returns
///
/// Returns an `APIResult` containing a vector of strings. Each string represents the name
/// of a CSL style available in the corresponding directory.
///
/// # Errors
///
/// This function will return an error in the following cases:
/// * If the `data_path/csl_styles` directory does not exist or cannot be accessed.
/// * If there is an error while reading the directory entries.
/// * If there is an error in file operations (e.g., file name extraction).
///
/// # Example
///
/// ```ignore
/// let settings = Settings {
///     data_path: String::from("/path/to/data"),
/// };
///
/// let available_styles = list_available_styles(&settings).await?;
///
/// // Example output: ["apa", "mla", "chicago"]
/// println!("{:?}", available_styles);
/// ```
///
/// # Notes
///
/// * This function uses the Tokio asynchronous runtime for directory reading operations.
/// * The `.csl` suffix is automatically trimmed from the filenames in the result.
///
pub async fn list_available_styles(settings: &Settings) -> Result<Vec<String>, ApiError> {
    let mut available_styles = Vec::new();
    let path = format!("{}/csl_styles", settings.data_path);

    let mut dir = tokio::fs::read_dir(&path).await?;
    loop {
        let entry = dir.next_entry().await?;
        if let Some(entry) = entry {
            let file_name = entry.file_name().to_string_lossy().to_string();
            if file_name.ends_with(".csl") {
                available_styles.push(file_name[..file_name.len() - 4].to_string());
            }
        } else {
            break;
        }
    }

    Ok(available_styles)
}

/// Asynchronously retrieves a list of available CSL (Citation Style Language) locales.
///
/// This function scans the directory specified by `settings.data_path` under the
/// folder `csl_locales` for files with a `.xml` extension. It extracts the filenames
/// without the `.xml` extension and returns them as a list of strings.
///
/// # Arguments
///
/// * `settings` - A reference to a `Settings` struct that contains the path to the data directory.
///
/// # Returns
///
/// Returns an `APIResult` containing a vector of strings. Each string represents the name
/// of a CSL locale available in the corresponding directory.
///
/// # Errors
///
/// This function will return an error in the following cases:
/// * If the `data_path/csl_locales` directory does not exist or cannot be accessed.
/// * If there is an error while reading the directory entries.
/// * If there is an error in file operations (e.g., file name extraction).
///
/// # Example
///
/// ```ignore
/// let settings = Settings {
///     data_path: String::from("/path/to/data"),
/// };
///
/// let available_locales = list_available_locales(&settings).await?;
///
/// // Example output: ["en-US", "de-DE", "fr-FR"]
/// println!("{:?}", available_locales);
/// ```
///
/// # Notes
///
/// * The `.xml` suffix is automatically trimmed from the filenames in the result.
///
pub async fn list_available_locales(settings: &Settings) -> Result<Vec<String>, ApiError> {
    let mut available_locales = Vec::new();
    let path = format!("{}/csl_locales", settings.data_path);

    let mut dir = tokio::fs::read_dir(&path).await?;
    loop {
        let entry = dir.next_entry().await?;
        if let Some(entry) = entry {
            let file_name = entry.file_name().to_string_lossy().to_string();
            if file_name.ends_with(".xml") {
                let mut name = file_name[..file_name.len() - 4].to_string();
                if name.starts_with("locales-") {
                    name = name[8..].to_string();
                }
                available_locales.push(name);
            }
        } else {
            break;
        }
    }

    Ok(available_locales)
}

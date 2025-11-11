use crate::settings::Settings;
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
    let mut files = std::fs::read_dir(format!("{}/csl_locales", settings.data_path)).unwrap();
    let mut locales = Vec::new();
    while let Some(file) = files.next() {
        let file = file.unwrap();
        let content = std::fs::read_to_string(file.path()).unwrap();
        let locale = LocaleFile::from_xml(&content).unwrap().into();
        locales.push(locale);
    }

    locales
}

pub fn load_styles(settings: &Settings) -> HashMap<String, IndependentStyle> {
    let mut styles = HashMap::new();

    let mut files = std::fs::read_dir(format!("{}/csl_styles", settings.data_path)).unwrap();
    while let Some(file) = files.next() {
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

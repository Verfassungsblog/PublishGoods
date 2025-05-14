use lingua::{Language, LanguageDetectorBuilder};
use crate::import::wordpress::Post;
use crate::projects::{BlockData, SectionV4};

/// Attempts to detect the language of a WordPress post's content.
///
/// This function uses a language detector with low-accuracy mode to analyze the rendered content
/// of the post and returns the detected language in BCP-47 format.
///
/// # Arguments
/// * `post` - Reference to a WordPress post containing the content to analyze
///
/// # Returns
/// * `Some(Language)` - The detected language in BCP-47 format
/// * `None` - If the language could not be detected or could not be mapped to a BCP-47 code
pub fn detect_language_for_post(post: &Post) -> Option<language::Language>{
    debug!("Trying to detect language for post");
    
    let detector = LanguageDetectorBuilder::from_all_languages().with_low_accuracy_mode().build();
    let detected_lang = detector.detect_language_of(&post.content.rendered);
    
    match detected_lang {
        Some(lang) => match language_to_bcp47(lang){
            Some(lang) => Some(lang),
            None => None,
        },
        None => None
    }
}

/// Attempts to detect the primary language of a section's content.
///
/// This function analyzes the text content of paragraph blocks within the given section
/// to determine its language. It uses a low-accuracy mode for faster processing.
///
/// # Arguments
/// * `section` - The section whose language should be detected
///
/// # Returns
/// * `Some(Language)` - The detected language as a BCP-47 language tag
/// * `None` - If the language could not be detected or is not supported
pub fn detect_language_for_section(section: &SectionV4) -> Option<language::Language>{
    debug!("Trying to detect language for section");
    
    let content_to_analyze : String = section.children.iter().map(|block|{
        match &block.data{
            BlockData::Paragraph { text } => {
                text.clone()
            }
            _ => {
                String::new()
            }
        }
    }).collect();
    let detector = LanguageDetectorBuilder::from_all_languages().with_low_accuracy_mode().build();
    let detected_lang = detector.detect_language_of(&content_to_analyze);
    match detected_lang {
        Some(lang) => match language_to_bcp47(lang){
            Some(lang) => Some(lang),
            None => None,
        },
        None => None
    }
}
/// Converts a [`Language`] to a BCP-47 compliant [`language::Language`].
///
/// This function maps language codes from the lingua crate's format to BCP-47 language tags
/// that include both language and region codes (e.g., "en-US", "de-DE").
///
/// # Arguments
/// * `lang` - The source language from the lingua crate to convert
///
/// # Returns
/// * `Some(Language)` - The corresponding BCP-47 language tag if a mapping exists
/// * `None` - If no mapping exists for the given language
///
/// # Note
/// Not all lingua languages have a corresponding BCP-47 mapping. In such cases,
/// the function returns `None`.
fn language_to_bcp47(lang: Language) -> Option<language::Language> {
    match lang {
        Language::Afrikaans => Some(language::Language::AfZa),
        Language::Albanian => Some(language::Language::SqAl),
        Language::Arabic => Some(language::Language::ArSa),
        Language::Armenian => Some(language::Language::HyAm),
        Language::Azerbaijani => Some(language::Language::AzLatnAz),
        Language::Basque => Some(language::Language::EuEs),
        Language::Belarusian => Some(language::Language::BeBy),
        Language::Bokmal => Some(language::Language::NbNo),
        Language::Bosnian => Some(language::Language::BsLatnBa),
        Language::Bulgarian => Some(language::Language::BgBg),
        Language::Catalan => Some(language::Language::CaEs),
        Language::Chinese => Some(language::Language::ZhCn),
        Language::Croatian => Some(language::Language::HrHr),
        Language::Czech => Some(language::Language::CsCz),
        Language::Danish => Some(language::Language::DaDk),
        Language::Dutch => Some(language::Language::NlNl),
        Language::English => Some(language::Language::EnUs),
        Language::Estonian => Some(language::Language::EtEe),
        Language::Finnish => Some(language::Language::FiFi),
        Language::French => Some(language::Language::FrFr),
        Language::Georgian => Some(language::Language::KaGe),
        Language::German => Some(language::Language::DeDe),
        Language::Greek => Some(language::Language::ElGr),
        Language::Gujarati => Some(language::Language::GuIn),
        Language::Hebrew => Some(language::Language::HeIl),
        Language::Hindi => Some(language::Language::HiIn),
        Language::Hungarian => Some(language::Language::HuHu),
        Language::Icelandic => Some(language::Language::IsIs),
        Language::Indonesian => Some(language::Language::IdId),
        Language::Irish => Some(language::Language::GaIe),
        Language::Italian => Some(language::Language::ItIt),
        Language::Japanese => Some(language::Language::JaJp),
        Language::Kazakh => Some(language::Language::KkKz),
        Language::Korean => Some(language::Language::KoKr),
        Language::Latvian => Some(language::Language::LvLv),
        Language::Lithuanian => Some(language::Language::LtLt),
        Language::Macedonian => Some(language::Language::MkMk),
        Language::Malay => Some(language::Language::MsMy),
        Language::Maori => Some(language::Language::MiNz),
        Language::Marathi => Some(language::Language::MrIn),
        Language::Mongolian => Some(language::Language::MnMn),
        Language::Nynorsk => Some(language::Language::NnNo),
        Language::Persian => Some(language::Language::FaIr),
        Language::Polish => Some(language::Language::PlPl),
        Language::Portuguese => Some(language::Language::PtPt),
        Language::Punjabi => Some(language::Language::PaIn),
        Language::Romanian => Some(language::Language::RoRo),
        Language::Russian => Some(language::Language::RuRu),
        Language::Serbian => Some(language::Language::SrLatnRs),
        Language::Slovak => Some(language::Language::SkSk),
        Language::Slovene => Some(language::Language::SlSi),
        Language::Somali => Some(language::Language::SoSo),
        Language::Sotho => Some(language::Language::StZa),
        Language::Spanish => Some(language::Language::EsEs),
        Language::Swahili => Some(language::Language::SwKe),
        Language::Swedish => Some(language::Language::SvSe),
        Language::Tamil => Some(language::Language::TaIn),
        Language::Telugu => Some(language::Language::TeIn),
        Language::Thai => Some(language::Language::ThTh),
        Language::Tsonga => Some(language::Language::TsZa),
        Language::Tswana => Some(language::Language::TnZa),
        Language::Turkish => Some(language::Language::TrTr),
        Language::Ukrainian => Some(language::Language::UkUa),
        Language::Urdu => Some(language::Language::UrPk),
        Language::Vietnamese => Some(language::Language::ViVn),
        Language::Welsh => Some(language::Language::CyGb),
        Language::Xhosa => Some(language::Language::XhZa),
        Language::Yoruba => Some(language::Language::YoNg),
        Language::Zulu => Some(language::Language::ZuZa),
        _ => None,
    }
}
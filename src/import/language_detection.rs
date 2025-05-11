use lingua::LanguageDetectorBuilder;
use language::Language;
use crate::import::wordpress::Post;

pub fn detect_language_for_post(post: &Post) -> Option<Language>{
    debug!("Trying to detect language for_post");
    
    let detector = LanguageDetectorBuilder::from_all_languages().with_low_accuracy_mode().build();
    let detected_lang = detector.detect_language_of(&post.content.rendered);
    todo!()
}
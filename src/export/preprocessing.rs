use crate::storage::data_storage::DataStorage;
use crate::storage::project_storage::current::PersonUuidOrString;
use crate::storage::project_storage::sections::content::current::{
    decode_yjs_content, BlockData, NewContentBlock,
};
use crate::storage::project_storage::sections::Section;
use crate::storage::project_storage::ProjectData;
use crate::utils::csl::CslData;
use async_recursion::async_recursion;
use base64::engine::general_purpose;
use base64::Engine;
use hayagriva::citationberg::LocaleCode;
use hayagriva::BibliographyDriver;
use hayagriva::BibliographyRequest;
use hayagriva::CitationItem;
use hayagriva::CitationRequest;
use hyphenation::{Hyphenator, Load, Standard};
use image::{DynamicImage, ImageOutputFormat};
use language::Language;
use std::collections::HashMap;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::OnceLock;
use vb_exchange::projects::PreparedProject;
use vb_exchange::projects::{
    PersonOrString, PreparedContentBlock, PreparedEndnote, PreparedLicense, PreparedMetadata,
    PreparedSection, PreparedSectionMetadata,
};
use vb_exchange::RenderingError;
use html5ever::{parse_document, parse_fragment, ParseOpts, serialize, QualName};
use html5ever::tendril::{StrTendril, TendrilSink};
use markup5ever_rcdom::{Handle, NodeData, RcDom, SerializableHandle};
use markup5ever::{ns, local_name, Attribute};
use std::borrow::Borrow;
use markup5ever::interface::{NodeOrText, TreeSink};

/// Prepares a project for rendering or export by processing its metadata, authors, editors, and sections.
///
/// This function takes the given `project_data` along with references to necessary shared data
/// like `data_storage` and `csl_data`. Optionally, a subset of section UUIDs can be specified
/// in `sections_to_include` in order to only prepare and render those sections.
/// The current project's UUID must be provided.
///
/// The preparation process includes:
///  - Rendering citations for the project
///  - Validating and collecting the project metadata
///  - Gathering and resolving authors and editors (resolving UUIDs to stored persons, or keeping name strings)
///  - Selecting and preparing the relevant sections (either all, or only those listed in `sections_to_include`)
///  - Collecting any additional authors and editors mentioned in the rendered sections, ensuring all relevant contributors are included
///  - Sorting the final lists of authors and editors
///
/// # Arguments
/// * `project_data` - Full data of the project to be prepared.
/// * `data_storage` - Shared reference to user and entity storage for resolving person UUIDs.
/// * `csl_data` - Shared reference for citation style language data required for rendering citations.
/// * `sections_to_include` - Optional list of UUIDs representing the sections to include in preparation.
/// * `project_id` - The UUID of the current project.
///
/// # Returns
/// Returns `Ok(PreparedProject)` on success with all relevant data ready for export or further processing.
/// Returns `Err(RenderingError)` if required metadata is missing or other preparation steps fail.
pub async fn prepare_project(
    project_data: ProjectData,
    data_storage: Arc<DataStorage>,
    csl_data: Arc<CslData>,
    sections_to_include: Option<Vec<uuid::Uuid>>,
    project_id: &uuid::Uuid,
) -> Result<PreparedProject, RenderingError> {
    let citation_bib = render_citations(&project_data, csl_data);

    let metadata = match project_data.metadata {
        Some(metadata) => metadata,
        None => return Err(RenderingError::ProjectMetadataMissing),
    };

    let add_soft_hyphens = match &project_data.settings {
        Some(settings) => settings.add_soft_hyphens,
        None => false,
    };

    let mut authors = vec![];
    for author in metadata.authors.unwrap_or_default() {
        match author {
            PersonUuidOrString::PersonUuid(id) => {
                let person = match data_storage.get_person_cloned(&id) {
                    Some(person) => person,
                    None => {
                        eprintln!(
                            "Author with id {} not found while rendering section for export!",
                            id
                        );
                        continue;
                    }
                };
                authors.push(PersonOrString::Person(person));
            }
            PersonUuidOrString::NameString(name_str) => {
                authors.push(PersonOrString::NameString(name_str));
            }
        }
    }

    let mut editors = vec![];
    for editor in metadata.editors.unwrap_or_default() {
        match editor {
            PersonUuidOrString::PersonUuid(id) => {
                let person = match data_storage.get_person_cloned(&id) {
                    Some(person) => person,
                    None => {
                        eprintln!(
                            "Editor with id {} not found while rendering section for export!",
                            id
                        );
                        continue;
                    }
                };
                editors.push(PersonOrString::Person(person));
            }
            PersonUuidOrString::NameString(name_str) => {
                editors.push(PersonOrString::NameString(name_str));
            }
        }
    }

    let license = if let Some(license) = metadata.license {
        Some(PreparedLicense::from(license))
    } else {
        None
    };

    let mut data = vec![];
    for section in project_data.sections {
        if let Some(id) = section.id {
            // Check if only specified sections should be included
            match &sections_to_include {
                Some(sections_to_include) => {
                    // Only prepare specified sections
                    if sections_to_include.contains(&id) {
                        data.push(
                            render_section(
                                section,
                                data_storage.clone(),
                                &citation_bib,
                                project_id,
                                add_soft_hyphens,
                            )
                            .await,
                        )
                    }
                }
                None => data.push(
                    render_section(
                        section,
                        data_storage.clone(),
                        &citation_bib,
                        project_id,
                        add_soft_hyphens,
                    )
                    .await,
                ), // Prepare all sections
            }
        }
    }

    for section in data.iter() {
        add_remaining_authors_editors_from_section(section, &mut authors, &mut editors);
    }

    // Sort authors and editors by last name
    authors.sort();
    editors.sort();

    let published = match metadata.published {
        Some(date) => Some(date.into()),
        None => None,
    };

    let prepared_metadata = PreparedMetadata {
        title: metadata.title,
        subtitle: metadata.subtitle,
        authors,
        editors,
        web_url: metadata.web_url,
        identifiers: metadata.identifiers,
        published,
        languages: metadata.languages,
        number_of_pages: metadata.number_of_pages,
        short_abstract: metadata.short_abstract,
        long_abstract: metadata.long_abstract,
        keywords: metadata.keywords,
        ddc: metadata.ddc,
        license,
        series: metadata.series,
        volume: metadata.volume,
        edition: metadata.edition,
        publisher: metadata.publisher,
        custom_fields: metadata.custom_fields,
    };

    Ok(PreparedProject {
        metadata: prepared_metadata,
        settings: project_data.settings,
        sections: data,
    })
}

/// Renders formatted citation strings for all bibliography entries in a project according to the current CSL style and language settings.
///
/// This function iterates over all bibliography entries from the provided `project` data,
/// collecting them into a Hayagriva library. It determines the correct CSL style and locale from the
/// project's settings or falls back to default values from `csl_data` as necessary. For each entry,
/// it then generates a formatted citation string using the determined CSL style and locale, writing
/// the output as HTML.
///
/// The resulting citations are returned as a `HashMap` mapping each citation key (as a `String`)
/// to its rendered HTML representation (as a `String`).
///
/// # Parameters
/// - `project`: Reference to the loaded project data, which includes bibliography entries and citation style settings.
/// - `csl_data`: Shared reference to the loaded CSL style and locale data used for formatting citations.
///
/// # Returns
/// A `HashMap` containing for each citation key its rendered citation as a HTML `String`.
///
/// # Warnings
/// - If a CSL style specified in the project settings cannot be found, a fallback style is used and a warning is logged.
/// - If any citations exist without a matching entry, a warning is logged for them.
///
/// # Panics
/// - Panics if no CSL style is available in `csl_data`.
pub fn render_citations(project: &ProjectData, csl_data: Arc<CslData>) -> HashMap<String, String> {
    //TODO: remove unused citation entrys to avoid bibliography entries with no citations
    let mut driver: BibliographyDriver<hayagriva::Entry> = BibliographyDriver::new();
    let mut res = HashMap::new();

    let mut keys = Vec::new();
    let mut library = hayagriva::Library::new();
    for (key, _) in project.bibliography.entries.iter() {
        if let Some(entry) = project.bibliography.get_entry_as_hayagriva(key) {
            library.push(&entry);
            keys.push(key.to_string());
        }
    }

    let mut items = Vec::new();
    for entry in library.iter() {
        items.push(CitationItem::with_entry(entry));
    }

    let style = match &project.settings {
        None => {
            csl_data
                .styles
                .iter()
                .next()
                .expect("No CSL styles found")
                .1
        }
        Some(settings) => match &settings.csl_style {
            None => {
                csl_data
                    .styles
                    .iter()
                    .next()
                    .expect("No CSL styles found")
                    .1
            }
            Some(style) => match csl_data.styles.get(style) {
                None => {
                    eprintln!(
                        "Couldn't find CSL style with id {}, using first csl style",
                        style
                    );
                    csl_data
                        .styles
                        .iter()
                        .next()
                        .expect("No CSL styles found")
                        .1
                }
                Some(style) => style,
            },
        },
    };

    for entry in items {
        driver.citation(CitationRequest::from_items(
            vec![entry],
            style,
            csl_data.locales.as_slice(),
        ));
    }

    let csl_locale = match &project.settings {
        Some(settings) => match &settings.csl_language_code {
            Some(str) => LocaleCode(str.clone()),
            None => LocaleCode("en-us".to_string()),
        },
        None => LocaleCode("en-us".to_string()),
    };

    let result = driver.finish(BibliographyRequest {
        style,
        locale: Some(csl_locale),
        locale_files: csl_data.locales.as_slice(),
    });

    for (i, citation) in result.citations.iter().enumerate() {
        if let Some(key) = keys.get(i) {
            // Render citation explicitly as HTML using Hayagriva's writer
            let mut content = String::new();
            if let Err(e) = citation
                .citation
                .write_buf(&mut content, hayagriva::BufWriteFormat::Html)
            {
                eprintln!("Failed to render citation as HTML for key {}: {}", key, e);
            }
            res.insert(key.clone(), content);
        } else {
            eprintln!(
                "Citation with index {} has no corresponding bibliography entry",
                i
            );
        }
    }
    res
}


/// Renders a `Section` into a `PreparedSection`, resolving metadata, author/editor references, and formatting content.
///
/// This function processes a section and all of its subsections, recursively rendering their content blocks, metadata, and endnotes,
/// and optionally adds soft hyphens for better text breaking based on language hyphenation dictionaries.
///
/// # Arguments
/// * `section` - The section to render.
/// * `data_storage` - Shared access to the `DataStorage`, used for resolving author/editor UUIDs.
/// * `citation_bib` - A map from citation keys to their corresponding bibliography data.
/// * `project_id` - The UUID identifying the project this section belongs to.
/// * `add_soft_hyphens` - If true, adds soft hyphens to title and subtitle based on the detected language.
///
/// # Returns
/// Returns a `PreparedSection` with all processed information, content blocks, metadata, authors/editors with resolved names, and endnotes.
#[async_recursion]
pub async fn render_section(
    section: Section,
    data_storage: Arc<DataStorage>,
    citation_bib: &HashMap<String, String>,
    project_id: &uuid::Uuid,
    add_soft_hyphens: bool,
) -> PreparedSection {
    let published = match section.metadata.published {
        Some(date) => Some(date.into()),
        None => None,
    };

    let mut authors = vec![];
    for author in section.metadata.authors {
        let person = match author {
            PersonUuidOrString::PersonUuid(id) => match data_storage.get_person_cloned(&id) {
                Some(person) => PersonOrString::Person(person),
                None => {
                    eprintln!(
                        "Author with id {} not found while rendering section for export!",
                        id
                    );
                    continue;
                }
            },
            PersonUuidOrString::NameString(name) => PersonOrString::NameString(name),
        };

        authors.push(person);
    }

    let mut editors = vec![];
    for editor in section.metadata.editors {
        let person = match editor {
            PersonUuidOrString::PersonUuid(id) => match data_storage.get_person_cloned(&id) {
                Some(person) => PersonOrString::Person(person),
                None => {
                    eprintln!(
                        "Editor with id {} not found while rendering section for export!",
                        id
                    );
                    continue;
                }
            },
            PersonUuidOrString::NameString(name) => PersonOrString::NameString(name),
        };

        editors.push(person);
    }

    // Load hyphenation dictionary for the language
    let dict = match &section.metadata.lang {
        Some(lang) => get_hyphenation_dict(lang)
            .unwrap_or_else(|| Standard::from_embedded(hyphenation::Language::EnglishUS).unwrap()),
        None => Standard::from_embedded(hyphenation::Language::EnglishUS).unwrap(),
    };

    let subtitle = match section.metadata.subtitle {
        Some(subtitle) => {
            if add_soft_hyphens {
                Some(hyphenate_text(subtitle.clone(), &dict))
            } else {
                Some(subtitle)
            }
        }
        None => None,
    };

    let title = if add_soft_hyphens {
        hyphenate_text(section.metadata.title.clone(), &dict)
    } else {
        section.metadata.title.clone()
    };

    let metadata = PreparedSectionMetadata {
        title,
        toc_title_subtitle_override: section.metadata.toc_title_subtitle_override,
        subtitle,
        authors,
        editors,
        web_url: section.metadata.web_url,
        identifiers: section.metadata.identifiers,
        published,
        lang: section.metadata.lang,
    };

    let mut content = vec![];

    // Store all endnote contents for this section. They will be rendered at the end of the section based on their order in the storage
    let mut endnote_storage: Vec<(uuid::Uuid, String)> = vec![];

    let blocks = decode_yjs_content(&section.content).unwrap_or_default();

    for content_block in blocks {
        content.push(
            render_content_block(
                content_block,
                &mut endnote_storage,
                &dict,
                &citation_bib,
                project_id,
                add_soft_hyphens,
            )
            .await,
        );
    }

    let mut sub_sections = vec![];
    for sub_section in section.sub_sections {
        sub_sections.push(
            render_section(
                sub_section,
                data_storage.clone(),
                &citation_bib,
                project_id,
                add_soft_hyphens,
            )
            .await,
        );
    }

    let mut endnotes = vec![];
    for i in 0..endnote_storage.len() {
        let end = endnote_storage.get(i).unwrap();
        endnotes.push(PreparedEndnote {
            num: i + 1,
            id: end.0,
            content: unescape_html(&end.1.clone()),
        });
    }

    PreparedSection {
        id: section.id.unwrap_or_default(),
        sub_sections,
        children: content,
        metadata,
        visible_in_toc: section.visible_in_toc,
        endnotes,
    }
}

/// Returns the hyphenation dictionary for the given language, if available.
///
/// The function attempts to map the provided `Language` enum variant to its corresponding
/// `hyphenation::Language` variant. If a supported mapping is found, it loads and returns
/// the hyphenation dictionary using `Standard::from_embedded`.  
/// If the language is not supported, `None` is returned.
///
/// # Arguments
/// * `language` - A reference to a `Language` enum variant indicating the language for which
///   the hyphenation dictionary should be retrieved.
///
/// # Returns
/// An `Option<Standard>` instance containing the hyphenation dictionary if available,
/// or `None` if the language has no hyphenation support.
///
fn get_hyphenation_dict(language: &Language) -> Option<Standard> {
    let lang = match language {
        Language::AfZa => Some(hyphenation::Language::Afrikaans),
        Language::SqAl => Some(hyphenation::Language::Albanian),
        Language::HyAm => Some(hyphenation::Language::Armenian),
        Language::AsIn => Some(hyphenation::Language::Assamese),
        Language::EuEs => Some(hyphenation::Language::Basque),
        Language::BeBy => Some(hyphenation::Language::Belarusian),
        Language::BnBd => Some(hyphenation::Language::Bengali),
        Language::BnIn => Some(hyphenation::Language::Bengali),
        Language::CaEs => Some(hyphenation::Language::Catalan),
        Language::CaEsValencia => Some(hyphenation::Language::Catalan),
        Language::ZhCn => Some(hyphenation::Language::Chinese),
        Language::ZhHk => Some(hyphenation::Language::Chinese),
        Language::ZhMo => Some(hyphenation::Language::Chinese),
        Language::ZhSg => Some(hyphenation::Language::Chinese),
        Language::ZhTw => Some(hyphenation::Language::Chinese),
        Language::HrBa => Some(hyphenation::Language::Croatian),
        Language::HrHr => Some(hyphenation::Language::Croatian),
        Language::CsCz => Some(hyphenation::Language::Czech),
        Language::DaDk => Some(hyphenation::Language::Danish),
        Language::NlNl => Some(hyphenation::Language::Dutch),
        Language::NlBe => Some(hyphenation::Language::Dutch),
        Language::EnGb => Some(hyphenation::Language::EnglishGB),
        Language::EnUs => Some(hyphenation::Language::EnglishUS),
        Language::EtEe => Some(hyphenation::Language::Estonian),
        Language::FiFi => Some(hyphenation::Language::Finnish),
        Language::Fr029
        | Language::FrBe
        | Language::FrCa
        | Language::FrCd
        | Language::FrCh
        | Language::FrCi
        | Language::FrCm
        | Language::FrFr
        | Language::FrHt
        | Language::FrLu
        | Language::FrMa
        | Language::FrMc
        | Language::FrMl
        | Language::FrRe
        | Language::FrSn => Some(hyphenation::Language::French),
        Language::GlEs => Some(hyphenation::Language::Galician),
        Language::KaGe => Some(hyphenation::Language::Georgian),
        Language::DeDe | Language::DeAt | Language::DeLi | Language::DeLu => {
            Some(hyphenation::Language::German1996)
        }
        Language::DeCh => Some(hyphenation::Language::GermanSwiss),
        Language::GuIn => Some(hyphenation::Language::Gujarati),
        Language::HiIn => Some(hyphenation::Language::Hindi),
        Language::HuHu => Some(hyphenation::Language::Hungarian),
        Language::IsIs => Some(hyphenation::Language::Icelandic),
        Language::IdId => Some(hyphenation::Language::Indonesian),
        Language::GaIe => Some(hyphenation::Language::Irish),
        Language::ItCh | Language::ItIt => Some(hyphenation::Language::Italian),
        Language::KnIn => Some(hyphenation::Language::Kannada),
        Language::LaVa => Some(hyphenation::Language::Latin),
        Language::LvLv => Some(hyphenation::Language::Latvian),
        Language::LtLt => Some(hyphenation::Language::Lithuanian),
        Language::MkMk => Some(hyphenation::Language::Macedonian),
        Language::MlIn => Some(hyphenation::Language::Malayalam),
        Language::MrIn => Some(hyphenation::Language::Marathi),
        Language::MnMn | Language::MnMongMn => Some(hyphenation::Language::Mongolian),
        Language::NbNo => Some(hyphenation::Language::NorwegianBokmal),
        Language::NnNo => Some(hyphenation::Language::NorwegianNynorsk),
        Language::OcFr => Some(hyphenation::Language::Occitan),
        Language::OrIn => Some(hyphenation::Language::Oriya),
        Language::PaIn | Language::PaArabPk => Some(hyphenation::Language::Panjabi),
        Language::PlPl => Some(hyphenation::Language::Polish),
        Language::PtPt | Language::PtBr => Some(hyphenation::Language::Portuguese),
        Language::RoMd | Language::RoRo => Some(hyphenation::Language::Romanian),
        Language::RmCh => Some(hyphenation::Language::Romansh),
        Language::RuMd | Language::RuRu => Some(hyphenation::Language::Russian),
        Language::SaIn => Some(hyphenation::Language::Sanskrit),
        Language::SrCyrlBa | Language::SrCyrlMe | Language::SrCyrlRs => {
            Some(hyphenation::Language::SerbianCyrillic)
        }
        Language::SkSk => Some(hyphenation::Language::Slovak),
        Language::SlSi => Some(hyphenation::Language::Slovenian),
        Language::Es419
        | Language::EsAr
        | Language::EsBo
        | Language::EsCl
        | Language::EsCo
        | Language::EsCr
        | Language::EsCu
        | Language::EsDo
        | Language::EsEc
        | Language::EsEs
        | Language::EsGt
        | Language::EsHn
        | Language::EsMx
        | Language::EsNi
        | Language::EsPa
        | Language::EsPe
        | Language::EsPr
        | Language::EsPy
        | Language::EsSv
        | Language::EsUs
        | Language::EsUy
        | Language::EsVe => Some(hyphenation::Language::Spanish),
        Language::SvFi | Language::SvSe => Some(hyphenation::Language::Swedish),
        Language::TaIn | Language::TaLk => Some(hyphenation::Language::Tamil),
        Language::TeIn => Some(hyphenation::Language::Telugu),
        Language::ThTh => Some(hyphenation::Language::Thai),
        Language::TrTr => Some(hyphenation::Language::Turkish),
        Language::TkTm => Some(hyphenation::Language::Turkmen),
        Language::UkUa => Some(hyphenation::Language::Ukrainian),
        Language::HsbDe => Some(hyphenation::Language::Uppersorbian),
        Language::CyGb => Some(hyphenation::Language::Welsh),
        _ => None,
    };
    match lang {
        Some(lang) => Some(Standard::from_embedded(lang).unwrap()),
        None => None,
    }
}

/// Hyphenates each word in the provided text using the given hyphenation dictionary,
/// inserting soft hyphens (`\u{00ad}`) at appropriate positions.
///
/// Words that appear to be part of HTML tags or contain certain special characters
/// (`<`, `>`, `=`, `&`) are excluded from hyphenation and copied verbatim to the output.
/// The function preserves whitespace between words.
///
/// # Arguments
/// * `text` - The input text as a String, containing words to hyphenate.
/// * `dict` - A reference to a hyphenation dictionary implementing `hyphenation::Standard`.
///
/// # Returns
/// A new String with hyphenation applied, containing soft hyphens in the hyphenated positions
/// for eligible words.
pub fn hyphenate_text(text: String, dict: &hyphenation::Standard) -> String {
    let mut res = String::new();
    let mut word_iter = text.split_whitespace().peekable();
    while let Some(word) = word_iter.next() {
        if word.starts_with("class=\"")
            || word.contains("<")
            || word.contains(">")
            || word.contains("=")
            || word.contains("&")
        {
            res.push_str(&format!("{} ", word));
            continue;
        }
        let hyphenated = dict.hyphenate(word);

        let mut word_res = String::new();
        let mut iter = hyphenated.into_iter().segments().peekable();
        while let Some(segment) = iter.next() {
            word_res.push_str(&segment);
            if iter.peek().is_some() {
                word_res.push('\u{00ad}');
            }
        }

        res.push_str(&word_res);
        if word_iter.peek().is_some() {
            res.push(' ');
        }
    }
    res
}

/// Renders a content block into an HTML string wrapped within a `PreparedContentBlock`.
///
/// Accepts the content block structure, a mutable storage for endnotes,
/// a dictionary for text processing, citation mapping, the associated project ID,
/// and a flag to optionally add soft hyphens during rendering.
///
/// The rendering logic varies depending on the `BlockData` variant of the input block:
/// - For paragraphs, outputs formatted text with optional soft hyphens and processes endnotes and citations.
/// - For headings, wraps rendered text in the appropriate heading tag and level.
/// - For raw HTML, includes the provided HTML string as-is.
/// - For lists, renders the items as an ordered or unordered list, processing content as plain text.
/// - For quotes, applies alignment and processes both the quoted text and the caption.
/// - For images, asynchronously loads the image from disk, encodes it as base64, and embeds it as a data URI; on failure, emits no output and logs the error.
///
/// The function incorporates optional CSS classes for each element where applicable.
/// If image loading or processing fails, an error is logged to standard error and an empty string is rendered for that block.
///
/// # Arguments
/// * `block` - The content block to be rendered.
/// * `endnote_storage` - A mutable vector for endnote references, used in paragraph and text blocks.
/// * `dict` - Dictionary used for text rendering and possible hyphenation.
/// * `citation_bib` - Mapping for citations occurring in the content.
/// * `project_id` - The UUID of the project, needed to locate uploaded image files.
/// * `add_soft_hyphens` - If true, optionally insert soft hyphens in rendered text for hyphenation if vivliostyle is used (weasyprint supports hyphenation out of the box).
///
/// Returns a `PreparedContentBlock` containing the HTML string and associated metadata.
pub async fn render_content_block(
    block: NewContentBlock,
    endnote_storage: &mut Vec<(uuid::Uuid, String)>,
    dict: &Standard,
    citation_bib: &HashMap<String, String>,
    project_id: &uuid::Uuid,
    add_soft_hyphens: bool,
) -> PreparedContentBlock {
    let css_classes_raw = block.css_classes.join(" ");
    let css_classes = if block.css_classes.len() > 0 {
        format!(" class='{}'", block.css_classes.join(" "))
    } else {
        String::new()
    };
    let data: String = match block.data {
        BlockData::Paragraph { text } => {
            format!(
                "<p id='{}' {}>{}</p>",
                block.id,
                css_classes,
                render_text(text, endnote_storage, dict, citation_bib, add_soft_hyphens)
            )
        }
        BlockData::Heading { text, level } => {
            format!(
                "<h{} id='{}' {}>{}</h{}>",
                level,
                block.id,
                css_classes,
                render_text(text, endnote_storage, dict, citation_bib, add_soft_hyphens),
                level
            )
        }
        BlockData::Raw { html } => html,
        BlockData::List { style, items } => {
            let mut res = String::new();
            for item in items {
                res.push_str(&format!(
                    "<li id='{}'>{}</li>",
                    block.id,
                    render_text(item, endnote_storage, dict, citation_bib, add_soft_hyphens)
                ));
            }
            if style == "ordered" {
                format!("<ol id='{}' {}>{}</ol>", block.id, css_classes, res)
            } else {
                format!("<ul id='{}' {}>{}</ul>", block.id, css_classes, res)
            }
        }
        BlockData::Quote {
            text,
            caption,
            alignment,
        } => {
            format!("<blockquote id='{}' class=\"align-{} {}\"><p>{}</p><footer>{}</footer></blockquote>", block.id, alignment, css_classes_raw, render_text(text, endnote_storage, dict, citation_bib, add_soft_hyphens), render_text(caption, endnote_storage, dict, citation_bib, add_soft_hyphens))
        }
        BlockData::Image {
            file,
            caption,
            with_border: _,
            with_background: _,
            stretched: _,
        } => {
            // Load image and convert to base64
            let file = tokio::fs::read(PathBuf::from(format!(
                "data/projects/{}/uploads/{}",
                project_id, file.filename
            )))
            .await;
            match file {
                Ok(file) => {
                    let img = image::load_from_memory(file.as_slice());
                    match img {
                        Ok(img) => match image_to_base64(&img) {
                            Some(str) => format!(
                                "<img id='{}' src=\"{}\" alt=\"{}\" {}/>",
                                block.id,
                                str,
                                caption.unwrap_or_default(),
                                css_classes
                            ),
                            None => String::new(),
                        },
                        Err(e) => {
                            eprintln!("Couldn't load image: {}", e);
                            String::new()
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Couldn't load included file: {}", e);
                    String::new()
                }
            }
        }
    };
    PreparedContentBlock {
        id: block.id,
        block_type: block.block_type,
        html: data,
    }
}

/// Converts a given `DynamicImage` to a PNG image and returns its data as a base64-encoded string with a data URL prefix.
///
/// The returned string is prefixed with `"data:image/png;base64,"` to be used directly as a data URL, suitable for embedding in HTML or other contexts that accept base64-encoded images.
///
/// # Arguments
///
/// * `img` - Reference to a `DynamicImage` that will be converted to base64.
///
/// # Returns
///
/// * Some with data:image/png;base64,+base64-value if successful
/// * None if image couldn't be converted to png
fn image_to_base64(img: &DynamicImage) -> Option<String> {
    let mut image_data: Vec<u8> = Vec::new();
    if let Err(e) = img.write_to(&mut Cursor::new(&mut image_data), ImageOutputFormat::Png) {
        eprintln!("Couldn't convert image to png: {}", e);
        return None;
    }

    let res_base64 = general_purpose::STANDARD.encode(image_data);
    Some(format!("data:image/png;base64,{}", res_base64))
}

/// Renders a text string into HTML, handling citations, footnotes, endnotes, and optional soft hyphens.
///
/// This function performs the following steps on the input text:
/// 1. Converts inline citations (in the form `<citation data-key="...">C</citation>`) into endnote placeholders,
///    replacing them with endnote HTML spans. If the citation key is not found in `citation_bib`, a warning is printed
///    and a placeholder is inserted.
/// 2. Processes elements marked as footnotes or endnotes by replacing `<span>` tags with corresponding HTML for footnotes
///    or endnotes. Endnotes are appended to `endnote_storage` with a generated UUID and their content.
/// 3. Transforms custom inline-style and class tags (`<customstyle>`) into `<span>` elements, including inline CSS and classes.
/// 4. After all transformations, the function optionally performs soft hyphenation on the final HTML text if
///    `add_soft_hyphens` is set to true, using the given hyphenation `dict`.
///
/// # Arguments
/// * `text` - The input text, potentially containing special markers for citations, footnotes, endnotes, and styles.
/// * `endnote_storage` - Mutable reference to a vector that will be populated with UUIDs and contents of endnotes processed from the text.
/// * `dict` - Hyphenation dictionary used for soft hyphenation if `add_soft_hyphens` is true.
/// * `citation_bib` - Bibliography mapping citation keys to citation strings, used for rendering citations found in the text.
/// * `add_soft_hyphens` - Flag indicating if soft hyphens should be conditionally inserted for hyphenation in the output.
/// * `citations_as_footnote` - Flag indicating if citations should be rendered as foot- or endnote
///
/// # Returns
/// A string containing the rendered HTML representation of the input text, with citations, footnotes, endnotes,
/// and custom styles processed.
///
/// # Panics
/// Does not
/// # Side Effects
/// - Appends processed endnotes to `endnote_storage` with generated UUIDs.
/// - Prints warnings to stderr if a citation key is not found.
pub fn render_text(text: String, endnote_storage: &mut Vec<(uuid::Uuid, String)>, dict: &Standard, citation_bib: &HashMap<String, String>, add_soft_hyphens: bool, citations_as_footnote: bool) -> String{
    if let Ok(dom) = parse_document(RcDom::default(), ParseOpts::default()).from_utf8().read_from(&mut text.as_bytes()) {
        let mut mutations: Vec<ReplacementType> = Vec::new();

        find_replacements(&dom.document, &mut mutations, endnote_storage, &citation_bib, citations_as_footnote);

        for mutation in mutations {
            match mutation {
                ReplacementType::Footnote(footnote) => {
                    let span_node = dom.create_element(
                        QualName::new(None, ns!(html), local_name!("span")),
                        vec![
                            Attribute {
                                name: QualName::new(None, ns!(), local_name!("class")),
                                value: StrTendril::from("footnote"),
                            },
                            Attribute {
                                name: QualName::new(None, ns!(), local_name!("id")),
                                value: StrTendril::from(format!("footnote-{}", footnote.uuid)),
                            }],
                        Default::default()
                    );
                    let child_a_node = dom.create_element(
                        QualName::new(None, ns!(html), local_name!("a")),
                        vec![
                            Attribute {
                                name: QualName::new(None, ns!(), local_name!("class")),
                                value: StrTendril::from("footnote-marker"),
                            },
                            Attribute {
                                name: QualName::new(None, ns!(), local_name!("href")),
                                value: StrTendril::from(format!("#footnote-call-{}", footnote.uuid)),
                            }],
                        Default::default()
                    );
                    let a_node = dom.create_element(
                        QualName::new(None, ns!(html), local_name!("a")),
                        vec![
                            Attribute {
                                name: QualName::new(None, ns!(), local_name!("class")),
                                value: StrTendril::from("footnote-call"),
                            },
                            Attribute {
                                name: QualName::new(None, ns!(), local_name!("href")),
                                value: StrTendril::from(format!("#footnote-{}", footnote.uuid)),
                            },
                            Attribute {
                                name: QualName::new(None, ns!(), local_name!("id")),
                                value: StrTendril::from(format!("footnote-call-{}", footnote.uuid)),
                            }],
                        Default::default()
                    );

                    let context = QualName::new(
                        None,
                        ns!(html),
                        local_name!("div"),
                    );

                    dom.append_before_sibling(&footnote.node, NodeOrText::AppendNode(a_node.clone()));
                    dom.append_before_sibling(&a_node, NodeOrText::AppendNode(span_node.clone()));
                    dom.append(&span_node, NodeOrText::AppendNode(child_a_node.clone()));

                    if let Ok(fragment) = parse_fragment(RcDom::default(), ParseOpts::default(), context, Vec::<Attribute>::new(), false).from_utf8().read_from(&mut footnote.note_content.as_bytes()) {
                        if let Some(reparent_handle) = fragment.document.children.borrow().get(0){
                            fragment.reparent_children(&reparent_handle, &span_node);
                        }
                    }

                    dom.remove_from_parent(&footnote.node)
                },
                ReplacementType::Endnote(endnote) => {
                    let sup_node = dom.create_element(
                        QualName::new(None, ns!(html), local_name!("sup")),
                        vec![
                            Attribute {
                                name: QualName::new(None, ns!(), local_name!("class")),
                                value: StrTendril::from("endnote"),
                            }],
                        Default::default()
                    );
                    let child_a_node = dom.create_element(
                        QualName::new(None, ns!(html), local_name!("a")),
                        vec![
                            Attribute {
                                name: QualName::new(None, ns!(), local_name!("href")),
                                value: StrTendril::from(format!("#note-{}", endnote.uuid)),
                            }],
                        Default::default()
                    );
                    let endnote_number = StrTendril::from(format!("{}", endnote.endnote_key));
                    dom.append_before_sibling(&endnote.node, NodeOrText::AppendNode(sup_node.clone()));
                    dom.append(&sup_node, NodeOrText::AppendNode(child_a_node.clone()));
                    dom.append(&child_a_node, NodeOrText::AppendText(endnote_number));
                    dom.remove_from_parent(&endnote.node)
                },
                ReplacementType::CustomStyle(custom_style) => {
                    let span_node = dom.create_element(
                        QualName::new(None, ns!(html), local_name!("span")),
                        vec![
                            Attribute {
                                name: QualName::new(None, ns!(), local_name!("class")),
                                value: StrTendril::from(format!("{}", custom_style.classes)),
                            },
                            Attribute {
                                name: QualName::new(None, ns!(), local_name!("style")),
                                value: StrTendril::from(format!("{}", custom_style.inline_style)),
                            }],
                        Default::default()
                    );
                    dom.append_before_sibling(&custom_style.node, NodeOrText::AppendNode(span_node.clone()));
                    dom.reparent_children(&custom_style.node, &span_node);
                    dom.remove_from_parent(&custom_style.node)
                },
            }
        }

        let html = serialize_dom(dom);
        return match html {
            Ok(html) => {
                if add_soft_hyphens {
                    hyphenate_text(html, dict)
                } else {
                    html
                }
            },
            Err(error) => "".to_string(),
        }
    }
    "".to_string()
}



fn find_replacements(node: &Handle, mutations: & mut Vec<ReplacementType>, endnote_storage: &mut Vec<(uuid::Uuid, String)>, citation_bib: &HashMap<String, String>, citations_as_footnote: bool) {
    if let NodeData::Element { ref name, ref attrs, .. } = node.data {
        let node_name = name.local.to_string();
        let mut attributes: HashMap<String, String> = HashMap::new();
        for attribute in attrs.borrow().iter() {
            attributes.insert(attribute.name.local.to_string(), attribute.value.to_string());
        }

        if node_name == "span" && attributes.contains_key(&"note-type".to_string()){

            if attributes.get("note-type") == Some(&"footnote".to_string()) {
                let uuid = uuid::Uuid::new_v4();
                let note_content = match attributes.get("note-content") {
                    Some(value) => value,
                    None => &"Missing note content".to_string()
                };

                let replacement = ReplacementType::Footnote(Footnote{
                    node: node.clone(),
                    uuid,
                    note_content: note_content.clone()
                });
                mutations.push(replacement);

            }else if attributes.get("note-type") == Some(&"endnote".to_string()) {
                let uuid = uuid::Uuid::new_v4();
                let note_content = match attributes.get("note-content") {
                    Some(value) => escape_html(value),
                    None => "Missing note content".to_string()
                };
                endnote_storage.push((uuid, note_content));
                let replacement = ReplacementType::Endnote(Endnote{
                    node: node.clone(),
                    uuid,
                    endnote_key: endnote_storage.len()
                });
                mutations.push(replacement);
            }
        }

        if node_name == "citation" {
            let key =  match attributes.get("data-key") {
                Some(value) => value.to_string(),
                None => "".to_string()
            };
            let uuid = uuid::Uuid::new_v4();
            let citation = match citation_bib.get(&key) {
                Some(citation) => escape_html(citation),
                None => {
                    eprintln!("Citation with key {} not found", key);
                    String::from("!!INVALID CITATION!!")
                }
            };

            if citations_as_footnote{
                let replacement = ReplacementType::Footnote(Footnote{
                    node: node.clone(),
                    uuid,
                    note_content: citation
                });
                mutations.push(replacement);
            }else {
                endnote_storage.push((uuid, citation));
                let replacement = ReplacementType::Endnote(Endnote{
                    node: node.clone(),
                    uuid,
                    endnote_key: endnote_storage.len()
                });
                mutations.push(replacement);
            }

        }

        if node_name == "customstyle" {
            let inline_style = match attributes.get("inline-style") {
                Some(value) => value.to_string(),
                None => "".to_string()
            };
            let classes = match attributes.get("classes") {
                Some(value) => value.to_string(),
                None => "".to_string()
            };

            let replacement = ReplacementType::CustomStyle(CustomStyle{
                node: node.clone(),
                classes: classes.clone(),
                inline_style: inline_style.clone(),
            });

            mutations.push(replacement);
        }
    }

    for child in node.children.borrow().iter() {
        find_replacements(child, mutations, endnote_storage, citation_bib, citations_as_footnote);
    }
}

fn serialize_dom(dom: RcDom) -> Result<String, String> {
    let document: SerializableHandle = dom.document.clone().into();
    let mut buffer = Vec::new();
    serialize(& mut buffer, &document, Default::default()).expect("serialization failed");
    let string_result = String::from_utf8(buffer);
    match string_result {
        Ok(result) => Ok(result.replace("<html><head></head><body>", "").replace("</body></html>", "")),
        Err(_) => Err(String::from("could not serialize document")),
    }
}

struct Footnote {
    node: Handle,
    uuid: uuid::Uuid,
    note_content: String
}
struct Endnote {
    node: Handle,
    uuid: uuid::Uuid,
    endnote_key: usize
}

struct CustomStyle {
    node: Handle,
    classes: String,
    inline_style: String,
}

enum ReplacementType {
    Footnote(Footnote),
    Endnote(Endnote),
    CustomStyle(CustomStyle)
}

/// Escapes special HTML characters in the input text by replacing:
/// - '&' with '&amp;'
/// - '<' with '&lt;'
/// - '>' with '&gt;'
/// - '"' with '&quot;'
///
/// # Arguments
/// * `text` - The input string that should be escaped.
///
/// # Returns
/// A new `String` with special HTML characters replaced by their respective HTML entities.
fn escape_html(text: &str) -> String{
    text.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;").replace("\"", "&quot;")
}

/// Replaces common HTML escape sequences in the input string with their respective characters.
///
/// Specifically, this function converts:
/// - `&amp;` to `&`
/// - `&lt;` to `<`
/// - `&gt;` to `>`
/// - `&quot;` to `"`
///
/// Returns a new `String` with the replacements applied.
fn unescape_html(text: &str) -> String {
    text.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_citation_prefix_suffix_extraction() {
        let html = String::from(
            r#"<p>Before <citation data-key="5136e861-cb95-47d6-8236-a892aee6850c" data-prefix="TESTprefix" data-suffix="TESTsuffix">C</citation> After</p>"#,
        );

        // Prepare minimal inputs
        let mut endnotes: Vec<(uuid::Uuid, String)> = Vec::new();
        let dict = Standard::from_embedded(hyphenation::Language::EnglishUS).unwrap();
        let mut citation_bib = HashMap::new();
        citation_bib.insert(
            String::from("5136e861-cb95-47d6-8236-a892aee6850c"),
            String::from("RenderedCitation"),
        );

        let _rendered = render_text(html, &mut endnotes, &dict, &citation_bib, false);

        assert_eq!(endnotes.len(), 1, "Exactly one endnote should be created");
        let (_id, content) = &endnotes[0];
        // The content is alphanumeric only, so escaped/unescaped are the same
        assert_eq!(content, "TESTprefixRenderedCitationTESTsuffix");
    }
}

/// Recursively adds any authors and editors from the given `section`
/// and its sub-sections to the provided `authors` and `editors` lists,
/// ensuring that each author and editor is only added once.
///
/// # Arguments
/// * `section` - The section whose authors and editors are to be added.
/// * `authors` - The list to which unique authors will be appended.
/// * `editors` - The list to which unique editors will be appended.
fn add_remaining_authors_editors_from_section(
    section: &PreparedSection,
    authors: &mut Vec<PersonOrString>,
    editors: &mut Vec<PersonOrString>,
) {
    for author in section.metadata.authors.iter() {
        if !authors.contains(author) {
            authors.push(author.clone());
        }
    }
    for editor in section.metadata.editors.iter() {
        if !editors.contains(editor) {
            editors.push(editor.clone());
        }
    }
    for sub_section in section.sub_sections.iter() {
        add_remaining_authors_editors_from_section(sub_section, authors, editors);
    }
}

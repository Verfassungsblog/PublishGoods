use hayagriva::{BufWriteFormat, CitationItem, CitationRequest};
use vb_exchange::projects::{Language, Person, PreparedContentBlock, PreparedEndnote, PreparedLanguage, PreparedMetadata, PreparedSection, PreparedSectionMetadata};
use vb_exchange::projects::PreparedLicense;
use std::collections::HashMap;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Arc;
use async_recursion::async_recursion;
use base64::Engine;
use base64::engine::general_purpose;
use hayagriva::{BibliographyDriver, BibliographyRequest};
use hayagriva::citationberg::LocaleCode;
use hyphenation::{Hyphenator, Load, Standard};
use image::{DynamicImage, ImageOutputFormat};
use regex::Regex;
use vb_exchange::projects::PreparedProject;
use vb_exchange::RenderingError;
use crate::data_storage::{DataStorage, ProjectDataV6};
use crate::projects::{BlockData, NewContentBlock, SectionV3, SectionOrTocV3};
use crate::utils::csl::CslData;

pub async fn prepare_project(project_data: ProjectDataV6, data_storage: Arc<DataStorage>, csl_data: Arc<CslData>, sections_to_include: Option<Vec<uuid::Uuid>>, project_id: &uuid::Uuid) -> Result<PreparedProject, RenderingError>{
    let citation_bib = render_citations(&project_data, csl_data);

    let metadata = match project_data.metadata{
        Some(metadata) => metadata,
        None => return Err(RenderingError::ProjectMetadataMissing)
    };
    
    let add_soft_hyphens = match &project_data.settings{
        Some(settings) => {
            settings.add_soft_hyphens
        }
        None => false
    };

    let mut authors = vec![];
    for author in metadata.authors.unwrap_or_default(){
        let person = match data_storage.get_person(&author){
            Some(person) => person.read().unwrap().clone(),
            None => {
                eprintln!("Author with id {} not found while rendering section for export!", author);
                continue
            }
        };
        authors.push(person);
    }

    let mut editors = vec![];
    for editor in metadata.editors.unwrap_or_default(){
        let person = match data_storage.get_person(&editor){
            Some(person) => person.read().unwrap().clone(),
            None => {
                eprintln!("Editor with id {} not found while rendering section for export!", editor);
                continue
            }
        };
        editors.push(person);
    }

    let license = if let Some(license) = metadata.license{
        Some(PreparedLicense::from(license))
    }else{
        None
    };

    let mut data = vec![];
    for section in project_data.sections{
        if let SectionOrTocV3::Section(section) = section{
            if let Some(id) = section.id{
                // Check if only specified sections should be included
                match &sections_to_include{
                    Some(sections_to_include) => { // Only prepare specified sections
                        if sections_to_include.contains(&id){
                            data.push(render_section(section, data_storage.clone(), &citation_bib, project_id, add_soft_hyphens).await)
                        }
                    },
                    None => data.push(render_section(section, data_storage.clone(), &citation_bib, project_id, add_soft_hyphens).await) // Prepare all sections
                }

            }
        }
    }

    for section in data.iter() {
        add_remaining_authors_editors_from_section(section,&mut authors, &mut editors);
    }

    // Sort authors and editors by last name
    authors.sort_by(|a, b| a.last_names.cmp(&b.last_names));
    editors.sort_by(|a, b| a.last_names.cmp(&b.last_names));

    let published = match metadata.published{
        Some(date) => Some(date.into()),
        None => None
    };

    let metadata = PreparedMetadata{
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
    };

    Ok(PreparedProject{
        metadata,
        settings: project_data.settings,
        sections: data,
    })
}

pub fn render_citations(project: &ProjectDataV6, csl_data: Arc<CslData>) -> HashMap<String, String>{
    //TODO: remove unused citation entrys to avoid bibliography entries with no citations
    let mut driver: BibliographyDriver<hayagriva::Entry> = BibliographyDriver::new();
    let mut res = HashMap::new();

    let mut bib = hayagriva::Library::new();
    for (_, entry) in project.bibliography.iter() {
        let entry: hayagriva::Entry = entry.clone().into();
        bib.push(&entry);
    }

    let mut items = Vec::new();
    for entry in bib.iter(){
        let cit_entry = CitationItem::with_entry(entry);
        items.push(cit_entry);
    }

    let style = match &project.settings{
        None => {
            csl_data.styles.iter().next().expect("No CSL styles found").1
        }
        Some(settings) => {
            match &settings.csl_style{
                None => {
                    csl_data.styles.iter().next().expect("No CSL styles found").1
                }
                Some(style) => {
                    match csl_data.styles.get(style){
                        None => {
                            eprintln!("Couldn't find CSL style with id {}, using first csl style", style);
                            csl_data.styles.iter().next().expect("No CSL styles found").1
                        }
                        Some(style) => {
                            style
                        }
                    }
                }
            }
        }
    };

    for entry in items{
        driver.citation(CitationRequest::from_items(vec![entry], style, csl_data.locales.as_slice()));
    }

    let csl_locale = match &project.settings.clone(){
        Some(settings) => {
            match &settings.csl_language_code {
                Some(str) => LocaleCode(str.clone()),
                None => LocaleCode("en-us".to_string()),
            }
        },
        None => {
            LocaleCode("en-us".to_string())
        }
    };

    let result = driver.finish(BibliographyRequest{
        style,
        locale: Some(csl_locale),
        locale_files: &csl_data.locales.as_slice(),
    });
    for (i, citation) in result.citations.iter().enumerate(){
        match project.bibliography.iter().nth(i){
            Some((key, _)) => {
                let mut content = String::new();
                citation.citation.write_buf(&mut content, BufWriteFormat::Html).unwrap();
                res.insert(key.to_string(),content);
            }
            None => {
                eprintln!("Citation with index {} has no corresponding bibliography entry", i);
            }
        }
    }
    res
}

#[async_recursion]
pub async fn render_section(section: SectionV3, data_storage: Arc<DataStorage>, citation_bib: &HashMap<String, String>, project_id: &uuid::Uuid, add_soft_hyphens: bool) -> PreparedSection{
    let published = match section.metadata.published{
        Some(date) => Some(date.into()),
        None => None
    };

    let mut authors = vec![];
    for author in section.metadata.authors{
        let person = match data_storage.get_person(&author){
            Some(person) => person.read().unwrap().clone(),
            None => {
                eprintln!("Author with id {} not found while rendering section for export!", author);
                continue
            }
        };
        authors.push(person);
    }

    let mut editors = vec![];
    for editor in section.metadata.editors{
        let person = match data_storage.get_person(&editor){
            Some(person) => person.read().unwrap().clone(),
            None => {
                eprintln!("Editor with id {} not found while rendering section for export!", editor);
                continue
            }
        };
        editors.push(person);
    }

    // Load hyphenation dictionary for the language
    let dict = match &section.metadata.lang{
        Some(lang) => {
            match lang{
                Language::DE => Standard::from_embedded(hyphenation::Language::German1996).unwrap(),
                Language::EN => Standard::from_embedded(hyphenation::Language::EnglishGB).unwrap()
            }
        }
        None => Standard::from_embedded(hyphenation::Language::EnglishGB).unwrap()
    };

    let lang = match section.metadata.lang{
        Some(lang) => {
            match lang{
                Language::DE => PreparedLanguage{de: true, en: false},
                Language::EN => PreparedLanguage{de: false, en: true}
            }
        }
        None => PreparedLanguage{de: false, en: false}
    };

    let subtitle = match section.metadata.subtitle{
        Some(subtitle) => {
            if add_soft_hyphens{
                Some(hyphenate_text(subtitle.clone(), &dict))
            }else {
                Some(subtitle)
            }
        },
        None => None
    };
    
    let title = if add_soft_hyphens{
        hyphenate_text(section.metadata.title.clone(), &dict)
    }else{
        section.metadata.title.clone()
    };

    let metadata = PreparedSectionMetadata{
        title,
        toc_title_override: section.metadata.toc_title_override,
        subtitle,
        toc_subtitle_override: section.metadata.toc_subtitle_override,
        authors,
        editors,
        web_url: section.metadata.web_url,
        identifiers: section.metadata.identifiers,
        published,
        lang,
    };

    let mut content = vec![];

    // Store all endnote contents for this section. They will be rendered at the end of the section based on their order in the storage
    let mut endnote_storage: Vec<(uuid::Uuid, String)> = vec![];

    for content_block in section.children{
        content.push(render_content_block(content_block, &mut endnote_storage, &dict, &citation_bib, project_id, add_soft_hyphens).await);
    }

    let mut sub_sections = vec![];
    for sub_section in section.sub_sections{
        sub_sections.push(render_section(sub_section, data_storage.clone(), &citation_bib, project_id, add_soft_hyphens).await);
    }

    let mut endnotes = vec![];
    for i in 0..endnote_storage.len(){
        let end = endnote_storage.get(i).unwrap();
        endnotes.push(PreparedEndnote{ num: i+1, id: end.0, content: unescape_html(&end.1.clone()) });
    }

    PreparedSection{
        id: section.id.unwrap_or_default(),
        sub_sections,
        children: content,
        metadata,
        visible_in_toc: section.visible_in_toc,
        endnotes
    }
}

pub fn hyphenate_text(text: String, dict: &hyphenation::Standard) -> String{
    let mut res = String::new();
    let mut word_iter = text.split_whitespace().peekable();
    while let Some(word) = word_iter.next(){
        if word.starts_with("class=\"") || word.contains("<") || word.contains(">") || word.contains("=") || word.contains("&"){
            res.push_str(&format!("{} ", word));
            continue
        }
        let hyphenated = dict.hyphenate(word);

        let mut word_res = String::new();
        let mut iter = hyphenated.into_iter().segments().peekable();
        while let Some(segment) = iter.next(){
            word_res.push_str(&segment);
            if iter.peek().is_some(){
                word_res.push('\u{00ad}');
            }
        }

        res.push_str(&word_res);
        if word_iter.peek().is_some(){
            res.push(' ');
        }
    }
    res
}


pub async fn render_content_block(block: NewContentBlock, endnote_storage: &mut Vec<(uuid::Uuid, String)>, dict: &Standard, citation_bib: &HashMap<String, String>, project_id: &uuid::Uuid, add_soft_hyphens: bool) -> PreparedContentBlock{
    let css_classes_raw = block.css_classes.join(" ");
    let css_classes = if block.css_classes.len() > 0{
        format!(" class='{}'", block.css_classes.join(" "))
    }else{
        String::new()
    };
    let data: String = match block.data{
        BlockData::Paragraph {text} => {
            format!("<p id='{}' {}>{}</p>", block.id, css_classes, render_text(text, endnote_storage, dict, citation_bib, add_soft_hyphens))
        }
        BlockData::Heading { text , level} => {
            format!("<h{} id='{}' {}>{}</h{}>", level, block.id, css_classes, render_text(text, endnote_storage, dict, citation_bib, add_soft_hyphens), level)
        }
        BlockData::Raw { html } => {
            html
        }
        BlockData::List { style, items} => {
            let mut res = String::new();
            for item in items{
                res.push_str(&format!("<li id='{}'>{}</li>", block.id, render_text(item, endnote_storage, dict, citation_bib, add_soft_hyphens)));
            }
            if style == "ordered"{
                format!("<ol id='{}' {}>{}</ol>", block.id, css_classes, res)
            }else{
                format!("<ul id='{}' {}>{}</ul>", block.id, css_classes, res)
            }
        },
        BlockData::Quote{text, caption, alignment} => {
            format!("<blockquote id='{}' class=\"align-{} {}\"><p>{}</p><footer>{}</footer></blockquote>", block.id, alignment, css_classes_raw, render_text(text, endnote_storage, dict, citation_bib, add_soft_hyphens), render_text(caption, endnote_storage, dict, citation_bib, add_soft_hyphens))
        }
        BlockData::Image {file, caption, with_border: _, with_background: _, stretched: _} => {
            // Load image and convert to base64
            let file = tokio::fs::read(PathBuf::from(format!("data/projects/{}/uploads/{}", project_id, file.filename))).await;
            match file{
                Ok(file) => {
                    let img = image::load_from_memory(file.as_slice());
                    match img{
                        Ok(img) => {
                            let img_as_base64 = image_to_base64(&img);
                            format!("<img id='{}' src=\"{}\" alt=\"{}\" {}/>", block.id, img_as_base64, caption.unwrap_or_default(), css_classes)
                        },
                        Err(e) => {
                            eprintln!("Couldn't load image: {}", e);
                            String::new()
                        }
                    }
                },
                Err(e) => {
                    eprintln!("Couldn't load included file: {}", e);
                    String::new()
                }
            }
        }
    };
    PreparedContentBlock{
        id: block.id,
        block_type: block.block_type,
        html: data
    }
}

fn image_to_base64(img: &DynamicImage) -> String {
    let mut image_data: Vec<u8> = Vec::new();
    img.write_to(&mut Cursor::new(&mut image_data), ImageOutputFormat::Png)
        .unwrap();
    let res_base64 = general_purpose::STANDARD.encode(image_data);
    format!("data:image/png;base64,{}", res_base64)
}


pub fn render_text(text: String, endnote_storage: &mut Vec<(uuid::Uuid, String)>, dict: &Standard, citation_bib: &HashMap<String, String>, add_soft_hyphens: bool) -> String{
    let re: Regex = Regex::new(r#"<span(?:[^>]*?\bnote-type="([^"]+)")?(?:[^>]*?\bnote-content="([^"]+)")?[^>]*>.*?</span>"#).unwrap(); //TODO: DO NOT RECOMPILE REGEX, it's bad for performance
    let re3 = Regex::new(r#"<citation data-key="([^"]*)">C</citation>"#).unwrap();

    // First Step: Convert Citations to Endnotes
    let res = re3.replace_all(&text, |caps: &regex::Captures| {
        let key = match caps.get(1){
            Some(key) => key.as_str(),
            None => return String::new()
        };

        // TODO: add setting if citations should be rendered as endnotes, in text or as footnotes
        match citation_bib.get(key){
            Some(citation) => {
                let test = format!("<span note-type=\"endnote\" note-content=\"{}\"></span>", escape_html(citation));
                println!("Citation got converted to: {}", test);
                test
            },
            None => {
                eprintln!("Citation with key {} not found", key);
                String::from("!!INVALID CITATION!!")
            }
        }
    });

    // Second Step: Convert Footnotes and Endnotes to HTML
    let binding = res.to_string();

    let res = re.replace_all(&binding, |caps: &regex::Captures| {
        let note_type = match caps.get(1){
            Some(note_type) => note_type.as_str(),
            None => return String::new()
        };
        let note_content = match caps.get(2){
            Some(note_content) => note_content.as_str(),
            None => return String::new()
        };

        if note_type == "endnote" {
            let uuid = uuid::Uuid::new_v4();
            endnote_storage.push((uuid, escape_html(note_content)));
            return format!("<sup class=\"endnote\"><a href=\"#note-{}\">{}</a></sup>", uuid, endnote_storage.len())
        }else if note_type == "footnote" {
            let uuid = uuid::Uuid::new_v4();
            return format!("<span class=\"footnote\" id=\"footnote-{}\"><a class=\"footnote-marker\" href=\"#footnote-call-{}\"></a>{}</span><a class=\"footnote-call\" href=\"#footnote-{}\" id=\"footnote-call-{}\"></a>", uuid, uuid,  note_content, uuid, uuid)
        }else{
            String::new()
        }
    });

    let re2 = Regex::new(r#"<customstyle(?:[^>]*?\binline-style="([^"]*?)")?(?:[^>]*?\bclasses="([^"]*?)")?[^>]*>(.*?)</customstyle>"#).unwrap();
    let binding = res.to_string();
    let res2 = re2.replace_all(&binding, |caps: &regex::Captures| {
        let inline_style = caps.get(1).map_or("", |m| m.as_str());
        let classes = caps.get(2).map_or("", |m| m.as_str());
        let content = caps.get(3).map_or("", |m| m.as_str());
        format!(r#"<span class="{}" style="{}">{}</span>"#, classes, inline_style, content)
    });
    let binding = res2.to_string();
    let res3 = re3.replace_all(&binding, |caps: &regex::Captures| {
        let key = match caps.get(1){
            Some(key) => key.as_str(),
            None => return String::new()
        };

        // TODO: add setting if citations should be rendered as endnotes, in text or as footnotes
        match citation_bib.get(key){
            Some(citation) => {
                let uuid = uuid::Uuid::new_v4();
                endnote_storage.push((uuid, citation.clone()));
                format!("<sup class=\"endnote\"><a href=\"#note-{}\">{}</a></sup>", uuid, endnote_storage.len())
            },
            None => {
                eprintln!("Citation with key {} not found", key);
                String::from("!!INVALID CITATION!!")
            }
        }
    });
    if(add_soft_hyphens){
        hyphenate_text(res3.to_string(), dict)
    }else{
        res3.to_string()
    }
}

fn escape_html(text: &str) -> String{
    text.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;").replace("\"", "&quot;")
}
fn unescape_html(text: &str) -> String{
    text.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"")
}

fn add_remaining_authors_editors_from_section(section: &PreparedSection, authors: &mut Vec<Person>, editors: &mut Vec<Person>){
    for author in section.metadata.authors.iter(){
        if !authors.contains(author){
            authors.push(author.clone());
        }
    }
    for editor in section.metadata.editors.iter(){
        if !editors.contains(editor){
            editors.push(editor.clone());
        }
    }
    for sub_section in section.sub_sections.iter(){
        add_remaining_authors_editors_from_section(sub_section, authors, editors);
    }
}
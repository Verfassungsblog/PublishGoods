use crate::projects::api::{UploadedImage, Patch};
use chrono::NaiveDateTime;
use bincode::{Encode, Decode};
use serde::{Serialize, Deserialize};
use vb_exchange::projects::*;

/// Enum to differentiate between real sections and the position of the table of contents
#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq)]
pub enum SectionOrToc{
    Section(Section),
    Toc,
}

impl SectionOrToc{
    pub fn into_section(self) -> Option<Section> {
        match self {
            SectionOrToc::Section(section) => Some(section),
            SectionOrToc::Toc => None,
        }
    }
}


/// Struct holds all project-level metadata
#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq, Default)]
pub struct ProjectMetadata{
    /// Book Title
    pub title: String,
    /// Subtitle of the book
    pub subtitle: Option<String>,
    /// List of ids of authors of the book
    #[bincode(with_serde)]
    pub authors: Option<Vec<uuid::Uuid>>,
    /// List of ids of editors of the book
    #[bincode(with_serde)]
    pub editors: Option<Vec<uuid::Uuid>>,
    /// URL to a web version of the book or reference
    pub web_url: Option<String>,
    /// List of identifiers of the book (e.g. ISBNs)
    // TODO: build identifier validator
    pub identifiers: Option<Vec<Identifier>>,
    /// Date of publication
    #[bincode(with_serde)]
    pub published: Option<NaiveDateTime>,
    /// Languages of the book
    pub languages: Option<Vec<Language>>,
    /// Number of pages of the book (should be automatically calculated)
    pub number_of_pages: Option<u32>,
    /// Short abstract of the book
    pub short_abstract: Option<String>,
    /// Long abstract of the book
    pub long_abstract: Option<String>,
    /// Keywords of the book
    pub keywords: Option<Vec<Keyword>>,
    /// Dewey Decimal Classification (DDC) classes (subject groups)
    pub ddc: Option<String>, //TODO: validate DDC on api set
    /// License of the book
    pub license: Option<License>,
    /// Series the book belongs to
    pub series: Option<String>,
    /// Volume of the book in the series
    pub volume: Option<String>,
    /// Edition of the book
    pub edition: Option<String>,
    /// Publisher of the book
    pub publisher: Option<String>,
}


/// Struct holds all data for a section (e.g. chapter, part, ...)
#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq)]
pub struct Section{
    /// Unique id of the section
    /// Only None if the section is not yet saved in the database
    #[bincode(with_serde)]
    pub id: Option<uuid::Uuid>,
    /// Additional classes to style the Section
    pub css_classes: Vec<String>,
    /// Holds all subsections
    pub sub_sections: Vec<Section>,
    // Holds all content blocks
    pub children: Vec<NewContentBlock>,
    /// If true, the section is visible in the table of contents
    pub visible_in_toc: bool,
    /// Metadata of the section
    pub metadata: SectionMetadata,
}

impl Section{
    pub fn clone_without_contentblocks(&self) -> Section {
        let mut new_section = self.clone();
        new_section.children = vec![];
        new_section
    }

    pub fn clone_without_subsections(&self) -> Section {
        let mut new_section = self.clone();
        new_section.sub_sections = vec![];
        new_section
    }

    pub fn insert_child_section_as_child(&mut self, parent_section_id: &uuid::Uuid, new_section: &Section) -> Option<()>{
        for section in self.sub_sections.iter_mut(){
                    if section.id == Some(*parent_section_id){
                        section.sub_sections.push(new_section.clone());
                        return Some(())
                    }else{
                        match section.insert_child_section_as_child(parent_section_id, new_section){
                            Some(_) => {
                                return Some(())
                            },
                            None => {},
                        }
                    }
        }
        None
    }

    pub fn insert_child_section_after(&mut self, section_id: &uuid::Uuid, new_section: &Section) -> Option<()>{
        for (i, section) in self.sub_sections.iter_mut().enumerate(){
                    if section.id == Some(*section_id){
                        self.sub_sections.insert(i+1,new_section.clone());
                        return Some(())
                    }else{
                        match section.insert_child_section_after(section_id, new_section){
                            Some(_) => {
                                return Some(())
                            },
                            None => {},
                        }
                    }
        }
        None
    }

    pub fn remove_child_section(&mut self, section_id: &uuid::Uuid) -> Option<Section>{
        let mut index = None;
        for (i, section) in self.sub_sections.iter_mut().enumerate(){
                    if section.id == Some(*section_id){
                        index = Some(i);
                    }else{
                        match section.remove_child_section(section_id){
                            Some(section) => {
                                return Some(section)
                            },
                            None => {},
                        }
            }
        }
        match index{
            Some(index) => {
                let section = self.sub_sections.remove(index);
                Some(section)
            },
            None => None,
        }
    }
}

impl Patch<PatchHeading, Heading> for Heading{
    fn patch(&mut self, patch: PatchHeading) -> Heading {
        let level = patch.level.unwrap_or_else(|| self.level);
        let contents = patch.contents.unwrap_or_else(|| self.contents.clone());
        Heading{
            level,
            contents,
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq)]
struct PatchHeading{
    pub level: Option<u8>,
    pub contents: Option<Vec<TextElement>>,
}

#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq)]
struct PatchList{
    pub items: Option<Vec<ListItem>>,
    pub list_type: Option<ListType>,
}

impl Patch<PatchList, List> for List{
    fn patch(&mut self, patch: PatchList) -> List {
        let items = patch.items.unwrap_or_else(|| self.items.clone());
        let list_type = patch.list_type.unwrap_or_else(|| self.list_type.clone());
        List{
            items,
            list_type,
        }
    }
}

/// Headline Content Block, contains the level and the contents
#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq)]
pub struct Heading {
    /// Level of the headline (e.g. h1, h2, ...)
    pub level: u8,
    /// Contents of the headline as TextElements
    pub contents: Vec<TextElement>
}

#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq)]
pub struct HorizontalRule{
}

#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq)]
pub enum ListType{
    Unordered,
    Ordered,
}

#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq)]
pub enum TextElementOrList{
    List(List),
    TextElement(TextElement),
}

#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq)]
pub struct ListItem{
    /// A list item can contain text elements and other (nested) lists
    pub contents: Vec<TextElementOrList>,
}

/// List
#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq)]
pub struct List{
    pub items: Vec<ListItem>,
    pub list_type: ListType,
}

/// Paragraph Content Block holding TextElements
#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq)]
pub struct Paragraph{
    /// Contents of the paragraph
    pub contents: Vec<TextElement>,
    /// Optional block-level alignment of the paragraph
    pub alignment: Option<Alignment>,
}

/// Alignment of a paragraph
#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq)]
pub enum Alignment {
    Left,
    Center,
    Right,
    Justify,
}

/// Enum to differentiate between different text elements
#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq)]
pub enum TextElement{
    /// Simple text
    String(String),
    /// Formatted text (e.g. bold, italic, ...) which can contain other text elements
    FormattedText(FormattedText),
    /// Weblink
    Link(Link),
    /// Footnote or Endnote
    Note(Note),
    /// Linebreak
    LineBreak(LineBreak)
}

/// Weblink to url with optional link text
///
/// If no link text is given, the url is used as link text
#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq)]
pub struct Link{
    pub url: String,
    pub text: Option<Vec<TextElement>>,
}

/// Footnote or Endnote
#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq)]
pub struct Note{
    /// Type of the note (footnote or endnote)
    pub note_type: NoteType,
    /// Contents of the note
    pub content: Vec<TextElement>,
}

#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq)]
pub struct LineBreak{
}

/// Enum to differentiate between footnote and endnote
#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq)]
pub enum NoteType{
    Footnote,
    Endnote,
}

/// Container to hold text elements and set the format of these text elements
///
/// You may capsule other FormattedText elements to create nested formatting
#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq)]
pub struct FormattedText{
    pub contents: Vec<TextElement>,
    pub format: TextFormat,
}

/// Enum to differentiate between different text formats
#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq)]
pub enum TextFormat{
    Bold,
    Italic,
    Underline,
    Strikethrough,
    Superscript,
    Subscript,
    None,
}

/// Enum to differentiate between different section levels
#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq)]
pub enum SectionLevel{
    Part,
    Chapter,
    Custom
}

/// Struct holds all metadata of a section
#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq)]
pub struct SectionMetadata{
    pub title: String,
    pub toc_title_override: Option<String>,
    pub subtitle: Option<String>,
    pub toc_subtitle_override: Option<String>,
    #[bincode(with_serde)]
    pub authors: Vec<uuid::Uuid>,
    #[bincode(with_serde)]
    pub editors: Vec<uuid::Uuid>,
    pub web_url: Option<String>,
    pub identifiers: Vec<Identifier>,
    #[bincode(with_serde)]
    pub published: Option<NaiveDateTime>,
    #[bincode(with_serde)]
    pub last_changed: Option<NaiveDateTime>,
    pub lang: Option<Language>,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct NewContentBlockEditorJSFormat{
    pub id: String,
    #[serde(rename = "type")]
    pub block_type: String,
    pub data: BlockDataEditorJSFormat,
    pub tunes: BlockTuneEditorJSFormat,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct BlockTuneEditorJSFormat{
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_style_tune: Option<BlockStyleTuneEditorJS>
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct BlockStyleTuneEditorJS{
    pub css_classes: String,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct BlockDataEditorJSFormat{
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub items: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub html: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alignment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<UploadedImage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub withBorder: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub withBackground: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stretched: Option<bool>,
}

#[derive(Deserialize, Serialize, Debug, Encode, Decode, Clone, PartialEq)]
pub struct Footnote{
    pub id: String,
    pub content: String,
    pub superscript: u16,
}

#[derive(Debug, Serialize, Deserialize, Encode, Decode, Clone, PartialEq)]
pub struct NewContentBlock{
    pub id: String,
    pub block_type: BlockType,
    pub data: BlockData,
    pub css_classes: Vec<String>,
    #[bincode(with_serde)]
    pub revision_id: Option<uuid::Uuid>,
}

impl TryFrom<NewContentBlockEditorJSFormat> for NewContentBlock{
    type Error = String;

    fn try_from(value: NewContentBlockEditorJSFormat) -> Result<Self, Self::Error> {
        let css_classes = match value.tunes.block_style_tune{
            Some(tune) => tune.css_classes.split(" ").map(|s| s.to_string()).collect(),
            None => vec![],
        };
        match value.block_type.as_str(){
            "paragraph" => {
               let text = value.data.text.ok_or("Missing field 'text' in paragraph block".to_string())?;
                Ok(NewContentBlock {
                     id: value.id,
                     block_type: BlockType::Paragraph,
                     data: BlockData::Paragraph { text },
                    css_classes,
                    revision_id: None,
                })
            },
            "header" => {
                let level = value.data.level.ok_or("Missing field 'level' in header block".to_string())?;
                let text = value.data.text.ok_or("Missing field 'text' in header block".to_string())?;

                Ok(NewContentBlock {
                    id: value.id,
                    block_type: BlockType::Heading,
                    data: BlockData::Heading { text, level },
                    css_classes,
                    revision_id: None,
                })
            },
            "raw" => {
                let html = value.data.html.ok_or("Missing field 'html' in raw block".to_string())?;

                Ok(NewContentBlock {
                    id: value.id,
                    block_type: BlockType::Heading,
                    data: BlockData::Raw {html},
                    css_classes,
                    revision_id: None,
                })
            },
            "list" => {
                let items = value.data.items.ok_or("Missing field 'items' in raw block".to_string())?;
                let style = value.data.style.ok_or("Missing field 'style' in raw block".to_string())?;
                Ok(NewContentBlock {
                    id: value.id,
                    block_type: BlockType::Heading,
                    data: BlockData::List {style, items},
                    css_classes,
                    revision_id: None,
                })
            },
            "quote" => {
                let text = value.data.text.ok_or("Missing field 'text' in quote block".to_string())?;
                let caption = value.data.caption.ok_or("Missing field 'caption' in quote block".to_string())?;
                let alignment = value.data.alignment.ok_or("Missing field 'alignment' in quote block".to_string())?;
                Ok(NewContentBlock {
                    id: value.id,
                    block_type: BlockType::Heading,
                    data: BlockData::Quote {text, caption, alignment},
                    css_classes,
                    revision_id: None,
                })
            },
            "image" => {
                let file = value.data.file.ok_or("Missing field 'file' in image block".to_string())?;
                let caption = value.data.caption;
                let with_border = value.data.withBorder.unwrap_or(false);
                let with_background = value.data.withBackground.unwrap_or(false);
                let stretched = value.data.stretched.unwrap_or(false);
                Ok(NewContentBlock {
                    id: value.id,
                    block_type: BlockType::Heading,
                    data: BlockData::Image {file, caption, with_border, with_background, stretched},
                    css_classes,
                    revision_id: None,
                })
            }
            _ => Err("Unknown block type".to_string()),
        }
    }
}

impl From<NewContentBlock> for NewContentBlockEditorJSFormat{

    fn from(value: NewContentBlock) -> Self {
        let mut tunes = BlockTuneEditorJSFormat{
            block_style_tune: None,
        };
        if value.css_classes.len() > 0{
            tunes.block_style_tune = Some(BlockStyleTuneEditorJS{
                css_classes: value.css_classes.join(" "),
            });
        }
        match value.data{
            BlockData::Paragraph { text } => {
                NewContentBlockEditorJSFormat {
                    id: value.id,
                    block_type: "paragraph".to_string(),
                    data: BlockDataEditorJSFormat {
                        text: Some(text),
                        level: None,
                        items: None,
                        html: None,
                        caption: None,
                        alignment: None,
                        style: None,
                        file: None,
                        withBorder: None,
                        withBackground: None,
                        stretched: None,
                    },
                    tunes,
                }
            },
            BlockData::Heading { text, level } => {
                NewContentBlockEditorJSFormat {
                    id: value.id,
                    block_type: "header".to_string(),
                    data: BlockDataEditorJSFormat {
                        text: Some(text),
                        level: Some(level),
                        items: None,
                        html: None,
                        caption: None,
                        alignment: None,
                        style: None,
                        file: None,
                        withBorder: None,
                        withBackground: None,
                        stretched: None,
                    },
                    tunes,
                }
            },
            BlockData::Raw {html} => {
                NewContentBlockEditorJSFormat {
                    id: value.id,
                    block_type: "raw".to_string(),
                    data: BlockDataEditorJSFormat {
                        text: None,
                        level: None,
                        items: None,
                        html: Some(html),
                        caption: None,
                        alignment: None,
                        style: None,
                        file: None,
                        withBorder: None,
                        withBackground: None,
                        stretched: None,
                    },
                    tunes,
                }
            },
            BlockData::List {items, style} => {
                NewContentBlockEditorJSFormat {
                    id: value.id,
                    block_type: "list".to_string(),
                    data: BlockDataEditorJSFormat {
                        text: None,
                        level: None,
                        items: Some(items),
                        html: None,
                        caption: None,
                        alignment: None,
                        style: Some(style),
                        file: None,
                        withBorder: None,
                        withBackground: None,
                        stretched: None,
                    },
                    tunes,
                }
            },
            BlockData::Quote {text, caption, alignment} => {
                NewContentBlockEditorJSFormat {
                    id: value.id,
                    block_type: "quote".to_string(),
                    data: BlockDataEditorJSFormat {
                        text: Some(text),
                        level: None,
                        items: None,
                        html: None,
                        caption: Some(caption),
                        alignment: Some(alignment),
                        style: None,
                        file: None,
                        withBorder: None,
                        withBackground: None,
                        stretched: None,
                    },
                    tunes,
                }
            },
            BlockData::Image {file, caption, with_border, with_background, stretched} => {
                NewContentBlockEditorJSFormat {
                    id: value.id,
                    block_type: "image".to_string(),
                    data: BlockDataEditorJSFormat {
                        text: None,
                        level: None,
                        items: None,
                        html: None,
                        caption,
                        alignment: None,
                        style: None,
                        file: Some(file),
                        withBorder: Some(with_border),
                        withBackground: Some(with_background),
                        stretched: Some(stretched),
                    },
                    tunes,
                }
            }
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Encode, Decode, Clone, PartialEq)]
pub enum BlockData{
    Paragraph{text: String},
    Heading{text: String, level: u8},
    Raw{html: String},
    List{style: String, items: Vec<String>},
    Quote{text: String, caption: String, alignment: String},
    Image{file: UploadedImage, caption: Option<String>, with_border: bool, with_background: bool, stretched: bool},
}

/// Test function to test the deserialization of a content block
#[test]
pub fn test_deserialize_and_serialize_content_block(){
    let json = r#"{
        "id": "123",
        "type": "header",
        "data": {
            "text": "Test",
            "level": 1
        },
        "tunes": {
            "block_style_tune": {
                "css_classes": "test"
            }
        }
    }"#;
    let content_block: NewContentBlockEditorJSFormat = serde_json::from_str(json).unwrap();
    let content_block: NewContentBlock = content_block.try_into().unwrap();
    let new: NewContentBlockEditorJSFormat = content_block.try_into().unwrap();
    assert_eq!(json.parse::<serde_json::Value>().unwrap(), serde_json::to_string(&new).unwrap().parse::<serde_json::Value>().unwrap());
}

pub mod create;
pub mod editor;
pub mod list;
pub mod api;
pub mod bibliography_editor;
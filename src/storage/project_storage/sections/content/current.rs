use crate::projects::api::UploadedImage;
use bincode::{Decode, Encode};
use rocket::serde::{Deserialize, Serialize};
use vb_exchange::projects::BlockType;

#[derive(Debug, Serialize, Deserialize, Encode, Decode, Clone, PartialEq)]
pub struct NewContentBlock {
    pub id: String,
    pub block_type: BlockType,
    pub data: BlockData,
    pub css_classes: Vec<String>,
    #[bincode(with_serde)]
    pub revision_id: Option<uuid::Uuid>,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct NewContentBlockEditorJSFormat {
    pub id: String,
    #[serde(rename = "type")]
    pub block_type: String,
    pub data: BlockDataEditorJSFormat,
    pub tunes: BlockTuneEditorJSFormat,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct BlockTuneEditorJSFormat {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_style_tune: Option<BlockStyleTuneEditorJS>,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct BlockStyleTuneEditorJS {
    pub css_classes: String,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct BlockDataEditorJSFormat {
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
impl TryFrom<NewContentBlockEditorJSFormat> for NewContentBlock {
    type Error = String;

    fn try_from(value: NewContentBlockEditorJSFormat) -> Result<Self, Self::Error> {
        let css_classes = match value.tunes.block_style_tune {
            Some(tune) => tune.css_classes.split(" ").map(|s| s.to_string()).collect(),
            None => vec![],
        };
        match value.block_type.as_str() {
            "paragraph" => {
                let text = value
                    .data
                    .text
                    .ok_or("Missing field 'text' in paragraph block".to_string())?;
                Ok(NewContentBlock {
                    id: value.id,
                    block_type: BlockType::Paragraph,
                    data: BlockData::Paragraph { text },
                    css_classes,
                    revision_id: None,
                })
            }
            "header" => {
                let level = value
                    .data
                    .level
                    .ok_or("Missing field 'level' in header block".to_string())?;
                let text = value
                    .data
                    .text
                    .ok_or("Missing field 'text' in header block".to_string())?;

                Ok(NewContentBlock {
                    id: value.id,
                    block_type: BlockType::Heading,
                    data: BlockData::Heading { text, level },
                    css_classes,
                    revision_id: None,
                })
            }
            "raw" => {
                let html = value
                    .data
                    .html
                    .ok_or("Missing field 'html' in raw block".to_string())?;

                Ok(NewContentBlock {
                    id: value.id,
                    block_type: BlockType::Heading,
                    data: BlockData::Raw { html },
                    css_classes,
                    revision_id: None,
                })
            }
            "list" => {
                let items = value
                    .data
                    .items
                    .ok_or("Missing field 'items' in raw block".to_string())?;
                let style = value
                    .data
                    .style
                    .ok_or("Missing field 'style' in raw block".to_string())?;
                Ok(NewContentBlock {
                    id: value.id,
                    block_type: BlockType::Heading,
                    data: BlockData::List { style, items },
                    css_classes,
                    revision_id: None,
                })
            }
            "quote" => {
                let text = value
                    .data
                    .text
                    .ok_or("Missing field 'text' in quote block".to_string())?;
                let caption = value
                    .data
                    .caption
                    .ok_or("Missing field 'caption' in quote block".to_string())?;
                let alignment = value
                    .data
                    .alignment
                    .ok_or("Missing field 'alignment' in quote block".to_string())?;
                Ok(NewContentBlock {
                    id: value.id,
                    block_type: BlockType::Heading,
                    data: BlockData::Quote {
                        text,
                        caption,
                        alignment,
                    },
                    css_classes,
                    revision_id: None,
                })
            }
            "image" => {
                let file = value
                    .data
                    .file
                    .ok_or("Missing field 'file' in image block".to_string())?;
                let caption = value.data.caption;
                let with_border = value.data.withBorder.unwrap_or(false);
                let with_background = value.data.withBackground.unwrap_or(false);
                let stretched = value.data.stretched.unwrap_or(false);
                Ok(NewContentBlock {
                    id: value.id,
                    block_type: BlockType::Heading,
                    data: BlockData::Image {
                        file,
                        caption,
                        with_border,
                        with_background,
                        stretched,
                    },
                    css_classes,
                    revision_id: None,
                })
            }
            _ => Err("Unknown block type".to_string()),
        }
    }
}

impl From<NewContentBlock> for NewContentBlockEditorJSFormat {
    fn from(value: NewContentBlock) -> Self {
        let mut tunes = BlockTuneEditorJSFormat {
            block_style_tune: None,
        };
        if value.css_classes.len() > 0 {
            tunes.block_style_tune = Some(BlockStyleTuneEditorJS {
                css_classes: value.css_classes.join(" "),
            });
        }
        match value.data {
            BlockData::Paragraph { text } => NewContentBlockEditorJSFormat {
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
            },
            BlockData::Heading { text, level } => NewContentBlockEditorJSFormat {
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
            },
            BlockData::Raw { html } => NewContentBlockEditorJSFormat {
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
            },
            BlockData::List { items, style } => NewContentBlockEditorJSFormat {
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
            },
            BlockData::Quote {
                text,
                caption,
                alignment,
            } => NewContentBlockEditorJSFormat {
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
            },
            BlockData::Image {
                file,
                caption,
                with_border,
                with_background,
                stretched,
            } => NewContentBlockEditorJSFormat {
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
            },
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Encode, Decode, Clone, PartialEq)]
pub enum BlockData {
    Paragraph {
        text: String,
    },
    Heading {
        text: String,
        level: u8,
    },
    Raw {
        html: String,
    },
    List {
        style: String,
        items: Vec<String>,
    },
    Quote {
        text: String,
        caption: String,
        alignment: String,
    },
    Image {
        file: UploadedImage,
        caption: Option<String>,
        with_border: bool,
        with_background: bool,
        stretched: bool,
    },
}

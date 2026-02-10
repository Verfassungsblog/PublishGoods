use crate::projects::api::UploadedImage;
use bincode::{Decode, Encode};
use rocket::serde::{Deserialize, Serialize};
use vb_exchange::projects::BlockType;
use yrs::types::array::Array;
use yrs::types::map::Map;
use yrs::updates::decoder::Decode as _;
use yrs::updates::encoder::Encode as _;
use yrs::{types::map::MapRef, Doc, GetString, MapPrelim, ReadTxn, Transact, Transaction};

pub struct MapRefWithTransaction<'a, 'b> {
    pub map_ref: MapRef,
    pub txn: &'a Transaction<'b>,
}

/// Decodes a yrs update into a Vec of NewContentBlock's
pub fn decode_yjs_content(content: &[u8]) -> Result<Vec<NewContentBlock>, String> {
    if content.is_empty() {
        return Ok(vec![]);
    }

    let doc = Doc::new();
    {
        let mut txn = doc.transact_mut();
        if let Ok(update) = yrs::Update::decode_v1(content) {
            if txn.apply_update(update).is_err() {
                return Err("Could not apply update to yrs doc".to_string());
            }
        } else {
            return Err("Could not decode yrs update".to_string());
        }
    }

    let blocks_array = doc.get_or_insert_array("blocks");
    let txn = doc.transact();

    let mut blocks = Vec::new();

    for block_val in blocks_array.iter(&txn) {
        if let Ok(block_map) = block_val.cast::<MapRef>() {
            let map_ref_with_txn = MapRefWithTransaction {
                map_ref: block_map,
                txn: &txn,
            };

            if let Ok(block) = NewContentBlock::try_from(map_ref_with_txn) {
                blocks.push(block);
            }
        }
    }

    Ok(blocks)
}

impl<'a, 'b> TryFrom<MapRefWithTransaction<'a, 'b>> for NewContentBlock {
    type Error = String;

    fn try_from(value: MapRefWithTransaction<'a, 'b>) -> Result<Self, Self::Error> {
        let id = value
            .map_ref
            .get(value.txn, "id")
            .ok_or("Missing field 'id'")?
            .to_string(value.txn);

        let block_type_str = value
            .map_ref
            .get(value.txn, "type")
            .ok_or("Missing field 'type'")?
            .to_string(value.txn);

        let data_ref = value
            .map_ref
            .get(value.txn, "data")
            .ok_or("Missing field 'data'")?
            .cast::<MapRef>()
            .map_err(|_| "Field 'data' is not a MapRef")?;

        let mut css_classes = Vec::new();
        if let Some(tunes_val) = value.map_ref.get(value.txn, "tunes") {
            if let Ok(tunes) = tunes_val.cast::<MapRef>() {
                if let Some(style_tunes_val) = tunes.get(value.txn, "block_style_tunes") {
                    if let Ok(style_tunes) = style_tunes_val.cast::<MapRef>() {
                        if let Some(css_classes_str) = style_tunes.get(value.txn, "css_classes") {
                            let classes = css_classes_str.to_string(value.txn);
                            if !classes.is_empty() {
                                css_classes = classes.split(" ").map(|s| s.to_string()).collect();
                            }
                        }
                    }
                }
            }
        }

        match block_type_str.as_str() {
            "paragraph" => {
                if let Some(text_val) = data_ref.get(value.txn, "text") {
                    Ok(NewContentBlock {
                        id,
                        block_type: BlockType::Paragraph,
                        data: BlockData::Paragraph {
                            text: text_val.to_string(value.txn),
                        },
                        css_classes,
                        revision_id: None,
                    })
                } else if let Some(html_val) = data_ref.get(value.txn, "html") {
                    Ok(NewContentBlock {
                        id,
                        block_type: BlockType::Raw,
                        data: BlockData::Raw {
                            html: html_val.to_string(value.txn),
                        },
                        css_classes,
                        revision_id: None,
                    })
                } else {
                    Err("Missing 'text' or 'html' in paragraph/raw block".to_string())
                }
            }
            "header" => {
                let text = data_ref
                    .get(value.txn, "text")
                    .ok_or("Missing field 'text' in header block")?
                    .to_string(value.txn);
                let level = match data_ref
                    .get(value.txn, "level")
                    .ok_or("Missing field 'level' in header block")?
                {
                    yrs::Out::Any(yrs::Any::Number(v)) => v as u8,
                    yrs::Out::Any(yrs::Any::BigInt(v)) => v as u8,
                    _ => return Err("Field 'level' is not an integer".to_string()),
                };

                Ok(NewContentBlock {
                    id,
                    block_type: BlockType::Heading,
                    data: BlockData::Heading { text, level },
                    css_classes,
                    revision_id: None,
                })
            }
            "list" => {
                let style = data_ref
                    .get(value.txn, "style")
                    .ok_or("Missing field 'style' in list block")?
                    .to_string(value.txn);
                let items_val = data_ref
                    .get(value.txn, "items")
                    .ok_or("Missing field 'items' in list block")?;
                let items_array = items_val
                    .cast::<yrs::types::array::ArrayRef>()
                    .map_err(|_| "Field 'items' is not an ArrayRef")?;

                let mut items = Vec::new();
                for item in items_array.iter(value.txn) {
                    items.push(item.to_string(value.txn));
                }

                Ok(NewContentBlock {
                    id,
                    block_type: BlockType::List,
                    data: BlockData::List { style, items },
                    css_classes,
                    revision_id: None,
                })
            }
            "quote" => {
                let text = data_ref
                    .get(value.txn, "text")
                    .ok_or("Missing field 'text' in quote block")?
                    .to_string(value.txn);
                let caption = data_ref
                    .get(value.txn, "caption")
                    .ok_or("Missing field 'caption' in quote block")?
                    .to_string(value.txn);
                let alignment = data_ref
                    .get(value.txn, "alignment")
                    .ok_or("Missing field 'alignment' in quote block")?
                    .to_string(value.txn);

                Ok(NewContentBlock {
                    id,
                    block_type: BlockType::Quote,
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
                let file_ref = data_ref
                    .get(value.txn, "file")
                    .ok_or("Missing field 'file' in image block")?
                    .cast::<MapRef>()
                    .map_err(|_| "Field 'file' is not a MapRef")?;

                let url = file_ref
                    .get(value.txn, "url")
                    .ok_or("Missing field 'url' in image file")?
                    .to_string(value.txn);
                let filename = file_ref
                    .get(value.txn, "filename")
                    .ok_or("Missing field 'filename' in image file")?
                    .to_string(value.txn);

                let file = UploadedImage { url, filename };

                let caption = data_ref
                    .get(value.txn, "caption")
                    .map(|v| v.to_string(value.txn));
                let with_border = match data_ref.get(value.txn, "withBorder") {
                    Some(yrs::Out::Any(yrs::Any::Bool(v))) => v,
                    _ => false,
                };
                let with_background = match data_ref.get(value.txn, "withBackground") {
                    Some(yrs::Out::Any(yrs::Any::Bool(v))) => v,
                    _ => false,
                };
                let stretched = match data_ref.get(value.txn, "stretched") {
                    Some(yrs::Out::Any(yrs::Any::Bool(v))) => v,
                    _ => false,
                };

                Ok(NewContentBlock {
                    id,
                    block_type: BlockType::Image,
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
            _ => Err(format!("Unknown block type: {}", block_type_str)),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Encode, Decode, Clone, PartialEq)]
pub struct NewContentBlock {
    pub id: String,
    pub block_type: BlockType,
    pub data: BlockData,
    pub css_classes: Vec<String>,
    #[bincode(with_serde)]
    pub revision_id: Option<uuid::Uuid>,
}

impl From<NewContentBlock> for yrs::MapPrelim {
    fn from(value: NewContentBlock) -> Self {
        let mut map: Vec<(String, yrs::In)> = Vec::new();

        map.push(("id".to_string(), value.id.into()));
        match value.data {
            BlockData::Paragraph { text } => {
                map.push(("type".to_string(), "paragraph".into()));
                let data: Vec<(String, yrs::In)> =
                    vec![("text".to_string(), yrs::TextPrelim::new(text).into())];
                map.push(("data".to_string(), MapPrelim::from_iter(data).into()));
            }
            BlockData::Heading { text, level } => {
                map.push(("type".to_string(), "header".into()));
                let data: Vec<(String, yrs::In)> = vec![
                    ("text".to_string(), yrs::TextPrelim::new(text).into()),
                    ("level".to_string(), (level as i16).into()),
                ];
                map.push(("data".to_string(), MapPrelim::from_iter(data).into()));
            }
            BlockData::Raw { html } => {
                map.push(("type".to_string(), "paragraph".into()));
                let data: Vec<(String, yrs::In)> =
                    vec![("html".to_string(), yrs::TextPrelim::new(html).into())];
                map.push(("data".to_string(), MapPrelim::from_iter(data).into()));
            }
            BlockData::List { style, items } => {
                map.push(("type".to_string(), "list".into()));
                let items_prelim: Vec<yrs::In> = items
                    .into_iter()
                    .map(|item| yrs::TextPrelim::new(item).into())
                    .collect();
                let data: Vec<(String, yrs::In)> = vec![
                    ("style".to_string(), style.into()),
                    (
                        "items".to_string(),
                        yrs::ArrayPrelim::from_iter(items_prelim).into(),
                    ),
                ];
                map.push(("data".to_string(), MapPrelim::from_iter(data).into()));
            }
            BlockData::Quote {
                text,
                caption,
                alignment,
            } => {
                map.push(("type".to_string(), "quote".into()));
                let data: Vec<(String, yrs::In)> = vec![
                    ("text".to_string(), yrs::TextPrelim::new(text).into()),
                    ("caption".to_string(), yrs::TextPrelim::new(caption).into()),
                    ("alignment".to_string(), alignment.into()),
                ];
                map.push(("data".to_string(), MapPrelim::from_iter(data).into()));
            }
            BlockData::Image {
                file,
                caption,
                with_border,
                with_background,
                stretched,
            } => {
                map.push(("type".to_string(), "image".into()));
                let file_map: Vec<(String, yrs::In)> = vec![
                    ("url".to_string(), file.url.into()),
                    ("filename".to_string(), file.filename.into()),
                ];
                let mut data: Vec<(String, yrs::In)> = vec![
                    ("file".to_string(), MapPrelim::from_iter(file_map).into()),
                    ("withBorder".to_string(), with_border.into()),
                    ("withBackground".to_string(), with_background.into()),
                    ("stretched".to_string(), stretched.into()),
                ];
                if let Some(caption) = caption {
                    data.push(("caption".to_string(), yrs::TextPrelim::new(caption).into()));
                }
                map.push(("data".to_string(), MapPrelim::from_iter(data).into()));
            }
        }
        if !value.css_classes.is_empty() {
            let block_style_tunes: Vec<(String, yrs::In)> = vec![(
                "css_classes".to_string(),
                value.css_classes.join(" ").into(),
            )];
            let tunes_data: Vec<(String, yrs::In)> = vec![(
                "block_style_tunes".to_string(),
                MapPrelim::from_iter(block_style_tunes).into(),
            )];
            map.push(("tunes".to_string(), MapPrelim::from_iter(tunes_data).into()));
        }

        MapPrelim::from_iter(map)
    }
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
                    block_type: BlockType::Raw,
                    data: BlockData::Raw { html },
                    css_classes,
                    revision_id: None,
                })
            }
            "list" => {
                let items = value
                    .data
                    .items
                    .ok_or("Missing field 'items' in list block".to_string())?;
                let style = value
                    .data
                    .style
                    .ok_or("Missing field 'style' in list block".to_string())?;
                Ok(NewContentBlock {
                    id: value.id,
                    block_type: BlockType::List,
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
                    block_type: BlockType::Quote,
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
                    block_type: BlockType::Image,
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

#[cfg(test)]
mod tests {
    use super::*;
    use yrs::{Array, Doc, Map, Transact};

    fn setup_y_map(doc: &Doc, block: NewContentBlock) -> yrs::types::map::MapRef {
        let map = doc.get_or_insert_map("test");
        let mut txn = doc.transact_mut();
        let prelim: MapPrelim = block.into();
        map.insert(&mut txn, "block", prelim)
    }

    #[test]
    fn test_paragraph_conversion() {
        let block = NewContentBlock {
            id: uuid::Uuid::new_v4().to_string(),
            block_type: BlockType::Paragraph,
            data: BlockData::Paragraph {
                text: "Hello world".to_string(),
            },
            css_classes: vec![],
            revision_id: None,
        };

        let doc = Doc::new();
        let map_ref = setup_y_map(&doc, block.clone());
        let txn = doc.transact();

        assert_eq!(map_ref.get(&txn, "id").unwrap().to_string(&txn), block.id);
        assert_eq!(
            map_ref.get(&txn, "type").unwrap().to_string(&txn),
            "paragraph"
        );

        let data = map_ref
            .get(&txn, "data")
            .unwrap()
            .cast::<yrs::types::map::MapRef>()
            .unwrap();
        assert_eq!(
            data.get(&txn, "text").unwrap().to_string(&txn),
            "Hello world"
        );
    }

    #[test]
    fn test_heading_conversion() {
        let block = NewContentBlock {
            id: uuid::Uuid::new_v4().to_string(),
            block_type: BlockType::Heading,
            data: BlockData::Heading {
                text: "Heading".to_string(),
                level: 2,
            },
            css_classes: vec![],
            revision_id: None,
        };

        let doc = Doc::new();
        let map_ref = setup_y_map(&doc, block.clone());
        let txn = doc.transact();

        assert_eq!(map_ref.get(&txn, "id").unwrap().to_string(&txn), block.id);
        assert_eq!(map_ref.get(&txn, "type").unwrap().to_string(&txn), "header");
        let data = map_ref
            .get(&txn, "data")
            .unwrap()
            .cast::<yrs::types::map::MapRef>()
            .unwrap();
        assert_eq!(data.get(&txn, "text").unwrap().to_string(&txn), "Heading");
        assert_eq!(data.get(&txn, "level").unwrap().to_string(&txn), "2");
    }

    #[test]
    fn test_list_conversion() {
        let block = NewContentBlock {
            id: uuid::Uuid::new_v4().to_string(),
            block_type: BlockType::Heading,
            data: BlockData::List {
                style: "ordered".to_string(),
                items: vec!["item1".to_string(), "item2".to_string()],
            },
            css_classes: vec![],
            revision_id: None,
        };

        let doc = Doc::new();
        let map_ref = setup_y_map(&doc, block.clone());
        let txn = doc.transact();

        assert_eq!(map_ref.get(&txn, "id").unwrap().to_string(&txn), block.id);
        assert_eq!(map_ref.get(&txn, "type").unwrap().to_string(&txn), "list");
        let data = map_ref
            .get(&txn, "data")
            .unwrap()
            .cast::<yrs::types::map::MapRef>()
            .unwrap();
        assert_eq!(data.get(&txn, "style").unwrap().to_string(&txn), "ordered");

        let items = data
            .get(&txn, "items")
            .unwrap()
            .cast::<yrs::types::array::ArrayRef>()
            .unwrap();
        assert_eq!(items.len(&txn), 2);
        assert_eq!(items.get(&txn, 0).unwrap().to_string(&txn), "item1");
        assert_eq!(items.get(&txn, 1).unwrap().to_string(&txn), "item2");
    }

    #[test]
    fn test_quote_conversion() {
        let block = NewContentBlock {
            id: uuid::Uuid::new_v4().to_string(),
            block_type: BlockType::Heading,
            data: BlockData::Quote {
                text: "To be or not to be".to_string(),
                caption: "Shakespeare".to_string(),
                alignment: "center".to_string(),
            },
            css_classes: vec![],
            revision_id: None,
        };

        let doc = Doc::new();
        let map_ref = setup_y_map(&doc, block.clone());
        let txn = doc.transact();

        assert_eq!(map_ref.get(&txn, "id").unwrap().to_string(&txn), block.id);
        assert_eq!(map_ref.get(&txn, "type").unwrap().to_string(&txn), "quote");
        let data = map_ref
            .get(&txn, "data")
            .unwrap()
            .cast::<yrs::types::map::MapRef>()
            .unwrap();
        assert_eq!(
            data.get(&txn, "text").unwrap().to_string(&txn),
            "To be or not to be"
        );
        assert_eq!(
            data.get(&txn, "caption").unwrap().to_string(&txn),
            "Shakespeare"
        );
        assert_eq!(
            data.get(&txn, "alignment").unwrap().to_string(&txn),
            "center"
        );
    }

    #[test]
    fn test_image_conversion() {
        let block = NewContentBlock {
            id: uuid::Uuid::new_v4().to_string(),
            block_type: BlockType::Heading,
            data: BlockData::Image {
                file: UploadedImage {
                    url: "url".to_string(),
                    filename: "file.png".to_string(),
                },
                caption: Some("Caption".to_string()),
                with_border: true,
                with_background: false,
                stretched: true,
            },
            css_classes: vec![],
            revision_id: None,
        };

        let doc = Doc::new();
        let map_ref = setup_y_map(&doc, block.clone());
        let txn = doc.transact();

        assert_eq!(map_ref.get(&txn, "id").unwrap().to_string(&txn), block.id);
        assert_eq!(map_ref.get(&txn, "type").unwrap().to_string(&txn), "image");
        let data = map_ref
            .get(&txn, "data")
            .unwrap()
            .cast::<yrs::types::map::MapRef>()
            .unwrap();

        let file = data
            .get(&txn, "file")
            .unwrap()
            .cast::<yrs::types::map::MapRef>()
            .unwrap();
        assert_eq!(file.get(&txn, "url").unwrap().to_string(&txn), "url");
        assert_eq!(
            file.get(&txn, "filename").unwrap().to_string(&txn),
            "file.png"
        );

        assert_eq!(
            data.get(&txn, "caption").unwrap().to_string(&txn),
            "Caption"
        );
        assert_eq!(
            data.get(&txn, "withBorder").unwrap().to_string(&txn),
            "true"
        );
        assert_eq!(
            data.get(&txn, "withBackground").unwrap().to_string(&txn),
            "false"
        );
        assert_eq!(data.get(&txn, "stretched").unwrap().to_string(&txn), "true");
    }

    #[test]
    fn test_css_classes_tunes() {
        let block = NewContentBlock {
            id: uuid::Uuid::new_v4().to_string(),
            block_type: BlockType::Paragraph,
            data: BlockData::Paragraph {
                text: "Text".to_string(),
            },
            css_classes: vec!["class1".to_string(), "class2".to_string()],
            revision_id: None,
        };

        let doc = Doc::new();
        let map_ref = setup_y_map(&doc, block.clone());
        let txn = doc.transact();

        assert_eq!(map_ref.get(&txn, "id").unwrap().to_string(&txn), block.id);
        let tunes = map_ref
            .get(&txn, "tunes")
            .unwrap()
            .cast::<yrs::types::map::MapRef>()
            .unwrap();
        let style_tunes = tunes
            .get(&txn, "block_style_tunes")
            .unwrap()
            .cast::<yrs::types::map::MapRef>()
            .unwrap();
        assert_eq!(
            style_tunes
                .get(&txn, "css_classes")
                .unwrap()
                .to_string(&txn),
            "class1 class2"
        );
    }

    #[test]
    fn test_raw_conversion() {
        let block = NewContentBlock {
            id: uuid::Uuid::new_v4().to_string(),
            block_type: BlockType::Paragraph,
            data: BlockData::Raw {
                html: "<div>Raw</div>".to_string(),
            },
            css_classes: vec![],
            revision_id: None,
        };

        let doc = Doc::new();
        let map_ref = setup_y_map(&doc, block.clone());
        let txn = doc.transact();

        assert_eq!(map_ref.get(&txn, "id").unwrap().to_string(&txn), block.id);
        assert_eq!(
            map_ref.get(&txn, "type").unwrap().to_string(&txn),
            "paragraph"
        );
        let data = map_ref
            .get(&txn, "data")
            .unwrap()
            .cast::<yrs::types::map::MapRef>()
            .unwrap();
        assert_eq!(
            data.get(&txn, "html").unwrap().to_string(&txn),
            "<div>Raw</div>"
        );
    }

    #[test]
    fn test_map_ref_conversion() {
        use yrs::Doc;
        let block = NewContentBlock {
            id: uuid::Uuid::new_v4().to_string(),
            block_type: BlockType::Heading,
            data: BlockData::Heading {
                text: "Heading".to_string(),
                level: 2,
            },
            css_classes: vec!["class1".to_string()],
            revision_id: None,
        };

        let doc = Doc::new();
        let map_ref = setup_y_map(&doc, block.clone());
        let txn = doc.transact();

        let map_ref_with_txn = MapRefWithTransaction {
            map_ref: map_ref.clone(),
            txn: &txn,
        };

        let converted_block = NewContentBlock::try_from(map_ref_with_txn).unwrap();
        assert_eq!(block, converted_block);
    }

    #[test]
    fn test_map_ref_conversion_paragraph() {
        use yrs::Doc;
        let block = NewContentBlock {
            id: uuid::Uuid::new_v4().to_string(),
            block_type: BlockType::Paragraph,
            data: BlockData::Paragraph {
                text: "Paragraph".to_string(),
            },
            css_classes: vec![],
            revision_id: None,
        };

        let doc = Doc::new();
        let map_ref = setup_y_map(&doc, block.clone());
        let txn = doc.transact();

        let map_ref_with_txn = MapRefWithTransaction {
            map_ref: map_ref.clone(),
            txn: &txn,
        };

        let converted_block = NewContentBlock::try_from(map_ref_with_txn).unwrap();
        assert_eq!(block, converted_block);
    }

    #[test]
    fn test_map_ref_conversion_list() {
        use yrs::Doc;
        let block = NewContentBlock {
            id: uuid::Uuid::new_v4().to_string(),
            block_type: BlockType::List,
            data: BlockData::List {
                style: "ordered".to_string(),
                items: vec!["item1".to_string(), "item2".to_string()],
            },
            css_classes: vec![],
            revision_id: None,
        };

        let doc = Doc::new();
        let map_ref = setup_y_map(&doc, block.clone());
        let txn = doc.transact();

        let map_ref_with_txn = MapRefWithTransaction {
            map_ref: map_ref.clone(),
            txn: &txn,
        };

        let converted_block = NewContentBlock::try_from(map_ref_with_txn).unwrap();
        assert_eq!(block, converted_block);
    }

    #[test]
    fn test_map_ref_conversion_quote() {
        use yrs::Doc;
        let block = NewContentBlock {
            id: uuid::Uuid::new_v4().to_string(),
            block_type: BlockType::Quote,
            data: BlockData::Quote {
                text: "To be or not to be".to_string(),
                caption: "Shakespeare".to_string(),
                alignment: "center".to_string(),
            },
            css_classes: vec![],
            revision_id: None,
        };
        let doc = Doc::new();
        let map_ref = setup_y_map(&doc, block.clone());
        let txn = doc.transact();
        let map_ref_with_txn = MapRefWithTransaction { map_ref, txn: &txn };
        let converted_block = NewContentBlock::try_from(map_ref_with_txn).unwrap();
        assert_eq!(block, converted_block);
    }

    #[test]
    fn test_map_ref_conversion_image() {
        use yrs::Doc;
        let block = NewContentBlock {
            id: uuid::Uuid::new_v4().to_string(),
            block_type: BlockType::Image,
            data: BlockData::Image {
                file: UploadedImage {
                    url: "https://example.com/image.png".to_string(),
                    filename: "image.png".to_string(),
                },
                caption: Some("An image caption".to_string()),
                with_border: true,
                with_background: false,
                stretched: true,
            },
            css_classes: vec!["custom-image".to_string()],
            revision_id: None,
        };
        let doc = Doc::new();
        let map_ref = setup_y_map(&doc, block.clone());
        let txn = doc.transact();
        let map_ref_with_txn = MapRefWithTransaction { map_ref, txn: &txn };
        let converted_block = NewContentBlock::try_from(map_ref_with_txn).unwrap();
        assert_eq!(block, converted_block);
    }

    #[test]
    fn test_map_ref_conversion_raw() {
        use yrs::Doc;
        let block = NewContentBlock {
            id: uuid::Uuid::new_v4().to_string(),
            block_type: BlockType::Raw,
            data: BlockData::Raw {
                html: "<div>Raw HTML</div>".to_string(),
            },
            css_classes: vec![],
            revision_id: None,
        };
        let doc = Doc::new();
        let map_ref = setup_y_map(&doc, block.clone());
        let txn = doc.transact();
        let map_ref_with_txn = MapRefWithTransaction { map_ref, txn: &txn };
        let converted_block = NewContentBlock::try_from(map_ref_with_txn).unwrap();
        assert_eq!(block, converted_block);
    }

    #[test]
    fn test_decode_yjs_content() {
        use yrs::{Doc, Transact};
        let block1 = NewContentBlock {
            id: uuid::Uuid::new_v4().to_string(),
            block_type: BlockType::Paragraph,
            data: BlockData::Paragraph {
                text: "First block".to_string(),
            },
            css_classes: vec![],
            revision_id: None,
        };
        let block2 = NewContentBlock {
            id: uuid::Uuid::new_v4().to_string(),
            block_type: BlockType::Heading,
            data: BlockData::Heading {
                text: "Second block".to_string(),
                level: 1,
            },
            css_classes: vec![],
            revision_id: None,
        };

        let doc = Doc::new();
        let blocks_array = doc.get_or_insert_array("blocks");
        {
            let mut txn = doc.transact_mut();
            blocks_array.push_back(&mut txn, yrs::MapPrelim::from(block1.clone()));
            blocks_array.push_back(&mut txn, yrs::MapPrelim::from(block2.clone()));
        }

        let update = {
            let txn = doc.transact();
            txn.encode_diff_v1(&yrs::StateVector::default())
        };
        let decoded_blocks = decode_yjs_content(&update).unwrap();

        assert_eq!(decoded_blocks.len(), 2);
        assert_eq!(decoded_blocks[0], block1);
        assert_eq!(decoded_blocks[1], block2);
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

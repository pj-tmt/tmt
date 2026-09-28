//! Strict scene wire admission. This boundary grants no storage or host capability.

pub mod document;
pub mod image;
pub mod snapshot;

use crate::json_integer::{whole, whole_value};
use crate::office_whiteboard::{self as policy, Bounds, Content, Element, InvalidScene, Scene};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SceneWire {
    #[serde(deserialize_with = "whole")]
    format_version: u8,
    #[serde(deserialize_with = "whole")]
    width: u16,
    #[serde(deserialize_with = "whole")]
    height: u16,
    background: String,
    elements: Vec<ElementWire>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum ElementWire {
    Stroke(PathWire),
    Arrow(PathWire),
    Rectangle(ShapeWire),
    Ellipse(ShapeWire),
    Text(TextWire),
    Note(TextWire),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PathWire {
    id: String,
    #[serde(deserialize_with = "points")]
    points: Vec<[u16; 2]>,
    color: String,
    #[serde(deserialize_with = "whole")]
    stroke_width: u8,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ShapeWire {
    id: String,
    #[serde(deserialize_with = "whole")]
    x: u16,
    #[serde(deserialize_with = "whole")]
    y: u16,
    #[serde(deserialize_with = "whole")]
    width: u16,
    #[serde(deserialize_with = "whole")]
    height: u16,
    color: String,
    fill: String,
    #[serde(deserialize_with = "whole")]
    stroke_width: u8,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TextWire {
    id: String,
    #[serde(deserialize_with = "whole")]
    x: u16,
    #[serde(deserialize_with = "whole")]
    y: u16,
    #[serde(deserialize_with = "whole")]
    width: u16,
    #[serde(deserialize_with = "whole")]
    height: u16,
    text: String,
    color: String,
    fill: String,
    #[serde(deserialize_with = "whole")]
    font_size: u8,
}

fn points<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Vec<[u16; 2]>, D::Error> {
    Vec::<[serde_json::Number; 2]>::deserialize(deserializer)?
        .into_iter()
        .map(|[x, y]| {
            Ok([
                whole_value(x).map_err(serde::de::Error::custom)?,
                whole_value(y).map_err(serde::de::Error::custom)?,
            ])
        })
        .collect()
}

impl From<ElementWire> for Element {
    fn from(value: ElementWire) -> Self {
        let arrow = matches!(&value, ElementWire::Arrow(_));
        let ellipse = matches!(&value, ElementWire::Ellipse(_));
        let note = matches!(&value, ElementWire::Note(_));
        match value {
            ElementWire::Stroke(value) | ElementWire::Arrow(value) => Self {
                id: value.id,
                content: Content::Path {
                    arrow,
                    points: value.points,
                    color: value.color,
                    stroke_width: value.stroke_width,
                },
            },
            ElementWire::Rectangle(value) | ElementWire::Ellipse(value) => Self {
                id: value.id,
                content: Content::Shape {
                    ellipse,
                    bounds: Bounds {
                        x: value.x,
                        y: value.y,
                        width: value.width,
                        height: value.height,
                    },
                    color: value.color,
                    fill: value.fill,
                    stroke_width: value.stroke_width,
                },
            },
            ElementWire::Text(value) | ElementWire::Note(value) => Self {
                id: value.id,
                content: Content::Text {
                    note,
                    bounds: Bounds {
                        x: value.x,
                        y: value.y,
                        width: value.width,
                        height: value.height,
                    },
                    text: value.text,
                    color: value.color,
                    fill: value.fill,
                    font_size: value.font_size,
                },
            },
        }
    }
}

impl From<&Element> for ElementWire {
    fn from(value: &Element) -> Self {
        match &value.content {
            Content::Path {
                arrow,
                points,
                color,
                stroke_width,
            } => {
                let wire = PathWire {
                    id: value.id.clone(),
                    points: points.clone(),
                    color: color.clone(),
                    stroke_width: *stroke_width,
                };
                if *arrow {
                    Self::Arrow(wire)
                } else {
                    Self::Stroke(wire)
                }
            }
            Content::Shape {
                ellipse,
                bounds,
                color,
                fill,
                stroke_width,
            } => {
                let wire = ShapeWire {
                    id: value.id.clone(),
                    x: bounds.x,
                    y: bounds.y,
                    width: bounds.width,
                    height: bounds.height,
                    color: color.clone(),
                    fill: fill.clone(),
                    stroke_width: *stroke_width,
                };
                if *ellipse {
                    Self::Ellipse(wire)
                } else {
                    Self::Rectangle(wire)
                }
            }
            Content::Text {
                note,
                bounds,
                text,
                color,
                fill,
                font_size,
            } => {
                let wire = TextWire {
                    id: value.id.clone(),
                    x: bounds.x,
                    y: bounds.y,
                    width: bounds.width,
                    height: bounds.height,
                    text: text.clone(),
                    color: color.clone(),
                    fill: fill.clone(),
                    font_size: *font_size,
                };
                if *note {
                    Self::Note(wire)
                } else {
                    Self::Text(wire)
                }
            }
        }
    }
}

pub fn decode_scene(bytes: &[u8]) -> Result<Scene, InvalidScene> {
    if bytes.len() > policy::DOCUMENT_BYTES {
        return Err(InvalidScene);
    }
    let wire: SceneWire = serde_json::from_slice(bytes).map_err(|_| InvalidScene)?;
    if wire.format_version != 1 || wire.width != policy::WIDTH || wire.height != policy::HEIGHT {
        return Err(InvalidScene);
    }
    let scene = Scene {
        background: wire.background,
        elements: wire.elements.into_iter().map(Element::from).collect(),
    };
    scene.validate()?;
    Ok(scene)
}

pub fn encode_scene(scene: &Scene) -> Result<Vec<u8>, InvalidScene> {
    scene.validate()?;
    let bytes = serde_json::to_vec(&SceneWire {
        format_version: 1,
        width: policy::WIDTH,
        height: policy::HEIGHT,
        background: scene.background.clone(),
        elements: scene.elements.iter().map(ElementWire::from).collect(),
    })
    .map_err(|_| InvalidScene)?;
    if bytes.len() > policy::DOCUMENT_BYTES {
        return Err(InvalidScene);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests;

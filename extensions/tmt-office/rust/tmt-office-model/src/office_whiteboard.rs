//! Inert whiteboard scene policy, independent of JSON, rendering and storage.

pub mod document;
pub mod snapshot;

use std::collections::HashSet;
use uuid::{Uuid, Variant};

pub const WIDTH: u16 = 1600;
pub const HEIGHT: u16 = 1000;
pub const DOCUMENT_BYTES: usize = 2 * 1024 * 1024;
pub const ELEMENT_LIMIT: usize = 2048;
pub const POINT_LIMIT: usize = 65536;
pub const TEXT_BYTES: usize = 16 * 1024;
pub const TOTAL_TEXT_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scene {
    pub background: String,
    pub elements: Vec<Element>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    pub id: String,
    pub content: Content,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    Path {
        arrow: bool,
        points: Vec<[u16; 2]>,
        color: String,
        stroke_width: u8,
    },
    Shape {
        ellipse: bool,
        bounds: Bounds,
        color: String,
        fill: String,
        stroke_width: u8,
    },
    Text {
        note: bool,
        bounds: Bounds,
        text: String,
        color: String,
        fill: String,
        font_size: u8,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bounds {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidScene;

impl std::fmt::Display for InvalidScene {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Invalid whiteboard scene.")
    }
}
impl std::error::Error for InvalidScene {}

impl Scene {
    pub fn validate(&self) -> Result<(), InvalidScene> {
        if !valid_color(&self.background, false) || self.elements.len() > ELEMENT_LIMIT {
            return Err(InvalidScene);
        }
        let mut ids = HashSet::new();
        let mut points_count = 0_usize;
        let mut text_bytes = 0_usize;
        for element in &self.elements {
            if !valid_id(&element.id) || !ids.insert(&element.id) {
                return Err(InvalidScene);
            }
            match &element.content {
                Content::Path {
                    arrow,
                    points,
                    color,
                    stroke_width,
                } => {
                    if points.len() < 2
                        || points.len() > POINT_LIMIT
                        || (*arrow && points.len() != 2)
                        || points.iter().any(|[x, y]| *x > WIDTH || *y > HEIGHT)
                        || !valid_color(color, false)
                        || !(1..=16).contains(stroke_width)
                    {
                        return Err(InvalidScene);
                    }
                    points_count += points.len();
                }
                Content::Shape {
                    bounds,
                    color,
                    fill,
                    stroke_width,
                    ..
                } => {
                    if !bounds.valid()
                        || !valid_color(color, false)
                        || !valid_color(fill, true)
                        || !(1..=16).contains(stroke_width)
                    {
                        return Err(InvalidScene);
                    }
                }
                Content::Text {
                    bounds,
                    text,
                    color,
                    fill,
                    font_size,
                    ..
                } => {
                    if !bounds.valid()
                        || !valid_color(color, false)
                        || !valid_color(fill, true)
                        || !(12..=72).contains(font_size)
                        || !valid_text(text)
                    {
                        return Err(InvalidScene);
                    }
                    text_bytes += text.len();
                }
            }
            if points_count > POINT_LIMIT || text_bytes > TOTAL_TEXT_BYTES {
                return Err(InvalidScene);
            }
        }
        Ok(())
    }
}

impl Bounds {
    fn valid(&self) -> bool {
        self.width > 0
            && self.height > 0
            && u32::from(self.x) + u32::from(self.width) <= u32::from(WIDTH)
            && u32::from(self.y) + u32::from(self.height) <= u32::from(HEIGHT)
    }
}

fn valid_id(value: &str) -> bool {
    Uuid::parse_str(value).ok().is_some_and(|id| {
        id.to_string() == value
            && (1..=5).contains(&id.get_version_num())
            && id.get_variant() == Variant::RFC4122
    })
}

fn valid_text(value: &str) -> bool {
    value.len() <= TEXT_BYTES
        && !value
            .chars()
            .any(|ch| ch.is_control() && ch != '\n' && ch != '\t')
}

fn valid_color(value: &str, allow_none: bool) -> bool {
    (allow_none && value == "none")
        || (value.len() == 7
            && value.starts_with('#')
            && value.as_bytes()[1..]
                .iter()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte)))
}

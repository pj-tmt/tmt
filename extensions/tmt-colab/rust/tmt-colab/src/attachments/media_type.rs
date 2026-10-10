//! Byte-derived attachment types. Names and descriptor labels grant no type or preview.
use crate::limits::ATTACHMENT_PREVIEW_PIXELS;

pub const GENERIC: &str = "application/octet-stream";

fn tag(bytes: &[u8], offset: usize, value: &[u8]) -> bool {
    bytes.get(offset..offset + value.len()) == Some(value)
}
fn be16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes(bytes[offset..offset + 2].try_into().unwrap())
}
fn be32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap())
}
fn le16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
}
fn png(bytes: &[u8]) -> Option<(&'static str, u32, u32)> {
    if bytes.len() < 24 || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") || !tag(bytes, 12, b"IHDR") {
        return None;
    }
    Some(("image/png", be32(bytes, 16), be32(bytes, 20)))
}
fn jpeg(bytes: &[u8]) -> Option<(&'static str, u32, u32)> {
    if bytes.len() < 4 || !bytes.starts_with(&[0xff, 0xd8]) {
        return None;
    }
    let mut offset = 2;
    while offset + 4 <= bytes.len() {
        if bytes[offset] != 0xff {
            return None;
        }
        let marker = bytes[offset + 1];
        if marker == 0xff {
            offset += 1;
            continue;
        }
        if marker == 0x01 || (0xd0..=0xd8).contains(&marker) {
            offset += 2;
            continue;
        }
        if marker == 0xd9 || marker == 0xda {
            return None;
        }
        let length = usize::from(be16(bytes, offset + 2));
        if (0xc0..=0xcf).contains(&marker) && ![0xc4, 0xc8, 0xcc].contains(&marker) {
            if length < 8 || offset + 9 > bytes.len() {
                return None;
            }
            return Some((
                "image/jpeg",
                u32::from(be16(bytes, offset + 7)),
                u32::from(be16(bytes, offset + 5)),
            ));
        }
        if length < 2 {
            return None;
        }
        offset += 2 + length;
    }
    None
}
fn webp(bytes: &[u8]) -> Option<(&'static str, u32, u32)> {
    if bytes.len() < 25 || !tag(bytes, 0, b"RIFF") || !tag(bytes, 8, b"WEBP") {
        return None;
    }
    let dimensions = if tag(bytes, 12, b"VP8 ") {
        if bytes.len() < 30 || !tag(bytes, 23, &[0x9d, 0x01, 0x2a]) {
            return None;
        }
        (
            u32::from(le16(bytes, 26) & 0x3fff),
            u32::from(le16(bytes, 28) & 0x3fff),
        )
    } else if tag(bytes, 12, b"VP8L") {
        if bytes[20] != 0x2f {
            return None;
        }
        let bits = u32::from_le_bytes(bytes[21..25].try_into().unwrap());
        ((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1)
    } else if tag(bytes, 12, b"VP8X") && bytes.len() >= 30 {
        let le24 =
            |offset| u32::from_le_bytes([bytes[offset], bytes[offset + 1], bytes[offset + 2], 0]);
        (le24(24) + 1, le24(27) + 1)
    } else {
        return None;
    };
    Some(("image/webp", dimensions.0, dimensions.1))
}

/// The same bounded-header raster and PDF/text rules as the browser's storedMediaType.
pub fn stored_media_type(bytes: &[u8]) -> &'static str {
    if let Some((kind, width, height)) = png(bytes).or_else(|| jpeg(bytes)).or_else(|| webp(bytes))
        && width > 0
        && height > 0
        && u64::from(width) * u64::from(height) <= ATTACHMENT_PREVIEW_PIXELS
    {
        return kind;
    }
    if bytes.starts_with(b"%PDF-") {
        return "application/pdf";
    }
    if !bytes.is_empty()
        && std::str::from_utf8(bytes).is_ok()
        && !bytes
            .iter()
            .any(|byte| *byte < 0x20 && !b"\t\n\r".contains(byte))
    {
        return "text/plain";
    }
    GENERIC
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_media_type_vector() {
        let vector: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../contracts/vectors/attachment-media-type-v1.json"
        ))
        .unwrap();
        for case in vector["cases"].as_array().unwrap() {
            let hex = case["hex"].as_str().unwrap();
            let bytes: Vec<_> = (0..hex.len())
                .step_by(2)
                .map(|offset| u8::from_str_radix(&hex[offset..offset + 2], 16).unwrap())
                .collect();
            assert_eq!(
                stored_media_type(&bytes),
                case["mediaType"].as_str().unwrap(),
                "{}",
                case["name"]
            );
        }
    }
}

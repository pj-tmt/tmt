//! Admit a browser-rendered snapshot as inert pixels, never as trusted scene semantics.

use std::io::Cursor;

use crate::office_whiteboard::{HEIGHT, WIDTH};

use tmt_core::content_digest::framed_sha256;

pub const SNAPSHOT_PNG_LIMIT: usize = 8 * 1024 * 1024;
const PIXEL_BYTES: usize = WIDTH as usize * HEIGHT as usize * 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedSnapshotImage {
    bytes: Vec<u8>,
    pixel_digest: String,
}

impl ValidatedSnapshotImage {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    /// Pixel identity survives compression/metadata differences and encoder upgrades.
    pub fn pixel_digest(&self) -> &str {
        &self.pixel_digest
    }

    /// Revalidate stored pixels while retaining the originally published PNG bytes.
    pub fn restore(bytes: Vec<u8>, digest: &str) -> Result<Self, InvalidSnapshotImage> {
        let pixels = decode_pixels(&bytes)?;
        let pixel_digest = framed_sha256(b"tmt:whiteboard:png:v1\0", &pixels);
        if pixel_digest != digest {
            return Err(InvalidSnapshotImage);
        }
        Ok(Self {
            bytes,
            pixel_digest,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidSnapshotImage;

impl std::fmt::Display for InvalidSnapshotImage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(
            "Snapshot image must be a static, opaque 1600x1000 RGB/RGBA 8-bit PNG within 8 MiB.",
        )
    }
}
impl std::error::Error for InvalidSnapshotImage {}

fn decode_pixels(bytes: &[u8]) -> Result<Vec<u8>, InvalidSnapshotImage> {
    if bytes.is_empty() || bytes.len() > SNAPSHOT_PNG_LIMIT {
        return Err(InvalidSnapshotImage);
    }
    let limits = png::Limits {
        bytes: 16 * 1024 * 1024,
    };
    let mut decoder = png::Decoder::new_with_limits(Cursor::new(bytes), limits);
    decoder.set_ignore_text_chunk(true);
    decoder.set_ignore_iccp_chunk(true);
    let header = decoder
        .read_header_info()
        .map_err(|_| InvalidSnapshotImage)?;
    if header.width != u32::from(WIDTH)
        || header.height != u32::from(HEIGHT)
        || header.bit_depth != png::BitDepth::Eight
        || !matches!(
            header.color_type,
            png::ColorType::Rgb | png::ColorType::Rgba
        )
    {
        return Err(InvalidSnapshotImage);
    }
    let mut reader = decoder.read_info().map_err(|_| InvalidSnapshotImage)?;
    if reader.info().animation_control.is_some() || reader.info().trns.is_some() {
        return Err(InvalidSnapshotImage);
    }
    let size = reader
        .output_buffer_size()
        .filter(|size| *size <= PIXEL_BYTES)
        .ok_or(InvalidSnapshotImage)?;
    let mut decoded = vec![0; size];
    let output = reader
        .next_frame(&mut decoded)
        .map_err(|_| InvalidSnapshotImage)?;
    reader.finish().map_err(|_| InvalidSnapshotImage)?;
    if reader.info().animation_control.is_some()
        || reader.info().trns.is_some()
        || output.width != u32::from(WIDTH)
        || output.height != u32::from(HEIGHT)
    {
        return Err(InvalidSnapshotImage);
    }
    let rgba = match output.color_type {
        png::ColorType::Rgba if output.buffer_size() == PIXEL_BYTES => decoded,
        png::ColorType::Rgb if output.buffer_size() == PIXEL_BYTES / 4 * 3 => {
            let mut rgba = Vec::with_capacity(PIXEL_BYTES);
            for pixel in decoded.chunks_exact(3) {
                rgba.extend_from_slice(pixel);
                rgba.push(255);
            }
            rgba
        }
        _ => return Err(InvalidSnapshotImage),
    };
    if rgba.chunks_exact(4).any(|pixel| pixel[3] != 255) {
        return Err(InvalidSnapshotImage);
    }
    Ok(rgba)
}

/// Verify a trusted companion's stored image without recompressing its bytes.
pub fn validate_image_response(bytes: &[u8]) -> Result<(), InvalidSnapshotImage> {
    decode_pixels(bytes).map(|_| ())
}

pub fn decode_snapshot_image(bytes: &[u8]) -> Result<ValidatedSnapshotImage, InvalidSnapshotImage> {
    let rgba = decode_pixels(bytes)?;
    let pixel_digest = framed_sha256(b"tmt:whiteboard:png:v1\0", &rgba);
    // Only decoded pixels are retained. No text/ICC chunks, trailing data or input
    // compression stream is copied into storage or exported to an agent.
    let mut normalized = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut normalized, u32::from(WIDTH), u32::from(HEIGHT));
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|_| InvalidSnapshotImage)?;
        writer
            .write_image_data(&rgba)
            .map_err(|_| InvalidSnapshotImage)?;
        writer.finish().map_err(|_| InvalidSnapshotImage)?;
    }
    if normalized.len() > SNAPSHOT_PNG_LIMIT {
        return Err(InvalidSnapshotImage);
    }
    Ok(ValidatedSnapshotImage {
        bytes: normalized,
        pixel_digest,
    })
}

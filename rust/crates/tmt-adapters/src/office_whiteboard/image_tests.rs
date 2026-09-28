//! Shared in-memory PNG fixtures and model contract tests.
use std::io::Cursor;
use tmt_core::content_digest::framed_sha256;
use tmt_office_model::codec::office_whiteboard::image::*;
use tmt_office_model::office_whiteboard::{HEIGHT, WIDTH};
const PIXEL_BYTES: usize = WIDTH as usize * HEIGHT as usize * 4;

pub(crate) mod test_support {
    pub fn png(
        width: u32,
        height: u32,
        color: png::ColorType,
        pixels: &[u8],
        text: bool,
    ) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, width, height);
            encoder.set_color(color);
            encoder.set_depth(png::BitDepth::Eight);
            if text {
                encoder
                    .add_text_chunk(
                        "Comment".into(),
                        "INERT-METADATA-MUST-NOT-BE-EXPORTED".into(),
                    )
                    .unwrap();
            }
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(pixels).unwrap();
            writer.finish().unwrap();
        }
        bytes
    }

    pub fn solid(pixel: [u8; 4]) -> Vec<u8> {
        let pixels = pixel.repeat(super::PIXEL_BYTES / 4);
        png(
            u32::from(super::WIDTH),
            u32::from(super::HEIGHT),
            png::ColorType::Rgba,
            &pixels,
            false,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_preserves_pixels_not_metadata_or_compression_identity() {
        let rgb = [24, 98, 81].repeat(usize::from(WIDTH) * usize::from(HEIGHT));
        let raw = test_support::png(
            u32::from(WIDTH),
            u32::from(HEIGHT),
            png::ColorType::Rgb,
            &rgb,
            true,
        );
        let first = decode_snapshot_image(&raw).unwrap();
        let second = decode_snapshot_image(&test_support::solid([24, 98, 81, 255])).unwrap();
        assert_eq!(first.pixel_digest(), second.pixel_digest());
        assert_eq!(first.bytes(), second.bytes());
        assert!(!first.bytes().windows(5).any(|bytes| bytes == b"INERT"));
        let decoder = png::Decoder::new(Cursor::new(first.bytes()));
        let mut reader = decoder.read_info().unwrap();
        assert_eq!(reader.info().color_type, png::ColorType::Rgba);
        assert!(reader.info().uncompressed_latin1_text.is_empty());
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        reader.next_frame(&mut pixels).unwrap();
        assert!(
            pixels
                .chunks_exact(4)
                .all(|pixel| pixel == [24, 98, 81, 255])
        );
        assert_eq!(
            first.pixel_digest(),
            framed_sha256(b"tmt:whiteboard:png:v1\0", &pixels)
        );
    }

    #[test]
    fn wrong_dimensions_transparency_crc_truncation_and_oversize_fail() {
        assert!(
            decode_snapshot_image(&test_support::png(
                1,
                1,
                png::ColorType::Rgba,
                &[1, 2, 3, 255],
                false
            ))
            .is_err()
        );
        assert!(decode_snapshot_image(&test_support::solid([1, 2, 3, 254])).is_err());
        let valid = test_support::solid([1, 2, 3, 255]);
        assert!(decode_snapshot_image(&valid[..valid.len() - 8]).is_err());
        let mut corrupt = valid.clone();
        corrupt[29] ^= 1;
        assert!(decode_snapshot_image(&corrupt).is_err());
        // Keep a valid PNG as the control: malformed bytes would reject even if
        // the encoded-input budget were accidentally removed.
        let mut padded = valid;
        padded.resize(SNAPSHOT_PNG_LIMIT, 0);
        assert!(decode_snapshot_image(&padded).is_ok());
        padded.push(0);
        assert!(decode_snapshot_image(&padded).is_err());
        assert!(decode_snapshot_image(b"<svg onload='alert(1)'/>").is_err());
    }

    #[test]
    fn animation_is_rejected_even_with_an_opaque_full_size_frame() {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, u32::from(WIDTH), u32::from(HEIGHT));
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.set_animated(1, 0).unwrap();
            let mut writer = encoder.write_header().unwrap();
            writer
                .write_image_data(&[24, 98, 81, 255].repeat(PIXEL_BYTES / 4))
                .unwrap();
            writer.finish().unwrap();
        }
        let reader = png::Decoder::new(Cursor::new(&bytes)).read_info().unwrap();
        assert!(reader.info().animation_control.is_some());
        assert!(decode_snapshot_image(&bytes).is_err());
        assert!(decode_snapshot_image(&test_support::solid([24, 98, 81, 255])).is_ok());
    }

    #[test]
    fn rgb_transparency_chunk_cannot_be_silently_flattened_to_opaque_pixels() {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, u32::from(WIDTH), u32::from(HEIGHT));
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.set_trns(vec![0, 24, 0, 98, 0, 81]);
            let mut writer = encoder.write_header().unwrap();
            writer
                .write_image_data(&[24, 98, 81].repeat(PIXEL_BYTES / 4))
                .unwrap();
            writer.finish().unwrap();
        }
        let reader = png::Decoder::new(Cursor::new(&bytes)).read_info().unwrap();
        assert_eq!(reader.info().color_type, png::ColorType::Rgb);
        assert!(reader.info().trns.is_some());
        assert!(decode_snapshot_image(&bytes).is_err());
        assert!(decode_snapshot_image(&test_support::solid([24, 98, 81, 255])).is_ok());
    }
}

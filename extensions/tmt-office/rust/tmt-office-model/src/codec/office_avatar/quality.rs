//! Advisory art checks after admission; never a second parser or acceptance gate.
use super::ValidatedAvatarPack;
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Debug, Serialize)]
pub struct QualityWarning {
    pub avatar: String,
    pub code: &'static str,
    pub message: &'static str,
}

pub fn quality_warnings(pack: &ValidatedAvatarPack) -> Vec<QualityWarning> {
    let mut warnings = Vec::new();
    let format = pack.format();
    for avatar in &pack.pack().avatars {
        let mut warn = |code, message| {
            warnings.push(QualityWarning {
                avatar: avatar.key.clone(),
                code,
                message,
            })
        };
        if crate::indexed_art::has_opaque_edge(&avatar.pixels, format.index_width) {
            warn(
                "opaque-edge",
                "Visible pixels touch the canvas edge; inspect for clipped artwork unless this is intentional.",
            );
        }
        let mut visible = 0;
        let mut colors = BTreeSet::new();
        for row in &avatar.pixels {
            for pixel in row.as_bytes().chunks_exact(format.index_width) {
                let index =
                    usize::from_str_radix(std::str::from_utf8(pixel).expect("admitted ASCII"), 16)
                        .expect("admitted palette index");
                if index != 0 {
                    visible += 1;
                    colors.insert(&pack.pack().palette[index]);
                }
            }
        }
        if visible < format.width * format.height / 8 {
            warn(
                "small-silhouette",
                "Less than one eighth of the canvas is opaque; inspect character readability at normal zoom.",
            );
        }
        if colors.len() == 1 {
            warn(
                "single-color",
                "Visible artwork uses one color; inspect feature separation unless a flat silhouette is intentional.",
            );
        }
    }
    warnings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::office_avatar::validate_pack;
    use serde_json::json;

    fn pack(pixels: Vec<String>, palette: Vec<&str>) -> ValidatedAvatarPack {
        validate_pack(
            &serde_json::to_vec(&json!({
                "formatVersion": 1, "label": "Fixture", "credit": "TMT", "license": "MIT",
                "palette": palette, "avatars": [{"key": "bot", "label": "Bot", "pixels": pixels}]
            }))
            .unwrap(),
        )
        .unwrap()
    }

    fn codes(pack: &ValidatedAvatarPack) -> Vec<&'static str> {
        quality_warnings(pack)
            .iter()
            .map(|warning| warning.code)
            .collect()
    }

    #[test]
    fn warns_about_clipping_and_small_flat_art_without_changing_admitted_bytes_or_digest() {
        let mut pixels = vec!["0000000000000000".to_owned(); 24];
        pixels[12] = "1000000000000000".into();
        let pack = pack(pixels, vec!["#00000000", "#ffffffff"]);
        let before = (pack.bytes().to_vec(), pack.digest().to_owned());
        assert_eq!(
            codes(&pack),
            ["opaque-edge", "small-silhouette", "single-color"]
        );
        assert!(
            quality_warnings(&pack)
                .iter()
                .all(|warning| warning.avatar == "bot")
        );
        assert_eq!((pack.bytes().to_vec(), pack.digest().to_owned()), before);
        assert_eq!(validate_pack(pack.bytes()).unwrap(), pack);
    }

    #[test]
    fn counts_actual_colors_not_unused_palette_entries_or_duplicate_color_indices() {
        let mut pixels = vec!["0000000000000000".to_owned(); 24];
        for row in &mut pixels[6..18] {
            *row = "0000001221000000".into();
        }
        let art = pack(
            pixels.clone(),
            vec!["#00000000", "#ffffffff", "#ffffffff", "#ff0000ff"],
        );
        assert_eq!(codes(&art), ["single-color"]);
        let art = pack(pixels, vec!["#00000000", "#ffffffff", "#223344ff"]);
        assert!(codes(&art).is_empty());
    }

    #[test]
    fn sparse_boundary_is_strict_and_each_avatar_keeps_its_own_warning_key() {
        let mut pixels = vec!["0000000000000000".to_owned(); 24];
        for row in &mut pixels[6..18] {
            *row = "0000001221000000".into();
        }
        pixels[6] = "0000000221000000".into();
        let small = pack(pixels, vec!["#00000000", "#ffffffff", "#223344ff"]);
        assert_eq!(codes(&small), ["small-silhouette"]);
        let mut value = serde_json::to_value(small.pack()).unwrap();
        let mut second = value["avatars"][0].clone();
        second["key"] = json!("second");
        value["avatars"].as_array_mut().unwrap().push(second);
        let pair = validate_pack(&serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(
            quality_warnings(&pair)
                .iter()
                .map(|warning| warning.avatar.as_str())
                .collect::<Vec<_>>(),
            ["bot", "second"]
        );
    }
}

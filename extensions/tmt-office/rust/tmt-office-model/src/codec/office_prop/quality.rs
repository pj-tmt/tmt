//! Advisory authoring checks over an already-admitted pack, not another validator.
use super::ValidatedPropPack;
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QualityWarning {
    pub code: &'static str,
    pub prop: String,
    pub rotation: usize,
    pub message: &'static str,
}

/// Warnings guide authoring; symmetric art, edge-to-edge tiles and deliberate
/// stretching remain valid. Appearance still requires visual review.
pub fn quality_warnings(pack: &ValidatedPropPack) -> Vec<QualityWarning> {
    let directional = pack.pack().format_version == 2;
    let index_width = if directional { 2 } else { 1 };
    let mut warnings = Vec::new();
    for prop in &pack.pack().props {
        let rasters = prop.rasters();
        let base_width = rasters[0][0].len() / index_width;
        let base_height = rasters[0].len();
        for (rotation, raster) in rasters.iter().enumerate() {
            let mut warn = |code, message| {
                warnings.push(QualityWarning {
                    code,
                    prop: prop.key.clone(),
                    rotation,
                    message,
                })
            };
            let width = raster[0].len() / index_width;
            let height = raster.len();
            let (foot_width, foot_height) = if rotation % 2 == 0 {
                (
                    usize::from(prop.footprint.width),
                    usize::from(prop.footprint.height),
                )
            } else {
                (
                    usize::from(prop.footprint.height),
                    usize::from(prop.footprint.width),
                )
            };
            if width * foot_height != height * foot_width {
                warn(
                    "nonuniform-scale",
                    "Raster proportions stretch pixels within the displayed footprint.",
                );
            }
            if directional
                && (width * usize::from(prop.footprint.width) != base_width * foot_width
                    || height * usize::from(prop.footprint.height) != base_height * foot_height)
            {
                warn(
                    "directional-scale-drift",
                    "Source-pixel scale changes between views; use consistent pixels per tile.",
                );
            }
            if crate::indexed_art::has_opaque_edge(raster, index_width) {
                warn(
                    "opaque-edge",
                    "Visible pixels touch the raster edge; inspect for clipped artwork unless this is intentional tiling.",
                );
            }
            if prop
                .customization
                .as_ref()
                .and_then(|value| value.text.as_ref())
                .is_some_and(|text| text.regions[rotation].height < 8)
            {
                warn(
                    "small-text-region",
                    "The display-text region is under eight source pixels high; inspect readability at normal zoom.",
                );
            }
        }
    }
    warnings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::office_prop::validate_pack;
    use serde_json::json;

    #[test]
    fn quality_is_advisory_and_detects_stretched_drifting_clipped_art() {
        let bytes = serde_json::to_vec(&json!({
            "formatVersion": 2, "label": "Fixture", "credit": "TMT", "license": "MIT",
            "palette": ["#00000000", "#ffffffff"],
            "props": [{"key": "lamp", "label": "Lamp", "footprint": {"width": 2, "height": 1},
                "frames": [["00000000", "00010100"], ["01", "01", "01", "01"],
                    ["00000000", "00010100"], ["0000", "0101", "0101", "0000"]],
                "customization": {"text": {"color": "#ffffff", "regions": [
                    {"x": 0, "y": 0, "width": 4, "height": 2},
                    {"x": 0, "y": 0, "width": 1, "height": 4},
                    {"x": 0, "y": 0, "width": 4, "height": 2},
                    {"x": 0, "y": 0, "width": 2, "height": 4}
                ]}}
            }]
        }))
        .unwrap();
        let pack = validate_pack(&bytes).unwrap();
        let warnings = quality_warnings(&pack);
        let codes = |rotation| {
            warnings
                .iter()
                .filter(|warning| warning.rotation == rotation)
                .map(|warning| warning.code)
                .collect::<Vec<_>>()
        };
        assert_eq!(codes(0), ["opaque-edge", "small-text-region"]);
        assert_eq!(
            codes(1),
            [
                "nonuniform-scale",
                "directional-scale-drift",
                "opaque-edge",
                "small-text-region"
            ]
        );
        assert_eq!(codes(3), ["opaque-edge", "small-text-region"]);
        assert_eq!(pack.bytes(), bytes);
        assert!(warnings.iter().all(|warning| warning.prop == "lamp"));
    }

    #[test]
    fn padded_symmetric_directional_art_does_not_need_four_different_views() {
        let bytes = serde_json::to_vec(&json!({
            "formatVersion": 2, "label": "Fixture", "credit": "TMT", "license": "MIT",
            "palette": ["#00000000", "#ffffffff"],
            "props": [{"key": "tile", "label": "Tile", "footprint": {"width": 1, "height": 1},
                "frames": vec![vec!["000000", "000100", "000000"]; 4]}]
        }))
        .unwrap();
        assert!(quality_warnings(&validate_pack(&bytes).unwrap()).is_empty());
    }

    #[test]
    fn v1_checks_its_single_raster_without_inventing_directional_requirements() {
        let bytes = serde_json::to_vec(&json!({
            "formatVersion": 1, "label": "Fixture", "credit": "TMT", "license": "MIT",
            "palette": ["#00000000", "#ffffffff"],
            "props": [{"key": "tile", "label": "Tile", "footprint": {"width": 2, "height": 1},
                "pixels": ["000", "010", "000"]}]
        }))
        .unwrap();
        let warnings = quality_warnings(&validate_pack(&bytes).unwrap());
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].code, "nonuniform-scale");
        assert_eq!(warnings[0].rotation, 0);
    }
}

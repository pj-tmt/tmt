use super::*;

const SAMPLE: &[u8] = include_bytes!(
    "../../../../../extensions/tmt-office/contracts/avatar-pack-v2-sample.tmtavatar.json"
);

#[test]
fn v2_literal_sample_vectors_and_projection_match() {
    let sample: Value = serde_json::from_slice(SAMPLE).unwrap();
    let vectors: Value = serde_json::from_str(include_str!(
        "../../../../../extensions/tmt-office/contracts/avatar-pack-v2-vectors.json"
    ))
    .unwrap();
    for case in vectors["cases"].as_array().unwrap() {
        let mut value = sample.clone();
        *value.pointer_mut(case["path"].as_str().unwrap()).unwrap() = case["value"].clone();
        assert_eq!(
            validate_pack(&serde_json::to_vec(&value).unwrap()).is_ok(),
            case["valid"].as_bool().unwrap(),
            "{}",
            case["name"]
        );
    }
    let pack = validate_pack(SAMPLE).unwrap();
    assert_eq!(
        pack.digest(),
        "sha256:371a80e4b0e24cd0d8f5b948e506074ab3be536529c8220db3819e0a34d76831"
    );
    assert_eq!(pack.cell_count(), 1_536);
    assert_eq!(pack.format().index_width, 2);
    assert_eq!(
        pack_projection(&pack)["avatars"][0]["raster"],
        json!({"width":32,"height":48})
    );
    assert_ne!(pack.digest(), framed_digest(SAMPLE, 1));
    assert_eq!(pack.bytes(), SAMPLE);
}

#[test]
fn v2_keeps_existing_cell_and_file_budgets() {
    let mut value: Value = serde_json::from_slice(SAMPLE).unwrap();
    let avatar = value["avatars"][0].clone();
    value["avatars"] = json!(
        (0..4)
            .map(|index| {
                let mut copy = avatar.clone();
                copy["key"] = json!(format!("bot-{index}"));
                copy
            })
            .collect::<Vec<_>>()
    );
    let mut bytes = serde_json::to_vec(&value).unwrap();
    assert_eq!(validate_pack(&bytes).unwrap().cell_count(), PACK_CELL_LIMIT);
    bytes.resize(PACK_INPUT_LIMIT, b' ');
    assert!(validate_pack(&bytes).is_ok());
    bytes.push(b' ');
    assert_eq!(validate_pack(&bytes), Err(AvatarPackError::TooLarge));
    value["avatars"].as_array_mut().unwrap().push(avatar);
    assert_eq!(
        validate_pack(&serde_json::to_vec(&value).unwrap()),
        Err(AvatarPackError::Invalid)
    );
}

#[test]
fn v2_checks_full_byte_palette_indices_and_opaque_cells() {
    let mut value: Value = serde_json::from_slice(SAMPLE).unwrap();
    value["palette"] = json!(
        std::iter::once("#00000000".to_owned())
            .chain((1..256).map(|index| format!("#{index:02x}0000ff")))
            .collect::<Vec<_>>()
    );
    value["avatars"][0]["pixels"][0] = json!(format!("ff{}", "00".repeat(31)));
    assert!(validate_pack(&serde_json::to_vec(&value).unwrap()).is_ok());
    value["palette"]
        .as_array_mut()
        .unwrap()
        .push(json!("#ffffffff"));
    assert_eq!(
        validate_pack(&serde_json::to_vec(&value).unwrap()),
        Err(AvatarPackError::Invalid)
    );
    value["palette"].as_array_mut().unwrap().pop();
    value["avatars"][0]["pixels"] = json!(vec!["00".repeat(32); 48]);
    assert_eq!(
        validate_pack(&serde_json::to_vec(&value).unwrap()),
        Err(AvatarPackError::Invalid)
    );
    value["avatars"][0]["pixels"] = json!(vec!["01".repeat(32); 47]);
    assert_eq!(
        validate_pack(&serde_json::to_vec(&value).unwrap()),
        Err(AvatarPackError::Invalid)
    );
}

#[test]
fn v2_quality_counts_cells_and_resolved_colors_not_nibbles() {
    let mut value: Value = serde_json::from_slice(SAMPLE).unwrap();
    let mut pixels = vec!["00".repeat(32); 48];
    for row in &mut pixels[12..36] {
        *row = format!("{}{}{}", "00".repeat(12), "10".repeat(8), "00".repeat(12));
    }
    value["avatars"][0]["pixels"] = json!(pixels);
    let pack = validate_pack(&serde_json::to_vec(&value).unwrap()).unwrap();
    assert_eq!(
        quality_warnings(&pack)
            .iter()
            .map(|warning| warning.code)
            .collect::<Vec<_>>(),
        ["single-color"]
    );
    pixels[12].replace_range(24..26, "00");
    value["avatars"][0]["pixels"] = json!(pixels);
    let pack = validate_pack(&serde_json::to_vec(&value).unwrap()).unwrap();
    assert_eq!(
        quality_warnings(&pack)
            .iter()
            .map(|warning| warning.code)
            .collect::<Vec<_>>(),
        ["small-silhouette", "single-color"]
    );
}

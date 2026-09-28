use super::office_block::{
    BLOCK_SIZE, BlockLayout, Furniture, FurnitureAsset, INPUT_LIMIT, LayoutError, MAX_REVISION,
    OBJECT_LIMIT,
};
use serde_json::Value;

fn strict_item(value: &Value) -> Option<Furniture> {
    let object = value.as_object()?;
    if object.len() != 4
        || !object
            .keys()
            .all(|key| matches!(key.as_str(), "asset" | "x" | "y" | "rotation"))
    {
        return None;
    }
    Some(Furniture {
        asset: FurnitureAsset::parse(object.get("asset")?.as_str()?)?,
        x: object.get("x")?.as_i64()?.try_into().ok()?,
        y: object.get("y")?.as_i64()?.try_into().ok()?,
        rotation: object.get("rotation")?.as_i64()?.try_into().ok()?,
    })
}

#[test]
fn catalog_is_ordered_and_has_contract_metadata() {
    assert_eq!(
        FurnitureAsset::ALL,
        [
            FurnitureAsset::Desk,
            FurnitureAsset::Chair,
            FurnitureAsset::Plant,
            FurnitureAsset::Rug,
        ]
    );
    assert_eq!(FurnitureAsset::parse("desk"), Some(FurnitureAsset::Desk));
    assert_eq!(FurnitureAsset::parse("Desk"), None);
    assert_eq!(FurnitureAsset::Desk.name(), "desk");
    assert_eq!(FurnitureAsset::Desk.token(), 'd');
    assert_eq!(FurnitureAsset::Desk.dimensions(), (4, 2));
    assert_eq!(FurnitureAsset::Chair.dimensions(), (2, 2));
    assert_eq!(FurnitureAsset::Plant.dimensions(), (2, 2));
    assert_eq!(FurnitureAsset::Rug.dimensions(), (6, 4));
    assert_eq!(BLOCK_SIZE, 32);
    assert_eq!(OBJECT_LIMIT, 16);
    assert_eq!(MAX_REVISION, 9_007_199_254_740_991);
    assert_eq!(INPUT_LIMIT, 65_536);
}

#[test]
fn layout_preserves_order_and_allows_intentional_overlap() {
    let first = Furniture {
        asset: FurnitureAsset::Desk,
        x: 0,
        y: 0,
        rotation: 0,
    };
    let second = Furniture {
        asset: FurnitureAsset::Rug,
        x: 0,
        y: 0,
        rotation: 0,
    };
    let layout = BlockLayout::new(vec![first, second]).unwrap();
    assert_eq!(layout.objects(), &[first, second]);
    assert_eq!(layout.encode(), ["d000", "r000"]);
}

#[test]
fn layout_accepts_empty_and_maximum_but_rejects_the_seventeenth_object() {
    let object = Furniture {
        asset: FurnitureAsset::Chair,
        x: 0,
        y: 0,
        rotation: 0,
    };
    assert_eq!(BlockLayout::new(Vec::new()).unwrap().objects(), &[]);
    assert!(BlockLayout::new(vec![object; OBJECT_LIMIT]).is_ok());
    assert_eq!(
        BlockLayout::new(vec![object; OBJECT_LIMIT + 1]),
        Err(LayoutError::TooManyObjects)
    );
}

#[test]
fn layout_validates_rotated_footprints_at_room_boundaries() {
    assert!(
        BlockLayout::new(vec![Furniture {
            asset: FurnitureAsset::Desk,
            x: 28,
            y: 30,
            rotation: 0,
        }])
        .is_ok()
    );
    assert!(
        BlockLayout::new(vec![Furniture {
            asset: FurnitureAsset::Desk,
            x: 30,
            y: 28,
            rotation: 1,
        }])
        .is_ok()
    );
    assert_eq!(
        BlockLayout::new(vec![Furniture {
            asset: FurnitureAsset::Desk,
            x: 29,
            y: 30,
            rotation: 0,
        }]),
        Err(LayoutError::OutOfBounds)
    );
    assert_eq!(
        BlockLayout::new(vec![Furniture {
            asset: FurnitureAsset::Rug,
            x: 0,
            y: 0,
            rotation: 4,
        }]),
        Err(LayoutError::InvalidRotation)
    );
}

#[test]
fn codec_round_trip_is_canonical_and_rejects_malformed_tokens() {
    let objects = vec![
        Furniture {
            asset: FurnitureAsset::Desk,
            x: 30,
            y: 28,
            rotation: 1,
        },
        Furniture {
            asset: FurnitureAsset::Rug,
            x: 28,
            y: 26,
            rotation: 3,
        },
    ];
    let layout = BlockLayout::new(objects.clone()).unwrap();
    assert_eq!(layout.encode(), ["d1us", "r3sq"]);
    assert_eq!(
        BlockLayout::decode(&layout.encode()).unwrap().objects(),
        objects
    );

    for token in [
        "D000",
        "d400",
        "d00w",
        "d00A",
        "d0000",
        "d00",
        "https://example.test",
    ] {
        assert_eq!(
            BlockLayout::decode(&[token.to_owned()]),
            Err(LayoutError::InvalidToken),
            "{token}"
        );
    }
    let oversized = vec!["d000".to_owned(); OBJECT_LIMIT + 1];
    assert_eq!(
        BlockLayout::decode(&oversized),
        Err(LayoutError::TooManyObjects)
    );
}

#[test]
fn literal_contract_vectors_match_in_both_directions() {
    let corpus: Value =
        serde_json::from_str(include_str!("../../../contracts/block-v1.vectors.json")).unwrap();
    for vector in corpus.as_array().unwrap() {
        let valid = vector["valid"].as_bool().unwrap();
        let stored = vector["stored"].as_str();
        let item = strict_item(&vector["item"]);
        if valid {
            let item = item.unwrap();
            let layout = BlockLayout::new(vec![item]).unwrap();
            assert_eq!(layout.encode(), [stored.unwrap()]);
            assert_eq!(
                BlockLayout::decode(&[stored.unwrap().to_owned()]).unwrap(),
                layout
            );
        } else {
            if let Some(stored) = stored {
                assert!(BlockLayout::decode(&[stored.to_owned()]).is_err());
            }
            if let Some(item) = item {
                assert!(BlockLayout::new(vec![item]).is_err());
            }
        }
    }
}

#[test]
fn vector_deserialization_rejects_unknown_item_fields() {
    let corpus: Value =
        serde_json::from_str(include_str!("../../../contracts/block-v1.vectors.json")).unwrap();
    let item = corpus
        .as_array()
        .unwrap()
        .iter()
        .find(|vector| vector["name"] == "extra content")
        .unwrap()["item"]
        .clone();
    assert!(strict_item(&item).is_none());
}

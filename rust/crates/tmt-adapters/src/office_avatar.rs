//! Office avatar file and command adapters.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::Path;
use tmt_office_model::codec::office_avatar::*;
use tmt_office_model::office_protocol::{OfficeError, OfficeInvocation};
#[cfg(test)]
mod version_tests;

pub fn read_pack_file(path: &Path) -> Result<ValidatedAvatarPack, AvatarPackError> {
    let bytes =
        crate::bounded_file::read_no_follow(path, PACK_INPUT_LIMIT).map_err(
            |error| match error {
                crate::bounded_file::FileReadError::TooLarge => AvatarPackError::TooLarge,
                crate::bounded_file::FileReadError::Io(error) => AvatarPackError::Io(error),
            },
        )?;
    validate_pack(&bytes)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AvatarCommandInput {
    #[serde(default)]
    bytes: Option<String>,
    #[serde(default)]
    digest: Option<String>,
    #[serde(default)]
    expected_revision: Option<u64>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    cursor: Option<String>,
}

pub fn execute(operation: OfficeInvocation, input: &[u8]) -> Vec<u8> {
    serde_json::to_vec(
        &execute_inner(operation, input).unwrap_or_else(|error| json!({"error":error.code()})),
    )
    .unwrap_or_else(|_| br#"{"error":"OFFICE_AVATAR_CORRUPT"}"#.to_vec())
}

fn execute_inner(operation: OfficeInvocation, input: &[u8]) -> Result<Value, OfficeError> {
    // 32 KiB raw input expands to 43,692 base64 bytes plus a bounded JSON envelope.
    if input.len() > PROTOCOL_INPUT_LIMIT {
        return Err(OfficeError::AvatarInvalid);
    }
    let input: AvatarCommandInput =
        serde_json::from_slice(input).map_err(|_| OfficeError::AvatarInvalid)?;
    if operation == OfficeInvocation::LocalAvatarValidate {
        return Ok(pack_projection(&command_pack(&input)?));
    }
    let paths =
        crate::config::ConfigPaths::discover().map_err(|_| OfficeError::CredentialsUnavailable)?;
    let mut storage =
        crate::storage::Storage::open(paths.database).map_err(storage_avatar_error)?;
    let result = match operation {
        OfficeInvocation::LocalAvatarInstall => {
            let candidate = command_pack(&input)?;
            storage
                .install_local_avatar_pack(
                    input.expected_revision.ok_or(OfficeError::AvatarInvalid)?,
                    &candidate,
                )
                .map(|mutation| mutation_projection(mutation, true))
                .map_err(catalog_error)
        }
        OfficeInvocation::LocalAvatarRemove => storage
            .remove_local_avatar_pack(
                input.expected_revision.ok_or(OfficeError::AvatarInvalid)?,
                input.digest.as_deref().ok_or(OfficeError::AvatarInvalid)?,
            )
            .map(|mutation| mutation_projection(mutation, false))
            .map_err(catalog_error),
        OfficeInvocation::LocalAvatarList => storage
            .list_local_avatar_packs(input.limit.unwrap_or(20), input.cursor.as_deref())
            .map(list_projection)
            .map_err(catalog_error),
        OfficeInvocation::LocalAvatarShow => storage
            .show_local_avatar_pack(input.digest.as_deref().ok_or(OfficeError::AvatarInvalid)?)
            .map(snapshot_projection)
            .map_err(catalog_error),
        _ => Err(OfficeError::CredentialsInvalid),
    };
    let close = storage.close().map_err(storage_avatar_error);
    match (result, close) {
        (Err(error), _) => Err(error),
        (Ok(value), Ok(())) => Ok(value),
        (Ok(_), Err(error)) => Err(error),
    }
}

fn command_pack(input: &AvatarCommandInput) -> Result<ValidatedAvatarPack, OfficeError> {
    let encoded = input.bytes.as_deref().ok_or(OfficeError::AvatarInvalid)?;
    if encoded.len() > 43_692 {
        return Err(OfficeError::AvatarInvalid);
    }
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|_| OfficeError::AvatarInvalid)?;
    validate_pack(&bytes).map_err(|_| OfficeError::AvatarInvalid)
}

fn pack_projection(pack: &ValidatedAvatarPack) -> Value {
    let format = pack.format();
    json!({
        "digest":pack.digest(), "formatVersion":pack.pack().format_version,
        "label":pack.pack().label, "credit":pack.pack().credit, "license":pack.pack().license,
        "fileBytes":pack.bytes().len(), "cellCount":pack.cell_count(),
        "avatars":pack.pack().avatars.iter().map(|avatar| json!({
            "key":avatar.key,"label":avatar.label,"raster":{"width":format.width,"height":format.height}
        })).collect::<Vec<_>>()
    })
}
fn snapshot_projection(snapshot: crate::storage::LocalAvatarSnapshot) -> Value {
    let mut value = pack_projection(&snapshot.pack);
    value["catalogRevision"] = json!(snapshot.catalog_revision);
    value["installedAtMs"] = json!(snapshot.installed_at_ms);
    value["builtin"] = json!(snapshot.installed_at_ms.is_none());
    value
}
fn mutation_projection(mutation: crate::storage::LocalAvatarMutation, include_pack: bool) -> Value {
    if include_pack {
        let mut value =
            snapshot_projection(mutation.snapshot.expect("avatar install returns snapshot"));
        value["changed"] = json!(mutation.changed);
        value
    } else {
        json!({"digest":mutation.digest,"catalogRevision":mutation.catalog_revision,"changed":mutation.changed})
    }
}
fn list_projection(list: crate::storage::LocalAvatarCatalogList) -> Value {
    json!({
        "catalogRevision":list.catalog_revision,
        "builtins":list.builtins.into_iter().map(snapshot_projection).collect::<Vec<_>>(),
        "packs":list.packs.into_iter().map(snapshot_projection).collect::<Vec<_>>(),
        "excluded":list.excluded.into_iter().map(|excluded| json!({
            "digest":excluded.digest,"reason":excluded.reason.code()
        })).collect::<Vec<_>>(),
        "nextCursor":list.next_cursor
    })
}
fn catalog_error(error: crate::storage::LocalAvatarCatalogError) -> OfficeError {
    use crate::storage::LocalAvatarCatalogError as Error;
    match error {
        Error::Invalid => OfficeError::AvatarInvalid,
        Error::Corrupt => OfficeError::AvatarCorrupt,
        Error::NotFound => OfficeError::AvatarNotFound,
        Error::Limit => OfficeError::AvatarLimit,
        Error::RevisionConflict | Error::RevisionExhausted => {
            OfficeError::AvatarCatalogRevisionConflict
        }
        Error::CursorInvalid => OfficeError::AvatarCatalogCursorInvalid,
        Error::CursorStale => OfficeError::AvatarCatalogCursorStale,
        Error::Storage(error) => storage_avatar_error(error),
    }
}
fn storage_avatar_error(error: impl std::error::Error) -> OfficeError {
    let _ = error;
    OfficeError::CredentialsUnavailable
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_bytes() -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "formatVersion":1,"label":"Signal bots","credit":"tmux-team","license":"MIT",
            "palette":["#00000000","#ffffffff"],
            "avatars":[{"key":"signal-bot","label":"Signal bot","pixels":vec!["1111111111111111";24]}]
        })).unwrap()
    }

    #[test]
    fn shared_projection_vectors_match_and_literal_digest_is_frozen() {
        let vectors: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../extensions/tmt-office/contracts/avatar-pack-vectors.json"
        ))
        .unwrap();
        for case in vectors["packCases"].as_array().unwrap() {
            let bytes = serde_json::to_vec(&case["value"]).unwrap();
            assert_eq!(
                validate_pack(&bytes).is_ok(),
                case["valid"].as_bool().unwrap(),
                "{}",
                case["name"]
            );
        }
        for case in vectors["boundaryCases"].as_array().unwrap() {
            let mut value = vectors["packCases"][0]["value"].clone();
            match case["field"].as_str().unwrap() {
                "avatarKey" => value["avatars"][0]["key"] = case["value"].clone(),
                "packLabel" => value["label"] = case["value"].clone(),
                "paletteColor" => value["palette"][1] = case["value"].clone(),
                field => panic!("unknown boundary field {field}"),
            }
            let bytes = serde_json::to_vec(&value).unwrap();
            assert_eq!(
                validate_pack(&bytes).is_ok(),
                case["valid"].as_bool().unwrap(),
                "{}",
                case["name"]
            );
        }
        let bytes = include_bytes!(
            "../../../../extensions/tmt-office/contracts/avatar-pack-v1-sample.tmtavatar.json"
        );
        let pack = validate_pack(bytes).unwrap();
        assert_eq!(bytes.len(), 948);
        assert_eq!(
            pack.digest(),
            "sha256:54ef27de6954607e1eebaa0acc976821865198ae5cdcb615ab867f60e48e4610"
        );
    }

    #[test]
    fn accepts_exact_geometry_and_uses_avatar_digest_domain() {
        let bytes = valid_bytes();
        let pack = validate_pack(&bytes).unwrap();
        assert_eq!(pack.cell_count(), 384);
        assert_ne!(
            pack.digest(),
            format!(
                "sha256:{}",
                tmt_core::content_digest::framed_sha256(b"TMT-OFFICE-PROP-PACK-V1\0", &bytes)
            )
        );
        assert_eq!(
            parse_avatar_reference(&format!("{}/signal-bot", pack.digest())),
            Some((pack.digest(), "signal-bot"))
        );
    }

    #[test]
    fn rejects_unknown_duplicate_transparent_and_bad_geometry() {
        let valid = String::from_utf8(valid_bytes()).unwrap();
        assert!(validate_pack(valid.as_bytes()).is_ok());
        for changed in [
            valid.replacen(
                "\"formatVersion\":1",
                "\"formatVersion\":1,\"extra\":true",
                1,
            ),
            valid.replacen(
                "\"label\":\"Signal bots\"",
                "\"label\":\"Signal bots\",\"label\":\"Again\"",
                1,
            ),
            valid.replacen(
                "\"key\":\"signal-bot\"",
                "\"key\":\"signal-bot\",\"key\":\"again\"",
                1,
            ),
            valid.replace("1111111111111111", "0000000000000000"),
            valid.replacen("1111111111111111", "111111111111111", 1),
        ] {
            assert_eq!(
                validate_pack(changed.as_bytes()),
                Err(AvatarPackError::Invalid)
            );
        }
        let lone_surrogate =
            valid.replacen("\"label\":\"Signal bots\"", "\"label\":\"\\ud800\"", 1);
        assert_eq!(
            validate_pack(lone_surrogate.as_bytes()),
            Err(AvatarPackError::Invalid)
        );
        let emoji = valid.replacen(
            "\"label\":\"Signal bots\"",
            "\"label\":\"Signal 🤖 bots\"",
            1,
        );
        assert!(validate_pack(emoji.as_bytes()).is_ok());
        assert_eq!(
            validate_pack(&[0xef, 0xbb, 0xbf, b'{', b'}']),
            Err(AvatarPackError::Invalid)
        );
        assert_eq!(validate_pack(&[0xff, 0xfe]), Err(AvatarPackError::Invalid));
    }

    #[test]
    fn exact_input_byte_limit_is_valid_and_one_more_is_rejected() {
        let mut exact = valid_bytes();
        exact.resize(PACK_INPUT_LIMIT, b' ');
        assert_eq!(exact.len(), PACK_INPUT_LIMIT);
        assert!(validate_pack(&exact).is_ok());
        exact.push(b' ');
        assert_eq!(validate_pack(&exact), Err(AvatarPackError::TooLarge));
    }

    #[test]
    fn exact_avatar_and_cell_capacity_is_admitted_but_one_more_rejects() {
        let mut value: serde_json::Value = serde_json::from_slice(&valid_bytes()).unwrap();
        let template = value["avatars"][0].clone();
        value["avatars"] = json!(
            (0..PACK_AVATAR_LIMIT)
                .map(|index| {
                    let mut avatar = template.clone();
                    avatar["key"] = json!(format!("bot-{index}"));
                    avatar
                })
                .collect::<Vec<_>>()
        );
        let capacity = validate_pack(&serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(capacity.pack().avatars.len(), PACK_AVATAR_LIMIT);
        assert_eq!(capacity.cell_count(), PACK_CELL_LIMIT);
        value["avatars"].as_array_mut().unwrap().push(template);
        assert_eq!(
            validate_pack(&serde_json::to_vec(&value).unwrap()),
            Err(AvatarPackError::Invalid)
        );
    }

    #[test]
    fn maximum_legal_list_projection_fits_the_owned_response_budget() {
        let pixels = vec!["1111111111111111"; AVATAR_HEIGHT];
        let bytes = serde_json::to_vec(&json!({
            "formatVersion":1,
            "label":"\"".repeat(tmt_office_model::indexed_art::LABEL_LIMIT),
            "credit":"\"".repeat(tmt_office_model::indexed_art::CREDIT_LIMIT),
            "license":"M".repeat(tmt_office_model::indexed_art::LICENSE_LIMIT),
            "palette":["#00000000","#ffffffff"],
            "avatars":(0..PACK_AVATAR_LIMIT).map(|index| json!({
                "key":format!("avatar-{index}"),
                "label":"\"".repeat(tmt_office_model::indexed_art::LABEL_LIMIT),
                "pixels":pixels,
            })).collect::<Vec<_>>()
        }))
        .unwrap();
        let pack = validate_pack(&bytes).unwrap();
        let list = crate::storage::LocalAvatarCatalogList {
            builtins: Vec::new(),
            catalog_revision: 1,
            packs: (0..20)
                .map(|_| crate::storage::LocalAvatarSnapshot {
                    catalog_revision: 1,
                    installed_at_ms: Some(1),
                    pack: pack.clone(),
                })
                .collect(),
            excluded: Vec::new(),
            next_cursor: None,
        };
        let encoded = serde_json::to_vec(&list_projection(list)).unwrap();
        assert!(encoded.len() > 48_000);
        assert!(encoded.len() <= PROTOCOL_OUTPUT_LIMIT, "{}", encoded.len());
    }

    #[test]
    fn file_acquisition_is_bounded_regular_and_no_follow() {
        let directory = crate::test_support::TestDirectory::new();
        let regular = directory.path.join("pack.json");
        std::fs::write(&regular, valid_bytes()).unwrap();
        assert!(read_pack_file(&regular).is_ok());
        let oversized = directory.path.join("oversized.json");
        std::fs::write(&oversized, vec![b' '; PACK_INPUT_LIMIT + 1]).unwrap();
        assert_eq!(read_pack_file(&oversized), Err(AvatarPackError::TooLarge));
        let link = directory.path.join("link.json");
        std::os::unix::fs::symlink(&regular, &link).unwrap();
        assert!(matches!(read_pack_file(&link), Err(AvatarPackError::Io(_))));
        assert!(matches!(
            read_pack_file(&directory.path),
            Err(AvatarPackError::Io(_))
        ));
    }
}

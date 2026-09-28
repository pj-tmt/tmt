//! Verified, bounded execution of the optional companion; never resolves PATH.

mod whiteboard;
pub use whiteboard::{SnapshotResource, read_office_snapshot};
mod extension;
pub use extension::validate_office_extension;
mod world;
pub use world::invoke_office_world;

use crate::{
    native_install::{self, Product},
    process::{CommandRequest, RunningCommand, UnixCommandRunner},
};
use std::{
    ffi::OsString,
    io,
    path::Path,
    time::{Duration, Instant},
};
use tmt_office_model::office_protocol::{
    OFFICE_HOOK_BATCH_LIMIT, OFFICE_PROTOCOL_OUTPUT_LIMIT, OfficeError, OfficeInvocation,
    OfficeSyncReport, decode_office_capabilities, decode_office_probe,
};

pub struct PairingCall<'a> {
    pub world: &'a str,
    pub identity_id: &'a str,
    pub emulator: bool,
    pub read_only: bool,
}

pub enum PairingReply {
    Inspected(bool),
    Pending(String),
    State(String),
    Error(OfficeError),
}

/// Launch selection is short; all remote/vault work happens after releasing
/// the installer lock. Only nonsecret scope selectors cross stdin.
pub fn invoke_office_pairing(
    executable: &Path,
    operation: OfficeInvocation,
    call: &PairingCall<'_>,
    deadline: Instant,
) -> io::Result<PairingReply> {
    if matches!(
        operation,
        OfficeInvocation::Probe
            | OfficeInvocation::Capabilities
            | OfficeInvocation::Sync
            | OfficeInvocation::BlockShow
            | OfficeInvocation::BlockApply
            | OfficeInvocation::LocalWorldShow
            | OfficeInvocation::LocalWorldApply
            | OfficeInvocation::LocalProfileShow
            | OfficeInvocation::LocalProfileApply
            | OfficeInvocation::BoardPost
            | OfficeInvocation::BoardList
            | OfficeInvocation::BoardShow
            | OfficeInvocation::BoardReply
            | OfficeInvocation::BoardEdit
            | OfficeInvocation::BoardDelete
            | OfficeInvocation::BoardCategories
            | OfficeInvocation::WhiteboardSnapshotShow
            | OfficeInvocation::WhiteboardSnapshotImage
    ) {
        return Err(invalid_pairing());
    }
    let input = serde_json::to_vec(
        &serde_json::json!({"world":call.world,"identityId":call.identity_id,"emulator":call.emulator,"readOnly":call.read_only}),
    )?;
    if input.len() > 4096 {
        return Err(invalid_pairing());
    }
    let bytes = invoke_json(executable, operation, &input, deadline)?;
    decode_pairing_reply(&bytes, call.world)
}

pub fn invoke_local_office_profile(
    executable: &Path,
    identity_id: &str,
    edit: Option<(&tmt_office_model::office_profile::LocalProfile, u64)>,
    deadline: Instant,
) -> io::Result<Result<serde_json::Value, OfficeError>> {
    let mut input = serde_json::json!({"identityId":identity_id});
    let operation = if let Some((profile, revision)) = edit {
        if revision > tmt_office_model::office_profile::MAX_REVISION || profile.validate().is_err()
        {
            return Ok(Err(OfficeError::ProfileInvalid));
        }
        input["profile"] = tmt_office_model::codec::office_profile_wire::encode_value(profile);
        input["expectedRevision"] = serde_json::json!(revision);
        OfficeInvocation::LocalProfileApply
    } else {
        OfficeInvocation::LocalProfileShow
    };
    let input = serde_json::to_vec(&input)?;
    if input.len() > 4096 {
        return Err(invalid_pairing());
    }
    let bytes = match invoke_json(executable, operation, &input, deadline) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
            return Ok(Err(OfficeError::Busy));
        }
        Err(error) => return Err(error),
    };
    decode_local_profile_reply(&bytes, identity_id, edit.is_some())
}

pub fn invoke_local_office_prop(
    executable: &Path,
    operation: OfficeInvocation,
    input: &serde_json::Value,
    deadline: Instant,
) -> io::Result<Result<serde_json::Value, OfficeError>> {
    if !matches!(
        operation,
        OfficeInvocation::LocalPropValidate
            | OfficeInvocation::LocalPropInstall
            | OfficeInvocation::LocalPropRemove
            | OfficeInvocation::LocalPropList
            | OfficeInvocation::LocalPropShow
    ) {
        return Err(invalid_pairing());
    }
    let input = serde_json::to_vec(input)?;
    if input.len() > tmt_office_model::codec::office_prop::PROTOCOL_INPUT_LIMIT {
        return Err(invalid_pairing());
    }
    let bytes = match invoke_bytes_bounded(executable, operation, &input, deadline, 131_072) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
            return Ok(Err(OfficeError::Busy));
        }
        Err(error) => return Err(error),
    };
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| invalid_pairing())?;
    let object = value.as_object().ok_or_else(invalid_pairing)?;
    if object.len() == 1
        && let Some(error) = value["error"].as_str().and_then(OfficeError::parse)
    {
        return Ok(Err(error));
    }
    if !valid_local_prop_reply(operation, &value) {
        return Err(invalid_pairing());
    }
    Ok(Ok(value))
}

pub fn invoke_local_office_avatar(
    executable: &Path,
    operation: OfficeInvocation,
    input: &serde_json::Value,
    deadline: Instant,
) -> io::Result<Result<serde_json::Value, OfficeError>> {
    if !matches!(
        operation,
        OfficeInvocation::LocalAvatarValidate
            | OfficeInvocation::LocalAvatarInstall
            | OfficeInvocation::LocalAvatarRemove
            | OfficeInvocation::LocalAvatarList
            | OfficeInvocation::LocalAvatarShow
    ) {
        return Err(invalid_pairing());
    }
    let input = serde_json::to_vec(input)?;
    if input.len() > tmt_office_model::codec::office_avatar::PROTOCOL_INPUT_LIMIT {
        return Err(invalid_pairing());
    }
    let bytes = match invoke_bytes_bounded(
        executable,
        operation,
        &input,
        deadline,
        tmt_office_model::codec::office_avatar::PROTOCOL_OUTPUT_LIMIT,
    ) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
            return Ok(Err(OfficeError::Busy));
        }
        Err(error) => return Err(error),
    };
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| invalid_pairing())?;
    let object = value.as_object().ok_or_else(invalid_pairing)?;
    if object.len() == 1
        && let Some(error) = value["error"].as_str().and_then(OfficeError::parse)
    {
        return Ok(Err(error));
    }
    if !valid_local_avatar_reply(operation, &value) {
        return Err(invalid_pairing());
    }
    Ok(Ok(value))
}

fn valid_local_avatar_reply(operation: OfficeInvocation, value: &serde_json::Value) -> bool {
    match operation {
        OfficeInvocation::LocalAvatarValidate => valid_avatar_pack_projection(value, &[]),
        OfficeInvocation::LocalAvatarInstall => {
            valid_avatar_pack_projection(
                value,
                &["builtin", "catalogRevision", "installedAtMs", "changed"],
            ) && safe_u64(&value["catalogRevision"])
                && valid_install_time(value)
                && (value["builtin"] != true || value["changed"] == false)
                && value["changed"].is_boolean()
        }
        OfficeInvocation::LocalAvatarRemove => {
            exact_object(value, &["digest", "catalogRevision", "changed"])
                && valid_avatar_digest(&value["digest"])
                && safe_u64(&value["catalogRevision"])
                && value["changed"].is_boolean()
        }
        OfficeInvocation::LocalAvatarShow => {
            valid_avatar_pack_projection(value, &["builtin", "catalogRevision", "installedAtMs"])
                && safe_u64(&value["catalogRevision"])
                && valid_install_time(value)
        }
        OfficeInvocation::LocalAvatarList => {
            exact_object(
                value,
                &[
                    "catalogRevision",
                    "builtins",
                    "packs",
                    "excluded",
                    "nextCursor",
                ],
            ) && safe_u64(&value["catalogRevision"])
                && value["builtins"].as_array().is_some_and(|items| {
                    items.len() <= tmt_office_model::codec::office_avatar::builtin_packs().len()
                        && items.iter().all(|item| {
                            valid_avatar_pack_projection(
                                item,
                                &["builtin", "catalogRevision", "installedAtMs"],
                            ) && item["catalogRevision"] == value["catalogRevision"]
                                && item["builtin"] == true
                                && valid_install_time(item)
                        })
                })
                && value["packs"].as_array().is_some_and(|items| {
                    items.len() <= 20
                        && items.iter().all(|item| {
                            valid_avatar_pack_projection(
                                item,
                                &["builtin", "catalogRevision", "installedAtMs"],
                            ) && item["catalogRevision"] == value["catalogRevision"]
                                && item["builtin"] == false
                                && valid_install_time(item)
                        })
                })
                && value["excluded"].as_array().is_some_and(|items| {
                    items.len() <= 20
                        && items.iter().all(|item| {
                            exact_object(item, &["digest", "reason"])
                                && valid_avatar_digest(&item["digest"])
                                && matches!(
                                    item["reason"].as_str(),
                                    Some("oversized" | "digestMismatch" | "invalidDocument")
                                )
                        })
                })
                && prop_cursor_value(&value["nextCursor"])
        }
        _ => false,
    }
}

fn valid_avatar_pack_projection(value: &serde_json::Value, extra: &[&str]) -> bool {
    let Some(format) = value["formatVersion"]
        .as_u64()
        .and_then(tmt_office_model::codec::office_avatar::avatar_format)
    else {
        return false;
    };
    let mut fields = vec![
        "digest",
        "formatVersion",
        "label",
        "credit",
        "license",
        "fileBytes",
        "cellCount",
        "avatars",
    ];
    fields.extend_from_slice(extra);
    exact_object(value, &fields)
        && valid_avatar_digest(&value["digest"])
        && valid_indexed_text(&value["label"], tmt_office_model::indexed_art::LABEL_LIMIT)
        && valid_indexed_text(
            &value["credit"],
            tmt_office_model::indexed_art::CREDIT_LIMIT,
        )
        && value["license"]
            .as_str()
            .is_some_and(tmt_office_model::indexed_art::valid_license)
        && value["fileBytes"]
            .as_u64()
            .is_some_and(|bytes| (1..=32 * 1024).contains(&bytes))
        && value["cellCount"].as_u64().is_some_and(|cells| {
            ((format.width * format.height) as u64
                ..=tmt_office_model::codec::office_avatar::PACK_CELL_LIMIT as u64)
                .contains(&cells)
        })
        && value["avatars"].as_array().is_some_and(|avatars| {
            !avatars.is_empty()
                && avatars.len() <= tmt_office_model::codec::office_avatar::PACK_AVATAR_LIMIT
                && value["cellCount"].as_u64()
                    == Some((avatars.len() * format.width * format.height) as u64)
                && avatars
                    .iter()
                    .all(|avatar| valid_avatar_summary(avatar, format))
        })
}
fn valid_avatar_summary(
    value: &serde_json::Value,
    format: tmt_office_model::codec::office_avatar::AvatarFormat,
) -> bool {
    exact_object(value, &["key", "label", "raster"])
        && value["key"]
            .as_str()
            .is_some_and(tmt_office_model::indexed_art::valid_key)
        && valid_indexed_text(&value["label"], tmt_office_model::indexed_art::LABEL_LIMIT)
        && exact_object(&value["raster"], &["width", "height"])
        && value["raster"]["width"].as_u64() == Some(format.width as u64)
        && value["raster"]["height"].as_u64() == Some(format.height as u64)
}
fn valid_avatar_digest(value: &serde_json::Value) -> bool {
    value
        .as_str()
        .and_then(tmt_office_model::codec::office_avatar::parse_pack_digest)
        .is_some()
}

fn valid_local_prop_reply(operation: OfficeInvocation, value: &serde_json::Value) -> bool {
    match operation {
        OfficeInvocation::LocalPropValidate => valid_prop_pack_projection(value, &[]),
        OfficeInvocation::LocalPropInstall => {
            valid_prop_pack_projection(
                value,
                &["builtin", "catalogRevision", "installedAtMs", "changed"],
            ) && value["builtin"].is_boolean()
                && safe_u64(&value["catalogRevision"])
                && valid_install_time(value)
                && value["changed"].is_boolean()
        }
        OfficeInvocation::LocalPropRemove => {
            exact_object(value, &["digest", "catalogRevision", "changed"])
                && valid_prop_digest(&value["digest"])
                && safe_u64(&value["catalogRevision"])
                && value["changed"].is_boolean()
        }
        OfficeInvocation::LocalPropShow => {
            valid_prop_pack_projection(value, &["builtin", "catalogRevision", "installedAtMs"])
                && value["builtin"].is_boolean()
                && safe_u64(&value["catalogRevision"])
                && valid_install_time(value)
        }
        OfficeInvocation::LocalPropList => {
            exact_object(
                value,
                &[
                    "catalogRevision",
                    "builtins",
                    "packs",
                    "excluded",
                    "nextCursor",
                ],
            ) && safe_u64(&value["catalogRevision"])
                && value["builtins"].as_array().is_some_and(|items| {
                    items.len() == tmt_office_model::codec::office_prop::builtin_packs().len()
                        && items.iter().all(|item| {
                            valid_prop_snapshot_projection(item)
                                && item["builtin"].as_bool() == Some(true)
                                && item["installedAtMs"].is_null()
                                && item["catalogRevision"] == value["catalogRevision"]
                        })
                })
                && value["packs"].as_array().is_some_and(|items| {
                    items.len() <= 20
                        && items.iter().all(|item| {
                            valid_prop_snapshot_projection(item)
                                && item["builtin"].as_bool() == Some(false)
                                && item["installedAtMs"].as_u64().is_some_and(|time| time > 0)
                                && item["catalogRevision"] == value["catalogRevision"]
                        })
                })
                && value["excluded"].as_array().is_some_and(|items| {
                    items.len() <= 20
                        && items.iter().all(|item| {
                            exact_object(item, &["digest", "reason"])
                                && valid_prop_digest(&item["digest"])
                                && matches!(
                                    item["reason"].as_str(),
                                    Some("oversized" | "digestMismatch" | "invalidDocument")
                                )
                        })
                })
                && prop_cursor_value(&value["nextCursor"])
        }
        _ => false,
    }
}

fn valid_prop_snapshot_projection(value: &serde_json::Value) -> bool {
    valid_prop_pack_projection(value, &["builtin", "catalogRevision", "installedAtMs"])
        && value["builtin"].is_boolean()
        && safe_u64(&value["catalogRevision"])
        && (value["installedAtMs"].is_null() || safe_u64(&value["installedAtMs"]))
}

fn valid_install_time(value: &serde_json::Value) -> bool {
    match value["builtin"].as_bool() {
        Some(true) => value["installedAtMs"].is_null(),
        Some(false) => value["installedAtMs"].as_u64().is_some_and(|time| time > 0),
        None => false,
    }
}

fn valid_prop_pack_projection(value: &serde_json::Value, extra: &[&str]) -> bool {
    let mut fields = vec![
        "digest",
        "formatVersion",
        "label",
        "credit",
        "license",
        "fileBytes",
        "pixelCount",
        "props",
    ];
    fields.extend_from_slice(extra);
    exact_object(value, &fields)
        && valid_prop_digest(&value["digest"])
        && matches!(value["formatVersion"].as_u64(), Some(1 | 2))
        && valid_indexed_text(&value["label"], tmt_office_model::indexed_art::LABEL_LIMIT)
        && valid_indexed_text(
            &value["credit"],
            tmt_office_model::indexed_art::CREDIT_LIMIT,
        )
        && value["license"]
            .as_str()
            .is_some_and(tmt_office_model::indexed_art::valid_license)
        && value["fileBytes"].as_u64().is_some_and(|bytes| {
            (1..=if value["formatVersion"] == 2 {
                tmt_office_model::codec::office_prop::PACK_INPUT_LIMIT as u64
            } else {
                tmt_office_model::codec::office_prop::V1_PACK_INPUT_LIMIT as u64
            })
                .contains(&bytes)
        })
        && value["pixelCount"].as_u64().is_some_and(|pixels| {
            (1..=if value["formatVersion"] == 2 {
                131_072
            } else {
                65_536
            })
                .contains(&pixels)
        })
        && value["props"].as_array().is_some_and(|props| {
            !props.is_empty()
                && props.len() <= 16
                && props
                    .iter()
                    .all(|prop| valid_prop_summary(prop, value["formatVersion"] == 2))
        })
}

fn valid_prop_summary(value: &serde_json::Value, directional: bool) -> bool {
    let mut keys = vec![
        "key",
        "label",
        "footprint",
        if directional { "frames" } else { "raster" },
    ];
    if let Some(fields) = value.get("customizable") {
        if !directional
            || ![
                serde_json::json!(["tint"]),
                serde_json::json!(["text"]),
                serde_json::json!(["tint", "text"]),
            ]
            .contains(fields)
        {
            return false;
        }
        keys.push("customizable");
    }
    exact_object(value, &keys)
        && value["key"]
            .as_str()
            .is_some_and(tmt_office_model::indexed_art::valid_key)
        && valid_indexed_text(&value["label"], tmt_office_model::indexed_art::LABEL_LIMIT)
        && valid_dimensions(
            &value["footprint"],
            if directional {
                tmt_office_model::office_block::PROP_FOOTPRINT_LIMIT as u64
            } else {
                tmt_office_model::codec::office_prop::FOOTPRINT_LIMIT as u64
            },
        )
        && if directional {
            value["frames"].as_array().is_some_and(|frames| {
                frames.len() == 4 && frames.iter().all(|frame| valid_dimensions(frame, 128))
            })
        } else {
            valid_dimensions(&value["raster"], 64)
        }
}

fn valid_dimensions(value: &serde_json::Value, limit: u64) -> bool {
    exact_object(value, &["width", "height"])
        && value["width"]
            .as_u64()
            .is_some_and(|size| (1..=limit).contains(&size))
        && value["height"]
            .as_u64()
            .is_some_and(|size| (1..=limit).contains(&size))
}

fn valid_prop_digest(value: &serde_json::Value) -> bool {
    value
        .as_str()
        .and_then(tmt_office_model::codec::office_prop::parse_pack_digest)
        .is_some()
}

fn valid_indexed_text(value: &serde_json::Value, limit: usize) -> bool {
    value
        .as_str()
        .is_some_and(|text| tmt_office_model::indexed_art::valid_text(text, limit))
}

fn exact_object(value: &serde_json::Value, fields: &[&str]) -> bool {
    value.as_object().is_some_and(|object| {
        object.len() == fields.len() && fields.iter().all(|field| object.contains_key(*field))
    })
}

fn decode_local_profile_reply(
    bytes: &[u8],
    identity_id: &str,
    editing: bool,
) -> io::Result<Result<serde_json::Value, OfficeError>> {
    if bytes.len() > 4096 {
        return Err(invalid_pairing());
    }
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| invalid_pairing())?;
    let object = value.as_object().ok_or_else(invalid_pairing)?;
    if object.len() == 1
        && let Some(error) = value["error"].as_str().and_then(OfficeError::parse)
    {
        return Ok(Err(error));
    }
    let mut expected = vec![
        "identityId",
        "identityName",
        "exists",
        "revision",
        "profile",
        "updatedAtMs",
        "catalog",
    ];
    if editing {
        expected.push("changed");
    }
    if object.len() != expected.len()
        || !expected.iter().all(|key| object.contains_key(*key))
        || value["identityId"].as_str() != Some(identity_id)
        || value["identityName"].as_str().is_none_or(str::is_empty)
    {
        return Err(invalid_pairing());
    }
    tmt_office_model::codec::office_profile_wire::decode_value(value["profile"].clone())
        .map_err(|_| invalid_pairing())?;
    let expected_catalog = serde_json::json!({
        "hairStyles": tmt_office_model::office_profile::HAIR_STYLES,
        "hairColors": tmt_office_model::office_profile::HAIR_COLORS,
        "skinTones": tmt_office_model::office_profile::SKIN_TONES,
        "shirtColors": tmt_office_model::office_profile::SHIRT_COLORS,
    });
    if value["catalog"] != expected_catalog {
        return Err(invalid_pairing());
    }
    let exists = value["exists"].as_bool().ok_or_else(invalid_pairing)?;
    let revision = value["revision"].as_u64().ok_or_else(invalid_pairing)?;
    if editing && value["changed"].as_bool().is_none() {
        return Err(invalid_pairing());
    }
    if revision > tmt_office_model::office_profile::MAX_REVISION
        || value["updatedAtMs"].as_u64().is_some_and(|timestamp| {
            timestamp == 0 || timestamp > tmt_core::limits::MAX_JS_SAFE_INTEGER
        })
        || (exists && (revision == 0 || value["updatedAtMs"].as_u64().is_none()))
        || (!exists && (revision != 0 || !value["updatedAtMs"].is_null()))
    {
        return Err(invalid_pairing());
    }
    Ok(Ok(value))
}

pub fn invoke_office_board(
    executable: &Path,
    operation: OfficeInvocation,
    input: &[u8],
    deadline: Instant,
) -> io::Result<Result<serde_json::Value, tmt_office_model::office_board::BoardErrorCode>> {
    if !matches!(
        operation,
        OfficeInvocation::BoardPost
            | OfficeInvocation::BoardList
            | OfficeInvocation::BoardShow
            | OfficeInvocation::BoardReply
            | OfficeInvocation::BoardEdit
            | OfficeInvocation::BoardDelete
            | OfficeInvocation::BoardCategories
    ) || input.len() > crate::office_board::BOARD_WIRE_LIMIT
    {
        return Err(invalid_pairing());
    }
    let capability_launch =
        native_install::with_active_product(Product::Office, executable, |installed| {
            start_selected(
                &installed.active_executable,
                OfficeInvocation::Capabilities,
                &[],
                deadline,
                OFFICE_PROTOCOL_OUTPUT_LIMIT,
            )
            .map(|running| (running, installed.release_id()))
        })
        .map_err(unsupported_office)?;
    let (capability_process, pinned_release) = capability_launch.map_err(unsupported_office)?;
    let capability_bytes = finish_selected(capability_process)
        .map_err(|error| io::Error::new(io::ErrorKind::Unsupported, error))?;
    require_board_capability(&capability_bytes, || ())?;
    let board_launch = native_install::with_active_product(
        Product::Office,
        executable,
        |installed| {
            if installed.release_id() != pinned_release {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "Office release changed after capability verification; refusing board dispatch.",
                ));
            }
            start_selected(
                &installed.active_executable,
                operation,
                input,
                deadline,
                crate::office_board::BOARD_OUTPUT_LIMIT,
            )
        },
    )
    .map_err(unsupported_office)?;
    let board_process = board_launch.map_err(unsupported_office)?;
    let bytes = finish_selected(board_process)?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| invalid_pairing())?;
    if value.as_object().is_some_and(|object| object.len() == 1)
        && let Some(code) = value["error"].as_str()
    {
        let error = match code {
            "BOARD_INVALID" => tmt_office_model::office_board::BoardErrorCode::Invalid,
            "BOARD_THREAD_NOT_FOUND" => {
                tmt_office_model::office_board::BoardErrorCode::ThreadNotFound
            }
            "BOARD_ENTRY_NOT_FOUND" => {
                tmt_office_model::office_board::BoardErrorCode::EntryNotFound
            }
            "BOARD_REVISION_CONFLICT" => {
                tmt_office_model::office_board::BoardErrorCode::RevisionConflict
            }
            "BOARD_FORBIDDEN" => tmt_office_model::office_board::BoardErrorCode::Forbidden,
            "BOARD_IDEMPOTENCY_CONFLICT" => {
                tmt_office_model::office_board::BoardErrorCode::IdempotencyConflict
            }
            "BOARD_CURSOR_INVALID" => tmt_office_model::office_board::BoardErrorCode::CursorInvalid,
            "BOARD_CURSOR_STALE" => tmt_office_model::office_board::BoardErrorCode::CursorStale,
            "STORAGE_ERROR" => tmt_office_model::office_board::BoardErrorCode::Storage,
            _ => return Err(invalid_pairing()),
        };
        return Ok(Err(error));
    }
    if !valid_board_success(operation, &value) {
        return Err(invalid_pairing());
    }
    Ok(Ok(value))
}

fn exact_keys(value: &serde_json::Value, keys: &[&str]) -> bool {
    value
        .as_object()
        .is_some_and(|o| o.len() == keys.len() && keys.iter().all(|k| o.contains_key(*k)))
}
fn uuid_value(value: &serde_json::Value) -> bool {
    value.as_str().is_some_and(|s| {
        uuid::Uuid::parse_str(s)
            .ok()
            .is_some_and(|u| u.to_string() == s)
    })
}
fn safe_u64(value: &serde_json::Value) -> bool {
    value
        .as_u64()
        .is_some_and(|v| v <= tmt_core::limits::MAX_JS_SAFE_INTEGER)
}
fn positive_revision(value: &serde_json::Value) -> bool {
    safe_u64(value) && value.as_u64() != Some(0)
}
fn cursor_value(value: &serde_json::Value) -> bool {
    value.is_null()
        || value.as_str().is_some_and(|s| {
            !s.is_empty() && s.len() <= tmt_office_model::office_board::CURSOR_MAX_BYTES
        })
}
fn prop_cursor_value(value: &serde_json::Value) -> bool {
    value.is_null()
        || value.as_str().is_some_and(|s| {
            !s.is_empty()
                && s.len() <= tmt_office_model::codec::office_prop::CATALOG_CURSOR_MAX_BYTES
        })
}
fn category_value(value: &serde_json::Value) -> bool {
    exact_keys(value, &["kind"]) && value["kind"] == "general"
        || exact_keys(value, &["kind", "repositoryId"])
            && value["kind"] == "repository"
            && value["repositoryId"]
                .as_str()
                .is_some_and(tmt_office_model::office_board::valid_repository_id)
        || exact_keys(value, &["kind", "roomId"])
            && value["kind"] == "room"
            && value["roomId"]
                .as_str()
                .is_some_and(tmt_core::dispatch::canonical_id)
}
fn actor_value(value: &serde_json::Value) -> bool {
    exact_keys(value, &["kind"]) && value["kind"] == "owner"
        || exact_keys(value, &["kind", "identityId", "name"])
            && value["kind"] == "identity"
            && uuid_value(&value["identityId"])
            && value["name"].as_str().is_some_and(|s| {
                !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control)
            })
}
fn entry_value(value: &serde_json::Value, summary: bool) -> bool {
    let Some(o) = value.as_object() else {
        return false;
    };
    let required = [
        "id",
        "threadId",
        "category",
        "author",
        "revision",
        "deleted",
        "createdAtMs",
        "updatedAtMs",
    ];
    if !required.iter().all(|k| o.contains_key(*k))
        || !uuid_value(&value["id"])
        || !uuid_value(&value["threadId"])
        || !category_value(&value["category"])
        || !actor_value(&value["author"])
        || !positive_revision(&value["revision"])
        || value["deleted"].as_bool().is_none()
        || !safe_u64(&value["createdAtMs"])
        || !safe_u64(&value["updatedAtMs"])
    {
        return false;
    }
    let optional = if summary {
        &["title", "replyCount", "activitySequence"][..]
    } else {
        &["title", "body"][..]
    };
    if o.keys()
        .any(|k| !required.contains(&k.as_str()) && !optional.contains(&k.as_str()))
    {
        return false;
    }
    if value["deleted"] == true && (o.contains_key("title") || o.contains_key("body")) {
        return false;
    }
    if value["deleted"] == false
        && if summary {
            !o.contains_key("title")
        } else {
            !o.contains_key("body")
        }
    {
        return false;
    }
    if o.get("title").is_some_and(|v| {
        v.as_str().is_none_or(|s| {
            s.is_empty()
                || s.len() > tmt_office_model::office_board::TITLE_MAX_BYTES
                || s.chars().any(char::is_control)
        })
    }) {
        return false;
    }
    let body_max = if value["id"] == value["threadId"] {
        tmt_office_model::office_board::ROOT_BODY_MAX_BYTES
    } else {
        tmt_office_model::office_board::REPLY_BODY_MAX_BYTES
    };
    if o.get("body").is_some_and(|v| {
        v.as_str().is_none_or(|s| {
            s.is_empty()
                || s.len() > body_max
                || s.chars().any(|c| c.is_control() && c != '\n' && c != '\t')
        })
    }) {
        return false;
    }
    (!summary
        || value["id"] == value["threadId"]
            && safe_u64(&value["replyCount"])
            && positive_revision(&value["activitySequence"]))
        && o.get("title").is_none_or(|v| v.as_str().is_some())
        && o.get("body").is_none_or(|v| v.as_str().is_some())
}
fn valid_board_success(op: OfficeInvocation, value: &serde_json::Value) -> bool {
    match op {
        OfficeInvocation::BoardPost | OfficeInvocation::BoardReply => {
            exact_keys(
                value,
                &["entryId", "threadId", "revision", "created", "operationId"],
            ) && uuid_value(&value["entryId"])
                && uuid_value(&value["threadId"])
                && positive_revision(&value["revision"])
                && value["created"] == true
                && uuid_value(&value["operationId"])
                && if matches!(op, OfficeInvocation::BoardPost) {
                    value["entryId"] == value["threadId"]
                } else {
                    value["entryId"] != value["threadId"]
                }
        }
        OfficeInvocation::BoardEdit => {
            exact_keys(value, &["entryId", "revision", "changed", "operationId"])
                && uuid_value(&value["entryId"])
                && positive_revision(&value["revision"])
                && value["changed"].as_bool().is_some()
                && uuid_value(&value["operationId"])
        }
        OfficeInvocation::BoardDelete => {
            exact_keys(
                value,
                &[
                    "entryId",
                    "revision",
                    "deleted",
                    "changed",
                    "moderated",
                    "operationId",
                ],
            ) && uuid_value(&value["entryId"])
                && positive_revision(&value["revision"])
                && value["deleted"] == true
                && value["changed"].as_bool().is_some()
                && value["moderated"].as_bool().is_some()
                && uuid_value(&value["operationId"])
        }
        OfficeInvocation::BoardList => {
            exact_keys(value, &["threads", "nextCursor", "boardRevision"])
                && value["threads"]
                    .as_array()
                    .is_some_and(|a| a.len() <= 50 && a.iter().all(|v| entry_value(v, true)))
                && cursor_value(&value["nextCursor"])
                && safe_u64(&value["boardRevision"])
        }
        OfficeInvocation::BoardShow => {
            exact_keys(value, &["thread", "replies", "nextCursor", "boardRevision"])
                && entry_value(&value["thread"], false)
                && value["thread"]["id"] == value["thread"]["threadId"]
                && (value["thread"]["deleted"] == true
                    || value["thread"].as_object().unwrap().contains_key("title"))
                && value["replies"].as_array().is_some_and(|a| {
                    a.len() <= 50
                        && a.iter().all(|v| {
                            entry_value(v, false)
                                && v["threadId"] == value["thread"]["id"]
                                && v["id"] != v["threadId"]
                                && !v.as_object().unwrap().contains_key("title")
                                && v["category"] == value["thread"]["category"]
                        })
                })
                && cursor_value(&value["nextCursor"])
                && safe_u64(&value["boardRevision"])
        }
        OfficeInvocation::BoardCategories => {
            exact_keys(value, &["categories", "nextCursor", "boardRevision"])
                && value["categories"]
                    .as_array()
                    .is_some_and(|a| a.len() <= 50 && a.iter().all(category_value))
                && cursor_value(&value["nextCursor"])
                && safe_u64(&value["boardRevision"])
        }
        _ => false,
    }
}

fn require_board_capability<T>(bytes: &[u8], dispatch: impl FnOnce() -> T) -> io::Result<T> {
    decode_office_capabilities(bytes)
        .map_err(|message| io::Error::new(io::ErrorKind::Unsupported, message))?;
    Ok(dispatch())
}

pub fn invoke_office_block(
    executable: &Path,
    call: &PairingCall<'_>,
    block_id: Option<&str>,
    edit: Option<(&tmt_office_model::office_block::BlockLayout, u64)>,
    deadline: Instant,
) -> io::Result<Result<tmt_office_model::codec::office_block::BlockSnapshot, OfficeError>> {
    let mut input = serde_json::json!({"world":call.world,"identityId":call.identity_id,"emulator":call.emulator,"readOnly":false,"blockId":block_id});
    let operation = if let Some((layout, revision)) = edit {
        if revision >= tmt_office_model::office_block::MAX_REVISION {
            return Ok(Err(OfficeError::LayoutInvalid));
        }
        input["layout"] = tmt_office_model::codec::office_block::layout_value(layout);
        input["expectedRevision"] = serde_json::json!(revision);
        OfficeInvocation::BlockApply
    } else {
        OfficeInvocation::BlockShow
    };
    let input = serde_json::to_vec(&input)?;
    if input.len() > 4096 {
        return Err(invalid_pairing());
    }
    let bytes = match invoke_json(executable, operation, &input, deadline) {
        Ok(bytes) => bytes,
        // The installer lock failed before the child was launched. Command
        // failures after launch are wrapped as Other by invoke_json and must
        // remain uncertain; never advertise a possibly committed write as busy.
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
            return Ok(Err(OfficeError::Busy));
        }
        Err(error) => return Err(error),
    };
    decode_block_reply(&bytes, block_id)
}

fn decode_block_reply(
    bytes: &[u8],
    block_id: Option<&str>,
) -> io::Result<Result<tmt_office_model::codec::office_block::BlockSnapshot, OfficeError>> {
    if bytes.len() > 4096 {
        return Err(invalid_pairing());
    }
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| invalid_pairing())?;
    if value.as_object().is_some_and(|object| object.len() == 1)
        && let Some(error) = value["error"].as_str().and_then(OfficeError::parse)
    {
        return Ok(Err(error));
    }
    let snapshot = tmt_office_model::codec::office_block::BlockSnapshot::decode(bytes)
        .map_err(|_| invalid_pairing())?;
    if block_id.is_some_and(|id| id != snapshot.block_id) {
        return Err(invalid_pairing());
    }
    Ok(Ok(snapshot))
}

pub fn invoke_office_sync(
    executable: &Path,
    deadline: Instant,
) -> io::Result<Result<OfficeSyncReport, OfficeError>> {
    let bytes = invoke_json(executable, OfficeInvocation::Sync, b"{}", deadline)?;
    decode_sync_reply(&bytes)
}

fn invoke_json(
    executable: &Path,
    operation: OfficeInvocation,
    input: &[u8],
    deadline: Instant,
) -> io::Result<Vec<u8>> {
    invoke_bytes_bounded(executable, operation, input, deadline, 4096)
}

fn invoke_bytes_bounded(
    executable: &Path,
    operation: OfficeInvocation,
    input: &[u8],
    deadline: Instant,
    max_output_bytes: usize,
) -> io::Result<Vec<u8>> {
    let args = operation.arguments().map(OsString::from);
    let running = native_install::with_active_product(Product::Office, executable, |installed| {
        UnixCommandRunner
            .start(CommandRequest {
                program: installed.active_executable.as_os_str(),
                args: &args,
                input,
                deadline,
                max_output_bytes,
            })
            .map_err(io::Error::other)
    })??;
    let output = running.wait().map_err(io::Error::other)?;
    if !output.stderr.is_empty() {
        return Err(invalid_pairing());
    }
    Ok(output.stdout)
}

fn start_selected(
    executable: &Path,
    operation: OfficeInvocation,
    input: &[u8],
    deadline: Instant,
    max_output_bytes: usize,
) -> io::Result<RunningCommand> {
    let args = operation.arguments().map(OsString::from);
    UnixCommandRunner
        .start(CommandRequest {
            program: executable.as_os_str(),
            args: &args,
            input,
            deadline,
            max_output_bytes,
        })
        .map_err(io::Error::other)
}

fn finish_selected(running: RunningCommand) -> io::Result<Vec<u8>> {
    let output = running.wait().map_err(io::Error::other)?;
    if !output.stderr.is_empty() {
        return Err(invalid_pairing());
    }
    Ok(output.stdout)
}

fn unsupported_office(error: io::Error) -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, error)
}

fn decode_sync_reply(bytes: &[u8]) -> io::Result<Result<OfficeSyncReport, OfficeError>> {
    if bytes.len() > 4096 {
        return Err(invalid_pairing());
    }
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| invalid_pairing())?;
    let object = value.as_object().ok_or_else(invalid_pairing)?;
    if object.len() == 1
        && let Some(error) = value["error"].as_str().and_then(OfficeError::parse)
    {
        return Ok(Err(error));
    }
    if object.len() != 4
        || !["completed", "failed", "pending", "failureCode"]
            .iter()
            .all(|key| object.contains_key(*key))
    {
        return Err(invalid_pairing());
    }
    let completed = value["completed"].as_u64().ok_or_else(invalid_pairing)?;
    let failed = value["failed"].as_u64().ok_or_else(invalid_pairing)?;
    let pending = value["pending"].as_u64().ok_or_else(invalid_pairing)?;
    let failure = match &value["failureCode"] {
        serde_json::Value::Null => None,
        serde_json::Value::String(code) => {
            Some(OfficeError::parse(code).ok_or_else(invalid_pairing)?)
        }
        _ => return Err(invalid_pairing()),
    };
    if completed
        .checked_add(failed)
        .is_none_or(|count| count > OFFICE_HOOK_BATCH_LIMIT as u64)
        || (failed == 0) != failure.is_none()
    {
        return Err(invalid_pairing());
    }
    Ok(Ok(OfficeSyncReport {
        completed,
        failed,
        pending,
        failure,
    }))
}

fn decode_pairing_reply(bytes: &[u8], world: &str) -> io::Result<PairingReply> {
    if bytes.len() > 4096 {
        return Err(invalid_pairing());
    }
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| invalid_pairing())?;
    let object = value.as_object().ok_or_else(invalid_pairing)?;
    if object.len() == 1 {
        if let Some(exists) = value["blockExists"].as_bool() {
            return Ok(PairingReply::Inspected(exists));
        }
        if let Some(error) = value["error"].as_str().and_then(OfficeError::parse) {
            return Ok(PairingReply::Error(error));
        }
        if let Some(state @ ("unpaired" | "pending" | "credential" | "expired" | "revoked")) =
            value["state"].as_str()
        {
            return Ok(PairingReply::State(state.into()));
        }
    }
    if object.len() == 2
        && value["state"] == "pending"
        && let Some(link) = value["approvalUrl"].as_str().filter(|link| {
            link.starts_with(&format!("{world}/pair#tmt-pair="))
                && !link.chars().any(char::is_control)
        })
    {
        return Ok(PairingReply::Pending(link.into()));
    }
    Err(invalid_pairing())
}

fn invalid_pairing() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Invalid Office pairing response.",
    )
}

/// Local compatibility inspection only: does not pair, authenticate or start a service.
pub fn probe_office_companion(executable: &Path) -> io::Result<String> {
    let (running, version) =
        native_install::with_active_product(Product::Office, executable, |installed| {
            start_probe(&installed.active_executable)
                .map(|running| (running, installed.state.version.clone()))
        })??;
    finish_probe(running, &version)
}

pub(crate) fn probe_candidate(
    executable: &Path,
    expected_version: &semver::Version,
) -> io::Result<String> {
    finish_probe(start_probe(executable)?, expected_version)
}

fn start_probe(executable: &Path) -> io::Result<RunningCommand> {
    let args = OfficeInvocation::Probe.arguments().map(OsString::from);
    UnixCommandRunner
        .start(CommandRequest {
            program: executable.as_os_str(),
            args: &args,
            input: &[],
            deadline: Instant::now() + Duration::from_secs(5),
            max_output_bytes: OFFICE_PROTOCOL_OUTPUT_LIMIT,
        })
        .map_err(io::Error::other)
}

fn finish_probe(running: RunningCommand, expected_version: &semver::Version) -> io::Result<String> {
    let output = running.wait().map_err(io::Error::other)?;
    if !output.stderr.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Office handshake produced unexpected diagnostics.",
        ));
    }
    let version = decode_office_probe(&output.stdout)
        .map_err(|message| io::Error::new(io::ErrorKind::InvalidData, message))?;
    if &version != expected_version {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Office executable and installation versions disagree.",
        ));
    }
    Ok(version.to_string())
}

#[cfg(test)]
mod pairing_tests {
    use super::*;

    #[test]
    fn board_capability_prevents_dispatch_until_exactly_supported() {
        let mut dispatched = false;
        assert!(require_board_capability(b"TMT-OFFICE/1\n0.1.0\n", || dispatched = true).is_err());
        assert!(!dispatched);
        require_board_capability(
            tmt_office_model::office_protocol::encode_office_capabilities().as_bytes(),
            || dispatched = true,
        )
        .unwrap();
        assert!(dispatched);
    }

    #[test]
    fn board_success_replies_are_operation_exact_and_fail_closed() {
        let id = "11111111-1111-4111-8111-111111111111";
        let op = "22222222-2222-4222-8222-222222222222";
        let receipt = serde_json::json!({"entryId":id,"threadId":id,"revision":1,"created":true,"operationId":op});
        assert!(valid_board_success(OfficeInvocation::BoardPost, &receipt));
        let mut extra = receipt.clone();
        extra
            .as_object_mut()
            .unwrap()
            .insert("cursor".into(), serde_json::Value::Null);
        assert!(!valid_board_success(OfficeInvocation::BoardPost, &extra));
        assert!(!valid_board_success(OfficeInvocation::BoardEdit, &receipt));
        assert!(!valid_board_success(
            OfficeInvocation::BoardPost,
            &serde_json::json!({"entryId":"bad","threadId":id,"revision":1,"created":true,"operationId":op})
        ));
        let zero = serde_json::json!({"entryId":id,"threadId":id,"revision":0,"created":true,"operationId":op});
        assert!(!valid_board_success(OfficeInvocation::BoardPost, &zero));
        assert!(!valid_board_success(OfficeInvocation::BoardReply, &receipt));
        let mut not_created = receipt.clone();
        not_created["created"] = serde_json::json!(false);
        assert!(!valid_board_success(
            OfficeInvocation::BoardPost,
            &not_created
        ));
        assert!(!category_value(
            &serde_json::json!({"kind":"repository","repositoryId":"Bad Host/repo"})
        ));
        assert!(category_value(
            &serde_json::json!({"kind":"room","roomId":id})
        ));
        assert!(!category_value(
            &serde_json::json!({"kind":"room","roomId":"Review"})
        ));
        assert!(!category_value(
            &serde_json::json!({"kind":"room","roomId":id,"repositoryId":"example.com/repo"})
        ));
        assert!(!actor_value(
            &serde_json::json!({"kind":"identity","identityId":id,"name":"bad\nname"})
        ));
        let summary = serde_json::json!({"id":id,"threadId":id,"category":{"kind":"general"},"author":{"kind":"owner"},"revision":1,"deleted":false,"createdAtMs":1,"updatedAtMs":1,"title":"x".repeat(tmt_office_model::office_board::TITLE_MAX_BYTES+1),"replyCount":0,"activitySequence":1});
        assert!(!entry_value(&summary, true));
        assert!(!valid_board_success(
            OfficeInvocation::BoardCategories,
            &serde_json::json!({"categories":[],"nextCursor":"x".repeat(tmt_office_model::office_board::CURSOR_MAX_BYTES+1),"boardRevision":1})
        ));
    }

    #[test]
    fn board_show_entries_enforce_root_reply_tombstone_and_content_invariants() {
        let root = "11111111-1111-4111-8111-111111111111";
        let reply = "22222222-2222-4222-8222-222222222222";
        let entry = |id: &str, thread: &str| serde_json::json!({"id":id,"threadId":thread,"category":{"kind":"general"},"author":{"kind":"owner"},"revision":1,"deleted":false,"createdAtMs":1,"updatedAtMs":1,"body":"body"});
        let mut root_entry = entry(root, root);
        root_entry
            .as_object_mut()
            .unwrap()
            .insert("title".into(), serde_json::json!("title"));
        let reply_entry = entry(reply, root);
        let valid = serde_json::json!({"thread":root_entry,"replies":[reply_entry],"nextCursor":null,"boardRevision":1});
        assert!(valid_board_success(OfficeInvocation::BoardShow, &valid));
        for invalid in [
            {
                let mut v = valid.clone();
                v["thread"].as_object_mut().unwrap().remove("title");
                v
            },
            {
                let mut v = valid.clone();
                v["replies"][0].as_object_mut().unwrap().remove("body");
                v
            },
            {
                let mut v = valid.clone();
                v["replies"][0]["category"] =
                    serde_json::json!({"kind":"repository","repositoryId":"github.com/O/R"});
                v
            },
            {
                let mut v = valid.clone();
                v["thread"]["revision"] = serde_json::json!(0);
                v
            },
            {
                let mut v = valid.clone();
                v["replies"][0]["body"] = serde_json::json!(
                    "x".repeat(tmt_office_model::office_board::REPLY_BODY_MAX_BYTES + 1)
                );
                v
            },
            {
                let mut v = valid.clone();
                v["thread"]["deleted"] = serde_json::json!(true);
                v
            },
        ] {
            assert!(!valid_board_success(OfficeInvocation::BoardShow, &invalid));
        }
    }

    #[test]
    fn local_prop_replies_are_exact_and_bounded() {
        let pack = serde_json::json!({
            "digest":"sha256:1111111111111111111111111111111111111111111111111111111111111111",
            "formatVersion":1,
            "label":"Lamp pack",
            "credit":"Test",
            "license":"MIT",
            "fileBytes":128,
            "pixelCount":1,
            "props":[{
                "key":"lamp",
                "label":"Lamp",
                "footprint":{"width":1,"height":1},
                "raster":{"width":1,"height":1}
            }]
        });
        assert!(valid_local_prop_reply(
            OfficeInvocation::LocalPropValidate,
            &pack
        ));
        let mut installed = pack.clone();
        installed["builtin"] = serde_json::json!(false);
        installed["catalogRevision"] = serde_json::json!(1);
        installed["installedAtMs"] = serde_json::json!(1);
        installed["changed"] = serde_json::json!(true);
        assert!(valid_local_prop_reply(
            OfficeInvocation::LocalPropInstall,
            &installed
        ));
        installed["unexpected"] = serde_json::json!(true);
        assert!(!valid_local_prop_reply(
            OfficeInvocation::LocalPropInstall,
            &installed
        ));
        let list = serde_json::json!({
            "catalogRevision":0,
            "builtins":[{
                "digest":pack["digest"], "formatVersion":1, "label":"Built-ins",
                "credit":"TMT", "license":"MIT", "fileBytes":128, "pixelCount":1,
                "props":pack["props"], "builtin":true, "catalogRevision":0,
                "installedAtMs":null
            }, {
                "digest":tmt_office_model::codec::office_prop::WORKSHOP_DIGEST, "formatVersion":2, "label":"Workshop",
                "credit":"TMT", "license":"MIT", "fileBytes":512, "pixelCount":4,
                "props":[{"key":"desk","label":"Desk","footprint":{"width":1,"height":1},
                    "frames":[{"width":1,"height":1},{"width":1,"height":1},{"width":1,"height":1},{"width":1,"height":1}]}],
                "builtin":true, "catalogRevision":0, "installedAtMs":null
            }, {
                "digest":tmt_office_model::codec::office_prop::COMMONS_DIGEST, "formatVersion":2, "label":"Commons",
                "credit":"TMT", "license":"MIT", "fileBytes":512, "pixelCount":4,
                "props":[{"key":"noticeboard","label":"Noticeboard","footprint":{"width":12,"height":8},
                    "frames":[{"width":96,"height":64},{"width":64,"height":96},{"width":96,"height":64},{"width":64,"height":96}]}],
                "builtin":true, "catalogRevision":0, "installedAtMs":null
            }, {
                "digest":tmt_office_model::codec::office_prop::WHITEBOARD_DIGEST, "formatVersion":2, "label":"Whiteboard",
                "credit":"TMT", "license":"MIT", "fileBytes":512, "pixelCount":4,
                "props":[{"key":"whiteboard","label":"Whiteboard","footprint":{"width":12,"height":8},
                    "frames":[{"width":96,"height":64},{"width":64,"height":96},{"width":96,"height":64},{"width":64,"height":96}]}],
                "builtin":true, "catalogRevision":0, "installedAtMs":null
            }, {
                "digest":tmt_office_model::codec::office_prop::BROADCASTER_DIGEST, "formatVersion":2, "label":"Broadcaster",
                "credit":"TMT", "license":"MIT", "fileBytes":512, "pixelCount":4,
                "props":[{"key":"broadcast-station","label":"Broadcast station","footprint":{"width":8,"height":8},
                    "frames":[{"width":64,"height":64},{"width":64,"height":64},{"width":64,"height":64},{"width":64,"height":64}]}],
                "builtin":true, "catalogRevision":0, "installedAtMs":null
            }, {
                "digest":tmt_office_model::codec::office_prop::STUDY_DIGEST, "formatVersion":2, "label":"Study",
                "credit":"TMT", "license":"MIT", "fileBytes":512, "pixelCount":4,
                "props":[{"key":"oak-bookcase","label":"Bookcase","footprint":{"width":12,"height":12},
                    "frames":[{"width":96,"height":96},{"width":96,"height":96},{"width":96,"height":96},{"width":96,"height":96}]}],
                "builtin":true, "catalogRevision":0, "installedAtMs":null
            }, {
                "digest":tmt_office_model::codec::office_prop::WALL_DIGEST, "formatVersion":2, "label":"Walls",
                "credit":"TMT", "license":"CC0-1.0", "fileBytes":512, "pixelCount":4,
                "props":[{"key":"observatory-window","label":"Window","footprint":{"width":12,"height":8},
                    "frames":[{"width":96,"height":64},{"width":64,"height":96},{"width":96,"height":64},{"width":64,"height":96}]}],
                "builtin":true, "catalogRevision":0, "installedAtMs":null
            }, {
                "digest":tmt_office_model::codec::office_prop::MODULAR_WORKSTATION_DIGEST, "formatVersion":2, "label":"Modular workstation",
                "credit":"TMT", "license":"MIT", "fileBytes":283399, "pixelCount":129280,
                "props":[{"key":"workstation-chair","label":"Rolling chair","footprint":{"width":8,"height":8},
                    "frames":[{"width":64,"height":64},{"width":64,"height":64},{"width":64,"height":64},{"width":64,"height":64}]}],
                "builtin":true, "catalogRevision":0, "installedAtMs":null
            }, {
                "digest":tmt_office_model::codec::office_prop::MODULAR_MOUNTED_DIGEST, "formatVersion":2, "label":"Modular wall objects",
                "credit":"TMT", "license":"MIT", "fileBytes":256962, "pixelCount":116736,
                "props":[{"key":"mounted-window","label":"Celestial window","footprint":{"width":16,"height":16},
                    "frames":[{"width":128,"height":128},{"width":128,"height":128},{"width":128,"height":128},{"width":128,"height":128}]}],
                "builtin":true, "catalogRevision":0, "installedAtMs":null
            }, {
                "digest":tmt_office_model::codec::office_prop::MODULAR_LOUNGE_DIGEST, "formatVersion":2, "label":"Modular lounge",
                "credit":"TMT", "license":"MIT", "fileBytes":272183, "pixelCount":123904,
                "props":[{"key":"lounge-sofa","label":"Velvet sofa","footprint":{"width":16,"height":16},
                    "frames":[{"width":128,"height":128},{"width":128,"height":128},{"width":128,"height":128},{"width":128,"height":128}]}],
                "builtin":true, "catalogRevision":0, "installedAtMs":null
            }, {
                "digest":tmt_office_model::codec::office_prop::MODULAR_FACILITIES_DIGEST, "formatVersion":2, "label":"Modular facilities",
                "credit":"TMT", "license":"MIT", "fileBytes":259019, "pixelCount":118784,
                "props":[{"key":"lobby-radio","label":"Radio","footprint":{"width":8,"height":8},
                    "frames":[{"width":64,"height":64},{"width":64,"height":64},{"width":64,"height":64},{"width":64,"height":64}]}],
                "builtin":true, "catalogRevision":0, "installedAtMs":null
            }, {
                "digest":tmt_office_model::codec::office_prop::MODULAR_RECEPTION_DIGEST, "formatVersion":2, "label":"Modular reception",
                "credit":"TMT", "license":"MIT", "fileBytes":222410, "pixelCount":102400,
                "props":[{"key":"reception-armchair","label":"Reception armchair","footprint":{"width":12,"height":12},
                    "frames":[{"width":96,"height":96},{"width":96,"height":96},{"width":96,"height":96},{"width":96,"height":96}]}],
                "builtin":true, "catalogRevision":0, "installedAtMs":null
            }, {
                "digest":tmt_office_model::codec::office_prop::DIRECTIONAL_WORKSTATION_DIGEST, "formatVersion":2, "label":"Directional workstation",
                "credit":"TMT", "license":"MIT", "fileBytes":512, "pixelCount":4,
                "props":[{"key":"workstation-desk","label":"Furniture","footprint":{"width":16,"height":16},
                    "frames":[{"width":128,"height":128},{"width":128,"height":128},{"width":128,"height":128},{"width":128,"height":128}]}],
                "builtin":true, "catalogRevision":0, "installedAtMs":null
            }, {
                "digest":tmt_office_model::codec::office_prop::DIRECTIONAL_LOUNGE_DIGEST, "formatVersion":2, "label":"Directional lounge",
                "credit":"TMT", "license":"MIT", "fileBytes":512, "pixelCount":4,
                "props":[{"key":"lounge-sofa","label":"Furniture","footprint":{"width":16,"height":16},
                    "frames":[{"width":128,"height":128},{"width":128,"height":128},{"width":128,"height":128},{"width":128,"height":128}]}],
                "builtin":true, "catalogRevision":0, "installedAtMs":null
            }, {
                "digest":tmt_office_model::codec::office_prop::DIRECTIONAL_RECEPTION_DIGEST, "formatVersion":2, "label":"Directional reception",
                "credit":"TMT", "license":"MIT", "fileBytes":512, "pixelCount":4,
                "props":[{"key":"reception-armchair","label":"Furniture","footprint":{"width":12,"height":12},
                    "frames":[{"width":96,"height":96},{"width":96,"height":96},{"width":96,"height":96},{"width":96,"height":96}]}],
                "builtin":true, "catalogRevision":0, "installedAtMs":null
            }, {
                "digest":tmt_office_model::codec::office_prop::DIRECTIONAL_FACILITIES_DIGEST, "formatVersion":2, "label":"Directional facilities",
                "credit":"TMT", "license":"MIT", "fileBytes":512, "pixelCount":4,
                "props":[{"key":"lobby-radio","label":"Furniture","footprint":{"width":8,"height":8},
                    "frames":[{"width":64,"height":64},{"width":64,"height":64},{"width":64,"height":64},{"width":64,"height":64}]}],
                "builtin":true, "catalogRevision":0, "installedAtMs":null
            }],
            "packs":[], "excluded":[], "nextCursor":null
        });
        assert!(valid_local_prop_reply(
            OfficeInvocation::LocalPropList,
            &list
        ));
        for count in [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 17] {
            let mut invalid = list.clone();
            invalid["builtins"] = serde_json::json!(vec![list["builtins"][0].clone(); count]);
            assert!(!valid_local_prop_reply(
                OfficeInvocation::LocalPropList,
                &invalid
            ));
        }
        let mut cursor_list = list.clone();
        cursor_list["nextCursor"] = serde_json::json!(
            "x".repeat(tmt_office_model::codec::office_prop::CATALOG_CURSOR_MAX_BYTES)
        );
        assert!(valid_local_prop_reply(
            OfficeInvocation::LocalPropList,
            &cursor_list
        ));
        cursor_list["nextCursor"] = serde_json::json!(
            "x".repeat(tmt_office_model::codec::office_prop::CATALOG_CURSOR_MAX_BYTES + 1)
        );
        assert!(!valid_local_prop_reply(
            OfficeInvocation::LocalPropList,
            &cursor_list
        ));
    }

    #[test]
    fn directional_prop_replies_preserve_the_admitted_shape_and_reject_hostile_summaries() {
        let pack = tmt_office_model::codec::office_prop::validate_pack(include_bytes!(
            "../../../../extensions/tmt-office/contracts/prop-pack-v2-sample.tmtprop.json"
        ))
        .unwrap();
        // The companion's validate reply is exactly this model projection; its
        // storage-backed handler and envelope decoding are tested with the Office store.
        let summary: serde_json::Value = serde_json::from_slice(
            &serde_json::to_vec(&tmt_office_model::codec::office_prop::pack_projection(
                &pack,
            ))
            .unwrap(),
        )
        .unwrap();
        assert!(valid_local_prop_reply(
            OfficeInvocation::LocalPropValidate,
            &summary
        ));
        for fields in [
            serde_json::json!(["tint"]),
            serde_json::json!(["text"]),
            serde_json::json!(["tint", "text"]),
        ] {
            let mut customized = summary.clone();
            customized["props"][0]["customizable"] = fields;
            assert!(valid_local_prop_reply(
                OfficeInvocation::LocalPropValidate,
                &customized
            ));
        }
        for fields in [
            serde_json::json!([]),
            serde_json::json!(["tint", "tint"]),
            serde_json::json!(["text", "tint"]),
            serde_json::json!(["script"]),
            serde_json::Value::Null,
        ] {
            let mut customized = summary.clone();
            customized["props"][0]["customizable"] = fields;
            assert!(!valid_local_prop_reply(
                OfficeInvocation::LocalPropValidate,
                &customized
            ));
        }
        let mut largest_footprint = summary.clone();
        largest_footprint["props"][0]["footprint"] = serde_json::json!({"width":16,"height":16});
        assert!(valid_local_prop_reply(
            OfficeInvocation::LocalPropValidate,
            &largest_footprint
        ));
        for kind in [
            "version",
            "missing",
            "extra",
            "null",
            "mixed",
            "dimension",
            "footprint",
            "bytes",
            "cells",
        ] {
            let mut value = summary.clone();
            match kind {
                "version" => value["formatVersion"] = serde_json::json!(1),
                "missing" => {
                    value["props"][0]["frames"].as_array_mut().unwrap().pop();
                }
                "extra" => value["props"][0]["frames"]
                    .as_array_mut()
                    .unwrap()
                    .push(serde_json::json!({"width":1,"height":1})),
                "null" => value["props"][0]["frames"] = serde_json::Value::Null,
                "mixed" => value["props"][0]["raster"] = serde_json::json!({"width":1,"height":1}),
                "dimension" => value["props"][0]["frames"][0]["width"] = serde_json::json!(129),
                "footprint" => value["props"][0]["footprint"]["width"] = serde_json::json!(17),
                "bytes" => {
                    value["fileBytes"] = serde_json::json!(
                        tmt_office_model::codec::office_prop::PACK_INPUT_LIMIT + 1
                    )
                }
                "cells" => value["pixelCount"] = serde_json::json!(131_073),
                _ => unreachable!(),
            }
            assert!(
                !valid_local_prop_reply(OfficeInvocation::LocalPropValidate, &value),
                "{kind}"
            );
        }
    }

    #[test]
    fn local_avatar_replies_use_the_shared_indexed_art_bounds() {
        let pack = serde_json::json!({
            "digest":"sha256:1111111111111111111111111111111111111111111111111111111111111111",
            "formatVersion":1,
            "label":"Avatar pack",
            "credit":"Test",
            "license":"MIT",
            "fileBytes":128,
            "cellCount":384,
            "avatars":[{
                "key":"signal-bot",
                "label":"Signal bot",
                "raster":{"width":16,"height":24}
            }]
        });
        assert!(valid_local_avatar_reply(
            OfficeInvocation::LocalAvatarValidate,
            &pack
        ));
        let mut builtin = pack.clone();
        builtin["builtin"] = serde_json::json!(true);
        builtin["catalogRevision"] = serde_json::json!(0);
        builtin["installedAtMs"] = serde_json::Value::Null;
        assert!(valid_local_avatar_reply(
            OfficeInvocation::LocalAvatarShow,
            &builtin
        ));
        let list = serde_json::json!({
            "catalogRevision":0,"builtins":[builtin.clone()],"packs":[],"excluded":[],"nextCursor":null
        });
        assert!(valid_local_avatar_reply(
            OfficeInvocation::LocalAvatarList,
            &list
        ));
        let mut invalid = list.clone();
        invalid["builtins"][0]["installedAtMs"] = serde_json::json!(1);
        assert!(!valid_local_avatar_reply(
            OfficeInvocation::LocalAvatarList,
            &invalid
        ));
        builtin["changed"] = serde_json::json!(false);
        assert!(valid_local_avatar_reply(
            OfficeInvocation::LocalAvatarInstall,
            &builtin
        ));
        builtin["changed"] = serde_json::json!(true);
        assert!(!valid_local_avatar_reply(
            OfficeInvocation::LocalAvatarInstall,
            &builtin
        ));
        let mut detailed = pack.clone();
        detailed["formatVersion"] = serde_json::json!(2);
        detailed["cellCount"] = serde_json::json!(1536);
        detailed["avatars"][0]["raster"] = serde_json::json!({"width":32,"height":48});
        assert!(valid_local_avatar_reply(
            OfficeInvocation::LocalAvatarValidate,
            &detailed
        ));
        for (path, invalid) in [
            ("/formatVersion", serde_json::json!(3)),
            ("/cellCount", serde_json::json!(384)),
            ("/avatars/0/raster/width", serde_json::json!(64)),
            ("/avatars/0/raster/height", serde_json::json!(24)),
        ] {
            let mut value = detailed.clone();
            *value.pointer_mut(path).unwrap() = invalid;
            assert!(!valid_local_avatar_reply(
                OfficeInvocation::LocalAvatarValidate,
                &value
            ));
        }
        let mut maximum_key = pack.clone();
        maximum_key["avatars"][0]["key"] =
            serde_json::json!("a".repeat(tmt_office_model::office_art_reference::KEY_LIMIT));
        assert!(valid_local_avatar_reply(
            OfficeInvocation::LocalAvatarValidate,
            &maximum_key
        ));
        for invalid in [
            {
                let mut value = pack.clone();
                value["avatars"][0]["key"] = serde_json::json!(
                    "a".repeat(tmt_office_model::office_art_reference::KEY_LIMIT + 1)
                );
                value
            },
            {
                let mut value = pack.clone();
                value["label"] = serde_json::json!("bad\nlabel");
                value
            },
            {
                let mut value = pack.clone();
                value["license"] = serde_json::json!("MIT OR Apache-2.0");
                value
            },
        ] {
            assert!(!valid_local_avatar_reply(
                OfficeInvocation::LocalAvatarValidate,
                &invalid
            ));
        }
    }

    #[test]
    fn block_replies_are_bounded_typed_and_match_explicit_targets() {
        let id = "11111111-1111-4111-8111-111111111111";
        let value = serde_json::json!({"blockId":id,"revision":0,"objects":[]});
        let bytes = serde_json::to_vec(&value).unwrap();
        assert_eq!(
            decode_block_reply(&bytes, Some(id))
                .unwrap()
                .unwrap()
                .revision,
            0
        );
        assert!(decode_block_reply(&bytes, Some("22222222-2222-4222-8222-222222222222")).is_err());
        assert_eq!(
            decode_block_reply(br#"{"error":"OFFICE_REVISION_CONFLICT"}"#, None).unwrap(),
            Err(OfficeError::RevisionConflict)
        );
        for invalid in [
            serde_json::json!({"blockId":id,"revision":0,"objects":[{"asset":"desk","x":0,"y":0,"rotation":0}]}),
            serde_json::json!({"blockId":id,"revision":1,"objects":[],"token":"unexpected"}),
            serde_json::json!({"blockId":id,"revision":1,"objects":["d000"]}),
            serde_json::json!({"error":"UNKNOWN"}),
            serde_json::json!({"state":"credential"}),
        ] {
            assert!(decode_block_reply(&serde_json::to_vec(&invalid).unwrap(), None).is_err());
        }
        assert!(decode_block_reply(&vec![b' '; 4097], None).is_err());
    }

    #[test]
    fn sync_reports_are_bounded_and_expose_only_public_delivery_state() {
        let report = serde_json::json!({"completed":1,"failed":1,"pending":2,"failureCode":"OFFICE_REMOTE_UNCERTAIN"});
        assert_eq!(
            decode_sync_reply(report.to_string().as_bytes())
                .unwrap()
                .unwrap(),
            OfficeSyncReport {
                completed: 1,
                failed: 1,
                pending: 2,
                failure: Some(OfficeError::RemoteUncertain),
            }
        );
        for (key, value) in [
            ("completed", serde_json::json!(-1)),
            ("completed", serde_json::json!(16)),
            ("pending", serde_json::json!("2")),
            ("failed", serde_json::json!(0)),
            ("failureCode", serde_json::Value::Null),
            ("failureCode", serde_json::json!("private diagnostic")),
            ("refreshToken", serde_json::json!("private")),
        ] {
            let mut invalid = report.clone();
            invalid[key] = value;
            assert!(
                decode_sync_reply(invalid.to_string().as_bytes()).is_err(),
                "{key}"
            );
        }
        assert_eq!(
            decode_sync_reply(br#"{"error":"OFFICE_CREDENTIALS_UNAVAILABLE"}"#).unwrap(),
            Err(OfficeError::CredentialsUnavailable)
        );
        assert!(decode_sync_reply(br#"{"completed":0}"#).is_err());
        assert!(decode_sync_reply(&vec![b' '; 4097]).is_err());
    }

    #[test]
    fn pairing_response_accepts_only_public_fields_and_the_selected_world() {
        let world = "https://office.example/worlds/abcdefghijklmnopqrst";
        assert!(
            matches!(decode_pairing_reply(br#"{"state":"unpaired"}"#, world), Ok(PairingReply::State(state)) if state == "unpaired")
        );
        assert!(matches!(
            decode_pairing_reply(br#"{"blockExists":false}"#, world),
            Ok(PairingReply::Inspected(false))
        ));
        assert!(matches!(
            decode_pairing_reply(br#"{"error":"OFFICE_NOT_PAIRED"}"#, world),
            Ok(PairingReply::Error(OfficeError::NotPaired))
        ));
        let link = format!("{world}/pair#tmt-pair=e30");
        let pending = serde_json::json!({"state":"pending","approvalUrl":link});
        assert!(
            matches!(decode_pairing_reply(pending.to_string().as_bytes(), world), Ok(PairingReply::Pending(value)) if value == link)
        );
        for value in [
            serde_json::json!({"state":"credential","refreshToken":"private"}),
            serde_json::json!({"state":"unknown"}),
            serde_json::json!({"error":"arbitrary-provider-diagnostic"}),
            serde_json::json!({"blockExists":"false"}),
            serde_json::json!({"state":"pending","approvalUrl":"https://other.example/pair#tmt-pair=e30"}),
            serde_json::json!({"state":"pending","approvalUrl":format!("{link}\n")}),
            serde_json::json!([]),
        ] {
            assert!(decode_pairing_reply(value.to_string().as_bytes(), world).is_err());
        }
        assert!(decode_pairing_reply(b"\xff", world).is_err());
        assert!(decode_pairing_reply(&vec![b' '; 4097], world).is_err());
    }
}

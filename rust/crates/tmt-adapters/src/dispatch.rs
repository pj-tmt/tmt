//! Strict owner JSON admission and shared composition receipts/intent hashing.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tmt_core::request::{RequestKind, WakeState};

use tmt_core::{
    dispatch::{
        Acceptance, DispatchInput, DispatchItem, DispatchReceipt, DispatchRoom, MAX_RECIPIENTS,
        canonical_id,
    },
    exact_text::MAX_EXCHANGE_TEXT_BYTES,
    limits::MAX_JS_SAFE_INTEGER,
};

/// Worst-case JSON escaping of the canonical message plus a bounded UUID list.
pub const INPUT_LIMIT: usize = MAX_EXCHANGE_TEXT_BYTES * 6 + 4096;

#[cfg(test)]
mod tests;

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InputWire {
    operation_id: String,
    recipient_ids: Vec<String>,
    message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    room: Option<RoomWire>,
    #[serde(default, skip_serializing_if = "KindWire::is_request")]
    kind: KindWire,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
enum KindWire {
    #[default]
    Request,
    Announcement,
}

impl KindWire {
    fn is_request(&self) -> bool {
        matches!(self, Self::Request)
    }
}

#[derive(Deserialize, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum RoomWire {
    Direct { room_id: String },
    Roster { room_id: String, revision: u64 },
}

impl From<RoomWire> for DispatchRoom {
    fn from(room: RoomWire) -> Self {
        match room {
            RoomWire::Direct { room_id } => Self::Direct { room_id },
            RoomWire::Roster { room_id, revision } => Self::Roster { room_id, revision },
        }
    }
}
impl From<&DispatchRoom> for RoomWire {
    fn from(room: &DispatchRoom) -> Self {
        match room {
            DispatchRoom::Direct { room_id } => Self::Direct {
                room_id: room_id.clone(),
            },
            DispatchRoom::Roster { room_id, revision } => Self::Roster {
                room_id: room_id.clone(),
                revision: *revision,
            },
        }
    }
}

pub fn decode_input(bytes: &[u8]) -> Option<DispatchInput> {
    if bytes.len() > INPUT_LIMIT {
        return None;
    }
    let wire: InputWire = serde_json::from_slice(bytes).ok()?;
    DispatchInput {
        originator: tmt_core::request::Originator::Unknown,
        kind: match wire.kind {
            KindWire::Request => RequestKind::Request,
            KindWire::Announcement => RequestKind::Announcement,
        },
        operation_id: wire.operation_id,
        recipient_ids: wire.recipient_ids,
        message: wire.message,
        room: wire.room.map(Into::into),
    }
    .normalize()
}

pub(crate) fn intent_digest(input: &DispatchInput) -> String {
    let wire = InputWire {
        kind: match input.kind {
            RequestKind::Request => KindWire::Request,
            RequestKind::Announcement => KindWire::Announcement,
        },
        operation_id: input.operation_id.clone(),
        recipient_ids: input.recipient_ids.clone(),
        message: input.message.clone(),
        room: input.room.as_ref().map(Into::into),
    };
    // Hash the complete normalized intent, including direct vs roster semantics.
    // Identity provenance participates only when the CLI supplies a known sender.
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Sender<'a> {
        kind: &'a str,
        identity_id: &'a str,
    }
    #[derive(Serialize)]
    struct Intent<'a> {
        #[serde(flatten)]
        input: InputWire,
        #[serde(skip_serializing_if = "Option::is_none")]
        originator: Option<Sender<'a>>,
    }
    let intent = Intent {
        input: wire,
        originator: input.originator.identity_id().map(|identity_id| Sender {
            kind: input.originator.as_str(),
            identity_id,
        }),
    };
    tmt_core::content_digest::framed_sha256(
        b"tmt:office:dispatch:v1\0",
        &serde_json::to_vec(&intent).expect("string-only intent"),
    )
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReceiptWire {
    operation_id: String,
    created_at_ms: u64,
    items: Vec<ItemWire>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ItemWire {
    recipient_id: String,
    request_id: String,
    acceptance: AcceptanceWire,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
enum AcceptanceWire {
    Queued,
    RecipientUnavailable,
}

pub fn encode_receipt(receipt: &DispatchReceipt) -> Vec<u8> {
    serde_json::to_vec(&ReceiptWire {
        operation_id: receipt.operation_id.clone(),
        created_at_ms: receipt.created_at_ms,
        items: receipt
            .items
            .iter()
            .map(|item| ItemWire {
                recipient_id: item.recipient_id.clone(),
                request_id: item.request_id.clone(),
                acceptance: match item.acceptance {
                    Acceptance::Queued => AcceptanceWire::Queued,
                    Acceptance::RecipientUnavailable => AcceptanceWire::RecipientUnavailable,
                },
            })
            .collect(),
    })
    .expect("string-only receipt")
}

/// HTTP may report the advisory wake separately; the stored acceptance receipt
/// remains immutable and contains no notification outcome.
pub fn encode_receipt_with_wake(receipt: &DispatchReceipt, wake: Option<WakeState>) -> Vec<u8> {
    let mut document: Value =
        serde_json::from_slice(&encode_receipt(receipt)).expect("receipt JSON");
    if let Some(state) = wake {
        let (status, pane_attempted) = match state {
            WakeState::NotAttempted => ("notAttempted", Some(false)),
            WakeState::Claimed => ("unknown", None),
            WakeState::Sent => ("sent", Some(true)),
            WakeState::Unavailable => ("unavailable", Some(false)),
            WakeState::Uncertain => ("uncertain", Some(true)),
        };
        document["wake"] = json!({
            "status": status,
            "paneAttempted": pane_attempted,
            "agentProcessed": Value::Null,
        });
    }
    serde_json::to_vec(&document).expect("receipt JSON")
}

pub(crate) fn decode_receipt(bytes: &[u8]) -> Option<DispatchReceipt> {
    if bytes.len() > 16_384 {
        return None;
    }
    let wire: ReceiptWire = serde_json::from_slice(bytes).ok()?;
    if !canonical_id(&wire.operation_id)
        || wire.created_at_ms == 0
        || wire.created_at_ms > MAX_JS_SAFE_INTEGER
        || wire.items.is_empty()
        || wire.items.len() > MAX_RECIPIENTS
        || wire.items.iter().any(|item| {
            !canonical_id(&item.recipient_id)
                || !crate::request_runtime::valid_request_id(&item.request_id)
        })
        || wire
            .items
            .windows(2)
            .any(|pair| pair[0].recipient_id >= pair[1].recipient_id)
    {
        return None;
    }
    Some(DispatchReceipt {
        operation_id: wire.operation_id,
        created_at_ms: wire.created_at_ms,
        items: wire
            .items
            .into_iter()
            .map(|item| DispatchItem {
                recipient_id: item.recipient_id,
                request_id: item.request_id,
                acceptance: match item.acceptance {
                    AcceptanceWire::Queued => Acceptance::Queued,
                    AcceptanceWire::RecipientUnavailable => Acceptance::RecipientUnavailable,
                },
            })
            .collect(),
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LookupWire {
    operation_id: String,
}

pub fn decode_dispatch_lookup(bytes: &[u8]) -> Option<String> {
    if bytes.len() > 256 {
        return None;
    }
    let wire: LookupWire = serde_json::from_slice(bytes).ok()?;
    canonical_id(&wire.operation_id).then_some(wire.operation_id)
}

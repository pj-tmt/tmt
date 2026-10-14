use super::*;
use serde_json::json;
const GEN: &str = "efdc7a11-b37e-4b86-8baf-c6f613b0df91";
#[test]
fn nullable_wire_members_are_required_instead_of_silently_defaulting() {
    for body in [
        json!({"kind":"sync-result","version":1,"generation":GEN,"requestId":"1","method":"sync.read","outcome":{"result":"read"}}),
        json!({"kind":"sync-request","version":1,"generation":GEN,"requestId":"1","call":{"method":"sync.query","input":{"binding":GEN,"query":{"shape":"stream","resource":"rows","scope":{},"stream":"writer"}}}}),
        json!({"kind":"sync-result","version":1,"generation":GEN,"requestId":"1","method":"sync.read","outcome":{"result":"error","error":{"code":"REMOTE_STATE_UNAVAILABLE"}}}),
    ] {
        assert_eq!(
            decode(&serde_json::to_vec(&body).unwrap()),
            Err(ErrorClass::Shape)
        );
    }
}
fn document(bytes: usize, id: &str) -> Document {
    let fields = json!({"payload":{"stringValue":"x".repeat(bytes)}})
        .as_object()
        .unwrap()
        .clone();
    Document {
        target: Target {
            resource: "log".into(),
            id: id.into(),
        },
        fields,
        digest: "a".repeat(64),
    }
}
#[test]
fn the_largest_actual_query_frame_fits_and_the_next_byte_is_refused() {
    let mut frame = Frame::Result {
        version: 1,
        generation: GEN.into(),
        request_id: u64::MAX.to_string(),
        method: Method::Query,
        outcome: Outcome::Stream {
            documents: vec![document(350_000, "first"), document(0, "second")],
            cursor: None,
        },
    };
    let current = serde_json::to_vec(&frame).unwrap().len();
    if let Frame::Result {
        outcome: Outcome::Stream { documents, .. },
        ..
    } = &mut frame
    {
        documents[1] = document(limits::V3_FRAME_BYTES - current, "second");
    }
    assert_eq!(
        serde_json::to_vec(&frame).unwrap().len(),
        limits::V3_FRAME_BYTES
    );
    let bytes = encode(&frame).expect("one full v3 carrier page must fit");
    assert_eq!(bytes.len(), limits::V3_FRAME_BYTES + 4);
    assert_eq!(decode(&bytes[4..]).unwrap(), frame);
    if let Frame::Result {
        outcome: Outcome::Stream { documents, .. },
        ..
    } = &mut frame
    {
        let n = documents[1].fields["payload"]["stringValue"]
            .as_str()
            .unwrap()
            .len();
        documents[1] = document(n + 1, "second");
    }
    assert_eq!(encode(&frame), Err(ErrorClass::Length));
}

#[test]
fn the_prefix_uses_the_negotiated_cap_before_body_allocation() {
    let prefix = (limits::V3_FRAME_BYTES as u32).to_be_bytes();
    assert_eq!(
        crate::decode_length_for_version(prefix, 3),
        Ok(limits::V3_FRAME_BYTES)
    );
    for version in [1, 2] {
        assert_eq!(
            crate::decode_length_for_version(prefix, version),
            Err(ErrorClass::Length)
        );
    }
    assert_eq!(
        crate::decode_length_for_version(((limits::V3_FRAME_BYTES + 1) as u32).to_be_bytes(), 3),
        Err(ErrorClass::Length)
    );
}

#[test]
fn metadata_allowance_counts_the_received_bytes_not_a_shorter_reencoding() {
    let frame = Frame::Request {
        version: 1,
        generation: GEN.into(),
        request_id: "1".into(),
        call: Call::Bind {
            project: "project".into(),
            extension: "notes".into(),
            declaration_digest: "a".repeat(64),
            space: "space".into(),
        },
    };
    let mut body = serde_json::to_vec(&frame).unwrap();
    body.resize(limits::SYNC_METADATA_BYTES + 1, b' ');
    assert_eq!(decode(&body), Err(ErrorClass::Length));
}

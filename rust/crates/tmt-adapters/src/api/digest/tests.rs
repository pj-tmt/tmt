//! Flush admission is pinned to an active UUID and exposes no claim or transport knobs.
use super::*;

#[test]
fn flush_admits_only_a_canonical_uuid_without_originator_or_delivery_options() {
    let id = "10000000-0000-4000-8000-000000000001";
    let valid = json!({"version":1,"operation":"digest.checklist.flush","input":{"identityId":id}});
    let Request::Digest(operation) = super::super::decode(&valid.to_string()).unwrap() else {
        panic!("expected Digest operation");
    };
    assert!(matches!(*operation,Operation::Flush(identity) if identity==id));
    for input in [
        json!({}),
        json!({"identityId":"worker"}),
        json!({"identityId":"10000000-0000-4000-8000-00000000000A"}),
        json!({"identityId":id,"opportunity":"turn_boundary"}),
        json!({"identityId":id,"force":true}),
    ] {
        let mut request = valid.clone();
        request["input"] = input;
        assert!(super::super::decode(&request.to_string()).is_err());
    }
    for field in ["identity", "originator"] {
        let mut request = valid.clone();
        request[field] = json!("anonymous");
        assert!(super::super::decode(&request.to_string()).is_err());
    }
}

//! Room dispatch selects the caller's host only for an implicit sender
//! (#4449). The selection is injected, and the store is private.

use super::dispatched;
use crate::{
    caller_context::testing::{Private, Selections},
    invocation::RoomOperation,
};
use tmt_adapters::{host::CallerEnvironment, storage::Storage};
use tmt_core::{
    identity::{Lifetime, create_or_resolve},
    request::RequestKind,
    room::{self, MembershipChange, RoomRepository},
};

fn dispatch(identity: Option<&str>) -> RoomOperation {
    RoomOperation::Dispatch {
        room: "ops".into(),
        message: "hello".into(),
        identity: identity.map(str::to_owned),
        operation_id: None,
        kind: RequestKind::Request,
    }
}

/// A private store with room `ops`, whose one member is `bob`, and the
/// saved identity `alice`.
fn private_room(name: &str) -> Private {
    let private = Private::new(name);
    let mut storage = Storage::open(&private.paths().database).unwrap();
    let bob = create_or_resolve(&mut storage, "bob", Lifetime::Saved).unwrap();
    create_or_resolve(&mut storage, "alice", Lifetime::Saved).unwrap();
    let ops = room::create(&mut storage, "ops".into()).unwrap();
    storage
        .change_meeting_membership(&ops.id, &bob.identity.id, MembershipChange::Join)
        .unwrap();
    storage.close().unwrap();
    private
}

#[test]
fn an_explicit_sender_is_read_from_storage_and_never_selects_the_caller_host() {
    let private = private_room("dispatch-explicit");
    let selections = Selections::default();
    let select = |environment: &CallerEnvironment| selections.select(environment);
    let receipt = dispatched(dispatch(Some("alice")), || Ok(private.paths()), &select)
        .unwrap_or_else(|failure| panic!("{:?}", failure.document()));
    assert_eq!(receipt.items.len(), 1);
    let missing = dispatched(dispatch(Some("ghost")), || Ok(private.paths()), &select)
        .expect_err("the sender is missing");
    assert_eq!(missing.code, "NAME_NOT_FOUND");
    assert_eq!(selections.calls(), 0);
}

#[test]
fn an_implicit_sender_whose_selection_fails_cleanup_queues_nothing_and_opens_no_store() {
    let private = Private::new("dispatch-implicit");
    let selections = Selections::default();
    let select = |environment: &CallerEnvironment| selections.select(environment);
    let failure =
        dispatched(dispatch(None), || Ok(private.paths()), &select).expect_err("a cleanup failure");
    assert_eq!(failure.status, 1);
    assert_eq!(failure.document()["error"]["code"], "CLEANUP_ERROR");
    assert_eq!(selections.calls(), 1);
    assert!(!private.paths().database.exists());
}

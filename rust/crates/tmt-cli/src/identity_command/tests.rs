use super::{Report, run};
use crate::{
    caller_context::testing::{Private, Selections, composed},
    invocation::IdentityRequest,
};
use tmt_adapters::storage::Storage;
use tmt_core::identity::{Lifetime, create_or_resolve};

/// `identity show <id> --expect-name` is a named operation: `run`'s selector
/// step makes no caller selection, so a failed selection cleanup cannot stop
/// it (#4449). It does not count `pinned` itself, which reads presence through
/// `Host::for_stored_bindings` and so selects no caller host by construction.
#[test]
fn a_pinned_show_never_selects_the_caller_host() {
    let private = Private::new("identity-pinned");
    let mut storage = Storage::open(private.paths().database).unwrap();
    let alice = create_or_resolve(&mut storage, "alice", Lifetime::Saved)
        .unwrap()
        .identity
        .id;
    storage.close().unwrap();
    let selections = Selections::default();
    let report = composed(&selections, &private, |composition| {
        run(
            IdentityRequest::Show {
                name: Some(alice.clone()),
                expect_name: Some("alice".into()),
            },
            composition,
        )
    })
    .unwrap_or_else(|failure| panic!("pinned show failed: {}", failure.code));
    let Report::Shown(identity, _, presence, _) = report else {
        panic!("not a shown identity");
    };
    assert_eq!(identity.id, alice);
    assert!(presence.is_some(), "the pinned show reads presence");
    assert_eq!(selections.calls(), 0);
}

//! Actual signed tombstones and neutral carrier boundaries, not backend proof.
use super::*;
use crate::{
    management::Storage,
    readers::test_support::{Fixture, OTHER, PAGE, decoder_config},
    transitions::OwnerAction,
};

fn source(state: &Fixture) -> crate::page::save::SourceOpener {
    let root = state.root.clone();
    let config = decoder_config(std::env::current_exe().unwrap());
    Arc::new(move || crate::page::save::open_source(&root, config.clone()))
}
fn owner(channel: ObjectChannel) -> Arc<ChannelOwner> {
    let owner = Arc::new(ChannelOwner::new(Arc::new(|| None)));
    *locked(&owner.current) = Some(channel);
    owner
}
fn removed(remote: &Bus, start: u64) {
    let request = receive(remote);
    assert!(matches!(request.call, Call::RemoveNamespace(_)));
    for (id, boundary, disclosure) in [
        (1, Checkpoint::Acquire, None),
        (2, Checkpoint::Effect, None),
        (3, Checkpoint::Disclose, Some(Disclosure::Removed)),
    ] {
        assert_eq!(
            admit(remote, &request, id + start, boundary, disclosure).decision,
            Decision::Allow
        );
    }
    finish(remote, request, Outcome::Success(Success::Removed));
}

fn version_one() -> (ObjectChannel, Bus) {
    use std::io::Read;
    let mut workers = Vec::new();
    let mut attempt = 0;
    let link = negotiate(limit(), || {
        attempt += 1;
        let legacy = attempt == 2;
        let (remote, mut extension) = UnixStream::pair().unwrap();
        workers.push(thread::spawn(move || {
            let interrupt = extension.try_clone().unwrap();
            if !legacy {
                extension
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut head = Vec::new();
                while !head.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    extension.read_exact(&mut byte).unwrap();
                    head.push(byte[0]);
                    assert!(head.len() <= 8192);
                }
                return None;
            }
            let link = accept(
                extension,
                &Expect {
                    host: HOST.into(),
                    mount: MOUNT.into(),
                },
                &Budgets::contract(),
                limit(),
            )
            .unwrap();
            Some(ObjectChannel::start(link, interrupt).unwrap())
        }));
        Ok((
            remote,
            Offer {
                host: HOST.into(),
                mount: MOUNT.into(),
                generation: Uuid4::parse(GENERATION).unwrap(),
            },
        ))
    })
    .unwrap();
    assert_eq!(attempt, 2);
    let mut channels = workers
        .into_iter()
        .filter_map(|worker| worker.join().unwrap());
    let channel = channels.next().unwrap();
    assert!(channels.next().is_none());
    (
        channel,
        Bus::start(link, Budgets::contract(), Caps::contract(), None).unwrap(),
    )
}

#[test]
fn version_one_refuses_both_calls_before_request_correlation_and_keeps_the_exact_hint() {
    let mut state = Fixture::new();
    let source = source(&state);
    state.apply(OwnerAction::Delete { page: PAGE });
    let (channel, remote) = version_one();
    let owner = owner(channel);
    assert_eq!(
        owner.remove_page_namespace(PAGE, &source, limit()),
        Storage::Unsupported
    );
    let client = owner.client().unwrap();
    assert!(
        matches!(client.usage_details(PAGE, &source, limit()), crate::attachments::UsageDetails::Unsupported { hint } if hint == tmt_extension_objects::unsupported_hint())
    );
    assert_eq!(
        Storage::Unsupported.message(),
        format!(
            "! File data not freed.\n{}",
            tmt_extension_objects::unsupported_hint()
        )
    );
    let backend = thread::spawn(move || {
        let request = receive(&remote);
        assert_eq!(
            request.request_id,
            Counter::new(1).unwrap(),
            "unsupported methods never entered Client::request"
        );
        assert!(matches!(request.call, Call::Config(_)));
        finish(
            &remote,
            request,
            Outcome::Failure(tmt_extension_objects::ErrorCode::Unavailable),
        );
    });
    let admission = Arc::new(Current(AtomicBool::new(true), AtomicUsize::new(0)));
    let _ = client.request(Origin::LocalExtension, config(), admission, limit());
    backend.join().unwrap();
}

#[test]
fn usage_rechecks_the_exact_namespace_and_retains_all_accounting_fields() {
    let state = Fixture::new();
    let source = source(&state);
    let (channel, remote) = pair();
    let client = channel.client();
    let backend = thread::spawn(move || {
        let request = receive(&remote);
        assert!(matches!(
            request.call,
            Call::Usage(tmt_extension_objects::UsageScope::Namespace(_))
        ));
        let usage = tmt_extension_objects::Usage {
            charged_bytes: 2048,
            entries: 1,
            active_uploads: 2,
            retained_identities: 3,
        };
        assert_eq!(
            admit(&remote, &request, 1, Checkpoint::Acquire, None).decision,
            Decision::Allow
        );
        assert_eq!(
            admit(
                &remote,
                &request,
                2,
                Checkpoint::Disclose,
                Some(Disclosure::Usage(usage))
            )
            .decision,
            Decision::Allow
        );
        finish(&remote, request, Outcome::Success(Success::Usage(usage)));
    });
    let details = client.usage_details(PAGE, &source, limit());
    assert_eq!(
        serde_json::to_value(&details).unwrap(),
        serde_json::json!({"state":"known","chargedBytes":2048,"entries":1,"activeUploads":2,"retainedIdentities":3})
    );
    assert_eq!(
        details.to_string(),
        "Storage is full. Charged 2 KB (1 file, 2 uploading, small fixed records). This is a conservative allocation, not disk or billing bytes."
    );
    backend.join().unwrap();
}

#[test]
fn an_absent_channel_delete_is_freed_by_the_next_bounded_sweep_and_repeat_is_idempotent() {
    let mut state = Fixture::new();
    let source = source(&state);
    let owner = Arc::new(ChannelOwner::new(Arc::new(|| None)));
    assert_eq!(
        owner.remove_page_namespace(PAGE, &source, limit()),
        Storage::Pending
    );
    state.apply(OwnerAction::Delete { page: PAGE });
    assert_eq!(
        owner.remove_page_namespace(PAGE, &source, limit()),
        Storage::Pending
    );
    let (channel, remote) = pair();
    *locked(&owner.current) = Some(channel);
    let backend = thread::spawn(move || {
        removed(&remote, 0);
        removed(&remote, 3);
    });
    let mut after = None;
    assert!(
        !owner
            .sweep_deletions(&source, &source().unwrap(), &mut after, limit())
            .unwrap()
    );
    assert_eq!(
        owner.remove_page_namespace(PAGE, &source, limit()),
        Storage::Freed
    );
    backend.join().unwrap();
}

#[test]
fn removal_never_crosses_before_commit_or_for_another_page_and_unknown_is_not_success() {
    let mut state = Fixture::new();
    let source = source(&state);
    let (channel, remote) = pair();
    let owner = owner(channel);
    // No request is written by these refusals: the next wire request is PAGE's committed delete.
    assert_eq!(
        owner.remove_page_namespace(PAGE, &source, limit()),
        Storage::Pending
    );
    assert_eq!(
        owner.remove_page_namespace(OTHER, &source, limit()),
        Storage::Pending
    );
    state.apply(OwnerAction::Delete { page: PAGE });
    let backend = thread::spawn(move || {
        let request = receive(&remote);
        assert_eq!(
            request.call,
            Call::RemoveNamespace(tmt_extension_objects::NamespaceInput {
                namespace: Bytes32::from_bytes(
                    crate::attachments::namespace(&state.key.space_id, PAGE).unwrap()
                ),
            })
        );
        assert_eq!(
            admit(&remote, &request, 1, Checkpoint::Acquire, None).decision,
            Decision::Allow
        );
        finish(
            &remote,
            request,
            Outcome::Failure(tmt_extension_objects::ErrorCode::Unknown),
        );
    });
    assert_eq!(
        owner.remove_page_namespace(PAGE, &source, limit()),
        Storage::Unknown
    );
    backend.join().unwrap();
}

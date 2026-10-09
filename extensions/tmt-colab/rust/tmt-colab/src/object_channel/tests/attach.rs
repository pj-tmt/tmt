//! The narrow native-write admission of an attach (#2291), decided against a real owner store: only
//! the root-local writer's own document upload, at its captured base, on a writable page.
use super::super::attach::{self as flow, AttachFailure, Objects, Publish, Step, tests as fake};
use super::*;
use crate::{
    attachments::{
        self, CommittedObjectVerifier,
        slots::{Frozen, StagingSlot, StagingSlots},
    },
    fold::Snapshot,
    page,
    readers::test_support::{Fixture, PAGE},
    store::owner::Device,
    transitions::OwnerAction,
};
use std::sync::Mutex;
use tmt_colab_model::{
    attachment::{Descriptor, Source},
    crypto, values,
};
use tmt_extension_objects::Operation;
use tmt_extension_objects::{BeginInput, ErrorCode};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
struct World {
    state: Fixture,
    channel: ObjectChannel,
    _remote: Bus,
    source: page::save::SourceOpener,
    frozen: Frozen,
    upload: binding::FrozenUpload,
    original: BeginInput,
}
impl World {
    fn new() -> Self {
        let mut state = Fixture::new();
        with_object_source(&mut state);
        // The root-local writer is a device of the page, as page creation makes it.
        let (writer, ..) = state.key.local_writer().unwrap();
        let space = state.key.space_id.clone();
        let owner = state.key.owner_public();
        let chain = {
            let snapshot = Snapshot::capture(&state.store, &state.key, PAGE).unwrap();
            let issuer = state
                .store
                .owner_read(&space, &owner, |tx| Ok(tx.statement(1)?.unwrap().hash()?))
                .unwrap();
            page::writer_chain(&state.key, &snapshot.authority.head, &issuer, 1000).unwrap()
        };
        state
            .store
            .device_transaction(&space, &owner, |tx| {
                tx.put_device(&Device {
                    chain,
                    revoked: false,
                })?;
                Ok(Vec::new())
            })
            .unwrap();
        let (channel, remote) = pair();
        let ciphertext = vec![9u8; 100];
        let descriptor = Self::descriptor(&state, &writer, &ciphertext);
        let base = attachments::captured_base(&state.store, &state.key, &descriptor).unwrap();
        let frozen = Frozen {
            transfer_id: GENERATION.into(),
            base,
            descriptor,
        };
        let upload = binding::FrozenUpload::metadata(
            frozen.descriptor.clone(),
            frozen.base.clone(),
            &frozen.transfer_id,
        )
        .unwrap();
        let Call::Begin(original) = upload.begin() else {
            unreachable!()
        };
        let source = state.service.lock().unwrap().save_source().unwrap();
        Self {
            state,
            channel,
            _remote: remote,
            source,
            frozen,
            upload,
            original,
        }
    }
    fn descriptor(state: &Fixture, author: &str, ciphertext: &[u8]) -> Descriptor {
        let revision = state
            .store
            .owner_head(&state.key.space_id, &state.key.owner_public())
            .unwrap()
            .unwrap()
            .revision
            .to_string();
        Descriptor {
            version: 1,
            attachment_id: GENERATION.into(),
            space: state.key.space_id.clone(),
            page: PAGE.into(),
            epoch: "1".into(),
            namespace: "content".into(),
            object_id: "a".repeat(64),
            author_device: author.into(),
            membership_revision: revision,
            source: Source::Document {
                source_digest: "b".repeat(64),
            },
            envelope_hash: "c".repeat(64),
            signature: values::encode_binary(&[1u8; 64]),
            payload_sha256: hex(&crypto::digest(ciphertext)),
            payload_bytes: ciphertext.len().to_string(),
            plaintext_bytes: "5".into(),
            filename: "a.txt".into(),
            media_type: "text/plain".into(),
        }
    }
    fn admission(&self, input: Option<AdmitInput>) -> admission::RootWriteAdmission {
        admission::RootWriteAdmission {
            client: self.channel.client(),
            source: Arc::clone(&self.source),
            descriptor: self.frozen.descriptor.clone(),
            base: self.frozen.base.clone(),
            original: self.original.clone(),
            input,
            deadline: Instant::now() + Duration::from_secs(10),
        }
    }
    fn decide(
        &self,
        owner: &admission::RootWriteAdmission,
        context: Context,
        input: AdmitInput,
    ) -> Decision {
        owner.decide(
            &Admit {
                generation: self.channel.client().generation(),
                callback_id: Counter::new(1).unwrap(),
                request_id: Counter::new(1).unwrap(),
                boundary: Checkpoint::Acquire,
                context,
                operation: Operation {
                    input,
                    disclosure: None,
                },
            },
            Instant::now() + Duration::from_secs(10),
        )
    }
    fn call_input(&self, call: &Call) -> AdmitInput {
        match call {
            Call::Begin(input) => AdmitInput::Begin(input.clone()),
            Call::Status(input) => AdmitInput::Status(input.clone()),
            Call::Read(input) => AdmitInput::Read(input.clone()),
            _ => self.upload.admission_input(call).unwrap(),
        }
    }
    fn read(&self, key: [u8; 32], namespace: [u8; 32]) -> ReadInput {
        ReadInput {
            namespace: Bytes32::from_bytes(namespace),
            opaque_key: Bytes32::from_bytes(key),
            policy: self.original.policy.clone(),
            payload_sha256: self.original.payload_sha256,
            payload_bytes: self.original.payload_bytes,
            offset: 0,
            count: 16,
        }
    }
}

#[test]
fn only_the_exact_upload_calls_of_the_writers_own_original_are_admitted() {
    let world = World::new();
    let part = world.upload.streamed_part(0, vec![9u8; 100]).unwrap();
    for call in [
        world.upload.begin(),
        world.upload.status(),
        part,
        world.upload.commit(),
        world.upload.discard(),
    ] {
        let input = world.call_input(&call);
        let owner = world.admission(Some(input.clone()));
        assert_eq!(
            world.decide(&owner, Context::LocalExtension, input),
            Decision::Allow,
            "{call:?}"
        );
    }
    // The verify read of the committed bytes names the one original, any range of it.
    let verify = world.admission(None);
    let read = world.read(
        *world.original.opaque_key.as_bytes(),
        *world.original.namespace.as_bytes(),
    );
    assert_eq!(
        world.decide(&verify, Context::LocalExtension, AdmitInput::Read(read)),
        Decision::Allow
    );
}
#[test]
fn a_begin_receipt_must_carry_the_admitted_object_digest_and_size() {
    let world = World::new();
    let begin = AdmitInput::Begin(world.original.clone());
    let owner = world.admission(Some(begin.clone()));
    let receipt = |key: Bytes32, digest, bytes| Disclosure::Receipt {
        opaque_key: key,
        payload_sha256: digest,
        payload_bytes: bytes,
    };
    let at_disclose = |disclosure: Disclosure| {
        owner.decide(
            &Admit {
                generation: world.channel.client().generation(),
                callback_id: Counter::new(1).unwrap(),
                request_id: Counter::new(1).unwrap(),
                boundary: Checkpoint::Disclose,
                context: Context::LocalExtension,
                operation: Operation {
                    input: begin.clone(),
                    disclosure: Some(disclosure),
                },
            },
            Instant::now() + Duration::from_secs(10),
        )
    };
    let original = &world.original;
    assert_eq!(
        at_disclose(receipt(
            original.opaque_key,
            original.payload_sha256,
            original.payload_bytes
        )),
        Decision::Allow
    );
    // The same object under another digest, or another size, is not the admitted original.
    assert_eq!(
        at_disclose(receipt(
            original.opaque_key,
            tmt_extension_objects::Sha256Hex::from_bytes([5; 32]),
            original.payload_bytes
        )),
        Decision::Deny
    );
    assert_eq!(
        at_disclose(receipt(
            original.opaque_key,
            original.payload_sha256,
            original.payload_bytes + 1
        )),
        Decision::Deny
    );
}
#[test]
fn no_other_context_generation_or_call_is_admitted() {
    let world = World::new();
    let begin = AdmitInput::Begin(world.original.clone());
    let owner = world.admission(Some(begin.clone()));
    let origin_id = Uuid4::parse("22222222-2222-4222-8222-222222222222").unwrap();
    // A browser session or a mounted reader is never the root-local writer.
    for context in [
        Context::OwnerSession {
            origin_id,
            device_id: Uuid4::parse("33333333-3333-4333-8333-333333333333").unwrap(),
            grant_revision: 1,
        },
        Context::Mounted { origin_id },
    ] {
        assert_eq!(world.decide(&owner, context, begin.clone()), Decision::Deny);
    }
    // Another generation of the channel is a stale caller.
    assert_eq!(
        owner.decide(
            &Admit {
                generation: Uuid4::parse("99999999-9999-4999-8999-999999999999").unwrap(),
                callback_id: Counter::new(1).unwrap(),
                request_id: Counter::new(1).unwrap(),
                boundary: Checkpoint::Acquire,
                context: Context::LocalExtension,
                operation: Operation {
                    input: begin.clone(),
                    disclosure: None
                },
            },
            Instant::now() + Duration::from_secs(10)
        ),
        Decision::Deny
    );
    // A different object, another page's namespace, a config call or another method than the
    // one admitted is forged, whatever the context says.
    let mut forged = world.original.clone();
    forged.opaque_key = Bytes32::from_bytes([7; 32]);
    assert_eq!(
        world.decide(&owner, Context::LocalExtension, AdmitInput::Begin(forged)),
        Decision::Deny
    );
    assert_eq!(
        world.decide(
            &owner,
            Context::LocalExtension,
            AdmitInput::Status(tmt_extension_objects::StatusInput {
                transfer_id: world.original.transfer_id,
                namespace: world.original.namespace,
                policy: world.original.policy.clone(),
            })
        ),
        Decision::Deny
    );
    let verify = world.admission(None);
    for read in [
        world.read([7; 32], *world.original.namespace.as_bytes()),
        world.read(*world.original.opaque_key.as_bytes(), [7; 32]),
    ] {
        assert_eq!(
            world.decide(&verify, Context::LocalExtension, AdmitInput::Read(read)),
            Decision::Deny
        );
    }
    assert_eq!(
        world.decide(
            &verify,
            Context::LocalExtension,
            AdmitInput::Config(ConfigInput {
                namespace: world.original.namespace,
                policy: world.original.policy.clone(),
            })
        ),
        Decision::Deny
    );
}
#[test]
fn a_descriptor_that_is_not_the_writers_own_document_is_refused() {
    let world = World::new();
    let begin = AdmitInput::Begin(world.original.clone());
    let refused = |mutate: &dyn Fn(&mut admission::RootWriteAdmission)| {
        let mut owner = world.admission(Some(begin.clone()));
        mutate(&mut owner);
        world.decide(&owner, Context::LocalExtension, begin.clone())
    };
    assert_eq!(refused(&|_| {}), Decision::Allow);
    // Authored by a browser device, not the root-local writer.
    assert_eq!(
        refused(
            &|owner| owner.descriptor.author_device = "30000000-0000-4000-8000-000000000001".into()
        ),
        Decision::Deny
    );
    // A message attachment is not a native document attach.
    assert_eq!(
        refused(&|owner| {
            owner.descriptor.namespace = "own".into();
            owner.descriptor.source = Source::Message {
                writer_id: owner.descriptor.author_device.clone(),
                message_id: GENERATION.into(),
                message_revision: "1".into(),
            };
        }),
        Decision::Deny
    );
    // Another page's descriptor never matches this page's base.
    assert_eq!(
        refused(&|owner| owner.base = format!("v1:{}", "f".repeat(64))),
        Decision::Deny
    );
}
#[test]
fn an_archived_or_deleted_page_admits_nothing_and_not_even_a_discard() {
    for action in [0, 1] {
        let mut world = World::new();
        let discard = world.call_input(&world.upload.discard());
        let begin = AdmitInput::Begin(world.original.clone());
        assert_eq!(
            world.decide(
                &world.admission(Some(begin.clone())),
                Context::LocalExtension,
                begin.clone()
            ),
            Decision::Allow
        );
        world.state.apply(if action == 0 {
            OwnerAction::Archive { page: PAGE }
        } else {
            OwnerAction::Delete { page: PAGE }
        });
        let owner = world.admission(Some(begin.clone()));
        assert_eq!(
            world.decide(&owner, Context::LocalExtension, begin),
            Decision::Deny
        );
        let owner = world.admission(Some(discard.clone()));
        assert_eq!(
            world.decide(&owner, Context::LocalExtension, discard),
            Decision::Deny
        );
    }
}
#[test]
fn a_discard_needs_a_writable_page_but_not_the_base_the_upload_began_at() {
    let world = World::new();
    let discard = world.call_input(&world.upload.discard());
    let mut owner = world.admission(Some(discard.clone()));
    owner.base = format!("v1:{}", "f".repeat(64));
    assert_eq!(
        world.decide(&owner, Context::LocalExtension, discard),
        Decision::Allow
    );
}
#[test]
fn a_revoked_writer_device_admits_nothing() {
    let mut world = World::new();
    let (writer, ..) = world.state.key.local_writer().unwrap();
    let begin = AdmitInput::Begin(world.original.clone());
    let owner = world.admission(Some(begin.clone()));
    assert_eq!(
        world.decide(&owner, Context::LocalExtension, begin.clone()),
        Decision::Allow
    );
    world.state.apply(OwnerAction::DeviceRevoke {
        device_id: &writer,
        grant_revision: 1,
    });
    let owner = world.admission(Some(begin.clone()));
    assert_eq!(
        world.decide(&owner, Context::LocalExtension, begin),
        Decision::Deny
    );
}

/// The attach flow against a real owner store with the object channel replaced by an in-memory
/// backend: each stage can be reached, interrupted and then met by a page that moved.
struct Committed(Vec<u8>);
impl CommittedObjectVerifier for Committed {
    fn read_committed(&self, _: &[u8; 32], _: &[u8; 32], _: Instant) -> crate::Result<Vec<u8>> {
        Ok(self.0.clone())
    }
}
struct FakeObjects {
    backend: fake::Backend,
    ciphertext: Mutex<Vec<u8>>,
    before_upload: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    unreadable: AtomicBool,
    discards: AtomicUsize,
}
impl FakeObjects {
    fn new() -> Self {
        Self {
            backend: fake::Backend::new(fake::Transfer::Unobserved),
            ciphertext: Mutex::new(Vec::new()),
            before_upload: Mutex::new(None),
            unreadable: AtomicBool::new(false),
            discards: AtomicUsize::new(0),
        }
    }
}
impl Objects for FakeObjects {
    fn upload(&self, frozen: &Frozen, ciphertext: &[u8]) -> Step<()> {
        if let Some(hook) = self.before_upload.lock().unwrap().take() {
            hook();
        }
        *self.ciphertext.lock().unwrap() = ciphertext.to_vec();
        let upload = binding::FrozenUpload::metadata(
            frozen.descriptor.clone(),
            frozen.base.clone(),
            &frozen.transfer_id,
        )
        .unwrap();
        flow::upload(&self.backend, &upload, ciphertext)
    }
    fn committed(&self, _: &Frozen) -> crate::Result<Box<dyn CommittedObjectVerifier + '_>> {
        if self.unreadable.load(Ordering::SeqCst) {
            return Err(page::Fault::Unavailable.into());
        }
        Ok(Box::new(Committed(self.ciphertext.lock().unwrap().clone())))
    }
    fn discard(&self, _: &Frozen) {
        self.discards.fetch_add(1, Ordering::SeqCst);
    }
}
/// The serve's single writer, driven in process like the socket does, and counted.
struct Writer {
    server: Arc<crate::sync::Server<crate::registration::OwnerAdmission>>,
    published: Arc<AtomicUsize>,
    fail_from: AtomicUsize,
}
impl Publish for Writer {
    fn publish(
        &self,
        key: &crate::keyring::Keyring,
        frozen: &page::FrozenPublication,
    ) -> crate::Result<page::Published> {
        if self.published.load(Ordering::SeqCst) + 1 >= self.fail_from.load(Ordering::SeqCst) {
            return Err(page::Fault::Unavailable.into());
        }
        let body = page::ipc::local_write_body(key, frozen)?;
        let published = self.server.publish(
            &body,
            crate::registration::now_ms().unwrap(),
            Instant::now(),
        )?;
        self.published.fetch_add(1, Ordering::SeqCst);
        Ok(published)
    }
}
/// A different source for the page, published like any other write.
fn edit_page(
    source: &page::save::SourceOpener,
    server: &crate::sync::Server<crate::registration::OwnerAdmission>,
    published: &AtomicUsize,
) {
    let mut view = source().unwrap();
    let current = page::read(&view.store, &view.keyring, PAGE, &mut view.decoder).unwrap();
    drop(view);
    let prepared = page::save::prepare(
        source,
        &page::save::Save {
            page: PAGE.into(),
            operation_id: page::fresh_id().unwrap(),
            base_sha256: crypto::digest(current.source.as_bytes()),
            source: format!("{}<p>edited</p>", current.source),
            attachments: None,
        },
        &|| Ok(crate::registration::now_ms().unwrap()),
    )
    .unwrap();
    let page::save::Prepared::Write(frozen) = prepared else {
        panic!("an edit changes the page")
    };
    let view = source().unwrap();
    let body = page::ipc::local_write_body(&view.keyring, &frozen).unwrap();
    server
        .publish(
            &body,
            crate::registration::now_ms().unwrap(),
            Instant::now(),
        )
        .unwrap();
    published.fetch_add(1, Ordering::SeqCst);
}
struct Flow {
    world: World,
    slots: StagingSlots,
    objects: FakeObjects,
    writer: Writer,
}
impl Flow {
    fn new() -> Self {
        let world = World::new();
        let layout = crate::keyring::Layout::existing(&world.state.root)
            .unwrap()
            .unwrap();
        let writer = Writer {
            server: Arc::new(crate::sync::Server::new(
                crate::store::Store::open(&layout).unwrap(),
                crate::registration::OwnerAdmission(world.state.service.clone()),
            )),
            published: Arc::new(AtomicUsize::new(0)),
            fail_from: AtomicUsize::new(usize::MAX),
        };
        Self {
            slots: StagingSlots::open(&layout).unwrap(),
            world,
            objects: FakeObjects::new(),
            writer,
        }
    }
    const FILE: &'static [u8] = b"native attach bytes";
    /// A slot with the file staged, as the CLI leaves it.
    fn staged(&self) -> (StagingSlot, [u8; 32]) {
        let slot = self
            .slots
            .create(PAGE, "a.txt", "text/plain", 1000)
            .unwrap();
        std::fs::write(slot.source_path(), Self::FILE).unwrap();
        (slot, crypto::digest(Self::FILE))
    }
    fn run(&self, slot: &mut StagingSlot, expected: Option<[u8; 32]>) -> Step<flow::Attached> {
        flow::run(
            &self.objects,
            slot,
            expected,
            &self.world.source,
            &self.writer,
            Instant::now() + Duration::from_secs(20),
        )
    }
    /// A different source for the page, published like any other write: the page moves.
    fn edit(&self) {
        edit_page(
            &self.world.source,
            &self.writer.server,
            &self.writer.published,
        );
    }
    fn code(failure: &AttachFailure) -> Option<&'static str> {
        failure
            .error
            .downcast_ref::<page::Fault>()
            .map(page::Fault::code)
    }
    fn published(&self) -> usize {
        self.writer.published.load(Ordering::SeqCst)
    }
    fn begins(&self) -> usize {
        self.objects.backend.counts.lock().unwrap().begin
    }
}

#[test]
fn a_whole_attach_uploads_once_publishes_the_proof_then_the_list_and_a_retry_changes_nothing() {
    let flow = Flow::new();
    let (mut slot, expected) = flow.staged();
    let attached = flow.run(&mut slot, Some(expected)).unwrap();
    assert_eq!(attached.descriptor.filename, "a.txt");
    assert_eq!(
        (flow.begins(), flow.published()),
        (1, 2),
        "one begin; proof, then list"
    );
    assert_eq!(flow.objects.discards.load(Ordering::SeqCst), 0);
    // A crash before the slot was marked finished: the page already lists it, so nothing repeats.
    let again = flow.run(&mut slot, None).unwrap();
    assert_eq!(
        again.descriptor.attachment_id,
        attached.descriptor.attachment_id
    );
    assert_eq!((flow.begins(), flow.published()), (1, 2));
    // The finished slot answers the same attachment from its record alone.
    slot.finish(2000).unwrap();
    let mut done = flow.slots.load(slot.id()).unwrap();
    let answered = flow.run(&mut done, None).unwrap();
    assert_eq!(
        answered.descriptor.attachment_id,
        attached.descriptor.attachment_id
    );
    assert_eq!((flow.begins(), flow.published()), (1, 2));
}
#[test]
fn a_page_edited_after_sealing_and_before_upload_is_stale_terminal_and_never_publishes() {
    let flow = Flow::new();
    let (mut slot, expected) = flow.staged();
    // The first attempt sealed, began and sent its part, then lost the reply.
    flow.objects.backend.lose_reply_at(3);
    let first = flow.run(&mut slot, Some(expected)).unwrap_err();
    assert_eq!(Flow::code(&first), Some("COLAB_UNAVAILABLE"));
    assert!(!first.dispose);
    let mut slot = flow.slots.load(slot.id()).unwrap();
    assert!(slot.record.sealed.is_some());
    flow.edit();
    let stale = flow.run(&mut slot, None).unwrap_err();
    assert_eq!(Flow::code(&stale), Some("COLAB_STALE_BASE"));
    assert!(
        stale.dispose,
        "no resume can make the frozen base current again"
    );
    assert_eq!(flow.objects.discards.load(Ordering::SeqCst), 1);
    assert_eq!(
        (flow.begins(), flow.published()),
        (1, 1),
        "one begin; only the edit published"
    );
}
#[test]
fn a_page_edited_after_commit_and_before_the_proof_is_stale_terminal_and_never_publishes() {
    let flow = Flow::new();
    let (mut slot, expected) = flow.staged();
    flow.objects.unreadable.store(true, Ordering::SeqCst);
    let first = flow.run(&mut slot, Some(expected)).unwrap_err();
    assert_eq!(Flow::code(&first), Some("COLAB_UNAVAILABLE"));
    assert!(!first.dispose);
    assert!(matches!(
        &*flow.objects.backend.transfer_state(),
        fake::Transfer::Committed
    ));
    let mut slot = flow.slots.load(slot.id()).unwrap();
    flow.edit();
    flow.objects.unreadable.store(false, Ordering::SeqCst);
    let stale = flow.run(&mut slot, None).unwrap_err();
    assert_eq!(Flow::code(&stale), Some("COLAB_STALE_BASE"));
    assert!(stale.dispose);
    assert_eq!((flow.begins(), flow.published()), (1, 1));
}
#[test]
fn a_page_edited_after_the_proof_and_before_the_list_is_stale_terminal_and_lists_nothing() {
    let flow = Flow::new();
    let (mut slot, expected) = flow.staged();
    // The proof publishes; the list change is the second publication and fails to reach the writer.
    flow.writer.fail_from.store(2, Ordering::SeqCst);
    let first = flow.run(&mut slot, Some(expected)).unwrap_err();
    assert_eq!(Flow::code(&first), Some("COLAB_UNAVAILABLE"));
    assert!(!first.dispose);
    assert_eq!(flow.published(), 1, "the proof");
    let mut slot = flow.slots.load(slot.id()).unwrap();
    flow.writer.fail_from.store(usize::MAX, Ordering::SeqCst);
    flow.edit();
    assert_eq!(flow.published(), 2);
    let stale = flow.run(&mut slot, None).unwrap_err();
    assert_eq!(Flow::code(&stale), Some("COLAB_STALE_BASE"));
    assert!(stale.dispose);
    assert_eq!(
        (flow.begins(), flow.published()),
        (1, 2),
        "no list publication"
    );
}
#[test]
fn a_denied_upload_is_a_stale_base_only_when_the_page_moved() {
    // The base holds: the backend's refusal stays a denial, and the slot stays.
    let flow = Flow::new();
    let (mut slot, expected) = flow.staged();
    flow.objects.backend.refuse_begin_with(ErrorCode::Denied);
    let denied = flow.run(&mut slot, Some(expected)).unwrap_err();
    assert_eq!(Flow::code(&denied), Some("COLAB_DENIED"));
    assert!(!denied.dispose);
    // The page moves between the base check and the first call, which is what the refusal then
    // means: a stale base, terminal, with the original discarded.
    let flow = Flow::new();
    let (mut slot, expected) = flow.staged();
    flow.objects.backend.refuse_begin_with(ErrorCode::Denied);
    let source = Arc::clone(&flow.world.source);
    let server = Arc::clone(&flow.writer.server);
    let published = Arc::clone(&flow.writer.published);
    *flow.objects.before_upload.lock().unwrap() =
        Some(Box::new(move || edit_page(&source, &server, &published)));
    let stale = flow.run(&mut slot, Some(expected)).unwrap_err();
    assert_eq!(Flow::code(&stale), Some("COLAB_STALE_BASE"));
    assert!(stale.dispose);
    assert_eq!(flow.objects.discards.load(Ordering::SeqCst), 1);
    assert_eq!(flow.published(), 1, "only the edit");
}

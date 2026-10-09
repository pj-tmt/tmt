//! The narrow native-write admission of an attach (#2291), decided against a real owner store: only
//! the root-local writer's own document upload, at its captured base, on a writable page.
use super::*;
use crate::{
    attachments::{self, slots::Frozen},
    fold::Snapshot,
    page,
    readers::test_support::{Fixture, PAGE},
    store::owner::Device,
    transitions::OwnerAction,
};
use tmt_colab_model::{
    attachment::{Descriptor, Source},
    crypto, values,
};
use tmt_extension_objects::BeginInput;
use tmt_extension_objects::Operation;

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

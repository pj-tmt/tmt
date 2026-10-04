//! Root-local creation: isolated Yjs preparation, then one owner commit.
use super::{Engine, epoch, membership, request::OwnerContext};
use crate::{
    Result,
    decoder::BaselineInput,
    fold,
    keyring::Keyring,
    page,
    store::{
        Envelope, Namespace, Store, StreamScope,
        owner::{Device, Mutation, OwnerFault},
    },
};
use tmt_colab_model::{crypto, object, values, wrap};

pub(super) struct Selection<'a> {
    pub page: &'a str,
    pub title: &'a str,
    pub source: &'a str,
    pub publisher_agent: Option<&'a str>,
}
struct Prepared {
    /// The page's first state as ordered updates, each small enough for every reader.
    updates: Vec<Vec<u8>>,
    secret: [u8; 32],
}
impl Drop for Prepared {
    fn drop(&mut self) {
        self.secret.fill(0);
        self.updates.iter_mut().for_each(|u| u.fill(0));
    }
}
impl Engine {
    pub(super) fn create_page(
        &mut self,
        store: &mut Store,
        key: &Keyring,
        context: OwnerContext<'_>,
        selection: Selection<'_>,
        now: u64,
    ) -> Result<(Vec<u8>, bool)> {
        let Selection {
            page,
            title,
            source,
            publisher_agent,
        } = selection;
        values::generated_id(context.id)?;
        values::generated_id(page)?;
        values::time(now)?;
        if context.scope.is_some()
            || title.is_empty()
            || publisher_agent.is_some_and(|v| !crate::decoder::valid_publisher_agent(v))
        {
            return Err(OwnerFault::Invalid.into());
        }
        if title.len() > crate::decoder::BASELINE_TITLE_BYTES
            || source.len() > crate::decoder::BASELINE_BYTES
        {
            return Err(OwnerFault::Capacity.into());
        }
        let mut selection = serde_json::json!({"pageId":page,"title":title,"source":source});
        if let Some(agent) = publisher_agent {
            selection["publisherAgent"] = serde_json::json!(agent);
        }
        let digest = context.digest(key, "page.create", &selection)?;
        self.run_transition(
            store,
            key,
            Mutation {
                operation_id: context.id,
                digest,
                expected_revision: context.expected,
            },
            |engine, store| {
                store.owner_read(&key.space_id, &key.owner_public(), |tx| {
                    if tx.head().map_or(0, |h| h.revision) != context.expected {
                        return Err(OwnerFault::StaleHead.into());
                    }
                    if tx.page_epoch(page)?.is_some() {
                        return Err(OwnerFault::Conflict.into());
                    }
                    Ok(())
                })?;
                let baseline = engine.decoder(page)?.produce_page(
                    BaselineInput {
                        source: source.as_bytes(),
                        title,
                        publisher_agent,
                        source_digest: crypto::digest(source.as_bytes()),
                    },
                    None,
                )?;
                let mut secret = [0; 32];
                getrandom::fill(&mut secret)?;
                Ok(Some(Prepared {
                    updates: baseline.chunks,
                    secret,
                }))
            },
            |tx, prepared| {
                let mut statements = Vec::new();
                if let Some(genesis) = super::initialize_owner(tx, key)? {
                    statements.push(genesis);
                }
                tx.create_page(page)?;
                let shared = key.sign_statement(
                    tx.head(),
                    "page.share",
                    &serde_json::to_vec(
                        &serde_json::json!({"pageId":page,"mode":"private","epoch":"1"}),
                    )?,
                )?;
                tx.append_statement(&shared)?;
                statements.push(shared);
                tx.put_epoch_secret(page, 1, &prepared.secret)?;
                let (states, _) = fold::verify_log(&tx.log()?, key, page)?;
                let authority = states.last().ok_or(OwnerFault::Invalid)?;
                let devices = tx.devices()?;
                let targets = epoch::targets(tx, &devices, key, authority, page, now)?;
                if targets.len() > crate::limits::OWNER_WRAPS {
                    return Err(OwnerFault::Capacity.into());
                }
                let mut wraps = Vec::new();
                for ((kind, id), recipient_key) in targets {
                    let wrapped = key.seal_wrap(
                        &wrap::Header {
                            space: key.space_id.clone(),
                            page: page.into(),
                            epoch: "1".into(),
                            recipient_kind: kind,
                            recipient_id: id,
                            recipient_key,
                            signer_key: key.owner_public(),
                            membership_revision: authority.head.revision.to_string(),
                        },
                        &prepared.secret,
                    )?;
                    tx.put_wrap(&wrapped)?;
                    wraps.push(wrapped);
                }
                let writer = key.local_writer()?.0;
                if authority.revoked_devices.contains(&writer)
                    || tx.device(&writer)?.is_some_and(|d| d.revoked)
                {
                    return Err(page::Fault::Denied.into());
                }
                let issuer = tx.statement(1)?.ok_or(OwnerFault::Invalid)?.hash()?;
                let chain = page::writer_chain(key, &authority.head, &issuer, now)?;
                tx.put_device(&Device {
                    chain,
                    revoked: false,
                })?;
                let mut previous = [0; 32];
                for (index, update) in prepared.updates.iter().enumerate() {
                    let seq = index as u64 + 1;
                    let envelope = key.seal_content(
                        &object::Context {
                            space: key.space_id.clone(),
                            page: page.into(),
                            epoch: "1".into(),
                            kind: "update".into(),
                            namespace: "content".into(),
                            author_device: writer.clone(),
                            membership_revision: authority.head.revision.to_string(),
                            stream_seq: seq.to_string(),
                            prev_hash: previous,
                        },
                        &prepared.secret,
                        update,
                    )?;
                    let hash = envelope.hash()?;
                    tx.append_content(&Envelope {
                        scope: StreamScope {
                            page,
                            epoch: 1,
                            stream: &writer,
                        },
                        namespace: Namespace::Content,
                        seq,
                        hash,
                        previous,
                        bytes: &envelope.to_json()?,
                    })?;
                    previous = hash;
                }
                membership::outcome(&statements, wraps)
            },
        )
    }
}

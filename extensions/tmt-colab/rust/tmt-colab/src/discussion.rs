//! Root-local discussion reads and agent status actions. The authenticated fold
//! and export projection own interpretation; the isolated decoder alone edits Yjs.
use crate::{
    Result,
    decoder::{Decoder, OwnRecord, UpdateBatch},
    export::conversations::{self, Conversations},
    fold::{Snapshot, View as Folded},
    keyring::Keyring,
    page::{self, Fault},
    store::{Namespace, Store},
    threads::status::{Action, Reference},
};
use serde::Serialize;
use tmt_colab_model::values;

#[derive(Serialize)]
pub struct View {
    pub revision: String,
    #[serde(flatten)]
    pub conversations: Conversations,
}
fn project(key: &Keyring, page: &str, snapshot: &Snapshot, folded: &Folded) -> Conversations {
    Conversations::project(
        conversations::Capture {
            space_id: &key.space_id,
            page_id: page,
            title: &folded.title,
            epoch: &snapshot.epoch.to_string(),
            head: conversations::Head {
                revision: snapshot.authority.head.revision.to_string(),
                statement_hash: snapshot
                    .authority
                    .head
                    .hash
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect(),
            },
        },
        &folded.own,
        &folded.signing_keys,
        &folded.status_writers,
    )
}
pub fn read(store: &Store, key: &Keyring, page: &str, decoder: &mut Decoder) -> Result<View> {
    let snapshot = page::snapshot(store, key, page, false)?;
    let folded = snapshot.materialize(key, page, decoder)?;
    Ok(View {
        revision: page::token(
            &key.space_id,
            page,
            &snapshot.authority.head,
            snapshot.epoch,
            &snapshot.cuts,
        )?,
        conversations: project(key, page, &snapshot, &folded),
    })
}
/// No recipient dispatch: an agent CLI action only publishes its display-labelled
/// status in the local device's own stream. None is an already-effective no-op.
pub struct StatusEdit<'a> {
    pub thread: &'a str,
    pub resolved: bool,
    pub agent_name: Option<&'a str>,
}
pub fn prepare_status(
    store: &Store,
    key: &Keyring,
    page: &str,
    edit: StatusEdit<'_>,
    decoder: &mut Decoder,
    now: u64,
) -> Result<Option<(page::FrozenPublication, Action)>> {
    let StatusEdit {
        thread,
        resolved,
        agent_name,
    } = edit;
    values::generated_id(thread)?;
    let snapshot = page::snapshot(store, key, page, true)?;
    let (writer, _, _) = key.local_writer()?;
    if snapshot.authority.revoked_devices.contains(&writer) {
        return Err(Fault::Denied.into());
    }
    let folded = snapshot.materialize(key, page, decoder)?;
    let view = project(key, page, &snapshot, &folded);
    let matches: Vec<_> = view.threads.iter().filter(|row| row.id == thread).collect();
    if matches.len() != 1 {
        return Err(Fault::Invalid.into());
    }
    let target = matches[0];
    if target.deleted || (target.id == target.writer && target.anchor.is_none()) {
        return Err(Fault::Inactive.into());
    }
    if target.resolved == resolved {
        return Ok(None);
    }
    let action = Action {
        version: 1,
        kind: "thread-status".into(),
        space_id: key.space_id.clone(),
        page_id: page.into(),
        epoch: snapshot.epoch.to_string(),
        sender_device: writer.clone(),
        revision: "1".into(),
        deleted: false,
        device_name: "Local CLI".into(),
        at: now.to_string(),
        action_id: page::fresh_id()?,
        thread: Reference {
            writer: target.writer.clone(),
            id: target.id.clone(),
        },
        previous: target.status.as_ref().map(|value| value.reference.clone()),
        resolved,
        actor: "agent".into(),
        agent_name: agent_name.map(str::to_owned),
        recipients: Vec::new(),
    };
    let key_name = format!("{}:thread-status", action.action_id);
    let value = serde_json::to_value(&action)?;
    crate::threads::validate_record("messages", &key_name, &value)?;
    let records = [OwnRecord {
        root: "messages".into(),
        key: key_name,
        value,
    }];
    // Only that writer's authenticated structs enter the edit, never the shared
    // document or another writer's plain projection re-encoded as a new document.
    let updates: Vec<&[u8]> = folded
        .local_own_update
        .as_ref()
        .into_iter()
        .map(Vec::as_slice)
        .collect();
    let prepared = decoder.prepare_own(
        UpdateBatch {
            namespace: Namespace::Own,
            baseline: &[],
            updates: &updates,
        },
        &records,
        None,
    )?;
    let publication = page::prepare_own_publication(
        key,
        page,
        &snapshot,
        &prepared.merged,
        &folded.source,
        prepared.memory_limit,
        now,
    )?;
    Ok(Some((publication, action)))
}

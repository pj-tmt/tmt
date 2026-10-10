//! Root-local discussion reads and agent status actions. The authenticated fold
//! and export projection own interpretation; the isolated decoder alone edits Yjs.
use crate::{
    Result,
    decoder::{Decoder, OwnRecord},
    export::conversations::{self, Conversations},
    fold::{Snapshot, View as Folded},
    keyring::Keyring,
    page::{self, Fault},
    store::Store,
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

/// A complete proposal is published before its optional source placement. Retrying
/// the same ID compares authenticated metadata instead of creating another thread.
pub fn prepare_proposal(
    store: &Store,
    key: &Keyring,
    page: &str,
    proposal: &crate::threads::Proposal,
    decoder: &mut Decoder,
    now: u64,
) -> Result<Option<page::FrozenPublication>> {
    proposal.validate()?;
    let snapshot = page::snapshot(store, key, page, true)?;
    let (writer, _, _) = key.local_writer()?;
    if snapshot.authority.revoked_devices.contains(&writer) {
        return Err(Fault::Denied.into());
    }
    let folded = snapshot.materialize(key, page, decoder)?;
    let view = project(key, page, &snapshot, &folded);
    let existing: Vec<_> = view
        .threads
        .iter()
        .filter(|row| {
            row.proposal
                .as_ref()
                .is_some_and(|p| p.proposal_id == proposal.proposal_id)
        })
        .collect();
    if !existing.is_empty() {
        if existing.len() == 1
            && existing[0].writer == writer
            && !existing[0].deleted
            && existing[0].proposal.as_ref() == Some(proposal)
        {
            return Ok(None);
        }
        return Err(Fault::Invalid.into());
    }
    if proposal.proposal_id == writer {
        return Err(Fault::Invalid.into());
    }
    let mut proposals = std::collections::BTreeSet::new();
    for (owner, roots) in &folded.own {
        if let Some(threads) = roots["threads"].as_object() {
            for value in threads.values() {
                if value["kind"] == "thread"
                    && let Some(id) = value["proposal"]["proposalId"].as_str()
                {
                    proposals.insert((owner, id));
                }
            }
        }
    }
    if proposals.len() >= crate::limits::PAGE_PROPOSALS {
        return Err(crate::store::owner::OwnerFault::too_large_to_edit(
            page,
            format!(
                "it already retains the maximum {} proposals",
                crate::limits::PAGE_PROPOSALS
            ),
        )
        .into());
    }
    let record = serde_json::json!({
        "version":1,"kind":"thread","spaceId":key.space_id,"pageId":page,
        "epoch":snapshot.epoch.to_string(),"senderDevice":writer,"revision":"1",
        "deleted":false,"deviceName":"Local CLI","at":now.to_string(),
        "threadId":proposal.proposal_id,"anchor":null,"resolved":false,"proposal":proposal
    });
    let records = [OwnRecord {
        root: "threads".into(),
        key: format!("{}:1", proposal.proposal_id),
        value: record,
    }];
    Ok(Some(page::freeze_own_records(
        key, page, &snapshot, &folded, &records, decoder, now,
    )?))
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
    let publication =
        page::freeze_own_records(key, page, &snapshot, &folded, &records, decoder, now)?;
    Ok(Some((publication, action)))
}

/// Final decision preparation uses the same captured admission and revision as
/// publication. The generic own writer does not interpret discussion policy.
pub struct DecisionEdit<'a> {
    pub thread: &'a Reference,
    pub decision: &'a str,
}
pub fn prepare_decision(
    store: &Store,
    key: &Keyring,
    page: &str,
    edit: DecisionEdit<'_>,
    decoder: &mut Decoder,
    now: u64,
) -> Result<page::FrozenPublication> {
    let snapshot = page::snapshot(store, key, page, true)?;
    let (writer, _, _) = key.local_writer()?;
    if snapshot.authority.revoked_devices.contains(&writer) {
        return Err(Fault::Denied.into());
    }
    let folded = snapshot.materialize(key, page, decoder)?;
    let view = project(key, page, &snapshot, &folded);
    let target = view
        .threads
        .iter()
        .find(|row| row.writer == edit.thread.writer && row.id == edit.thread.id)
        .ok_or(Fault::Invalid)?;
    if target.deleted || target.proposal.is_none() || target.decision.is_some() {
        return Err(Fault::Invalid.into());
    }
    let action = crate::threads::status::Decision {
        version: 1,
        kind: "proposal-decision".into(),
        space_id: key.space_id.clone(),
        page_id: page.into(),
        epoch: snapshot.epoch.to_string(),
        sender_device: writer,
        revision: "1".into(),
        deleted: false,
        device_name: "Local CLI".into(),
        at: now.to_string(),
        action_id: page::fresh_id()?,
        thread: edit.thread.clone(),
        previous: None,
        decision: edit.decision.into(),
    };
    let records = [OwnRecord {
        root: "messages".into(),
        key: format!("{}:proposal-decision", action.action_id),
        value: serde_json::to_value(action)?,
    }];
    page::freeze_own_records(key, page, &snapshot, &folded, &records, decoder, now)
}

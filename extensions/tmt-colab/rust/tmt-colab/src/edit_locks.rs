//! Root-local edit-lock reads and own-action preparation. The decoder owns Yjs;
//! the CLI supplies current public identity/presence observations, never a timer.
use crate::{
    Result,
    decoder::{Decoder, OwnRecord},
    edit_lock::{self, Action, Holder, Target},
    fold::Snapshot,
    keyring::Keyring,
    page::{self, Fault},
    store::Store,
};
use serde::Serialize;

#[derive(Serialize)]
pub struct View {
    #[serde(skip)]
    pub writer: String,
    pub revision: String,
    pub locks: Vec<Action>,
}
fn project(
    key: &Keyring,
    page: &str,
    snapshot: &Snapshot,
    folded: &crate::fold::View,
) -> Result<Vec<Action>> {
    Ok(edit_lock::project(
        &folded.own,
        &folded.status_writers,
        &key.space_id,
        page,
        &snapshot.epoch.to_string(),
    )?)
}
pub fn read(store: &Store, key: &Keyring, page: &str, decoder: &mut Decoder) -> Result<View> {
    let snapshot = page::snapshot(store, key, page, false)?;
    let folded = snapshot.materialize(key, page, decoder)?;
    Ok(View {
        writer: key.local_writer()?.0,
        revision: page::token(
            &key.space_id,
            page,
            &snapshot.authority.head,
            snapshot.epoch,
            &snapshot.cuts,
        )?,
        locks: project(key, page, &snapshot, &folded)?,
    })
}
#[derive(Clone)]
pub enum Change {
    Acquire { holder: Holder, target: Target },
    Release { holder_id: String, lock_id: String },
}
/// One captured expected revision and one own batch, including any explicit stale cleanup.
/// A concurrent lock cannot be silently replaced by preparing against a newer snapshot.
pub fn prepare(
    store: &Store,
    key: &Keyring,
    page: &str,
    expected: &str,
    changes: &[Change],
    decoder: &mut Decoder,
    now: u64,
) -> Result<Option<(page::FrozenPublication, Vec<Action>)>> {
    let snapshot = page::snapshot(store, key, page, true)?;
    let revision = page::token(
        &key.space_id,
        page,
        &snapshot.authority.head,
        snapshot.epoch,
        &snapshot.cuts,
    )?;
    if revision != expected {
        return Err(Fault::StaleBase.into());
    }
    let (writer, _, _) = key.local_writer()?;
    let folded = snapshot.materialize(key, page, decoder)?;
    let mut locks = project(key, page, &snapshot, &folded)?;
    let mut actions = Vec::new();
    let mut records = Vec::new();
    for change in changes {
        let holder_id = match change {
            Change::Acquire { holder, .. } => &holder.id,
            Change::Release { holder_id, .. } => holder_id,
        };
        let previous = locks.iter().find(|lock| lock.holder.id == *holder_id);
        let (holder, target, lock_id, released) = match change {
            Change::Acquire { holder, target } => {
                target.validate()?;
                (holder.clone(), target.clone(), page::fresh_id()?, false)
            }
            Change::Release { lock_id, .. } => {
                let Some(previous) = previous else {
                    return Err(Fault::Invalid.into());
                };
                if previous.lock_id != *lock_id {
                    return Err(Fault::StaleBase.into());
                }
                if previous.released {
                    continue;
                }
                (
                    previous.holder.clone(),
                    previous.target.clone(),
                    lock_id.clone(),
                    true,
                )
            }
        };
        let action = Action {
            version: 1,
            kind: "edit-lock".into(),
            space_id: key.space_id.clone(),
            page_id: page.into(),
            epoch: snapshot.epoch.to_string(),
            sender_device: writer.clone(),

            lock_id,
            holder,
            revision: previous
                .map_or(Ok(1), |lock| lock.revision.parse::<u64>().map(|n| n + 1))?
                .to_string(),
            target,
            released,
        };
        action.validate()?;
        locks.retain(|lock| lock.holder.id != action.holder.id);
        locks.push(action.clone());
        let live = locks
            .iter()
            .filter(|lock| !lock.released && lock.sender_device == writer)
            .count();
        if live > crate::limits::EDIT_LOCKS_PER_WRITER {
            return Err("This writer already holds the maximum live edit locks. Unlock a finished edit before locking another range.".into());
        }
        records.push(OwnRecord {
            root: "messages".into(),
            key: format!("{}:edit-lock", action.holder.id),
            value: serde_json::to_value(&action)?,
        });
        actions.push(action);
    }
    if records.is_empty() {
        return Ok(None);
    }
    if records.len() > crate::limits::OWN_RECORDS {
        return Err(
            "Too many locks for one unlock. Select a lock ID and unlock it separately.".into(),
        );
    }
    let publication =
        page::freeze_own_records(key, page, &snapshot, &folded, &records, decoder, now)?;
    Ok(Some((publication, actions)))
}

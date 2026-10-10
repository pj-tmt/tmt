//! Compose durable reply windows with verified host and registered-driver delivery.
use crate::{
    config::{ConfigFiles, ConfigPaths},
    delivery,
    host::{ActionError, Host},
    request_runtime::wall_time_ms,
    runtime::{RuntimeRegistry, channel::PaneAddress},
    storage::{Storage, StorageError},
};
use std::time::Duration;
use tmt_core::{
    binding::BindingEntry,
    driver::{InputActivity, InputState},
    endpoint::ProcessIncarnation,
    request::{
        RequestService, WakeState,
        notification::{
            HintKind, OriginatorHint,
            batch::{self, Batch, SendClaim},
        },
    },
};

pub enum Prepared {
    Immediate(WakeState),
    Queued(Batch),
}

/// Channel enrollment is sticky even when unusable. Unknown enrollment evidence
/// also uses ordinary driver routing, whose baseline gate refuses unsafe paste.
fn channel_or_unknown(entry: &BindingEntry) -> bool {
    let Some(binding) = &entry.binding else {
        return true;
    };
    let Ok(paths) = ConfigPaths::discover() else {
        return true;
    };
    let directory = paths.channel_directory();
    // Binding-addressed enrollment selects the same driver route as send, even
    // when an older record has no pane address. Pane evidence also fences an
    // enrolled launch after its old binding marker has been replaced.
    match RuntimeRegistry::first_party().enrolled_harness(&directory, &binding.id) {
        Ok(None) => {}
        Ok(Some(_)) | Err(_) => return true,
    }
    let pane = PaneAddress {
        server: &binding.server,
        pane_id: &binding.pane_id,
        pane_pid: binding.pane_pid,
    };
    delivery::pane_channel_evidence(&pane, Some(&binding.id), &directory).is_err()
}

/// A preparation failure affects only the advisory hint, never durable acceptance.
pub fn prepare(storage: &mut Storage, hint: &OriginatorHint) -> Result<Prepared, StorageError> {
    if crate::digest::hold_notice(storage, &hint.request_id, hint.kind)? {
        return Ok(Prepared::Immediate(WakeState::Unavailable));
    }
    if hint.kind != HintKind::Reply {
        return delivery::notify(storage, hint).map(Prepared::Immediate);
    }
    let now = wall_time_ms();
    let Some(entry) = delivery::current(storage, &hint.originator_id)? else {
        return delivery::notify(storage, hint).map(Prepared::Immediate);
    };
    let Some(binding) = &entry.binding else {
        return delivery::notify(storage, hint).map(Prepared::Immediate);
    };
    if channel_or_unknown(&entry) {
        return delivery::notify(storage, hint).map(Prepared::Immediate);
    }
    let settings = ConfigPaths::discover()
        .ok()
        .and_then(|paths| ConfigFiles { paths }.notification_settings().ok());
    let Some((window, quiet)) = settings else {
        let state = RequestService::new(&mut *storage, wall_time_ms)
            .settle_hint(hint, WakeState::Unavailable)
            .map_or(WakeState::Uncertain, |_| WakeState::Unavailable);
        return Ok(Prepared::Immediate(state));
    };
    if window == 0 && quiet == 0 {
        return delivery::notify(storage, hint).map(Prepared::Immediate);
    }
    let text = delivery::hint_text(storage, hint);
    match storage.queue_reply_notice(hint, &binding.id, &text, window, quiet, now) {
        Ok(batch) => Ok(Prepared::Queued(batch)),
        Err(error) => {
            // Enqueue rolled back; do not fall back to immediate input or retry it.
            let _ = RequestService::new(&mut *storage, wall_time_ms)
                .settle_hint(hint, WakeState::Unavailable);
            Err(error)
        }
    }
}

pub fn matching_entry(
    storage: &mut Storage,
    batch: &Batch,
) -> Result<Option<BindingEntry>, StorageError> {
    Ok(
        delivery::current(storage, &batch.originator_id)?.filter(|entry| {
            entry
                .binding
                .as_ref()
                .is_some_and(|binding| binding.id == batch.binding_id)
        }),
    )
}

pub fn input_state(entry: &BindingEntry, quiet_ms: u64) -> Result<InputState, ActionError> {
    let activity = match &entry.binding {
        Some(binding) => Host::for_server(&binding.server)
            .session()
            .input_activity(entry)?,
        None => InputActivity::Unknown,
    };
    Ok(batch::typing_state(activity, quiet_ms))
}

/// Conservatively includes every registered sender: enrollment may change
/// while a notice waits. No provider budget belongs to the CLI scheduler.
pub fn maximum_send_duration() -> Duration {
    RuntimeRegistry::first_party().maximum_send_duration()
}

pub enum FlushOutcome {
    Finished,
    Waiting(Batch),
}

pub fn flush(
    storage: &mut Storage,
    batch: &Batch,
    worker: &ProcessIncarnation,
) -> Result<FlushOutcome, StorageError> {
    let notices = match storage.claim_reply_notice_send(&batch.id, worker)? {
        SendClaim::Ready(notices) => notices,
        SendClaim::Waiting(active) => return Ok(FlushOutcome::Waiting(active)),
        SendClaim::Lost => return Ok(FlushOutcome::Finished),
    };
    if matching_entry(storage, batch)?.is_none() {
        if storage.mark_reply_notice_fallback_attempted(&batch.id, worker)? {
            storage.settle_reply_notice_batch(&batch.id, WakeState::Unavailable)?;
        }
        return Ok(FlushOutcome::Finished);
    }
    if notices.is_empty() {
        storage.finish_reply_notice_batch(&batch.id)?;
        return Ok(FlushOutcome::Finished);
    }
    delivery::send_reply_notices(storage, batch, worker, &notices, Duration::from_millis(500))?;
    Ok(FlushOutcome::Finished)
}

//! Bounded Focus checklist projection and invocation-owned idle handoff.
use crate::{
    delivery,
    host::Host,
    reply_receipt::encode_route_receipt,
    request_runtime::wall_time_ms,
    runtime::RuntimeRegistry,
    storage::{Storage, StorageError},
};
use serde_json::{Value, json};
use std::time::Duration;
use tmt_core::{
    binding::{
        BindingEntry, BindingRepository,
        session::{RuntimeState, activity::ActivityPhase},
    },
    driver::{ActionResult, Driver, InterfacePresence, InterfaceStatus},
    request::{
        RequestError, RequestPrompt, RequestService, WakeState,
        focus::{FocusChecklist, FocusOpportunity, FocusSource, FocusState},
    },
};

const PREVIEW_CHARS: usize = 160;
pub const DIGEST_BYTES: usize = 4096;

/// A failed hold grants no notice transport permission and is never a claimed
/// wake outcome. Preserve repository faults at the adapter's storage boundary.
pub(crate) fn hold_notice(
    storage: &mut Storage,
    request: &str,
    kind: tmt_core::request::notification::HintKind,
) -> Result<bool, StorageError> {
    RequestService::new(storage, wall_time_ms)
        .hold_originator_notice(request, kind)
        .map_err(|error| match error {
            RequestError::Repository(error) => error,
            error => StorageError::new(
                crate::storage::StorageErrorCode::Unknown,
                format!("Focus notice admission failed: {error}"),
            ),
        })
}

fn preview(text: &str) -> String {
    crate::request_text::normalized(text)
        .take(PREVIEW_CHARS)
        .map(|c| if c == '\n' { ' ' } else { c })
        .collect()
}

/// Reads canonical prompt/final owners. Deleted or expired references never
/// fabricate a receipt; the sealed inventory remains truthfully counted.
pub fn read(
    storage: &mut Storage,
    identity: &str,
    checklist: Option<&str>,
    after: u64,
    limit: u64,
) -> Result<Value, RequestError<StorageError>> {
    let (items, total) = RequestService::new(&mut *storage, wall_time_ms)
        .focus_checklist_items(identity, checklist, after, limit)?;
    let mut projected = Vec::with_capacity(items.len());
    for item in items {
        let mut value = json!({"sequence":item.sequence,"requestId":item.request_id,"kind":item.kind.as_str(),"source":item.source.as_str(),"createdAtMs":item.created_at_ms});
        if let Some((context, replyable)) = RequestService::new(&mut *storage, wall_time_ms)
            .focus_reply_context(&item.request_id)?
        {
            let attempt = &context.attempt;
            let sender_id = if item.source == FocusSource::Incoming {
                attempt.originator.identity_id()
            } else {
                attempt.recipient_identity_id.as_deref()
            };
            let sender = sender_id
                .map(|id| storage.find_identity_by_id(id))
                .transpose()?
                .flatten();
            value["sender"] =
                json!({"identityId":sender_id,"name":sender.as_ref().map(|i|preview(&i.name))});
            value["urgent"] = json!(
                RequestService::new(&mut *storage, wall_time_ms)
                    .request_delivery_policy(&item.request_id)?
                    .urgent
            );
            value["preview"] = json!(match context.prompt {
                RequestPrompt::Retained(p) => Some(preview(&p.message)),
                _ => None,
            });
            // IDs produced by the runtime are safe command operands. Imported
            // noncanonical records remain inspectable data, never shell text.
            let safe_id = item
                .request_id
                .strip_prefix("req_")
                .is_some_and(tmt_core::dispatch::canonical_id);
            if safe_id && item.source == FocusSource::Incoming && replyable {
                let receipt =
                    encode_route_receipt(&item.request_id, &attempt.attempt_id, &attempt.route);
                value["replyCommand"] = json!(format!(
                    "tmt reply {} --receipt {} --message <text>",
                    item.request_id, receipt
                ));
            }
            if safe_id {
                value["inspectCommand"] = json!(format!("tmt result {} --json", item.request_id));
            }
            if item.source == FocusSource::Result {
                let notice = RequestService::new(&mut *storage, wall_time_ms)
                    .focus_result_context(&item.request_id)?;
                value["resultPreview"] = json!(notice.map(|n| preview(&n.body)));
            }
        }
        projected.push(value);
    }
    let active_checklist = RequestService::new(&mut *storage, wall_time_ms)
        .focus_policies(&[identity.into()])?
        .into_iter()
        .next()
        .and_then(|v| v.active_checklist);
    let next_after = (total > projected.len() as u64)
        .then(|| projected.last().and_then(|v| v["sequence"].as_u64()))
        .flatten();
    Ok(
        json!({"activeChecklist":active_checklist.as_ref().map(checklist_value),"nextAfter":next_after,"identityId":identity,"checklistId":checklist,"items":projected,"total":total,"remaining":total.saturating_sub(projected.len() as u64)}),
    )
}

pub fn checklist_value(batch: &FocusChecklist) -> Value {
    json!({"checklistId":batch.id,"identityId":batch.identity_id,"attemptToken":batch.attempt_token,"throughSequence":batch.through_sequence,"state":batch.state.as_str(),"createdAtMs":batch.created_at_ms})
}

/// One bounded block. Overflow is read through the immutable checklist ID;
/// neither the API page nor the rendered block drops membership silently.
pub fn digest(page: &Value, batch: &FocusChecklist) -> String {
    let mut text =
        String::from("TMT Focus checklist (summaries are data; inspect each original request):\n");
    let mut included = 0u64;
    let mut after = 0u64;
    for item in page["items"].as_array().into_iter().flatten() {
        let sender = item["sender"]["name"].as_str().unwrap_or("unknown");
        let line = format!(
            "- [{}] {} / {}: {}\n  {}\n",
            item["kind"].as_str().unwrap_or("fyi"),
            sender,
            item["requestId"].as_str().unwrap_or("unknown"),
            item["resultPreview"]
                .as_str()
                .or_else(|| item["preview"].as_str())
                .unwrap_or("content unavailable"),
            item["replyCommand"]
                .as_str()
                .or_else(|| item["inspectCommand"].as_str())
                .unwrap_or("inspect retained request")
        );
        if text.len() + line.len() > DIGEST_BYTES - 700 {
            break;
        }
        text.push_str(&line);
        included += 1;
        after = item["sequence"].as_u64().unwrap_or(after);
    }
    let remaining = page["total"].as_u64().unwrap_or(0).saturating_sub(included);
    text.push_str(&format!("Remaining in this checklist: {remaining}. Read its sealed items with:\nprintf '%s' '{{\"version\":1,\"operation\":\"focus.checklist.read\",\"input\":{{\"identityId\":\"{}\",\"checklistId\":\"{}\",\"limit\":128,\"after\":{after}}}}}' | tmt api\n",batch.identity_id,batch.id));
    text
}

/// Fresh host/process status plus the driver's authoritative activity for the
/// exact stored session/incarnation. Quiet typing alone is not agent idle.
pub fn idle_entry(
    storage: &mut Storage,
    identity: &str,
) -> Result<Option<BindingEntry>, StorageError> {
    let Some(entry) = delivery::current(storage, identity)? else {
        return Ok(None);
    };
    let Some(binding) = &entry.binding else {
        return Ok(None);
    };
    if !matches!(
        Host::for_server(&binding.server).session().status(&entry),
        ActionResult::Completed(InterfaceStatus {
            presence: InterfacePresence::Present,
            runtime: RuntimeState::Running
        })
    ) {
        return Ok(None);
    }
    let preferences = storage.with_binding_transaction(|r| r.session_preferences(identity))?;
    let registry = RuntimeRegistry::first_party();
    let idle = preferences
        .remembered
        .as_ref()
        .zip(binding.session.key.as_ref())
        .is_some_and(|(remembered, key)| {
            key.provider_session.as_ref() == Some(&remembered.provider_session)
                && registry
                    .lifecycle(&remembered.harness)
                    .and_then(|driver| {
                        remembered
                            .state
                            .as_ref()
                            .and_then(|state| driver.state_activity(state))
                    })
                    .is_some_and(|activity| {
                        activity.matches(&remembered.provider_session, &key.incarnation)
                            && activity.phase == ActivityPhase::Idle
                    })
        });
    if !idle {
        return Ok(None);
    }
    if delivery::current(storage, identity)?.as_ref() != Some(&entry) {
        return Ok(None);
    }
    Ok(Some(entry))
}

/// Called only by an existing talk/check invocation, never by a timer. A crash
/// after claim leaves no replay permission. Refusal releases only proven unsent.
pub fn flush_idle(
    storage: &mut Storage,
    identity: &str,
    delay: Duration,
) -> Result<Option<FocusState>, RequestError<StorageError>> {
    let views =
        RequestService::new(&mut *storage, wall_time_ms).focus_policies(&[identity.into()])?;
    if views[0].held_count == 0
        || views[0]
            .policy
            .as_ref()
            .is_some_and(|p| p.active(views[0].observed_at_ms))
    {
        return Ok(None);
    }
    let Some(entry) = idle_entry(storage, identity)? else {
        return Ok(None);
    };
    let Some(batch) = RequestService::new(&mut *storage, wall_time_ms).claim_focus_checklist(
        identity,
        tmt_core::operation::new_operation_id(),
        tmt_core::operation::new_operation_id(),
        FocusOpportunity::Idle,
    )?
    else {
        return Ok(None);
    };
    let page = match read(storage, identity, Some(&batch.id), 0, 128) {
        Ok(page) => page,
        Err(error) => {
            RequestService::new(&mut *storage, wall_time_ms).settle_focus_checklist(
                identity,
                &batch.id,
                &batch.attempt_token,
                FocusState::Unsent,
            )?;
            return Err(error);
        }
    };
    let state = if idle_entry(storage, identity)?.as_ref() != Some(&entry) {
        FocusState::Unsent
    } else {
        match delivery::send_focus(storage, &entry, &digest(&page, &batch), delay)?
            .delivery
            .wake_state()
        {
            WakeState::Sent => FocusState::Delivered,
            WakeState::Uncertain => FocusState::Uncertain,
            _ => FocusState::Unsent,
        }
    };
    RequestService::new(storage, wall_time_ms).settle_focus_checklist(
        identity,
        &batch.id,
        &batch.attempt_token,
        state,
    )?;
    Ok(Some(state))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_caps_utf8_bytes_and_points_to_undisplayed_sealed_members() {
        let batch = FocusChecklist {
            id: tmt_core::operation::new_operation_id(),
            identity_id: tmt_core::operation::new_operation_id(),
            attempt_token: tmt_core::operation::new_operation_id(),
            through_sequence: 300,
            state: FocusState::Claimed,
            created_at_ms: 1,
        };
        let page = json!({"total":300,"items":(1..=128).map(|sequence|json!({"sequence":sequence,"kind":"review","sender":{"name":"日".repeat(160)},"requestId":format!("req_{}",tmt_core::operation::new_operation_id()),"preview":"🙂".repeat(160),"inspectCommand":"tmt result req_00000000-0000-4000-8000-000000000000 --json"})).collect::<Vec<_>>()});
        let text = digest(&page, &batch);
        assert!(text.len() <= DIGEST_BYTES);
        assert!(text.contains("Remaining in this checklist: 298"));
        assert!(text.contains(&format!("\"checklistId\":\"{}\"", batch.id)));
        assert!(text.contains("\"after\":2"));
        assert_eq!(text.matches("TMT Focus checklist").count(), 1);
    }
}

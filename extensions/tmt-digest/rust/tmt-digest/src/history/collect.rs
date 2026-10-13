//! Reconcile public settlement evidence without invoking a delivery operation.

use super::{Store, invalid, model::Page};
use crate::core::{Core, Error};
use serde_json::json;
use std::time::Instant;

pub fn collect(core: &Core, store: &mut Store, id: &str, deadline: Instant) -> Result<(), Error> {
    if Instant::now() >= deadline {
        return Ok(());
    }
    let Some(mut after) = store.cursor(id)? else {
        let page = Page::parse(
            core.api(
                "digest.checklist.history",
                json!({"identityId":id,"limit":1}),
            )?,
            id,
        )?;
        return store.baseline(
            id,
            page.observed_at_ms,
            page.through_sequence,
            page.delivered_digests,
        );
    };
    // Backlogs persist incrementally. A bounded tick can stop without losing a page.
    for _ in 0..4 {
        if Instant::now() >= deadline {
            return Ok(());
        }
        let page = Page::parse(
            core.api(
                "digest.checklist.history",
                json!({"identityId":id,"after":after,"limit":32}),
            )?,
            id,
        )?;
        if page.through_sequence < after {
            return Err(invalid("History watermark moved backwards"));
        }
        for mut event in page.checklists {
            if event.settlement_sequence <= after {
                return Err(invalid("History cursor did not advance"));
            }
            for _ in 0..32 {
                let Some(item_after) = event.next_item_after else {
                    break;
                };
                if Instant::now() >= deadline {
                    return Ok(());
                }
                if event.items.last().map(|i| i.sequence) != Some(item_after) {
                    return Err(invalid("Invalid member continuation"));
                }
                let continuation = Page::parse(core.api("digest.checklist.history",json!({"identityId":id,"checklistId":event.checklist_id,"after":item_after,"limit":128}))?,id)?;
                if continuation.checklists.len() != 1 {
                    return Err(invalid("Checklist disappeared during collection"));
                }
                let next = continuation
                    .checklists
                    .into_iter()
                    .next()
                    .expect("one record");
                if next.checklist_id != event.checklist_id
                    || next.settlement_sequence != event.settlement_sequence
                    || next.sealed_count != event.sealed_count
                    || next.outcome != event.outcome
                    || next.settled_at_ms != event.settled_at_ms
                    || next.items.first().is_some_and(|i| i.sequence <= item_after)
                {
                    return Err(invalid("Checklist changed during continuation"));
                }
                event.membership_complete &= event.retained_count == next.retained_count;
                event.retained_count = event.retained_count.max(next.retained_count);
                event.membership_complete &= next.membership_complete;
                event.next_item_after = next.next_item_after;
                event.items.extend(next.items);
            }
            // A larger event remains explicitly incomplete; never guess its missing inputs.
            store.record(id, &event, page.observed_at_ms)?;
            after = event.settlement_sequence;
        }
        if let Some(next) = page.next_after {
            if next != after {
                return Err(invalid("Invalid settlement continuation"));
            }
        } else {
            return store.checkpoint(
                id,
                page.observed_at_ms,
                page.through_sequence,
                page.delivered_digests,
            );
        }
    }
    Ok(())
}

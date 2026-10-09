//! Which document attachments still wait for a re-seal under the page's current epoch (#2293).
//! The answer is read from the page itself, never remembered: a descriptor whose epoch is not the
//! page's was sealed before the last `epoch.advance`.
use crate::{Result, page};
use std::time::Instant;
use tmt_colab_model::attachment::Descriptor;

/// The document attachments sealed under an earlier epoch, in a stable order, and the epoch they
/// should move to. An archived, deleted or read-only page has none: it cannot take the swap.
pub(crate) fn pending(
    source: &page::save::SourceOpener,
    page_id: &str,
    deadline: Instant,
) -> Result<Vec<Descriptor>> {
    let mut view = source()?;
    let snapshot = match page::snapshot(&view.store, &view.keyring, page_id, true) {
        Ok(snapshot) => snapshot,
        Err(_) => return Ok(Vec::new()),
    };
    let folded = snapshot.materialize_until(&view.keyring, page_id, &mut view.decoder, deadline)?;
    let epoch = snapshot.epoch.to_string();
    let mut waiting = Vec::new();
    for item in folded
        .meta
        .get("attachments")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
    {
        let descriptor = Descriptor::from_json(&serde_json::to_vec(item)?)?;
        if descriptor.epoch != epoch {
            waiting.push(descriptor);
        }
    }
    waiting.sort_by(|a, b| a.attachment_id.cmp(&b.attachment_id));
    Ok(waiting)
}

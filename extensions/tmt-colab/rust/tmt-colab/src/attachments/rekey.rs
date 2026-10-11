//! Which document attachments still wait for a re-seal under the page's current epoch (#2293).
//! The answer is read from the page itself, never remembered: a descriptor whose epoch is not the
//! page's was sealed before the last `epoch.advance`.
use crate::{Result, page};
use std::time::Instant;
use tmt_colab_model::attachment::Descriptor;

/// Replace complete attachment references, leaving every unrelated byte untouched. The old ID
/// remains a candidate for its short prefix even though the swap removed it from the list.
pub(crate) fn repair_source(source: &str, old: &str, new: &str, listed: &[Descriptor]) -> String {
    const PREFIX: &str = "colab-attachment:";
    let short = &old[..8];
    let unique = !listed
        .iter()
        .any(|item| item.attachment_id != old && item.attachment_id.starts_with(short));
    let mut result = String::with_capacity(source.len());
    let mut rest = source;
    while let Some(start) = rest.find(PREFIX) {
        if rest[..start]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_ascii_alphanumeric() || "-_.:/?#%+=&;~@!$*".contains(c))
        {
            result.push_str(&rest[..start + PREFIX.len()]);
            rest = &rest[start + PREFIX.len()..];
            continue;
        }
        result.push_str(&rest[..start + PREFIX.len()]);
        rest = &rest[start + PREFIX.len()..];
        let end = rest
            .find(|c: char| !c.is_ascii_alphanumeric() && !"-_.:/?#%+=&;~@!$*".contains(c))
            .unwrap_or(rest.len());
        let token = &rest[..end];
        result.push_str(if token == old || (unique && token == short) {
            new
        } else {
            token
        });
        rest = &rest[end..];
    }
    result.push_str(rest);
    result
}

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

#[cfg(test)]
mod tests {
    use super::*;
    const OLD: &str = "20000000-0000-4000-8000-000000000092";
    const NEW: &str = "30000000-0000-4000-8000-000000000092";
    #[test]
    fn exact_full_and_short_references_change_and_every_other_byte_stays() {
        let source = format!(
            "é\n<img src='colab-attachment:{OLD}'><img src=\"colab-attachment:20000000\"> xcolab-attachment:{OLD} colab-attachment:{OLD}.png colab-attachment:{OLD}?x colab-attachment:200000000 colab-attachment:{NEW} 🦀"
        );
        let expected = source
            .replacen(
                &format!("'colab-attachment:{OLD}'"),
                &format!("'colab-attachment:{NEW}'"),
                1,
            )
            .replacen(
                "\"colab-attachment:20000000\"",
                &format!("\"colab-attachment:{NEW}\""),
                1,
            );
        assert_eq!(repair_source(&source, OLD, NEW, &[]), expected);
        assert_eq!(
            repair_source("unrelated é 🦀", OLD, NEW, &[]),
            "unrelated é 🦀"
        );
    }
}

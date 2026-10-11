//! Which document attachments still wait for a re-seal under the page's current epoch (#2293).
//! The answer is read from the page itself, never remembered: a descriptor whose epoch is not the
//! page's was sealed before the last `epoch.advance`.
use crate::{Result, page};
use std::time::Instant;
use tmt_colab_model::attachment::Descriptor;

/// Replace complete image source references, leaving every unrelated byte untouched. The old ID
/// remains a candidate for its short prefix even though the swap removed it from the list.
pub(crate) fn repair_source(
    source: &str,
    old: &str,
    new: &str,
    listed: &[Descriptor],
) -> Option<String> {
    let unique = !listed
        .iter()
        .any(|item| item.attachment_id != old && item.attachment_id.starts_with(&old[..8]));
    crate::page_check::repair_references(source, old, new, unique)
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
        assert_eq!(repair_source(&source, OLD, NEW, &[]), Some(expected));
        assert_eq!(
            repair_source("unrelated é 🦀", OLD, NEW, &[]),
            Some("unrelated é 🦀".into())
        );
    }
    #[test]
    fn prose_examples_other_attributes_and_comments_are_unchanged() {
        for other in [
            format!("<p>colab-attachment:{OLD}</p>"),
            format!("<code>&lt;img src='colab-attachment:{OLD}'&gt;</code>"),
            format!("<img alt='colab-attachment:{OLD}' data-src='colab-attachment:{OLD}'>"),
            format!("<!-- <img src='colab-attachment:{OLD}'> -->"),
            format!("<script>const example = \"<img src='colab-attachment:{OLD}'>\";</script>"),
            format!("<textarea><img src='colab-attachment:{OLD}'></textarea>"),
            format!("<a src='colab-attachment:{OLD}'>sample</a>"),
            format!(
                "<img src='colab-attachment:{OLD}.png'><img src='prefixcolab-attachment:{OLD}'>"
            ),
            format!("<!DOCTYPE html PUBLIC \"<img src='colab-attachment:{OLD}'>\">"),
        ] {
            let source = format!("{other}<IMG SRC=colab-attachment:{OLD}>");
            assert_eq!(
                repair_source(&source, OLD, NEW, &[]),
                Some(format!("{other}<IMG SRC=colab-attachment:{NEW}>"))
            );
        }
    }

    #[test]
    fn parsed_and_editable_image_counts_must_match() {
        for source in [
            format!("<template><img src='colab-attachment:{OLD}'></template>"),
            format!("<img src='colab-attachment&#58;{OLD}'>"),
            format!("<img src='colab-attachment:{OLD}' src='colab-attachment:{OLD}'>"),
        ] {
            assert_eq!(repair_source(&source, OLD, NEW, &[]), None);
        }
    }
}

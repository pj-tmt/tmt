//! The write owner's admission of one sub-page action, shared by the write command and the serve's
//! browser frame so the link rules cannot drift between them. The stored value never carries these
//! rules (contract: Sub-pages); they run before an action is prepared, and a refusal writes
//! nothing.
use crate::{
    decoder::Decoder,
    inspection,
    keyring::{Keyring, Layout},
    store::Store,
    work, work_index,
    work_links::{Graph, Refusal},
};
use serde_json::Value;
use std::path::Path;

/// A refused action: the stable code and the words a person reads.
#[derive(Debug)]
pub struct Refused {
    pub code: &'static str,
    pub message: String,
    pub reason: Option<&'static str>,
}
impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for Refused {}
pub(crate) fn refused(
    code: &'static str,
    message: impl Into<String>,
) -> Box<dyn std::error::Error + Send + Sync> {
    Box::new(Refused {
        code,
        message: message.into(),
        reason: None,
    })
}

pub fn invalid(message: &str) -> Box<dyn std::error::Error + Send + Sync> {
    refused("COLAB_INPUT_INVALID", message)
}

/// The data root a space is read from, as the admission needs it.
pub struct Space<'a> {
    pub root: &'a Path,
    pub layout: &'a Layout,
    pub store: &'a Store,
    pub key: &'a Keyring,
}

fn short(page: &str) -> &str {
    page.get(..8).unwrap_or(page)
}
/// Why `child` may not become a sub-page of `page`, in words.
pub fn link_refusal(
    why: &Refusal,
    page: &str,
    child: &str,
) -> Box<dyn std::error::Error + Send + Sync> {
    let message = match why {
        Refusal::Itself => "A page cannot be its own sub-page.".to_owned(),
        Refusal::Unavailable => format!(
            "{} is not a page of this space, or cannot be read now.",
            short(child)
        ),
        Refusal::HasParent { page: parent } => format!(
            "{} is already a sub-page of {}. A page has one parent: detach it there first (tmt colab subpage detach {} {}).",
            short(child),
            short(parent),
            short(parent),
            short(child)
        ),
        Refusal::InvalidHierarchy => format!(
            "{} belongs to a hierarchy with conflicting links. Inspect tmt colab show {} --json and repair it with tmt colab subpage detach before adding a link.",
            short(page),
            short(page)
        ),
        Refusal::Loop => format!(
            "{} has {} below it, so attaching it there would make a loop.",
            short(child),
            short(page)
        ),
    };
    Box::new(Refused {
        code: "COLAB_LINK_REFUSED",
        message,
        reason: Some(why.reason()),
    })
}

/// Preserve the reason when a CLI adds context to the original admission refusal.
pub fn reason(mut error: &(dyn std::error::Error + 'static)) -> Option<&'static str> {
    loop {
        if let Some(refused) = error.downcast_ref::<Refused>() {
            return refused.reason;
        }
        error = error.source()?;
    }
}

fn space_work(space: &Space<'_>, decoder: &mut Decoder) -> crate::Result<work_index::Found> {
    let pages = inspection::all_page_ids(space.store, space.key)?;
    work_index::scan_with_live(
        space.layout,
        space.store,
        space.key,
        decoder,
        &pages,
        Some(&pages),
    )
}

/// Shared by the CLI and the browser's `subpage.create`, before either makes a page.
pub fn admit_creation(space: &Space<'_>, decoder: &mut Decoder, page: &str) -> crate::Result<()> {
    if let Some(why) = Graph(&space_work(space, decoder)?).creation_refusal(page) {
        return Err(link_refusal(&why, page, ""));
    }
    Ok(())
}

/// Admits one sub-page action on `page`, whose list now is `before`: the action applies to the
/// list, and an attach takes a page that has no other parent and closes no loop. Detach and move
/// need only the list, so a stored conflict can always be repaired.
pub fn admit(
    space: &Space<'_>,
    decoder: &mut Decoder,
    page: &str,
    before: &Value,
    action: &str,
    body: &Value,
) -> crate::Result<()> {
    if action == "subpage.attach"
        && let Some(child) = body["pageId"].as_str()
        && let Some(why) = Graph(&space_work(space, decoder)?).refusal(page, child)
    {
        return Err(link_refusal(&why, page, child));
    }
    // The summary is the value's own rule (`work::apply`); only the list needs the space.
    if !work::SUBPAGE_ACTIONS.contains(&action) {
        return Ok(());
    }
    work::apply(before, action, body, &Value::Null).map_err(|_| match action {
        "subpage.attach" => invalid("That page is already a sub-page here; nothing was changed."),
        _ => invalid("That page is not a sub-page here; tmt colab subpage ls shows the list."),
    })?;
    Ok(())
}

//! The write owner's admission of one work action, shared by the write command and the serve's
//! browser frame so the limit and the link rules cannot drift between them. The stored value
//! never carries these rules (contract: Work); they run before an action is prepared, and a
//! refusal writes nothing.
use crate::{
    decoder::Decoder,
    inspection,
    keyring::{Keyring, Layout},
    store::Store,
    wip, work, work_index,
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
/// An action the value or its body cannot take, or a task the page does not have.
/// The refusal for an action that names a task the page does not have: one sentence for the write
/// command, the GitHub push, a convert and the browser's actions.
pub fn no_such_task(task: &str) -> String {
    format!("This page has no task {task}; tmt colab task ls shows its tasks.")
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

use crate::repair::argument as quoted;
fn short(page: &str) -> &str {
    page.get(..8).unwrap_or(page)
}
fn wip_refusal(refusal: &wip::Refusal) -> Box<dyn std::error::Error + Send + Sync> {
    let tasks: Vec<String> = refusal
        .held
        .iter()
        .map(|held| format!("{} {}", short(&held.page), held.task))
        .collect();
    let count = refusal.held.len() - 1;
    refused(
        "COLAB_WIP_LIMIT",
        format!(
            "{} already holds {count} task{} (limit {}): {}. Finish, block or reassign one, or raise the limit with: tmt colab settings wip-limit {} {}",
            refusal.agent,
            if count == 1 { "" } else { "s" },
            refusal.limit,
            tasks.join(", "),
            quoted(&refusal.agent),
            refusal.limit + 1,
        ),
    )
}
/// Why a task may not follow a page, in words.
pub fn link_refusal(
    why: &Refusal,
    page: &str,
    task: &str,
    child: &str,
) -> Box<dyn std::error::Error + Send + Sync> {
    let message = match why {
        Refusal::Itself => "A task cannot follow its own page.".to_owned(),
        Refusal::Unavailable => format!(
            "{} is not a page of this space, or cannot be read now.",
            short(child)
        ),
        Refusal::HasParent { page, task } => format!(
            "{} is already followed by {task} of {}. A page has one parent: stop that task following it first (tmt colab task unlink {} {task}).",
            short(child),
            short(page),
            short(page)
        ),
        Refusal::InvalidHierarchy => format!(
            "{} belongs to a hierarchy with conflicting links. Inspect tmt colab show {} --json and repair it with task unlink before adding a link.",
            short(page),
            short(page)
        ),
        Refusal::Loop => format!(
            "{} has {} below it, so {task} following it would make a loop.",
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

/// Shared by CLI and browser conversion, before either creates a page.
pub fn admit_conversion(
    space: &Space<'_>,
    decoder: &mut Decoder,
    page: &str,
    task: &str,
) -> crate::Result<()> {
    let pages = inspection::all_page_ids(space.store, space.key)?;
    let found = work_index::scan_with_live(
        space.layout,
        space.store,
        space.key,
        decoder,
        &pages,
        Some(&pages),
    )?;
    if let Some(why) = Graph(&found).conversion_refusal(page) {
        return Err(link_refusal(&why, page, task, ""));
    }
    Ok(())
}

/// Admits one `task.*` action on `page`, whose work now is `before`: the task it names exists, the
/// action applies, no agent ends up over its work-in-progress limit and a link closes no loop. The
/// space-wide scan only runs when the change puts a task into an agent's hands or links a page.
pub fn admit(
    space: &Space<'_>,
    decoder: &mut Decoder,
    page: &str,
    before: &Value,
    action: &str,
    body: &Value,
) -> crate::Result<()> {
    if let Some(task) = body["taskId"].as_str()
        && before["tasks"].get(task).is_none()
    {
        return Err(invalid(&no_such_task(task)));
    }
    if !action.starts_with("task.") {
        return Ok(());
    }
    let after = work::apply(before, action, body, &Value::Null)
        .map_err(|_| invalid("Invalid task value or action; nothing was changed."))?;
    // Unlink is a repair operation, including when its restored task exceeds WIP.
    if action != "task.unlink"
        && wip::held(page, &after)
            .iter()
            .any(|task| !wip::held(page, before).contains(task))
    {
        let catalog = inspection::catalog(space.store, space.key)?;
        let pages = inspection::catalog_active_page_ids(&catalog);
        let live = inspection::catalog_page_ids(&catalog);
        let found = work_index::scan_with_live(
            space.layout,
            space.store,
            space.key,
            decoder,
            &pages,
            Some(&live),
        )?;
        let elsewhere = work_index::held_elsewhere(&found, page);
        let limits = crate::settings::read_or_default(space.root);
        if let Some(over) = wip::admit(page, before, &after, &elsewhere, limits.wip()) {
            return Err(wip_refusal(&over));
        }
    }
    if action == "task.link" {
        let (task, child) = (
            body["taskId"].as_str().unwrap_or_default(),
            body["pageId"].as_str().unwrap_or_default(),
        );
        let pages = inspection::all_page_ids(space.store, space.key)?;
        let found = work_index::scan_with_live(
            space.layout,
            space.store,
            space.key,
            decoder,
            &pages,
            Some(&pages),
        )?;
        if let Some(why) = Graph(&found).refusal(page, task, child) {
            return Err(link_refusal(&why, page, task, child));
        }
    }
    Ok(())
}

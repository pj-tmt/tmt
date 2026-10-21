//! A browser's `task.convert`: a page made from a task (its title, its detail as paragraphs,
//! nothing else), then `task.link` to it, as one operation of the owner device. The page ID is
//! derived from the operation, so a lost reply is answered by whether that page exists and a
//! replay finds its page. A failure after the page is made leaves the page and names it; nothing
//! is retried, and no page is ever searched for or adopted.
use super::{
    save::{Prepared, SourceOpener},
    work::{WorkEdit, prepare_expecting},
};
use crate::{
    inspection::all_page_ids,
    work_admission::invalid,
    work_convert::{child_page_id, child_source, convertible},
};
use serde_json::json;

/// What preparing a convert came to: the page it made (when it got that far) and whether the link
/// is ready to commit.
pub struct Converted {
    pub child: Option<String>,
    pub prepared: crate::Result<(Prepared, Option<String>)>,
}

/// `create(page, title, source)` makes the page; the caller holds whatever lock it needs only
/// for that call. Everything else here runs on snapshots, outside every lock.
pub fn prepare(
    open: &SourceOpener,
    edit: &WorkEdit,
    clock: &dyn Fn() -> crate::Result<u64>,
    create: &mut dyn FnMut(&str, &str, &str) -> crate::Result<()>,
) -> Converted {
    let mut child = None;
    let prepared = (|| {
        // The body is exactly the task: nothing the caller supplies reaches the new page.
        let task_id = match edit.body.as_object() {
            Some(body) if body.len() == 1 => body.get("taskId").and_then(|id| id.as_str()),
            _ => None,
        }
        .ok_or_else(|| invalid("A convert names exactly one task."))?;
        let mut source = open()?;
        let read = super::read_work(
            &source.store,
            &source.keyring,
            &edit.page,
            &mut source.decoder,
        )?;
        let task = convertible(&read.work, task_id)
            .map_err(|blocked| invalid(&blocked.message(task_id)))?
            .clone();
        let page = child_page_id(&source.keyring.space_id, &edit.operation_id);
        let made = all_page_ids(&source.store, &source.keyring)?.contains(&page);
        let title = task["title"].as_str().unwrap_or_default();
        if !made {
            create(
                &page,
                title,
                &child_source(title, task["detail"].as_str().unwrap_or_default()),
            )?;
        }
        child = Some(page.clone());
        drop(source);
        prepare_expecting(
            open,
            &WorkEdit {
                page: edit.page.clone(),
                operation_id: edit.operation_id.clone(),
                action: "task.link".into(),
                body: json!({"taskId": task_id, "pageId": page}),
            },
            clock,
            Some((task_id, &task)),
        )
    })();
    Converted { child, prepared }
}

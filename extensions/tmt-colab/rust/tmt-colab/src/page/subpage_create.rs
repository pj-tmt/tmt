//! A browser's `subpage.create`: a page made from a title (the title as its heading, nothing
//! else), then `subpage.attach` of it, as one operation of the owner device. The page ID is
//! derived from the operation, so a lost reply is answered by whether that page exists and a
//! replay finds its page. A failure after the page is made leaves the page and names it; nothing
//! is retried, and no page is ever searched for or adopted.
use super::{
    save::{Prepared, SourceOpener},
    subpage::{SubpageEdit, prepare as prepare_attach},
};
use crate::{
    inspection::all_page_ids,
    subpage_create::{child_page_id, child_source, title},
    work_admission::{Space, admit_creation, invalid},
};
use serde_json::json;

/// What preparing a create came to: the page it made (when it got that far) and whether the
/// attach is ready to commit.
pub struct Created {
    pub child: Option<String>,
    pub prepared: crate::Result<Prepared>,
}

/// `create(page, title, source)` makes the page; the caller holds whatever lock it needs only
/// for that call. Everything else here runs on snapshots, outside every lock.
pub fn prepare(
    open: &SourceOpener,
    edit: &SubpageEdit,
    clock: &dyn Fn() -> crate::Result<u64>,
    create: &mut dyn FnMut(&str, &str, &str) -> crate::Result<()>,
) -> Created {
    let mut child = None;
    let prepared = (|| {
        // The body is exactly the title: nothing else the caller supplies reaches the new page.
        let title = match edit.body.as_object() {
            Some(body) if body.len() == 1 => body.get("title").and_then(|value| title(value).ok()),
            _ => None,
        }
        .ok_or_else(|| invalid("A new sub-page needs exactly a title."))?
        .to_owned();
        let mut source = open()?;
        let page = child_page_id(&source.keyring.space_id, &edit.operation_id);
        let made = all_page_ids(&source.store, &source.keyring)?.contains(&page);
        if made {
            child = Some(page.clone());
        }
        admit_creation(
            &Space {
                root: &source.root,
                layout: &source.layout,
                store: &source.store,
                key: &source.keyring,
            },
            &mut source.decoder,
            &edit.page,
        )?;
        if !made {
            create(&page, &title, &child_source(&title))?;
        }
        child = Some(page.clone());
        drop(source);
        prepare_attach(
            open,
            &SubpageEdit {
                page: edit.page.clone(),
                operation_id: edit.operation_id.clone(),
                action: "subpage.attach".into(),
                body: json!({"pageId": page}),
            },
            clock,
        )
    })();
    Created { child, prepared }
}

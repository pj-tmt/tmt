//! The owner's local plaintext page titles (#4218): a display-only projection a summary worker
//! writes from the verified fold it already runs, so Home, cards and search need no decrypt step.
//! It lives in the owner store (`page_titles`), is never a log statement and is never synced,
//! published, exported or shared; deleting or archiving a page removes its row, and losing the rows
//! loses nothing because the next fold writes them again (contract: Schema 16).
use crate::{
    keyring::{Keyring, Layout},
    store::Store,
};

/// One worker's best-effort writer: its own writable store handle, opened on first use. A write that
/// cannot happen (no handle, a busy or stale store) leaves the page without a row, never fails the
/// summary, and is tried again at the page's next fold.
pub(crate) struct TitleWriter {
    layout: Layout,
    store: Option<Store>,
}

impl TitleWriter {
    pub(crate) fn new(layout: &Layout) -> Self {
        Self {
            layout: layout.clone(),
            store: None,
        }
    }

    /// Records `title` for `page` when it is still at `expected`, the fingerprint its fold started
    /// from. `None` is a page that is not active (archived or deleted): it keeps no title.
    pub(crate) fn record(
        &mut self,
        keyring: &Keyring,
        page: &str,
        expected: &str,
        title: Option<&str>,
    ) {
        if self.store.is_none() {
            self.store = Store::open(&self.layout).ok();
        }
        let Some(store) = self.store.as_mut() else {
            return;
        };
        // Best effort by design (see the type); the error has nowhere useful to go.
        let _ =
            store.project_transaction(
                &keyring.space_id,
                &keyring.owner_public(),
                |tx| match title {
                    Some(title) => tx.record_page_title(page, expected, title).map(|_| ()),
                    None if tx.page_fingerprint(page)?.as_deref() == Some(expected) => {
                        tx.drop_page_title(page)
                    }
                    None => Ok(()),
                },
            );
    }
}

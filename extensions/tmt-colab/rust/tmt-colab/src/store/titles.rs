//! The local plaintext title projection (#4218, contract: colab-v1 "Local title projection"): each
//! open page's title as the owner's verified fold last read it. It is display state for the
//! owner's own surfaces, never authority: not signed, not in the owner log, never synced,
//! published or exported. Reads only select from it; the owner-side fold path writes it
//! ([`Store::record_page_title`]); a delete or an archive removes a page's row in its own
//! transaction (`OwnerTransaction`).
use super::{Store, StoreResult};
use std::collections::BTreeMap;

impl Store {
    /// Every stored title by page.
    pub fn page_titles(&self) -> StoreResult<BTreeMap<String, String>> {
        let mut query = self
            .connection
            .prepare("SELECT page,title FROM page_titles ORDER BY page")?;
        let rows = query.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
    /// Records what one owner fold says about a page's title, in one transaction with the checks
    /// that make it current: the page is still at the `fingerprint` the fold was made at (an
    /// accepted write, an archive or any membership change moves it, and the next fold records
    /// again) and still has an epoch secret (a fold that finishes after a delete never revives
    /// it). `Some` stores the title, `None` (an untitled or archived page) removes the row.
    /// Returns whether it recorded.
    pub fn record_page_title(
        &mut self,
        space: &str,
        root: &[u8; 32],
        page: &str,
        fingerprint: &str,
        title: Option<&str>,
    ) -> crate::Result<bool> {
        self.project_transaction(space, root, |tx| {
            tx.record_page_title(page, fingerprint, title)
        })
    }
    /// Whether the schema-16 backfill still has to fold the existing pages.
    pub fn title_backfill_pending(&self) -> StoreResult<bool> {
        Ok(self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM page_titles_backfill)",
            [],
            |r| r.get(0),
        )?)
    }
    /// The backfill folded every page it could; later folds keep the projection current.
    pub fn finish_title_backfill(&self) -> StoreResult<()> {
        self.connection
            .execute("DELETE FROM page_titles_backfill", [])?;
        Ok(())
    }
}

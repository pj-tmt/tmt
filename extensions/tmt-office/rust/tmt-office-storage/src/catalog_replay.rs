//! Private replay-state mechanics shared by independent typed catalog owners.

use tmt_core::limits::MAX_JS_SAFE_INTEGER;

#[derive(Debug)]
pub(super) struct CatalogReplayState {
    pub revision: u64,
    pub previous_kind: Option<String>,
    pub previous_digest: Option<String>,
    pub previous_base_revision: Option<u64>,
    pub previous_result_revision: Option<u64>,
}

impl CatalogReplayState {
    pub fn recognizes(&self, kind: &str, digest: &str, expected_revision: u64) -> bool {
        self.previous_kind.as_deref() == Some(kind)
            && self.previous_digest.as_deref() == Some(digest)
            && self.previous_base_revision == Some(expected_revision)
            && self.previous_result_revision == Some(self.revision)
    }

    pub fn next_revision(&self) -> Option<u64> {
        self.revision
            .checked_add(1)
            .filter(|revision| *revision <= MAX_JS_SAFE_INTEGER)
    }
}

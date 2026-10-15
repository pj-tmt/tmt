//! What the owner's serve decided about each edit request. A decision is final: a request is
//! applied at most once, and one that was refused is never evaluated again.
use super::{Fault, Store, StoreResult, bounded_id};
use rusqlite::params;
use std::collections::BTreeSet;
use tmt_colab_model::values;

/// How the serve settled one request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditState {
    /// The page holds the request's change (a commit, a merge, or nothing left to change).
    Applied,
    /// The page moved and the request's hunks no longer fit it.
    Conflict,
    /// The request could not be applied: its link ended, its page moved on, or it was refused.
    Refused,
}
impl EditState {
    fn name(self) -> &'static str {
        match self {
            Self::Applied => "applied",
            Self::Conflict => "conflict",
            Self::Refused => "refused",
        }
    }
}
/// The most decisions kept for one epoch of one page; further requests wait undecided.
pub const EDIT_DECISIONS: usize = 1024;

impl Store {
    /// The `(writer, operation)` pairs already decided for the epoch.
    pub fn edit_decisions(
        &self,
        page: &str,
        epoch: u64,
    ) -> StoreResult<BTreeSet<(String, String)>> {
        bounded_id(page)?;
        let mut query = self
            .connection
            .prepare("SELECT writer,operation FROM edit_decisions WHERE page=? AND epoch=?")?;
        let rows = query
            .query_map(params![page, epoch.to_string()], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    }
    /// Record the decision about one request. The first decision stands; a later one is ignored.
    pub fn decide_edit(
        &mut self,
        page: &str,
        epoch: u64,
        writer: &str,
        operation: &str,
        state: EditState,
    ) -> StoreResult<()> {
        bounded_id(page)?;
        values::generated_id(writer).map_err(|_| Fault::Invalid)?;
        values::generated_id(operation).map_err(|_| Fault::Invalid)?;
        let tx = self.connection.transaction()?;
        // A decision about an earlier epoch's request can never matter again.
        tx.execute(
            "DELETE FROM edit_decisions WHERE page=? AND epoch<>?",
            params![page, epoch.to_string()],
        )?;
        let count: i64 = tx.query_row(
            "SELECT count(*) FROM edit_decisions WHERE page=? AND epoch=?",
            params![page, epoch.to_string()],
            |r| r.get(0),
        )?;
        if count >= EDIT_DECISIONS as i64 {
            return Err(Fault::Capacity);
        }
        tx.execute(
            "INSERT OR IGNORE INTO edit_decisions VALUES (?,?,?,?,?)",
            params![page, epoch.to_string(), writer, operation, state.name()],
        )?;
        tx.commit()?;
        Ok(())
    }
}

//! Read-only views of the settlement store for the savings report.

use super::{Store, io, number};
use crate::{
    core::Error,
    history::{MAX, invalid, model::ChecklistSettlement},
};
use rusqlite::params;
use std::collections::BTreeMap;

/// Where one member's collected evidence begins and ends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Coverage {
    pub started: u64,
    pub through: u64,
    pub raw_from: u64,
    /// A recorded gap overlapping the window, if any.
    pub gap: Option<String>,
}

impl Store {
    /// Settled checklists in `(from, through]`, by member then settlement time. The
    /// stored input is the original content-free event, validated on the way out.
    pub fn settlements(
        &self,
        from: u64,
        through: u64,
    ) -> Result<Vec<(String, ChecklistSettlement)>, Error> {
        if from > through || through > MAX {
            return Err(invalid("Invalid settlement window"));
        }
        let mut statement = self
            .0
            .prepare("SELECT identity_id,inputs FROM events WHERE settled>?1 AND settled<=?2 ORDER BY identity_id,settled,sequence")
            .map_err(io)?;
        let rows = statement
            .query_map(params![from as i64, through as i64], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(io)?;
        let mut settlements = vec![];
        for row in rows {
            let (id, inputs) = row.map_err(io)?;
            let event: ChecklistSettlement =
                serde_json::from_str(&inputs).map_err(|_| invalid("Invalid stored settlement"))?;
            event.validate()?;
            settlements.push((id, event));
        }
        Ok(settlements)
    }

    /// Every collected member's evidence span and any gap overlapping the window.
    pub fn coverage(&self, from: u64, through: u64) -> Result<BTreeMap<String, Coverage>, Error> {
        let mut statement = self
            .0
            .prepare("SELECT id,started,through,raw_from,(SELECT reason FROM gaps WHERE identity_id=members.id AND from_ms<=?2 AND through_ms>=?1 LIMIT 1) FROM members")
            .map_err(io)?;
        let rows = statement
            .query_map(params![from as i64, through as i64], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    Coverage {
                        started: number(r, 1)?,
                        through: number(r, 2)?,
                        raw_from: number(r, 3)?,
                        gap: r.get(4)?,
                    },
                ))
            })
            .map_err(io)?;
        rows.map(|row| row.map_err(io)).collect()
    }
}

//! The Codex sessions a bound session started (#3828): one family per
//! consumption source and one row per rollout still being followed. Rows hold
//! counters and read cursors, never transcript content.
use super::{Storage, StorageError, StorageErrorCode, errors::classify};
use crate::runtime::consumption::{
    State,
    family::{Header, Member, Stored, Write},
};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::HashSet;

fn corrupt() -> StorageError {
    StorageError::new(
        StorageErrorCode::Corrupt,
        "Invalid consumption family evidence",
    )
}

fn counts_json(counts: &crate::runtime::consumption::Counts) -> Result<String, StorageError> {
    serde_json::to_string(counts).map_err(|_| corrupt())
}

fn state_json(state: &State) -> Result<String, StorageError> {
    serde_json::to_string(&state.document()).map_err(|_| corrupt())
}

impl Storage {
    /// The family stored for `id`, whatever session it belongs to. A row that
    /// does not read back is corrupt, never partly used.
    pub fn consumption_family(&self, id: &str) -> Result<Option<Stored>, StorageError> {
        let connection = self.connection()?;
        let header = connection
            .query_row(
                "SELECT session,scan,edge,closed,warm FROM consumption_family WHERE identity_id=?",
                [id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, i64>(4)?,
                    ))
                },
            )
            .optional()
            .map_err(|e| classify(e, "Read consumption family"))?;
        let Some((session, scan, edge, closed, warm)) = header else {
            return Ok(None);
        };
        let header = Header {
            session,
            scan,
            edge: serde_json::from_str(&edge).map_err(|_| corrupt())?,
            closed: serde_json::from_str(&closed).map_err(|_| corrupt())?,
            warm: match warm {
                0 => false,
                1 => true,
                _ => return Err(corrupt()),
            },
        };
        if !header.valid() {
            return Err(corrupt());
        }
        let mut statement = connection
            .prepare(
                "SELECT file,child,parent,base,state,touched_ms FROM consumption_family_files WHERE identity_id=? ORDER BY file",
            )
            .map_err(|e| classify(e, "Read consumption family"))?;
        let rows = statement
            .query_map([id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            })
            .map_err(|e| classify(e, "Read consumption family"))?;
        let mut members = Vec::new();
        for row in rows {
            let (file, child, parent, base, state, touched) =
                row.map_err(|e| classify(e, "Read consumption family"))?;
            let member = Member {
                file,
                child,
                parent,
                base: serde_json::from_str(&base).map_err(|_| corrupt())?,
                state: State::read(&serde_json::from_str(&state).map_err(|_| corrupt())?)
                    .ok_or_else(corrupt)?,
                touched_ms: u64::try_from(touched).map_err(|_| corrupt())?,
            };
            if !member.valid() {
                return Err(corrupt());
            }
            members.push(member);
        }
        Ok(Some(Stored { header, members }))
    }

    /// The sessions other seats of `driver` are bound to: they count only
    /// toward their own seat.
    pub fn other_bound_sessions(
        &self,
        id: &str,
        driver: &str,
    ) -> Result<HashSet<String>, StorageError> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare("SELECT session FROM consumption_sources WHERE driver=? AND identity_id<>?")
            .map_err(|e| classify(e, "Read bound sessions"))?;
        let sessions = statement
            .query_map(params![driver, id], |row| row.get::<_, String>(0))
            .map_err(|e| classify(e, "Read bound sessions"))?
            .collect::<Result<HashSet<_>, _>>();
        sessions.map_err(|e| classify(e, "Read bound sessions"))
    }
}

/// Inside the observation's transaction, after the source row exists.
pub(super) fn apply(connection: &Connection, id: &str, write: &Write) -> Result<(), StorageError> {
    let fail = |e| classify(e, "Write consumption family");
    if write.reset {
        connection
            .execute("DELETE FROM consumption_family WHERE identity_id=?", [id])
            .map_err(fail)?;
    }
    if let Some(header) = &write.header {
        if !header.valid() {
            return Err(corrupt());
        }
        connection
            .execute(
                "INSERT INTO consumption_family(identity_id,session,scan,edge,closed,warm) VALUES(?,?,?,?,?,?) \
                 ON CONFLICT(identity_id) DO UPDATE SET session=excluded.session,scan=excluded.scan,edge=excluded.edge,closed=excluded.closed,warm=excluded.warm",
                params![
                    id,
                    header.session,
                    header.scan,
                    serde_json::to_string(&header.edge).map_err(|_| corrupt())?,
                    counts_json(&header.closed)?,
                    i64::from(header.warm)
                ],
            )
            .map_err(fail)?;
    }
    for file in &write.deletes {
        connection
            .execute(
                "DELETE FROM consumption_family_files WHERE identity_id=? AND file=?",
                params![id, file],
            )
            .map_err(fail)?;
    }
    for member in &write.upserts {
        if !member.valid() {
            return Err(corrupt());
        }
        let touched = i64::try_from(member.touched_ms).map_err(|_| corrupt())?;
        connection
            .execute(
                "INSERT INTO consumption_family_files(identity_id,file,child,parent,base,state,touched_ms) VALUES(?,?,?,?,?,?,?) \
                 ON CONFLICT(identity_id,file) DO UPDATE SET child=excluded.child,parent=excluded.parent,base=excluded.base,state=excluded.state,touched_ms=excluded.touched_ms",
                params![
                    id,
                    member.file,
                    member.child,
                    member.parent,
                    counts_json(&member.base)?,
                    state_json(&member.state)?,
                    touched
                ],
            )
            .map_err(fail)?;
    }
    Ok(())
}

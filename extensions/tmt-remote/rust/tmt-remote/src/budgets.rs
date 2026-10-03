//! Persisted fixed-window budgets; clock rollback never resets a window.
use crate::{error::RemoteError, store::database};
use rusqlite::{OptionalExtension, Transaction, params};

pub const CALLS: i64 = 120;
pub const SENDS: i64 = 20;
pub const APPROVALS: i64 = 60;
pub const HELDS: i64 = 16;
const MINUTE: i64 = 60_000;
const KEYS: i64 = 100_000;
#[derive(Clone, Copy)]
pub(crate) enum Budget {
    Call,
    Send,
    Approval,
}
impl Budget {
    fn fields(self) -> (&'static str, i64) {
        match self {
            Self::Call => ("call", CALLS),
            Self::Send => ("send", SENDS),
            Self::Approval => ("approval", APPROVALS),
        }
    }
}
pub(crate) fn charge(
    tx: &Transaction<'_>,
    subject: &str,
    kind: Budget,
    now: u64,
) -> Result<(), RemoteError> {
    let now = i64::try_from(now).map_err(database)?;
    let (kind, maximum) = kind.fields();
    let old: Option<(i64, i64)> = tx
        .query_row(
            "SELECT started_ms,used FROM budgets WHERE subject=?1 AND kind=?2",
            params![subject, kind],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(database)?;
    let (started, used) = match old {
        Some((started, used)) if started >= 0 && (1..=maximum).contains(&used) => {
            if now.saturating_sub(started) >= MINUTE {
                (now, 0)
            } else {
                (started, used)
            }
        }
        Some(_) => return Err(database("invalid budget state")),
        None => {
            let keys: i64 = tx
                .query_row("SELECT COUNT(*) FROM budgets", [], |r| r.get(0))
                .map_err(database)?;
            if keys >= KEYS {
                return Err(database("budget capacity"));
            }
            (now, 0)
        }
    };
    if used >= maximum {
        return Err(RemoteError::new(
            "REMOTE_RATE_LIMITED",
            "Remote minute budget exhausted.",
        ));
    }
    tx.execute("INSERT INTO budgets VALUES (?1,?2,?3,?4) ON CONFLICT(subject,kind) DO UPDATE SET started_ms=excluded.started_ms,used=excluded.used",
        params![subject,kind,started,used+1]).map_err(database)?;
    Ok(())
}

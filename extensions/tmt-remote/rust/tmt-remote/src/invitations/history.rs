//! Bounded metadata history. Codes, words, keys and provider locators have no field here.
use super::{
    Context, invalid,
    service::{ProviderCleanup, State, Status},
};
use crate::{
    canonical,
    error::RemoteError,
    store::{Store, database, machine_in},
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::Serialize;

type Result<T> = std::result::Result<T, RemoteError>;
const RETENTION_MS: u64 = 7 * 24 * 60 * 60 * 1_000;
const CAPACITY: i64 = 200;
const PAGE: usize = 50;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Row {
    pub offer_id: String,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
    pub state: String,
    pub local_confirmation_known: bool,
    pub cleanup: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Page {
    pub remote_id: String,
    pub invitations: Vec<Row>,
    pub next_cursor: Option<String>,
}
pub(crate) fn phase(state: State) -> &'static str {
    match state {
        State::Open => "open",
        State::CandidateAwaitingOwner => "awaiting-owner",
        State::Confirmed => "confirmed",
        State::Refused => "refused",
        State::Cancelled => "cancelled",
        State::Expired => "expired",
        State::ProviderOutcomeUnconfirmed => "unconfirmed",
    }
}
fn cleanup(value: ProviderCleanup) -> &'static str {
    match value {
        ProviderCleanup::NotRequested => "not-requested",
        ProviderCleanup::Acknowledged => "acknowledged",
        ProviderCleanup::Unconfirmed => "unconfirmed",
    }
}
fn time(value: u64) -> Result<i64> {
    if value >= 1 << 53 {
        return Err(invalid());
    }
    i64::try_from(value).map_err(database)
}
pub(crate) fn interrupt(c: &Connection, now: u64) -> Result<()> {
    c.execute("UPDATE issued_invitations SET state='interrupted',terminal=1,updated_ms=?1 WHERE terminal=0", [time(now)?]).map_err(database)?;
    prune(c, now)
}
fn prune(c: &Connection, now: u64) -> Result<()> {
    if now < RETENTION_MS {
        return Ok(());
    }
    c.execute(
        "DELETE FROM issued_invitations WHERE terminal=1 AND updated_ms<=?1",
        [time(now.saturating_sub(RETENTION_MS))?],
    )
    .map_err(database)?;
    Ok(())
}
pub(crate) fn record(
    c: &Connection,
    context: &Context,
    status: &Status,
    now: u64,
    create: bool,
) -> Result<()> {
    context.validate()?;
    if status.offer.as_deref() != Some(context.offer.as_str())
        || status.expires_at_ms != Some(context.expires_at_ms)
        || machine_in(c)?.id != context.owner
    {
        return Err(invalid());
    }
    prune(c, now)?;
    let previous:Option<(i64,i64,String)>=c.query_row("SELECT issued_ms,expires_ms,state FROM issued_invitations WHERE remote_id=?1 AND offer_id=?2",params![context.owner,context.offer],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional().map_err(database)?;
    if let Some((issued, expires, state)) = previous {
        if issued != time(context.issued_at_ms)?
            || expires != time(context.expires_at_ms)?
            || state == "interrupted"
        {
            return Err(invalid());
        }
    } else {
        if !create {
            return Ok(());
        }
        let count: i64 = c
            .query_row("SELECT COUNT(*) FROM issued_invitations", [], |r| r.get(0))
            .map_err(database)?;
        if count >= CAPACITY {
            c.execute("DELETE FROM issued_invitations WHERE (remote_id,offer_id) IN (SELECT remote_id,offer_id FROM issued_invitations WHERE terminal=1 ORDER BY updated_ms,offer_id LIMIT ?1)",[count-CAPACITY+1]).map_err(database)?;
        }
        let count: i64 = c
            .query_row("SELECT COUNT(*) FROM issued_invitations", [], |r| r.get(0))
            .map_err(database)?;
        if count >= CAPACITY {
            return Err(RemoteError::new(
                "REMOTE_INVITATION_CAPACITY",
                "Invitation history is full.",
            ));
        }
    }
    let terminal = status.local_confirmation_known
        || matches!(
            status.state,
            State::Confirmed | State::Refused | State::Cancelled | State::Expired
        );
    c.execute("INSERT INTO issued_invitations VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)
        ON CONFLICT(remote_id,offer_id) DO UPDATE SET state=excluded.state,confirmation_known=excluded.confirmation_known,cleanup=excluded.cleanup,terminal=excluded.terminal,updated_ms=excluded.updated_ms
        WHERE state!=excluded.state OR confirmation_known!=excluded.confirmation_known OR cleanup!=excluded.cleanup OR terminal!=excluded.terminal",
        params![context.owner,context.offer,time(context.issued_at_ms)?,time(context.expires_at_ms)?,phase(status.state),status.local_confirmation_known,cleanup(status.cleanup),terminal,time(now)?]).map_err(database)?;
    Ok(())
}
impl Store {
    pub(crate) fn record_invitation(
        &mut self,
        context: &Context,
        status: &Status,
        now: u64,
        create: bool,
    ) -> Result<()> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database)?;
        record(&tx, context, status, now, create)?;
        tx.commit().map_err(database)
    }
    pub(crate) fn invitation_history(&self, cursor: Option<&str>, now: u64) -> Result<Page> {
        prune(&self.connection, now)?;
        let remote = machine_in(&self.connection)?.id;
        let after = cursor
            .map(|value| {
                if value.len() > 128 {
                    return Err(invalid());
                }
                let bytes = canonical::base64url_decode(value).map_err(|_| invalid())?;
                let text = String::from_utf8(bytes).map_err(|_| invalid())?;
                let (time, id) = text.split_once(':').ok_or_else(invalid)?;
                let time = time.parse::<u64>().map_err(|_| invalid())?;
                canonical::uuid(id).map_err(|_| invalid())?;
                if time >= 1 << 53 {
                    return Err(invalid());
                }
                Ok((i64::try_from(time).map_err(database)?, id.to_owned()))
            })
            .transpose()?;
        let mut query = self.connection.prepare("SELECT offer_id,issued_ms,expires_ms,state,confirmation_known,cleanup FROM issued_invitations
            WHERE remote_id=?1 AND (?2 IS NULL OR issued_ms<?2 OR (issued_ms=?2 AND offer_id<?3)) ORDER BY issued_ms DESC,offer_id DESC LIMIT 51").map_err(database)?;
        let mut rows = query
            .query_map(
                params![
                    remote,
                    after.as_ref().map(|a| a.0),
                    after.as_ref().map(|a| &a.1)
                ],
                |r| {
                    Ok(Row {
                        offer_id: r.get(0)?,
                        issued_at_ms: r.get::<_, i64>(1)? as u64,
                        expires_at_ms: r.get::<_, i64>(2)? as u64,
                        state: r.get(3)?,
                        local_confirmation_known: r.get(4)?,
                        cleanup: r.get(5)?,
                    })
                },
            )
            .map_err(database)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(database)?;
        let next_cursor = if rows.len() > PAGE {
            rows.pop();
            rows.last().map(|row| {
                canonical::base64url(format!("{}:{}", row.issued_at_ms, row.offer_id).as_bytes())
            })
        } else {
            None
        };
        Ok(Page {
            remote_id: remote,
            invitations: rows,
            next_cursor,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Layout, Serving};
    use std::path::PathBuf;

    struct Fixture {
        store: Store,
        serving: Serving,
        root: PathBuf,
        now: u64,
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "tmt-invite-history-{}",
                crate::store::uuid_v4().unwrap()
            ));
            let serving = Layout::open(&root).unwrap().serve_lock().unwrap();
            let store = Store::open(&serving).unwrap();
            let canonical = root.canonicalize().unwrap();
            assert!(canonical.starts_with(std::env::temp_dir().canonicalize().unwrap()));
            let database = store.connection.path().unwrap();
            assert!(
                PathBuf::from(database)
                    .canonicalize()
                    .unwrap()
                    .starts_with(&canonical)
            );
            Self {
                store,
                serving,
                root,
                now: crate::pairing::now_ms().unwrap(),
            }
        }
        fn offer(&mut self, offset: u64) -> (Context, Status) {
            let (mut context, _, _) = super::super::tests::fixture();
            context.owner = self.store.machine().unwrap().id;
            context.offer = crate::store::uuid_v4().unwrap();
            context.issued_at_ms = self.now + offset;
            context.expires_at_ms = context.issued_at_ms + 600_000;
            let status = Status {
                offer: Some(context.offer.clone()),
                expires_at_ms: Some(context.expires_at_ms),
                state: State::Open,
                local_confirmation_known: false,
                cleanup: ProviderCleanup::NotRequested,
            };
            (context, status)
        }
        fn reopen(&mut self) {
            self.store.connection = Connection::open_in_memory().unwrap();
            self.store = Store::open(&self.serving).unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn history_retains_active_offer_caps_terminal_rows_and_pages_without_secret_fields() {
        let mut f = Fixture::new();
        let (active, status) = f.offer(0);
        f.store
            .record_invitation(&active, &status, f.now, true)
            .unwrap();
        let mut oldest = None;
        for i in 1..=205 {
            let (context, mut status) = f.offer(i);
            if i == 1 {
                oldest = Some(context.offer.clone());
            }
            status.state = State::Cancelled;
            f.store
                .record_invitation(&context, &status, f.now + i, true)
                .unwrap();
        }
        let count: i64 = f
            .store
            .connection
            .query_row("SELECT COUNT(*) FROM issued_invitations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 200);
        let mut cursor = None;
        let mut offers = std::collections::BTreeSet::new();
        loop {
            let page = f
                .store
                .invitation_history(cursor.as_deref(), f.now + 206)
                .unwrap();
            assert_eq!(page.invitations.len(), 50);
            for row in &page.invitations {
                assert!(offers.insert(row.offer_id.clone()));
            }
            let encoded = serde_json::to_value(&page).unwrap();
            for row in encoded["invitations"].as_array().unwrap() {
                assert_eq!(
                    row.as_object()
                        .unwrap()
                        .keys()
                        .map(String::as_str)
                        .collect::<std::collections::BTreeSet<_>>(),
                    [
                        "offerId",
                        "issuedAtMs",
                        "expiresAtMs",
                        "state",
                        "localConfirmationKnown",
                        "cleanup"
                    ]
                    .into_iter()
                    .collect()
                );
            }
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(offers.len(), 200);
        assert!(offers.contains(&active.offer));
        assert!(!offers.contains(&oldest.unwrap()));
        assert!(
            f.store
                .invitation_history(Some("not-a-cursor"), f.now)
                .is_err()
        );
        let page = f
            .store
            .invitation_history(None, f.now + RETENTION_MS + 206)
            .unwrap();
        assert_eq!(page.invitations.len(), 1);
        assert_eq!(page.invitations[0].offer_id, active.offer);
    }

    #[test]
    fn restart_interrupts_unfinished_offer_and_retention_updates_never_recreate_history() {
        let mut f = Fixture::new();
        let (context, status) = f.offer(0);
        f.store
            .record_invitation(&context, &status, f.now, true)
            .unwrap();
        f.reopen();
        let page = f.store.invitation_history(None, f.now).unwrap();
        assert_eq!(page.invitations[0].state, "interrupted");
        assert!(!page.invitations[0].local_confirmation_known);
        assert!(
            f.store
                .record_invitation(&context, &status, f.now + 1, false)
                .is_err()
        );
        assert!(
            f.store
                .invitation_history(None, f.now + RETENTION_MS + 60_000)
                .unwrap()
                .invitations
                .is_empty()
        );
        f.store
            .record_invitation(&context, &status, f.now + RETENTION_MS + 60_000, false)
            .unwrap();
        assert!(
            f.store
                .invitation_history(None, f.now + RETENTION_MS + 60_001)
                .unwrap()
                .invitations
                .is_empty()
        );
    }
}

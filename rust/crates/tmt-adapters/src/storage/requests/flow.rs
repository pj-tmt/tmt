//! Content-free aggregate SQL; all mutation uses the caller's request transaction.
use super::*;
use tmt_core::request::flow::{self, FlowKind, FlowRow};

const INCREMENT: &str = "INSERT INTO exchange_flow(sender_identity_id,recipient_identity_id,kind,bucket_start_ms,count,bytes)
 VALUES (?1,?2,?3,?4,?5,?6)
 ON CONFLICT(sender_identity_id,recipient_identity_id,kind,bucket_start_ms)
 DO UPDATE SET count = exchange_flow.count + excluded.count, bytes = exchange_flow.bytes + excluded.bytes";

pub(super) fn increment(c: &Connection, row: &FlowRow) -> Result<(), StorageError> {
    c.execute(
        INCREMENT,
        params![
            row.sender_identity_id.as_deref().unwrap_or(""),
            row.recipient_identity_id.as_deref().unwrap_or(""),
            row.kind.as_str(),
            checked_i64(row.bucket_start_ms, "Flow bucket")?,
            checked_i64(row.count, "Flow count")?,
            checked_i64(row.bytes, "Flow bytes")?
        ],
    )
    .map_err(|e| classify(e, "Increment exchange flow"))?;
    Ok(())
}
fn optional_id(id: String) -> Option<String> {
    if id.is_empty() { None } else { Some(id) }
}
pub(super) fn read(c: &Connection, since: u64, now: u64) -> Result<Vec<FlowRow>, StorageError> {
    let mut q = c
        .prepare(
            "SELECT sender_identity_id,recipient_identity_id,kind,bucket_start_ms,count,bytes
 FROM exchange_flow WHERE bucket_start_ms >= ?1 AND bucket_start_ms <= ?2
 ORDER BY bucket_start_ms,sender_identity_id,recipient_identity_id,kind",
        )
        .map_err(|e| classify(e, "Prepare exchange flow"))?;
    let rows = q
        .query_map(
            params![
                checked_i64(since, "Flow window")?,
                checked_i64(flow::bucket(now), "Flow observation")?
            ],
            |r| {
                Ok(FlowRow {
                    sender_identity_id: optional_id(r.get(0)?),
                    recipient_identity_id: optional_id(r.get(1)?),
                    kind: FlowKind::parse(&r.get::<_, String>(2)?)
                        .ok_or(rusqlite::Error::InvalidQuery)?,
                    bucket_start_ms: super::rows::u64_at(r, 3)?,
                    count: super::rows::u64_at(r, 4)?,
                    bytes: super::rows::u64_at(r, 5)?,
                })
            },
        )
        .map_err(|e| classify(e, "Read exchange flow"))?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| classify(e, "Decode exchange flow"))
}
pub(super) fn prune(c: &Connection, through: u64, limit: u64) -> Result<(), StorageError> {
    c.execute(
        "DELETE FROM exchange_flow WHERE rowid IN (SELECT rowid FROM exchange_flow
 WHERE bucket_start_ms <= ?1 ORDER BY bucket_start_ms,rowid LIMIT ?2)",
        params![checked_i64(through, "Flow cutoff")?, checked_limit(limit)?],
    )
    .map_err(|e| classify(e, "Prune exchange flow"))?;
    Ok(())
}
pub(super) fn ackall(
    c: &Connection,
    identity: &str,
    incoming: bool,
    latest: u64,
    now: u64,
) -> Result<(), StorageError> {
    let (actor, other, revision, acknowledged, state, route) = if incoming {
        (
            "recipient_identity_id",
            "originator_identity_id",
            "recipient_attention_revision",
            "recipient_attention_acknowledged_revision",
            "request_recipient_attention_identities",
            "AND a.route_kind = 'inbox' AND a.status = 'queued'",
        )
    } else {
        (
            "originator_identity_id",
            "recipient_identity_id",
            "attention_revision",
            "attention_acknowledged_revision",
            "request_attention_identities",
            "",
        )
    };
    // This SELECT groups newly covered rows in the same snapshot BEFORE advancing
    // the watermark. Per-request acknowledgments and expired metadata are excluded.
    let sql = format!("INSERT INTO exchange_flow(sender_identity_id,recipient_identity_id,kind,bucket_start_ms,count,bytes)
 SELECT ?1,COALESCE(a.{other},''),'ack',?2,COUNT(*),0 FROM request_attempts AS a
 JOIN {state} AS state ON state.identity_id = a.{actor}
 WHERE a.{actor} = ?1 AND a.{revision} > state.acknowledged_through
 AND a.{revision} > a.{acknowledged} AND a.{revision} <= ?3 AND a.retention_expires_at_ms > ?4 {route}
 GROUP BY COALESCE(a.{other},'')
 ON CONFLICT(sender_identity_id,recipient_identity_id,kind,bucket_start_ms)
 DO UPDATE SET count = exchange_flow.count + excluded.count");
    c.execute(
        &sql,
        params![
            identity,
            checked_i64(flow::bucket(now), "Ack flow bucket")?,
            checked_i64(latest, "Ack flow snapshot")?,
            checked_now(now, "Ack flow retention")?
        ],
    )
    .map_err(|e| classify(e, "Record acknowledgment flow"))?;
    Ok(())
}

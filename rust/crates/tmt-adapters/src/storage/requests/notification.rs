use super::*;
use tmt_core::{
    binding::session::RuntimeIncarnation,
    request::notification::{NotificationPolicy, NotificationRecord},
};

fn state(value: &str) -> rusqlite::Result<WakeState> {
    match value {
        "not_attempted" => Ok(WakeState::NotAttempted),
        "claimed" => Ok(WakeState::Claimed),
        "sent" => Ok(WakeState::Sent),
        "unavailable" => Ok(WakeState::Unavailable),
        "uncertain" => Ok(WakeState::Uncertain),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

pub(super) fn read(db: &Connection, id: &str) -> Result<Option<NotificationRecord>, StorageError> {
    db.query_row(
        "SELECT deadline_ms, timeout_ms, waiter_pid, waiter_start,
              reply_state, timeout_state, observed
       FROM request_notifications WHERE request_id = ?",
        [id],
        |row| {
            let pid: Option<i64> = row.get(2)?;
            let start: Option<String> = row.get(3)?;
            let waiter = match (pid, start) {
                (Some(pid), Some(start)) => {
                    let pid = u64::try_from(pid).map_err(|_| rusqlite::Error::InvalidQuery)?;
                    Some(
                        RuntimeIncarnation::new(pid, &start)
                            .map_err(|_| rusqlite::Error::InvalidQuery)?,
                    )
                }
                (None, None) => None,
                _ => return Err(rusqlite::Error::InvalidQuery),
            };
            Ok(NotificationRecord {
                policy: NotificationPolicy {
                    deadline_ms: u64::try_from(row.get::<_, i64>(0)?)
                        .map_err(|_| rusqlite::Error::InvalidQuery)?,
                    timeout_ms: u64::try_from(row.get::<_, i64>(1)?)
                        .map_err(|_| rusqlite::Error::InvalidQuery)?,
                    waiter,
                },
                reply: state(&row.get::<_, String>(4)?)?,
                timeout: state(&row.get::<_, String>(5)?)?,
                observed: row.get(6)?,
            })
        },
    )
    .optional()
    .map_err(|error| classify(error, "Read request notification ownership"))
}

pub(super) fn insert_notification(
    db: &Connection,
    id: &str,
    policy: &NotificationPolicy,
) -> Result<(), StorageError> {
    let pid = policy
        .waiter
        .as_ref()
        .map(|value| checked_now(value.pid(), "Waiter PID"))
        .transpose()?;
    db.execute(
        "INSERT INTO request_notifications
         (request_id, deadline_ms, timeout_ms, waiter_pid, waiter_start)
         VALUES (?, ?, ?, ?, ?)",
        params![
            id,
            checked_now(policy.deadline_ms, "Notification deadline")?,
            checked_now(policy.timeout_ms, "Notification timeout")?,
            pid,
            policy
                .waiter
                .as_ref()
                .map(RuntimeIncarnation::start_identity)
        ],
    )
    .map_err(|error| classify(error, "Create request notification ownership"))?;
    Ok(())
}

pub(super) fn write(
    db: &Connection,
    id: &str,
    value: &NotificationRecord,
) -> Result<(), StorageError> {
    let changed = db
        .execute(
            "UPDATE request_notifications SET reply_state = ?, timeout_state = ?, observed = ?
         WHERE request_id = ?",
            params![
                value.reply.as_str(),
                value.timeout.as_str(),
                value.observed,
                id
            ],
        )
        .map_err(|error| classify(error, "Update request notification ownership"))?;
    if changed != 1 {
        return Err(StorageError::new(
            StorageErrorCode::Unknown,
            "Request notification disappeared",
        ));
    }
    Ok(())
}

//! Subscription registration and retirement share the identity transaction owner.

use rusqlite::{Connection, OptionalExtension, params};
use tmt_core::identity_hooks::{
    IdentityHook, IdentityHookState, MAX_PENDING_IDENTITY_HOOKS, PendingIdentityHook,
    valid_hook_consumer,
};

use super::{
    Storage, StorageError, StorageErrorCode, errors::classify,
    identities::with_immediate_transaction,
};

fn invalid() -> StorageError {
    StorageError::new(
        StorageErrorCode::Unknown,
        "Invalid identity hook input or state",
    )
}

fn state(value: &str) -> Result<IdentityHookState, StorageError> {
    match value {
        "registered" => Ok(IdentityHookState::Registered),
        "pending" => Ok(IdentityHookState::Pending),
        "delivered" => Ok(IdentityHookState::Delivered),
        _ => Err(invalid()),
    }
}

fn read_state(
    connection: &Connection,
    hook: &IdentityHook,
) -> Result<IdentityHookState, StorageError> {
    let value: String = connection.query_row(
        "SELECT state FROM identity_hooks WHERE consumer = ? AND identity_id = ? AND reference = ?",
        params![hook.consumer(), hook.identity_id(), hook.reference()], |row| row.get(0),
    ).map_err(|error| classify(error, "Read identity hook state"))?;
    state(&value)
}

pub(super) fn enqueue_retirement(
    connection: &Connection,
    identity_id: &str,
) -> Result<(), StorageError> {
    connection.execute("UPDATE identity_hooks SET state = 'pending' WHERE identity_id = ? AND state = 'registered'", [identity_id])
        .map_err(|error| classify(error, "Queue identity retirement hooks"))?;
    Ok(())
}

impl Storage {
    /// This consumer's hook state, or `None` when it has no such hook.
    pub fn identity_hook_state(
        &self,
        hook: &IdentityHook,
    ) -> Result<Option<IdentityHookState>, StorageError> {
        let value: Option<String> = self
            .connection()?
            .query_row(
                "SELECT state FROM identity_hooks WHERE consumer = ? AND identity_id = ? AND reference = ?",
                params![hook.consumer(), hook.identity_id(), hook.reference()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| classify(error, "Read identity hook state"))?;
        value.as_deref().map(state).transpose()
    }

    pub fn register_identity_hook(
        &mut self,
        hook: &IdentityHook,
    ) -> Result<IdentityHookState, StorageError> {
        with_immediate_transaction(self, "identity hook registration", |transaction| {
            let retired: bool = transaction
                .query_row(
                    "SELECT retired_at_ms IS NOT NULL FROM identities WHERE id = ?",
                    [hook.identity_id()],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|error| classify(error, "Read hook identity"))?
                .ok_or_else(invalid)?;
            transaction.execute("INSERT INTO identity_hooks (consumer, identity_id, reference, state) VALUES (?, ?, ?, ?) ON CONFLICT (consumer, identity_id, reference) DO NOTHING",
                params![hook.consumer(), hook.identity_id(), hook.reference(), if retired { "pending" } else { "registered" }])
                .map_err(|error| classify(error, "Register identity hook"))?;
            read_state(transaction, hook)
        })
    }

    pub fn pending_identity_hooks(
        &self,
        consumer: &str,
        limit: usize,
    ) -> Result<Vec<PendingIdentityHook>, StorageError> {
        if !valid_hook_consumer(consumer) || !(1..=MAX_PENDING_IDENTITY_HOOKS).contains(&limit) {
            return Err(invalid());
        }
        let mut statement = self.connection()?.prepare("SELECT consumer, identity_id, reference, attempt_count FROM identity_hooks WHERE consumer = ? AND state = 'pending' ORDER BY attempt_count, identity_id, reference LIMIT ?")
            .map_err(|error| classify(error, "Prepare pending identity hooks"))?;
        statement
            .query_map(params![consumer, limit as i64], |row| {
                let hook = IdentityHook::new(
                    &row.get::<_, String>(0)?,
                    &row.get::<_, String>(1)?,
                    &row.get::<_, String>(2)?,
                )
                .map_err(|_| rusqlite::Error::InvalidQuery)?;
                let attempt_count = u64::try_from(row.get::<_, i64>(3)?)
                    .map_err(|_| rusqlite::Error::InvalidQuery)?;
                Ok(PendingIdentityHook {
                    hook,
                    attempt_count,
                })
            })
            .and_then(|rows| rows.collect())
            .map_err(|error| classify(error, "Read pending identity hooks"))
    }

    pub fn count_pending_identity_hooks(&self, consumer: &str) -> Result<u64, StorageError> {
        if !valid_hook_consumer(consumer) {
            return Err(invalid());
        }
        self.connection()?
            .query_row(
                "SELECT COUNT(*) FROM identity_hooks WHERE consumer = ? AND state = 'pending'",
                [consumer],
                |row| {
                    u64::try_from(row.get::<_, i64>(0)?).map_err(|_| rusqlite::Error::InvalidQuery)
                },
            )
            .map_err(|error| classify(error, "Count pending identity hooks"))
    }

    pub fn acknowledge_identity_hook(&mut self, hook: &IdentityHook) -> Result<bool, StorageError> {
        update_pending(
            self,
            hook,
            "UPDATE identity_hooks SET state = 'delivered' WHERE consumer = ? AND identity_id = ? AND reference = ? AND state = 'pending'",
        )
    }

    pub fn record_identity_hook_attempt(
        &mut self,
        hook: &IdentityHook,
    ) -> Result<bool, StorageError> {
        update_pending(
            self,
            hook,
            "UPDATE identity_hooks SET attempt_count = attempt_count + 1 WHERE consumer = ? AND identity_id = ? AND reference = ? AND state = 'pending'",
        )
    }
}

fn update_pending(
    storage: &mut Storage,
    hook: &IdentityHook,
    sql: &str,
) -> Result<bool, StorageError> {
    with_immediate_transaction(storage, "identity hook delivery", |transaction| {
        match read_state(transaction, hook)? {
            IdentityHookState::Delivered => return Ok(false),
            IdentityHookState::Registered => return Err(invalid()),
            IdentityHookState::Pending => {}
        }
        transaction
            .execute(
                sql,
                params![hook.consumer(), hook.identity_id(), hook.reference()],
            )
            .map_err(|error| classify(error, "Update pending identity hook"))?;
        Ok(true)
    })
}

#[cfg(test)]
mod tests;

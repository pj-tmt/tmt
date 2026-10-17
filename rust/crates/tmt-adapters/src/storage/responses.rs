//! Recent pane output shares the admitted runtime observation transaction.
use super::{Storage, StorageError, StorageErrorCode, classify};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use tmt_core::{
    responses::{FinalText, MAX_RECENT},
    retention::retention_deadline,
};

pub struct ResponseCapture {
    pub text: FinalText,
    pub retention_days: u64,
}

pub(super) fn append(
    connection: &Connection,
    id: &str,
    provider: &str,
    session: &str,
    now: u64,
    capture: &ResponseCapture,
) -> Result<(), StorageError> {
    let expires = retention_deadline(now, capture.retention_days).ok_or_else(|| {
        StorageError::new(StorageErrorCode::Corrupt, "Invalid response retention")
    })?;
    connection.execute("INSERT INTO recent_responses(identity_id,text,ended_ms,provider,truncated,session_id,expires_ms) VALUES(?,?,?,?,?,?,?)", params![id,capture.text.text(),now as i64,provider,capture.text.truncated(),session,expires as i64]).map_err(|e| classify(e,"Retain recent response"))?;
    connection.execute("DELETE FROM recent_responses WHERE identity_id=?1 AND (expires_ms<=?2 OR sequence NOT IN (SELECT sequence FROM recent_responses WHERE identity_id=?1 ORDER BY sequence DESC LIMIT 10))",params![id,now as i64]).map_err(|e| classify(e,"Bound recent responses"))?;
    Ok(())
}
impl Storage {
    /// One read snapshot. No hook refresh, acknowledgement, pruning or renewal.
    pub fn recent_responses(
        &mut self,
        id: &str,
        limit: u64,
        now: u64,
    ) -> Result<Value, StorageError> {
        if !tmt_core::dispatch::canonical_id(id)
            || !(1..=MAX_RECENT).contains(&limit)
            || now > tmt_core::limits::MAX_JS_SAFE_INTEGER
        {
            return Err(StorageError::new(
                StorageErrorCode::Corrupt,
                "Invalid recent response query",
            ));
        }
        let transaction = self
            .connection_mut()?
            .transaction()
            .map_err(|e| classify(e, "Read recent responses"))?;
        let items = {
            let mut query = transaction.prepare("SELECT r.text,r.ended_ms,r.provider,r.truncated,r.session_id FROM recent_responses r JOIN identities i ON i.id=r.identity_id AND i.retired_at_ms IS NULL WHERE r.identity_id=? AND r.expires_ms>? ORDER BY r.sequence DESC LIMIT ?").map_err(|e| classify(e,"Prepare recent responses"))?;
            query.query_map(params![id,now as i64,limit as i64],|row| {
                let mut item = json!({"text":row.get::<_,String>(0)?,"endedMs":row.get::<_,i64>(1)?,"provider":row.get::<_,String>(2)?,"truncated":row.get::<_,bool>(3)?});
                if let Some(session) = row.get::<_,Option<String>>(4)? { item["sessionId"] = json!(session); }
                Ok(item)
            }).map_err(|e| classify(e,"Query recent responses"))?.collect::<Result<Vec<_>,_>>().map_err(|e| classify(e,"Decode recent responses"))?
        };
        transaction
            .commit()
            .map_err(|e| classify(e, "Finish recent responses"))?;
        Ok(json!({"identityId":id,"items":items}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_vector_preserves_order_text_and_frozen_expiry() {
        let vector: Value = serde_json::from_str(include_str!(
            "../../../../../contracts/recent-responses-v1.json"
        ))
        .unwrap();
        let directory = crate::test_support::TestDirectory::new();
        let mut storage = Storage::open(directory.path.join("responses.db")).unwrap();
        let id = tmt_core::identity::create_or_resolve(
            &mut storage,
            "Recent",
            tmt_core::identity::Lifetime::Saved,
        )
        .unwrap()
        .identity
        .id;
        for row in vector["captures"].as_array().unwrap() {
            super::super::identities::with_immediate_transaction(
                &mut storage,
                "fixture capture",
                |connection| {
                    append(
                        connection,
                        &id,
                        row["provider"].as_str().unwrap(),
                        row["sessionId"].as_str().unwrap(),
                        row["endedMs"].as_u64().unwrap(),
                        &ResponseCapture {
                            text: FinalText::new(row["text"].as_str().unwrap()).unwrap(),
                            retention_days: row["retentionDays"].as_u64().unwrap(),
                        },
                    )
                },
            )
            .unwrap();
        }
        let mut expected = vector["resource"].clone();
        expected["identityId"] = json!(id);
        assert_eq!(
            storage
                .recent_responses(&id, 10, vector["nowMs"].as_u64().unwrap())
                .unwrap(),
            expected
        );
        assert_eq!(
            storage.recent_responses(&id, 1, 10000).unwrap()["items"],
            json!([expected["items"][0]])
        );
        assert_eq!(
            storage
                .recent_responses(&id, 10, vector["expiry"]["nowMs"].as_u64().unwrap())
                .unwrap()["items"],
            vector["expiry"]["items"]
        );
        assert_eq!(
            storage
                .recent_responses(
                    vector["unknownIdentityResource"]["identityId"]
                        .as_str()
                        .unwrap(),
                    10,
                    10000
                )
                .unwrap(),
            vector["unknownIdentityResource"]
        );
        storage.close().unwrap();
    }

    #[test]
    fn expiry_overflow_rolls_back_instead_of_renewing_a_capture() {
        let directory = crate::test_support::TestDirectory::new();
        let mut storage = Storage::open(directory.path.join("responses.db")).unwrap();
        let id = tmt_core::identity::create_or_resolve(
            &mut storage,
            "Recent",
            tmt_core::identity::Lifetime::Saved,
        )
        .unwrap()
        .identity
        .id;
        let result = super::super::identities::with_immediate_transaction(
            &mut storage,
            "fixture capture",
            |connection| {
                append(
                    connection,
                    &id,
                    "codex",
                    "fixture-session",
                    tmt_core::limits::MAX_JS_SAFE_INTEGER,
                    &ResponseCapture {
                        text: FinalText::new("must not persist").unwrap(),
                        retention_days: 1,
                    },
                )
            },
        );
        assert!(result.is_err());
        assert_eq!(
            storage.recent_responses(&id, 10, 10000).unwrap()["items"],
            json!([])
        );
        storage.close().unwrap();
    }
}

//! An agent's own answers (provider-final text) share the admitted runtime observation
//! transaction. They are kept 7 days, at most 1,000 per agent and 16 KiB each (#4914); the newest
//! pruned answer is remembered so a read can say that older answers were not kept.
use super::{Storage, StorageError, StorageErrorCode, classify};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use tmt_core::responses::{FinalText, MAX_KEPT, MAX_PAGE, RETENTION_MS};

pub struct ResponseCapture {
    pub text: FinalText,
}

pub(super) fn append(
    connection: &Connection,
    id: &str,
    provider: &str,
    session: &str,
    now: u64,
    capture: &ResponseCapture,
) -> Result<(), StorageError> {
    let expires = now
        .checked_add(RETENTION_MS)
        .filter(|expires| *expires <= tmt_core::limits::MAX_JS_SAFE_INTEGER)
        .ok_or_else(|| {
            StorageError::new(StorageErrorCode::Corrupt, "Invalid response retention")
        })?;
    connection.execute(
        "INSERT INTO recent_responses(identity_id,text,ended_ms,provider,truncated,session_id,expires_ms,original_bytes) VALUES(?,?,?,?,?,?,?,?)",
        params![id,capture.text.text(),now as i64,provider,capture.text.truncated(),session,expires as i64,capture.text.original_bytes() as i64],
    ).map_err(|e| classify(e,"Retain recent response"))?;
    // What goes is whatever expired and everything past the newest `MAX_KEPT`; the newest of it
    // becomes the floor, in the same transaction, before it is deleted.
    let doomed = "identity_id=?1 AND (expires_ms<=?2 OR sequence<=COALESCE((SELECT sequence FROM recent_responses WHERE identity_id=?1 ORDER BY sequence DESC LIMIT 1 OFFSET ?3),0))";
    connection.execute(
        &format!("INSERT INTO recent_response_floor(identity_id,dropped_through_ms)
         SELECT identity_id,MAX(ended_ms) FROM recent_responses WHERE {doomed} GROUP BY identity_id
         ON CONFLICT(identity_id) DO UPDATE SET dropped_through_ms=MAX(dropped_through_ms,excluded.dropped_through_ms)"),
        params![id,now as i64,MAX_KEPT as i64],
    ).map_err(|e| classify(e,"Record dropped responses"))?;
    connection
        .execute(
            &format!("DELETE FROM recent_responses WHERE {doomed}"),
            params![id, now as i64, MAX_KEPT as i64],
        )
        .map_err(|e| classify(e, "Bound recent responses"))?;
    Ok(())
}

impl Storage {
    /// The driver this identity's remembered session belongs to, if it has one.
    pub fn remembered_driver(&mut self, id: &str) -> Result<Option<String>, StorageError> {
        self.connection_mut()?
            .query_row(
                "SELECT remembered_harness FROM identity_session_preferences WHERE identity_id=?",
                [id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map(Option::flatten)
            .map_err(|e| classify(e, "Read the remembered driver"))
    }

    /// One read snapshot. No hook refresh, acknowledgement, pruning or renewal. `before` is the
    /// `nextBefore` an earlier page returned: only answers captured before it are read.
    pub fn recent_responses(
        &mut self,
        id: &str,
        limit: u64,
        before: Option<u64>,
        now: u64,
    ) -> Result<Value, StorageError> {
        if !tmt_core::dispatch::canonical_id(id)
            || !(1..=MAX_PAGE).contains(&limit)
            || before.is_some_and(|before| before > tmt_core::limits::MAX_JS_SAFE_INTEGER)
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
        let upper = before.map_or(i64::MAX, |before| before as i64);
        let (items, next_before) = {
            let mut query = transaction.prepare("SELECT r.sequence,r.text,r.ended_ms,r.provider,r.truncated,r.session_id,r.original_bytes FROM recent_responses r JOIN identities i ON i.id=r.identity_id AND i.retired_at_ms IS NULL WHERE r.identity_id=? AND r.expires_ms>? AND r.sequence<? ORDER BY r.sequence DESC LIMIT ?").map_err(|e| classify(e,"Prepare recent responses"))?;
            // One more than asked tells whether an older answer is still kept.
            let mut rows = query.query_map(params![id,now as i64,upper,limit as i64 + 1],|row| {
                let mut item = json!({"text":row.get::<_,String>(1)?,"endedMs":row.get::<_,i64>(2)?,"provider":row.get::<_,String>(3)?,"truncated":row.get::<_,bool>(4)?,"originalBytes":row.get::<_,i64>(6)?});
                if let Some(session) = row.get::<_,Option<String>>(5)? { item["sessionId"] = json!(session); }
                Ok((row.get::<_,i64>(0)?,item))
            }).map_err(|e| classify(e,"Query recent responses"))?.collect::<Result<Vec<_>,_>>().map_err(|e| classify(e,"Decode recent responses"))?;
            let more = rows.len() as u64 > limit;
            rows.truncate(limit as usize);
            let next = more
                .then(|| rows.last().map(|(sequence, _)| *sequence))
                .flatten();
            (
                rows.into_iter().map(|(_, item)| item).collect::<Vec<_>>(),
                next,
            )
        };
        // Answers dropped by the writer (the floor) or expired and not yet pruned.
        let dropped_through: Option<i64> = transaction
            .query_row(
                "SELECT MAX(v) FROM (
                    SELECT dropped_through_ms v FROM recent_response_floor WHERE identity_id=?1
                    UNION ALL SELECT MAX(ended_ms) FROM recent_responses WHERE identity_id=?1 AND expires_ms<=?2)",
                params![id, now as i64],
                |row| row.get(0),
            )
            .map_err(|e| classify(e, "Read dropped response coverage"))?;
        transaction
            .commit()
            .map_err(|e| classify(e, "Finish recent responses"))?;
        let mut resource = json!({"identityId":id,"items":items,"coverage":{"olderDropped":dropped_through.is_some()}});
        if let Some(through) = dropped_through {
            resource["coverage"]["droppedThroughMs"] = json!(through);
        }
        if let Some(next) = next_before {
            resource["nextBefore"] = json!(next);
        }
        Ok(resource)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tmt_core::responses::MAX_PAGE;

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
                .recent_responses(&id, 10, None, vector["nowMs"].as_u64().unwrap())
                .unwrap(),
            expected
        );
        assert_eq!(
            storage.recent_responses(&id, 1, None, 10000).unwrap()["items"],
            json!([expected["items"][0]])
        );
        let expired = storage
            .recent_responses(&id, 10, None, vector["expiry"]["nowMs"].as_u64().unwrap())
            .unwrap();
        assert_eq!(expired["items"], vector["expiry"]["items"]);
        assert_eq!(expired["coverage"], vector["expiry"]["coverage"]);
        assert_eq!(
            storage
                .recent_responses(
                    vector["unknownIdentityResource"]["identityId"]
                        .as_str()
                        .unwrap(),
                    10,
                    None,
                    10000
                )
                .unwrap(),
            vector["unknownIdentityResource"]
        );
        storage.close().unwrap();
    }

    fn identity(storage: &mut Storage) -> String {
        tmt_core::identity::create_or_resolve(
            storage,
            "Recent",
            tmt_core::identity::Lifetime::Saved,
        )
        .unwrap()
        .identity
        .id
    }

    fn capture(storage: &mut Storage, id: &str, text: &str, ended_ms: u64) {
        super::super::identities::with_immediate_transaction(
            storage,
            "fixture capture",
            |connection| {
                append(
                    connection,
                    id,
                    "claude",
                    "fixture-session",
                    ended_ms,
                    &ResponseCapture {
                        text: FinalText::new(text).unwrap(),
                    },
                )
            },
        )
        .unwrap();
    }

    #[test]
    fn the_cap_prunes_oldest_first_and_the_read_says_older_answers_were_not_kept() {
        let directory = crate::test_support::TestDirectory::new();
        let mut storage = Storage::open(directory.path.join("responses.db")).unwrap();
        let id = identity(&mut storage);
        for index in 0..3 {
            capture(&mut storage, &id, &format!("answer-{index}"), 1000 + index);
        }
        let complete = storage.recent_responses(&id, 50, None, 5000).unwrap();
        assert_eq!(complete["coverage"], json!({"olderDropped": false}));
        assert!(complete.get("nextBefore").is_none());
        for index in 3..(MAX_KEPT + 3) {
            capture(&mut storage, &id, &format!("answer-{index}"), 1000 + index);
        }
        // 1,003 captured, 1,000 kept: the three oldest are gone and named as such.
        let (mut seen, mut before, mut pages) = (Vec::new(), None, 0);
        loop {
            let page = storage.recent_responses(&id, 50, before, 5000).unwrap();
            assert_eq!(
                page["coverage"],
                json!({"olderDropped": true, "droppedThroughMs": 1002}),
                "every page carries the same coverage"
            );
            seen.extend(
                page["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|item| item["text"].as_str().unwrap().to_owned()),
            );
            pages += 1;
            match page["nextBefore"].as_u64() {
                Some(next) => before = Some(next),
                None => break,
            }
        }
        assert_eq!((seen.len() as u64, pages), (MAX_KEPT, 20));
        assert_eq!(seen[0], format!("answer-{}", MAX_KEPT + 2));
        assert_eq!(seen.last().unwrap(), "answer-3");
        storage.close().unwrap();
    }

    #[test]
    fn an_answer_over_the_cap_keeps_its_first_bytes_and_its_original_length() {
        let directory = crate::test_support::TestDirectory::new();
        let mut storage = Storage::open(directory.path.join("responses.db")).unwrap();
        let id = identity(&mut storage);
        let long = format!("{}🦀", "x".repeat(tmt_core::responses::MAX_TEXT_BYTES - 1));
        capture(&mut storage, &id, &long, 1000);
        capture(&mut storage, &id, "short", 1001);
        let items = storage.recent_responses(&id, 10, None, 5000).unwrap()["items"].clone();
        assert_eq!(items[0]["originalBytes"], 5);
        assert_eq!(items[0]["truncated"], false);
        assert_eq!(
            items[1]["text"].as_str().unwrap().len(),
            tmt_core::responses::MAX_TEXT_BYTES - 1
        );
        assert_eq!(items[1]["truncated"], true);
        assert_eq!(items[1]["originalBytes"], long.len());
        storage.close().unwrap();
    }

    #[test]
    fn a_cursor_past_the_oldest_answer_reads_nothing_and_a_bad_page_size_is_refused() {
        let directory = crate::test_support::TestDirectory::new();
        let mut storage = Storage::open(directory.path.join("responses.db")).unwrap();
        let id = identity(&mut storage);
        capture(&mut storage, &id, "only", 1000);
        assert_eq!(
            storage.recent_responses(&id, 10, Some(1), 5000).unwrap()["items"],
            json!([])
        );
        for limit in [0, MAX_PAGE + 1] {
            assert!(
                storage.recent_responses(&id, limit, None, 5000).is_err(),
                "{limit}"
            );
        }
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
                    },
                )
            },
        );
        assert!(result.is_err());
        assert_eq!(
            storage.recent_responses(&id, 10, None, 10000).unwrap()["items"],
            json!([])
        );
        storage.close().unwrap();
    }
}

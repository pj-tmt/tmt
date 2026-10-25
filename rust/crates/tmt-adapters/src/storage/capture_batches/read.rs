//! A bounded, storage-only ordered projection. Message bodies stay with their
//! canonical request/response owners and missing bodies remain explicit.
use super::super::{Storage, StorageError, StorageErrorCode, classify};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use tmt_core::responses::{MAX_TEXT_BYTES, response_id};

fn natural(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let value = row.get::<_, i64>(index)?;
    u64::try_from(value)
        .ok()
        .filter(|n| *n <= tmt_core::limits::MAX_JS_SAFE_INTEGER)
        .ok_or(rusqlite::Error::IntegralValueOutOfRange(index, value))
}
fn optional_natural(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<Option<u64>> {
    row.get::<_, Option<i64>>(index)?
        .map(|value| {
            u64::try_from(value)
                .ok()
                .filter(|n| *n <= tmt_core::limits::MAX_JS_SAFE_INTEGER)
                .ok_or(rusqlite::Error::IntegralValueOutOfRange(index, value))
        })
        .transpose()
}

fn acknowledgements(
    db: &Connection,
    entry: u64,
    value: &mut Value,
    now: u64,
) -> Result<(), StorageError> {
    let mut statement = db
        .prepare(
            "SELECT k.identity_id,k.request_id,k.revision FROM capture_acknowledgements k
        JOIN request_attempts a ON a.request_id=k.request_id AND a.retention_expires_at_ms>?2
        WHERE k.entry_sequence=?1 ORDER BY k.request_id LIMIT 51",
        )
        .map_err(|e| classify(e, "Read painted-answer keys"))?;
    let mut keys=statement.query_map(params![entry as i64,now as i64],|row|Ok(json!({"identityId":row.get::<_,String>(0)?,"requestId":row.get::<_,String>(1)?,"revision":natural(row,2)?})))
        .map_err(|e|classify(e,"Query painted-answer keys"))?.collect::<Result<Vec<_>,_>>().map_err(|e|classify(e,"Decode painted-answer keys"))?;
    value["acknowledgementsTruncated"] = json!(keys.len() > 50);
    keys.truncate(50);
    value["acknowledgements"] = json!(keys);
    Ok(())
}

fn bounded_text(value: &mut Value, text: Option<String>, original: Option<u64>) {
    if let Some(text) = text {
        let kept = tmt_core::responses::cut_scalars(&text, MAX_TEXT_BYTES);
        let original = original.unwrap_or(text.len() as u64);
        value["availability"] = json!("retained");
        value["truncated"] = json!((kept.len() as u64) < original);
        value["originalBytes"] = json!(original);
        value["text"] = json!(kept);
    } else {
        value["availability"] = json!("unavailable");
    }
}

fn input_body(
    db: &Connection,
    value: &mut Value,
    request: &str,
    source: &str,
    now: u64,
) -> Result<(), StorageError> {
    let row = db
        .query_row(
            "SELECT
        CASE WHEN ?2='incoming' THEN a.originator_identity_id ELSE a.recipient_identity_id END,
        CASE WHEN ?2='incoming' THEN (SELECT name FROM identities WHERE id=a.originator_identity_id)
             ELSE (SELECT name FROM identities WHERE id=a.recipient_identity_id) END,
        CASE WHEN ?2='incoming' AND a.message_expires_at_ms>?3 THEN a.message_text
             WHEN ?2='result' AND r.response_expires_at_ms>?3 THEN r.body ELSE NULL END,
        CASE WHEN ?2='incoming' THEN a.message_bytes ELSE r.body_bytes END
        FROM request_attempts a LEFT JOIN request_responses r ON r.request_id=a.request_id
        WHERE a.request_id=?1 AND a.retention_expires_at_ms>?3",
            params![request, source, now as i64],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    optional_natural(row, 3)?,
                ))
            },
        )
        .optional()
        .map_err(|e| classify(e, "Read ordered input body"))?;
    value["sender"] = json!({});
    if let Some((id, name, text, bytes)) = row {
        if let Some(id) = id {
            value["sender"]["identityId"] = json!(id);
        }
        if let Some(name) = name {
            value["sender"]["name"] = json!(name);
        }
        bounded_text(value, text, bytes);
    } else {
        bounded_text(value, None, None);
    }
    Ok(())
}

fn final_body(
    db: &Connection,
    identity: &str,
    sequence: u64,
    now: u64,
) -> Result<Option<Value>, StorageError> {
    db.query_row("SELECT text,ended_ms,provider,truncated,session_id,original_bytes,
        (SELECT p.request_id FROM request_passthrough p JOIN request_attempts a ON a.request_id=p.request_id
         WHERE p.answer_response=r.sequence AND a.retention_expires_at_ms>?3 ORDER BY p.delivered_at_ms,p.request_id LIMIT 1),
        (SELECT batch_id FROM capture_entries WHERE kind='final' AND response_sequence=r.sequence)
        FROM recent_responses r WHERE r.identity_id=?1 AND r.sequence=?2 AND r.expires_ms>?3",params![identity,sequence as i64,now as i64],|row|{
        let mut value=json!({"responseId":response_id(sequence),"text":row.get::<_,String>(0)?,"endedMs":natural(row,1)?,"provider":row.get::<_,String>(2)?,"truncated":row.get::<_,bool>(3)?,"originalBytes":natural(row,5)?});
        for (index,name) in [(4,"sessionId"),(6,"requestId"),(7,"inputBatchId")] {
            if let Some(field)=row.get::<_,Option<String>>(index)? {value[name]=json!(field);}
        }
        Ok(value)
    }).optional().map_err(|e|classify(e,"Read ordered final body"))
}

impl Storage {
    pub fn conversation_timeline(
        &mut self,
        identity: &str,
        limit: u64,
        before: Option<u64>,
        now: u64,
    ) -> Result<Value, StorageError> {
        if !(1..=50).contains(&limit)
            || before.is_some_and(|n| n > tmt_core::limits::MAX_JS_SAFE_INTEGER)
        {
            return Err(StorageError::new(
                StorageErrorCode::Corrupt,
                "Invalid conversation page",
            ));
        }
        let db = self
            .connection_mut()?
            .transaction()
            .map_err(|e| classify(e, "Open conversation snapshot"))?;
        let mut rows = {
            let mut query=db.prepare("SELECT e.sequence,e.batch_id,e.kind,e.request_id,e.source,e.delivery,e.response_sequence,e.failure
                FROM capture_entries e JOIN identities i ON i.id=e.identity_id AND i.retired_at_ms IS NULL
                WHERE e.identity_id=?1 AND e.delivery!='pending' AND e.sequence<?2 AND e.created_ms>?3 ORDER BY e.sequence DESC LIMIT ?4")
                .map_err(|e|classify(e,"Read conversation page"))?;
            query
                .query_map(
                    params![
                        identity,
                        before.unwrap_or(tmt_core::limits::MAX_JS_SAFE_INTEGER) as i64,
                        now.saturating_sub(tmt_core::responses::RETENTION_MS) as i64,
                        limit as i64 + 1
                    ],
                    |row| {
                        Ok((
                            natural(row, 0)?,
                            row.get::<_, Option<String>>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, Option<String>>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, String>(5)?,
                            optional_natural(row, 6)?,
                            row.get::<_, Option<String>>(7)?,
                        ))
                    },
                )
                .map_err(|e| classify(e, "Query conversation page"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| classify(e, "Decode conversation page"))?
        };
        let more = rows.len() > limit as usize;
        rows.truncate(limit as usize);
        let next = more
            .then(|| rows.last().map(|row| format!("ce_{}", row.0)))
            .flatten();
        rows.reverse();
        let mut entries = Vec::new();
        let mut responses = Vec::new();
        for (sequence, batch, kind, request, source, delivery, response, failure) in rows {
            let mut value = json!({"entryId":format!("ce_{sequence}"),"kind":kind});
            if let Some(batch) = batch {
                value["inputBatchId"] = json!(batch);
            }
            match kind.as_str() {
                "input" => {
                    let request = request.ok_or_else(|| {
                        StorageError::new(StorageErrorCode::Corrupt, "Input has no request")
                    })?;
                    value["requestId"] = json!(request);
                    value["delivery"] = json!(delivery);
                    input_body(&db, &mut value, &request, &source, now)?;
                }
                "final" => {
                    let response = response.ok_or_else(|| {
                        StorageError::new(StorageErrorCode::Corrupt, "Final has no response")
                    })?;
                    value["responseId"] = json!(response_id(response));
                    if let Some(body) = final_body(&db, identity, response, now)? {
                        responses.push(body);
                        value["availability"] = json!("retained");
                    } else {
                        value["availability"] = json!("unavailable");
                    }
                }
                "reply" => {
                    let request = request.ok_or_else(|| {
                        StorageError::new(StorageErrorCode::Corrupt, "Reply has no request")
                    })?;
                    value["requestId"] = json!(request);
                    let body=db.query_row("SELECT body,body_bytes FROM request_responses WHERE request_id=?1 AND response_expires_at_ms>?2",params![request,now as i64],|row|Ok((row.get::<_,String>(0)?,natural(row,1)?))).optional().map_err(|e|classify(e,"Read ordered submitted reply"))?;
                    let (text, bytes) =
                        body.map_or((None, None), |(text, bytes)| (Some(text), Some(bytes)));
                    bounded_text(&mut value, text, bytes);
                }
                "failure" => {
                    value["failure"] = json!({"code":failure,"nextStep":"See the agent's pane."})
                }
                _ => {
                    return Err(StorageError::new(
                        StorageErrorCode::Corrupt,
                        "Unknown conversation entry",
                    ));
                }
            }
            if matches!(kind.as_str(), "final" | "reply") {
                acknowledgements(&db, sequence, &mut value, now)?;
            }
            entries.push(value);
        }
        let dropped=db.query_row("SELECT EXISTS(SELECT 1 FROM capture_floor WHERE identity_id=?1 UNION ALL SELECT 1 FROM capture_entries WHERE identity_id=?1 AND created_ms<=?2)",params![identity,now.saturating_sub(tmt_core::responses::RETENTION_MS) as i64],|row|row.get::<_,bool>(0)).map_err(|e|classify(e,"Read conversation coverage"))?;
        db.commit()
            .map_err(|e| classify(e, "Finish conversation snapshot"))?;
        let mut value = json!({"identityId":identity,"entries":entries,"responses":responses,"coverage":{"olderDropped":dropped}});
        if let Some(next) = next {
            value["nextBefore"] = json!(next);
        }
        Ok(value)
    }
}

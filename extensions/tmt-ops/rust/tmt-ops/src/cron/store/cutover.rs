//! Retain exact legacy bytes and fence old writers before the public Core import.
use super::*;
use crate::cron::QuiescentClock;
use rusqlite::{OptionalExtension, params};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrozenJobs {
    pub source_id: String,
    pub source_digest: String,
    pub bytes: Vec<u8>,
}
fn digest(bytes: &[u8]) -> String {
    let mut hash = Sha256::new();
    hash.update(b"tmt:orchestration:legacy-jobs:v1\0");
    hash.update((bytes.len() as u64).to_be_bytes());
    hash.update(bytes);
    hash.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
fn source(bytes: Option<&[u8]>) -> Result<Vec<u8>, Error> {
    match bytes {
        Some(bytes) => {
            decode(Some(bytes))?;
            Ok(bytes.to_vec())
        }
        None => serde_json::to_vec(&Jobs::default().document()).map_err(io),
    }
}
fn conflict() -> Error {
    Error::new(
        "SQUAD_CRON_CUTOVER_CONFLICT",
        "Cron changed during adoption. Inspect the current source before starting a new import.",
    )
}
impl Store {
    /// Preflight observes the exact source without freezing or archiving it.
    /// Core validates this immutable document before the freeze hash CAS.
    pub fn cutover_source(&self) -> Result<(String, Vec<u8>), Error> {
        let slot = self.slot();
        let bytes = match Database::open_existing(&self.root, &slot.legacy).map_err(refused)? {
            Some(mut database) => database.inspect(&slot, source).map_err(failed)?,
            None => source(None)?,
        };
        Ok((digest(&bytes), bytes))
    }
    pub fn freeze_for_core(
        &self,
        guard: &QuiescentClock,
        source_id: &str,
        expected_digest: &str,
    ) -> Result<FrozenJobs, Error> {
        if !guard.covers(&self.root)
            || !crate::operation::uuid_like(source_id)
            || source_id != source_id.to_ascii_lowercase()
        {
            return Err(conflict());
        }
        let slot = self.slot();
        let mut database = Database::open(&self.root).map_err(refused)?;
        database.update(&slot,|bytes| {
            let bytes=source(bytes)?;
            let source_digest=digest(&bytes);
            if source_digest!=expected_digest { return Err(conflict()); }
            let marker=json!({"version":3,"owner":"core.jobs","phase":"frozen","sourceId":source_id,"sourceDigest":source_digest});
            Ok((Some(serde_json::to_vec(&marker).map_err(io)?),FrozenJobs{source_id:source_id.into(),source_digest,bytes}))
        },|frozen,transaction| {
            // One Ops transaction publishes both the old-reader refusal and
            // the byte-preserving recovery source; neither can survive alone.
            transaction.execute("INSERT INTO documents(kind,key,body) VALUES('cron-cutover-source',?1,?2)",params![frozen.source_id,frozen.bytes]).map_err(io)?;
            Ok(())
        }).map_err(failed)
    }
    /// Recovery is read-only. An uncertain import is never thawed or resubmitted
    /// with a new source ID; the caller reads back this exact Core receipt.
    pub fn frozen_for_core(&self) -> Result<Option<FrozenJobs>, Error> {
        let slot = self.slot();
        let Some(mut database) =
            Database::open_existing(&self.root, &slot.legacy).map_err(refused)?
        else {
            return Ok(None);
        };
        let row = database
            .snapshot(|transaction| {
                let marker: Option<Vec<u8>> = transaction
                    .query_row(
                        "SELECT CASE WHEN length(body)<=16777216 THEN body END FROM documents WHERE kind='cron' AND key=''",
                        [],
                        |row| row.get(0),
                    )
                    .optional()?;
                let Some(marker) = marker else {
                    return Ok(None);
                };
                let value: Value =
                    serde_json::from_slice(&marker).map_err(|_| rusqlite::Error::InvalidQuery)?;
                if value["version"].as_u64().is_some_and(|version| version < 3) {
                    return Ok(None);
                }
                if value["version"] != 3
                    || value["owner"] != "core.jobs"
                    || !matches!(value["phase"].as_str(), Some("frozen" | "adopted"))
                {
                    return Err(rusqlite::Error::InvalidQuery);
                }
                let id = value["sourceId"]
                    .as_str()
                    .ok_or(rusqlite::Error::InvalidQuery)?
                    .to_owned();
                let digest = value["sourceDigest"]
                    .as_str()
                    .ok_or(rusqlite::Error::InvalidQuery)?
                    .to_owned();
                let bytes = transaction.query_row(
                    "SELECT CASE WHEN length(body)<=16777216 THEN body END FROM documents WHERE kind='cron-cutover-source' AND key=?",
                    [&id],
                    |row| row.get(0),
                )?;
                Ok(Some(FrozenJobs {
                    source_id: id,
                    source_digest: digest,
                    bytes,
                }))
            })
            .map_err(refused)?;
        if let Some(frozen) = &row {
            if frozen.bytes.len() as u64 > LIMIT || digest(&frozen.bytes) != frozen.source_digest {
                return Err(corrupt("Frozen cron source does not match its digest."));
            }
            decode(Some(&frozen.bytes))?;
        }
        Ok(row)
    }
}

#[cfg(test)]
mod tests;

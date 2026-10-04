//! Transaction-local reads, cut capture/pinning and immutable baseline storage.
use super::*;
use tmt_colab_model::{object, stream_cut};

pub struct StoredBaseline {
    pub descriptor: Vec<u8>,
    pub envelope: Vec<u8>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cut {
    pub page: String,
    pub epoch: u64,
    pub stream: String,
    pub namespace: String,
    pub checkpoint_seq: u64,
    pub checkpoint_hash: Option<[u8; 32]>,
    pub tail_seq: u64,
    pub tail_hash: [u8; 32],
}
impl Cut {
    pub fn payload(&self) -> Result<serde_json::Value> {
        Ok(
            serde_json::json!({"pageId":self.page,"epoch":self.epoch.to_string(),
            "namespace":self.namespace,"cut":values::encode_binary(&stream_cut::input(&stream_cut::StreamCut {
                stream_id:&self.stream, namespace:&self.namespace,
                checkpoint_hash:self.checkpoint_hash.as_ref(), checkpoint_seq:&self.checkpoint_seq.to_string(),
                tail_head_seq:&self.tail_seq.to_string(), tail_head_hash:&self.tail_hash,
            })?)}),
        )
    }
}
pub struct StoredObject {
    pub seq: u64,
    pub hash: [u8; 32],
    pub previous: [u8; 32],
    pub checkpoint: bool,
    pub bytes: Vec<u8>,
}
impl Store {
    /// A consistent SQLite snapshot, released before any decoder child runs.
    pub(crate) fn owner_read<T>(
        &self,
        space: &str,
        root: &[u8; 32],
        read: impl FnOnce(&OwnerTransaction<'_>) -> Result<T>,
    ) -> Result<T> {
        if crypto::space_id(root)? != space {
            return Err(OwnerFault::WrongOwner.into());
        }
        let tx = self.connection.unchecked_transaction()?;
        let head = read_head(&tx, space, root)?;
        read(&OwnerTransaction {
            tx: &tx,
            space,
            root,
            head,
            clock: self.clock.as_ref(),
        })
    }
    /// Caller must admit page/history access before returning these bytes remotely.
    pub fn baseline(&self, page: &str, epoch: u64) -> Result<Option<StoredBaseline>> {
        values::generated_id(page)?;
        read_baseline(&self.connection, page, epoch)
    }
}
impl OwnerTransaction<'_> {
    /// Stored statements were authenticated at append. Reuse the fold's reducer
    /// without repeated signatures or a writer reservation on every sync check.
    pub(crate) fn page_policy_at(
        &self,
        page: &str,
        revision: u64,
    ) -> Result<crate::fold::PagePolicy> {
        let mut policy = crate::fold::PagePolicy::default();
        let mut query = self
            .tx
            .prepare("SELECT envelope FROM membership_log WHERE revision<=? ORDER BY revision")?;
        for row in query.query_map([sequence(revision)], |r| r.get::<_, Vec<u8>>(0))? {
            let wire: serde_json::Value = serde_json::from_slice(&row?)?;
            let bytes =
                values::binary(wire["statement"].as_str().ok_or(OwnerFault::Invalid)?, 1024)?;
            let header = statement::decode(&bytes)?;
            let bytes = values::binary(
                wire["payload"].as_str().ok_or(OwnerFault::Invalid)?,
                payload::MAX_BYTES,
            )?;
            if crypto::digest(&bytes) != *header.payload_digest {
                return Err(OwnerFault::Invalid.into());
            }
            policy.apply(&payload::decode(header.operation, &bytes)?, page)?;
        }
        Ok(policy)
    }
    pub(crate) fn delete_page_data(&mut self, page: &str) -> Result<()> {
        // Keep pages, signed policy and operation receipts as permanent tombstones.
        // The retained page primary key also prevents create_page from reviving it.
        for sql in [
            "DELETE FROM receipts WHERE page=?",
            "DELETE FROM checkpoints WHERE page=?",
            "DELETE FROM streams WHERE page=?",
            "DELETE FROM baselines WHERE page=?",
            "DELETE FROM wraps WHERE page=?",
            "DELETE FROM epoch_secrets WHERE page=?",
        ] {
            self.tx.execute(sql, [page])?;
        }
        Ok(())
    }
    pub(crate) fn pages(&self) -> Result<Vec<String>> {
        let mut query = self.tx.prepare("SELECT page FROM pages ORDER BY page")?;
        Ok(query
            .query_map([], |r| r.get(0))?
            .collect::<std::result::Result<_, _>>()?)
    }
    pub(crate) fn recent_secrets(&self, page: &str, epoch: u64) -> Result<Vec<(u64, [u8; 32])>> {
        let mut query = self.tx.prepare("SELECT epoch,secret FROM epoch_secrets WHERE page=? AND epoch>=? AND epoch<=? ORDER BY epoch")?;
        let rows = query
            .query_map(
                params![
                    page,
                    sequence(epoch.saturating_sub(63).max(1)),
                    sequence(epoch)
                ],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?)),
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if rows.len() > 64 {
            return Err(OwnerFault::Capacity.into());
        }
        rows.into_iter()
            .map(|(e, s)| Ok((e.parse()?, array(s)?)))
            .collect()
    }
    pub(crate) fn tombstone(&mut self, id: &str, revision: u64) -> Result<()> {
        values::generated_id(id)?;
        if revision == 0 {
            return Err(OwnerFault::Invalid.into());
        }
        self.tx.execute("INSERT INTO device_registrations VALUES (?,NULL,1,?) ON CONFLICT(device_id) DO UPDATE SET binding=NULL,revoked=1,grant_revision=excluded.grant_revision",
            params![id,sequence(revision)])?;
        if let Some(mut device) = self.device(id)? {
            device.revoked = true;
            self.put_device(&device)?;
        }
        Ok(())
    }
    pub(crate) fn current_epoch(&self, page: &str) -> Result<u64> {
        values::generated_id(page)?;
        let epoch: String =
            self.tx
                .query_row("SELECT epoch FROM pages WHERE page=?", [page], |r| r.get(0))?;
        Ok(values::decimal(&epoch, false)?)
    }
    pub(crate) fn log(&self) -> Result<Vec<statement::Envelope>> {
        let mut query = self
            .tx
            .prepare("SELECT envelope FROM membership_log ORDER BY revision")?;
        query
            .query_map([], |r| r.get::<_, Vec<u8>>(0))?
            .map(|row| Ok(statement::Envelope::from_json(&row?)?))
            .collect()
    }
    pub(crate) fn devices(&self) -> Result<Vec<Device>> {
        let mut query = self.tx.prepare("SELECT record FROM devices ORDER BY id")?;
        query
            .query_map([], |r| r.get::<_, Vec<u8>>(0))?
            .map(|row| Ok(serde_json::from_slice(&row?)?))
            .collect()
    }
    pub(crate) fn saved_operation(&self, id: &str, digest: &[u8; 32]) -> Result<Option<Vec<u8>>> {
        let row: Option<(Vec<u8>, Vec<u8>)> = self
            .tx
            .query_row(
                "SELECT digest,outcome FROM owner_operations WHERE id=?",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        match row {
            Some((old, result)) if old == *digest => Ok(Some(result)),
            Some(_) => Err(OwnerFault::Conflict.into()),
            None => Ok(None),
        }
    }
    pub(crate) fn baseline(&self, page: &str, epoch: u64) -> Result<Option<StoredBaseline>> {
        read_baseline(self.tx, page, epoch)
    }
    pub(crate) fn put_baseline(
        &mut self,
        descriptor: &[u8],
        envelope: &object::Envelope,
    ) -> Result<()> {
        let d = payload::decode_baseline(descriptor)?;
        let h = object::Header::decode(envelope.header())?;
        if h.context.page != d.page_id
            || h.context.epoch != d.epoch
            || h.context.space != self.space
            || h.context.kind != "html"
            || h.context.membership_revision != d.membership_revision
            || values::binary(&d.object_envelope_hash, 32)? != envelope.hash()?
        {
            return Err(OwnerFault::Invalid.into());
        }
        let bytes = envelope.to_json()?;
        let epoch = values::decimal(&d.epoch, false)?;
        if let Some(old) = self.baseline(&d.page_id, epoch)? {
            return if old.descriptor == descriptor && old.envelope == bytes {
                Ok(())
            } else {
                Err(OwnerFault::Conflict.into())
            };
        }
        super::super::capacity(self.tx, &d.page_id, bytes.len(), false)?;
        self.tx.execute(
            "INSERT INTO baselines VALUES (?,?,?,?)",
            params![d.page_id, sequence(epoch), descriptor, bytes],
        )?;
        Ok(())
    }
    pub(crate) fn cuts(&self, page: &str, epoch: u64) -> Result<Vec<Cut>> {
        let frozen: bool = self.tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM streams WHERE page=? AND epoch=? AND frozen=1)",
            params![page, epoch.to_string()],
            |r| r.get(0),
        )?;
        if frozen {
            return Err(OwnerFault::Conflict.into());
        }
        let mut query = self
            .tx
            .prepare("SELECT stream FROM streams WHERE page=? AND epoch=? ORDER BY stream")?;
        let streams = query
            .query_map(params![page, epoch.to_string()], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if streams.len() > 256 {
            return Err(OwnerFault::Capacity.into());
        }
        let mut result = Vec::new();
        for namespace in ["content", "own"] {
            for stream in &streams {
                values::generated_id(stream)?;
                let cp: Option<(String,Vec<u8>)> = self.tx.query_row(
                    "SELECT seq,hash FROM checkpoints WHERE page=? AND epoch=? AND stream=? AND namespace=? ORDER BY seq DESC LIMIT 1",
                    params![page,epoch.to_string(),stream,namespace], |r| Ok((r.get(0)?,r.get(1)?)),
                ).optional()?;
                let cp_seq = cp.as_ref().map_or(Ok(0), |(seq, _)| seq.parse::<u64>())?;
                let tail: Option<(String,Vec<u8>)> = self.tx.query_row(
                    "SELECT seq,hash FROM receipts WHERE page=? AND epoch=? AND stream=? AND (namespace=? OR seq=?) ORDER BY seq DESC LIMIT 1",
                    params![page,epoch.to_string(),stream,namespace,sequence(cp_seq)], |r| Ok((r.get(0)?,r.get(1)?)),
                ).optional()?;
                let (tail_seq, tail_hash) = match tail {
                    Some((seq, hash)) => (seq.parse()?, array(hash)?),
                    None => (0, [0; 32]),
                };
                result.push(Cut {
                    page: page.into(),
                    epoch,
                    stream: stream.clone(),
                    namespace: namespace.into(),
                    checkpoint_seq: cp_seq,
                    checkpoint_hash: cp.map(|(_, hash)| array(hash)).transpose()?,
                    tail_seq,
                    tail_hash,
                });
            }
        }
        Ok(result)
    }
    pub(crate) fn pin_cuts(&mut self, cuts: &[Cut]) -> Result<()> {
        for cut in cuts.iter().filter(|c| c.checkpoint_seq != 0) {
            if self.tx.execute("UPDATE checkpoints SET pinned=1 WHERE page=? AND epoch=? AND stream=? AND namespace=? AND seq=? AND hash=? AND payload IS NOT NULL",
                params![cut.page,cut.epoch.to_string(),cut.stream,cut.namespace,sequence(cut.checkpoint_seq),cut.checkpoint_hash.as_ref().map(|h| h.as_slice())])? != 1 {
                return Err(OwnerFault::StaleHead.into());
            }
        }
        Ok(())
    }
    pub(crate) fn validate_retained_cut(&self, wrapper: &payload::Cut) -> Result<()> {
        let bytes = values::binary(&wrapper.cut, 1024)?;
        let cut = stream_cut::decode(&bytes)?;
        let tail = values::decimal(cut.tail_head_seq, true)?;
        if tail != 0 {
            let found: bool=self.tx.query_row("SELECT EXISTS(SELECT 1 FROM receipts WHERE page=? AND epoch=? AND stream=? AND seq=? AND hash=?)",
                params![wrapper.page_id,wrapper.epoch,cut.stream_id,sequence(tail),cut.tail_head_hash.as_slice()],|r|r.get(0))?;
            if !found {
                return Err(OwnerFault::Invalid.into());
            }
        }
        if let Some(hash) = cut.checkpoint_hash {
            let seq = values::decimal(cut.checkpoint_seq, false)?;
            let found: bool=self.tx.query_row("SELECT EXISTS(SELECT 1 FROM checkpoints WHERE page=? AND epoch=? AND stream=? AND namespace=? AND seq=? AND hash=? AND payload IS NOT NULL)",
                params![wrapper.page_id,wrapper.epoch,cut.stream_id,wrapper.namespace,sequence(seq),hash.as_slice()],|r|r.get(0))?;
            if !found {
                return Err(OwnerFault::Invalid.into());
            }
        }
        Ok(())
    }
    pub(crate) fn cut_objects(&self, cut: &Cut) -> Result<Vec<StoredObject>> {
        let mut out = Vec::new();
        if cut.checkpoint_seq != 0 {
            let (hash, previous, bytes): (Vec<u8>,Vec<u8>,Option<Vec<u8>>) = self.tx.query_row(
                "SELECT hash,head,payload FROM checkpoints WHERE page=? AND epoch=? AND stream=? AND namespace=? AND seq=?",
                params![cut.page,cut.epoch.to_string(),cut.stream,cut.namespace,sequence(cut.checkpoint_seq)],
                |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
            )?;
            out.push(StoredObject {
                seq: cut.checkpoint_seq,
                hash: array(hash)?,
                previous: array(previous)?,
                checkpoint: true,
                bytes: bytes.ok_or(OwnerFault::Invalid)?,
            });
        }
        let mut query = self.tx.prepare("SELECT seq,hash,payload FROM receipts WHERE page=? AND epoch=? AND stream=? AND namespace=? AND seq>? ORDER BY seq LIMIT ?")?;
        let rows = query.query_map(
            params![
                cut.page,
                cut.epoch.to_string(),
                cut.stream,
                cut.namespace,
                sequence(cut.checkpoint_seq),
                // One more than the cap, so an over-long tail is an error and never a truncation.
                (crate::decoder::UPDATES + 1) as i64
            ],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Vec<u8>>(1)?,
                    r.get::<_, Option<Vec<u8>>>(2)?,
                ))
            },
        )?;
        for row in rows {
            let (seq, hash, bytes) = row?;
            let seq: u64 = seq.parse()?;
            let previous = if seq == 1 {
                [0; 32]
            } else {
                array(self.tx.query_row(
                    "SELECT hash FROM receipts WHERE page=? AND epoch=? AND stream=? AND seq=?",
                    params![
                        cut.page,
                        cut.epoch.to_string(),
                        cut.stream,
                        sequence(seq - 1)
                    ],
                    |r| r.get(0),
                )?)?
            };
            out.push(StoredObject {
                seq,
                hash: array(hash)?,
                previous,
                checkpoint: false,
                bytes: bytes.ok_or(OwnerFault::Invalid)?,
            });
        }
        if out.len() > crate::decoder::UPDATES {
            // The query stops one past the cap; count the real tail only to report it.
            let total: i64 = self.tx.query_row(
                "SELECT count(*) FROM receipts WHERE page=? AND epoch=? AND stream=? AND namespace=? AND seq>?",
                params![
                    cut.page,
                    cut.epoch.to_string(),
                    cut.stream,
                    cut.namespace,
                    sequence(cut.checkpoint_seq)
                ],
                |r| r.get(0),
            )?;
            return Err(OwnerFault::too_large(
                &cut.page,
                format!(
                    "it has {total} updates since its last baseline (limit {})",
                    crate::decoder::UPDATES
                ),
            )
            .into());
        }
        Ok(out)
    }
}
fn array(bytes: Vec<u8>) -> Result<[u8; 32]> {
    bytes.try_into().map_err(|_| OwnerFault::Invalid.into())
}
fn read_baseline(c: &Connection, page: &str, epoch: u64) -> Result<Option<StoredBaseline>> {
    let sizes: Option<(i64, i64)> = c
        .query_row(
            "SELECT length(descriptor),length(envelope) FROM baselines WHERE page=? AND epoch=?",
            params![page, sequence(epoch)],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((descriptor, envelope)) = sizes else {
        return Ok(None);
    };
    if descriptor <= 0
        || descriptor > payload::MAX_BYTES as i64
        || envelope <= 0
        || envelope > crate::limits::OBJECT_BYTES as i64
    {
        return Err(OwnerFault::Capacity.into());
    }
    Ok(c.query_row(
        "SELECT descriptor,envelope FROM baselines WHERE page=? AND epoch=?",
        params![page, sequence(epoch)],
        |r| {
            Ok(StoredBaseline {
                descriptor: r.get(0)?,
                envelope: r.get(1)?,
            })
        },
    )
    .optional()?)
}

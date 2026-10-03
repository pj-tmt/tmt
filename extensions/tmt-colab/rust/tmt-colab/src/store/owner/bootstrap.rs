//! Bounded bootstrap reads on the existing owner snapshot, without secret export.
use super::*;

pub(crate) struct MembershipPage {
    pub statements: Vec<serde_json::Value>,
    pub transfer: Option<([u8; 32], Vec<u8>)>,
    pub revision: u64,
    pub more: bool,
}
impl OwnerTransaction<'_> {
    pub(crate) fn membership_page(
        &self,
        after: u64,
        target: &statement::Head,
        budget: usize,
        allow_reference: bool,
    ) -> Result<MembershipPage> {
        let current = self.head().ok_or(super::super::Fault::ResyncRequired)?;
        if after > target.revision || current.revision < target.revision {
            return Err(super::super::Fault::ResyncRequired.into());
        }
        let anchor = if after == 0 {
            None
        } else {
            let envelope = self
                .statement(after)?
                .ok_or(super::super::Fault::ResyncRequired)?;
            let bytes = envelope.to_json()?;
            let wire: serde_json::Value = serde_json::from_slice(&bytes)?;
            let input =
                values::binary(wire["statement"].as_str().ok_or(OwnerFault::Invalid)?, 1024)?;
            if values::decimal(statement::decode(&input)?.revision, false)? != after {
                return Err(super::super::Fault::ResyncRequired.into());
            }
            Some(statement::Head {
                revision: after,
                hash: envelope.hash()?,
                owner_member: target.owner_member.clone(),
            })
        };
        let mut head = anchor;
        let mut statements = Vec::new();
        let mut size = 0;
        let mut transfer = None;
        let mut revision = after;
        while revision < target.revision && statements.len() < 64 {
            let n = revision.checked_add(1).ok_or(OwnerFault::Capacity)?;
            let length: Option<i64> = self
                .tx
                .query_row(
                    "SELECT length(envelope) FROM membership_log WHERE revision=?",
                    [sequence(n)],
                    |r| r.get(0),
                )
                .optional()?;
            let length = length.ok_or(super::super::Fault::ResyncRequired)?;
            if length <= 0 || length > crate::limits::STATEMENT_BYTES as i64 {
                return Err(OwnerFault::Capacity.into());
            }
            // A reference occupies its own page. Defer it when the caller's
            // page supplies a baseline, or finish earlier inline entries first.
            if length > crate::limits::CHUNK_BYTES as i64
                && (!allow_reference || !statements.is_empty())
            {
                break;
            }
            let bytes: Vec<u8> = self.tx.query_row(
                "SELECT envelope FROM membership_log WHERE revision=?",
                [sequence(n)],
                |r| r.get(0),
            )?;
            let chunked = bytes.len() > crate::limits::CHUNK_BYTES;
            let encoded = if chunked {
                String::new()
            } else {
                values::encode_binary(&bytes)
            };
            if !chunked && size + encoded.len() + 4 > budget {
                break;
            }
            let envelope = statement::Envelope::from_json(&bytes)?;
            head = Some(
                envelope
                    .verify_next(self.space, self.root, head.as_ref())
                    .map_err(|_| super::super::Fault::ResyncRequired)?
                    .head,
            );
            revision = n;
            size += encoded.len() + 4;
            if chunked {
                let hash = envelope.hash()?;
                statements.push(serde_json::json!({"statementHash":values::encode_binary(&hash)}));
                transfer = Some((hash, bytes));
                break;
            }
            statements.push(serde_json::json!(encoded));
        }
        if revision == target.revision && head.as_ref().is_none_or(|h| h.hash != target.hash) {
            return Err(super::super::Fault::ResyncRequired.into());
        }
        Ok(MembershipPage {
            statements,
            transfer,
            revision,
            more: revision < target.revision,
        })
    }
    pub(crate) fn author_chain(&self, id: &str) -> Result<Vec<u8>> {
        let device = self.device(id)?.ok_or(OwnerFault::Invalid)?;
        let chain = certificate::Chain::from_json(&device.chain)?;
        let cert = chain.certificate()?;
        if cert.space != self.space || cert.device_id != id {
            return Err(OwnerFault::Invalid.into());
        }
        // Revocation is in the signed log. Retained chains let clients reject a
        // revoked author's objects themselves; a chain is never live authority.
        Ok(device.chain)
    }
    pub(crate) fn wrap_page(
        &self,
        page: &str,
        epoch: u64,
        device: &str,
        target: &statement::Head,
        offset: usize,
        recipients: &WrapRecipients,
    ) -> Result<(Vec<String>, bool)> {
        let (device, kind, recipient) = match recipients {
            WrapRecipients::Owner => (device, "member", target.owner_member.id.as_str()),
            WrapRecipients::Link(id) => ("", "link", id.as_str()),
            WrapRecipients::None => return Ok((Vec::new(), false)),
        };
        let mut query = self.tx.prepare("SELECT envelope,length(envelope) FROM wraps WHERE page=?1 AND epoch<=?2 AND revision<=?3
            AND ((kind='device' AND recipient=?4) OR (kind=?7 AND recipient=?5))
            AND epoch IN (SELECT epoch FROM epoch_secrets WHERE page=?1 AND epoch<=?2 ORDER BY epoch DESC LIMIT 64)
            ORDER BY epoch,kind,recipient,revision LIMIT 513 OFFSET ?6")?;
        let mut rows = query.query(params![
            page,
            sequence(epoch),
            sequence(target.revision),
            device,
            recipient,
            offset as i64,
            kind
        ])?;
        let mut out = Vec::new();
        let mut size = 0;
        while let Some(row) = rows.next()? {
            let length: i64 = row.get(1)?;
            if length <= 0 || length > 2048 {
                return Err(OwnerFault::Capacity.into());
            }
            let bytes: Vec<u8> = row.get(0)?;
            let envelope = wrap::Envelope::from_json(&bytes)?;
            let h = envelope.header()?;
            if h.space != self.space
                || h.page != page
                || values::decimal(&h.epoch, false)? > epoch
                || !((h.recipient_kind == "device" && h.recipient_id == device)
                    || (h.recipient_kind == kind && h.recipient_id == recipient))
            {
                return Err(OwnerFault::Invalid.into());
            }
            envelope.verify_owner(self.root)?;
            let encoded = values::encode_binary(&bytes);
            if out.len() == 512 || size + encoded.len() + 4 > 60 * 1024 {
                return Ok((out, true));
            }
            size += encoded.len() + 4;
            out.push(encoded);
        }
        Ok((out, false))
    }
    pub(crate) fn page_list(&self) -> Result<serde_json::Value> {
        let mut query = self
            .tx
            .prepare("SELECT page,epoch FROM pages ORDER BY page LIMIT 1001")?;
        let rows = query.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let mut pages = std::collections::BTreeMap::new();
        for row in rows {
            let (id, epoch) = row?;
            values::generated_id(&id)?;
            values::decimal(&epoch, false)?;
            if pages.len() == 1000 {
                return Err(OwnerFault::Capacity.into());
            }
            pages.insert(id.clone(),serde_json::json!({"pageId":id,"epoch":epoch,"sharing":"private","history":"shared","archived":false}));
        }
        // Existence/epoch are the local creation projection; presentation policy
        // comes only from owner-signed statements. Never expose encrypted titles.
        let mut head = None;
        let mut log = self
            .tx
            .prepare("SELECT length(envelope),envelope FROM membership_log ORDER BY revision")?;
        let mut rows = log.query([])?;
        while let Some(row) = rows.next()? {
            let size: i64 = row.get(0)?;
            if size <= 0 || size > ((payload::MAX_BYTES + 1024) * 4 / 3 + 2048) as i64 {
                return Err(OwnerFault::Capacity.into());
            }
            let bytes: Vec<u8> = row.get(1)?;
            let envelope = statement::Envelope::from_json(&bytes)?;
            let verified = envelope.verify_next(self.space, self.root, head.as_ref())?;
            head = Some(verified.head);
            match verified.payload {
                payload::Payload::PageShare(p) => {
                    if let Some(page) = pages.get_mut(&p.page_id) {
                        page["sharing"] = serde_json::json!(match p.mode {
                            payload::ShareMode::Private => "private",
                            payload::ShareMode::Link => "link",
                            payload::ShareMode::Public => "public",
                        });
                    }
                }
                payload::Payload::PageHistory(p) => {
                    if let Some(page) = pages.get_mut(&p.page_id) {
                        page["history"] = serde_json::json!(match p.mode {
                            payload::HistoryMode::Shared => "shared",
                            payload::HistoryMode::Current => "current",
                        });
                    }
                }
                payload::Payload::Archive(p) => {
                    if let Some(page) = pages.get_mut(&p.page_id) {
                        page["archived"] = true.into();
                    }
                }
                payload::Payload::Delete(p) => {
                    pages.remove(&p.page_id);
                }
                _ => {}
            }
        }
        if head.as_ref() != self.head() {
            return Err(OwnerFault::Invalid.into());
        }
        Ok(
            serde_json::json!({"spaceId":self.space,"ownerKey":values::encode_binary(self.root),"revision":self.head().map_or(0,|h| h.revision).to_string(),"pages":pages.into_values().collect::<Vec<_>>()}),
        )
    }
}

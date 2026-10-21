//! Bounded bootstrap reads on the existing owner snapshot, without secret export.
use super::*;
use crate::page_expiry::{PageRetention, SpaceExpiry, archive_at_ms, effective_days};

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
    /// The chains of the unrevoked devices link `id` certified, which a holder of the link may
    /// write streams for. They are published beside the writers' chains so the stream is found.
    pub(crate) fn link_device_chains(&self, id: &str) -> Result<Vec<Vec<u8>>> {
        let mut chains = Vec::new();
        for device in self.devices()? {
            let chain = certificate::Chain::from_json(&device.chain)?;
            let cert = chain.certificate()?;
            if !device.revoked && cert.issuer_kind == "link" && cert.issuer_id == id {
                chains.push(device.chain);
            }
        }
        Ok(chains)
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
    /// One opaque string per page that changes whenever the page's folded content, or the authority
    /// that admits it, can: the epoch, the membership head and the page's receipt and checkpoint
    /// counts (both only grow within an epoch). The owner list's summary cache keys on it.
    pub(crate) fn page_fingerprints(&self) -> Result<std::collections::BTreeMap<String, String>> {
        self.selected_page_fingerprints(None, true)
    }
    pub(crate) fn page_fingerprint(&self, page: &str) -> Result<Option<String>> {
        Ok(self
            .selected_page_fingerprints(Some(page), true)?
            .remove(page))
    }
    /// Like `page_fingerprints` without the membership head: it moves only when the page's own
    /// content does, so creating or sharing another page leaves it alone. For display-only
    /// readings that no authority change may be waited on (the work index).
    pub(crate) fn page_progress(&self) -> Result<std::collections::BTreeMap<String, String>> {
        self.selected_page_fingerprints(None, false)
    }
    /// The owner's local plaintext title of a page, display-only projection state (#4218). It is
    /// stored only while `expected` is still the page's fingerprint: the one the verified fold that
    /// produced `title` started from, so a page that changed meanwhile (an edit, a membership change,
    /// an archive or a delete) keeps its old row, or none, until its next fold. A title that is empty
    /// or over [`crate::limits::PAGE_TITLE_BYTES`] is not stored and removes the older row. Returns
    /// whether a row was written.
    pub fn record_page_title(&mut self, page: &str, expected: &str, title: &str) -> Result<bool> {
        values::generated_id(page).map_err(|_| OwnerFault::Invalid)?;
        if self.page_fingerprint(page)?.as_deref() != Some(expected) {
            return Ok(false);
        }
        if title.is_empty() || title.len() > crate::limits::PAGE_TITLE_BYTES {
            self.drop_page_title(page)?;
            return Ok(false);
        }
        let progress = self
            .selected_page_fingerprints(Some(page), false)?
            .remove(page)
            .ok_or(OwnerFault::Invalid)?;
        self.tx.execute(
            "INSERT INTO page_titles(page,progress,title) VALUES (?1,?2,?3)
             ON CONFLICT(page) DO UPDATE SET progress=excluded.progress,title=excluded.title",
            params![page, progress, title],
        )?;
        Ok(true)
    }
    /// An archive, like a delete, takes the page's title row away; unarchiving leaves it absent
    /// until the next verified fold writes it again.
    pub(crate) fn drop_page_title(&self, page: &str) -> Result<()> {
        self.tx
            .execute("DELETE FROM page_titles WHERE page=?", [page])?;
        Ok(())
    }
    /// The title rows still valid for their page: a row whose progress key is no longer the page's
    /// is stale and left out. Holding no row is never an error; the caller shows its own fallback.
    pub fn page_titles(&self) -> Result<std::collections::BTreeMap<String, String>> {
        let progress = self.page_progress()?;
        let mut query = self
            .tx
            .prepare("SELECT page,progress,title FROM page_titles ORDER BY page LIMIT 1001")?;
        let mut titles = std::collections::BTreeMap::new();
        for row in query.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })? {
            let (page, stored, title) = row?;
            if progress.get(&page) == Some(&stored) {
                titles.insert(page, title);
            }
        }
        Ok(titles)
    }
    fn selected_page_fingerprints(
        &self,
        page: Option<&str>,
        with_head: bool,
    ) -> Result<std::collections::BTreeMap<String, String>> {
        let head = if with_head {
            self.head().map(|h| h.revision).unwrap_or_default()
        } else {
            Default::default()
        };
        let mut query = self.tx.prepare(
            "SELECT p.page,p.epoch,
                (SELECT COUNT(*) FROM receipts r WHERE r.page=p.page AND r.epoch=p.epoch),
                (SELECT COUNT(*) FROM checkpoints c WHERE c.page=p.page AND c.epoch=p.epoch)
             FROM pages p WHERE (?1 IS NULL OR p.page=?1) ORDER BY p.page LIMIT 1001",
        )?;
        let rows = query.query_map(params![page], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })?;
        let mut out = std::collections::BTreeMap::new();
        for row in rows {
            let (page, epoch, receipts, checkpoints) = row?;
            out.insert(page, format!("{epoch}:{head}:{receipts}:{checkpoints}"));
        }
        Ok(out)
    }
    /// The catalog the browser is served: exactly the page fields the owner socket declares.
    pub(crate) fn page_list(&self) -> Result<serde_json::Value> {
        self.page_list_with(None)
    }
    /// The catalog of the local management CLI: the page fields plus when each page was last active
    /// and when the space's expiry setting says it is archived (#2600).
    pub(crate) fn page_list_for_cli(&self, space: SpaceExpiry) -> Result<serde_json::Value> {
        self.page_list_with(Some(space))
    }
    fn page_list_with(&self, space: Option<SpaceExpiry>) -> Result<serde_json::Value> {
        let mut query = self.tx.prepare(
            "SELECT page,epoch,last_update_at_ms,last_activity_at_ms FROM pages ORDER BY page LIMIT 1001",
        )?;
        let rows = query.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<i64>>(2)?,
                r.get::<_, Option<i64>>(3)?,
            ))
        })?;
        let mut pages = std::collections::BTreeMap::new();
        let mut activity = std::collections::BTreeMap::new();
        for row in rows {
            let (id, epoch, updated, active) = row?;
            let updated = updated
                .map(u64::try_from)
                .transpose()
                .map_err(|_| OwnerFault::Invalid)?;
            let active = active
                .map(u64::try_from)
                .transpose()
                .map_err(|_| OwnerFault::Invalid)?;
            activity.insert(id.clone(), active.max(updated));
            values::generated_id(&id)?;
            values::decimal(&epoch, false)?;
            if pages.len() == 1000 {
                return Err(OwnerFault::Capacity.into());
            }
            pages.insert(id.clone(),serde_json::json!({"pageId":id,"epoch":epoch,"sharing":"private","history":"shared","archived":false,"retentionDays":30,"lastUpdateAtMs":updated}));
        }
        let known_ids: Vec<String> = pages.keys().cloned().collect();
        let mut retention: std::collections::BTreeMap<String, PageRetention> = Default::default();
        // Projects are display-only owner-store state (never a log statement): the rows of the pages
        // that exist, sorted; a deleted page's rows are removed with its data.
        let mut projects: std::collections::BTreeMap<String, Vec<String>> = Default::default();
        let mut stored = self
            .tx
            .prepare("SELECT page,project FROM page_projects ORDER BY page,project")?;
        for row in stored.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
            let (page, project) = row?;
            projects.entry(page).or_default().push(project);
        }
        // Existence/epoch are the local creation projection; presentation policy
        // comes only from owner-signed statements. Never expose encrypted titles.
        let mut head = None;
        let mut policies = std::collections::BTreeMap::<String, crate::fold::PagePolicy>::new();
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
            if let Some(page) = verified.payload.page() {
                policies
                    .entry(page.to_owned())
                    .or_default()
                    .apply(&verified.payload, page)?;
            }
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
                payload::Payload::RetentionSet(p) => {
                    if let Some(page) = pages.get_mut(&p.page_id) {
                        page["retentionDays"] = match p.days {
                            payload::Days::Forever => serde_json::Value::Null,
                            payload::Days::Count(n) => serde_json::json!(n),
                        };
                        retention.insert(
                            p.page_id.clone(),
                            match p.days {
                                payload::Days::Forever => PageRetention::Forever,
                                payload::Days::Count(n) => PageRetention::Days(n),
                            },
                        );
                    }
                }
                payload::Payload::Archive(p) | payload::Payload::Unarchive(p) => {
                    if let Some(page) = pages.get_mut(&p.page_id) {
                        page["archived"] = policies
                            .get(&p.page_id)
                            .ok_or(OwnerFault::Invalid)?
                            .archived
                            .into();
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
        let now = (self.clock)()?;
        values::time(now)?;
        // The expiry is the cloud's own: what the last confirmed write of the page's owner row
        // made it, for the epoch the page is at now (never derived from local activity). A
        // machine that never had a page row confirmed has no cloud copy to warn about.
        let mut confirmed = std::collections::BTreeMap::new();
        let mut stored = self
            .tx
            .prepare("SELECT page,epoch,expires_at_ms FROM cloud_page_rows ORDER BY page")?;
        for row in stored.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })? {
            let (id, epoch, expires) = row?;
            let expires = u64::try_from(expires).map_err(|_| OwnerFault::Invalid)?;
            values::time(expires)?;
            confirmed.insert(id, (epoch, expires));
        }
        let cloud_in_use = !confirmed.is_empty();
        for (id, page) in pages.iter_mut() {
            if let Some(updated) = page["lastUpdateAtMs"].as_u64() {
                values::time(updated)?;
            }
            let expiry = confirmed
                .get(id)
                .filter(|(epoch, _)| page["epoch"].as_str() == Some(epoch.as_str()))
                .map(|&(_, expires)| expires);
            // A page kept forever locally still reports its confirmed cloud copy's deadline and
            // warnings, since losing that copy ends sharing; with no confirmed row it has neither.
            let (shown, warning) = match (page["retentionDays"].as_u64(), expiry) {
                (None, None) => (None, None),
                (Some(_), None) => (None, cloud_in_use.then_some("not-in-cloud")),
                (_, Some(expiry)) if expiry <= now => (Some(expiry), Some("not-in-cloud")),
                (_, Some(expiry)) if expiry - now <= 7 * 86_400_000 => {
                    (Some(expiry), Some("expires-soon"))
                }
                (_, Some(expiry)) => (Some(expiry), None),
            };
            page["expiresAtMs"] = serde_json::json!(shown);
            page["warnings"] = serde_json::json!(warning.into_iter().collect::<Vec<_>>());
        }
        for (id, page) in pages.iter_mut() {
            page["projects"] = serde_json::json!(projects.remove(id).unwrap_or_default());
        }
        if let Some(space) = space {
            for (id, page) in pages.iter_mut() {
                let active = activity.get(id).copied().flatten();
                let page_retention = retention.get(id).copied().unwrap_or(PageRetention::Inherit);
                let days = effective_days(page_retention, space);
                page["lastActivityAtMs"] = serde_json::json!(active);
                page["archiveAfterDays"] = serde_json::json!(days);
                // An archived page is past this; a page whose activity is unknown has no due time.
                page["archiveAtMs"] = serde_json::json!(if page["archived"] == true {
                    None
                } else {
                    archive_at_ms(active, days)
                });
            }
        }
        let page_ids: Vec<_> = known_ids
            .iter()
            .map(|id| serde_json::json!({"pageId":id,"deleted":!pages.contains_key(id)}))
            .collect();
        Ok(
            serde_json::json!({"pageIds":page_ids,"spaceId":self.space,"ownerKey":values::encode_binary(self.root),"revision":self.head().map_or(0,|h| h.revision).to_string(),"projects":self.projects()?,"pages":pages.into_values().collect::<Vec<_>>()}),
        )
    }
}

/// At most this many 32 KiB pieces carry one membership statement in the cloud mapping.
const MEMBERSHIP_PIECES: usize = 11;
impl OwnerTransaction<'_> {
    /// Every signed membership statement up to the head, exactly as stored. Consumers verify
    /// them in order against the space owner; nothing here is reserialized.
    pub(crate) fn membership_originals(&self, target: &statement::Head) -> Result<Vec<Vec<u8>>> {
        if target.revision > 4096 {
            return Err(OwnerFault::Capacity.into());
        }
        let mut out = Vec::new();
        for revision in 1..=target.revision {
            let bytes: Vec<u8> = self
                .tx
                .query_row(
                    "SELECT envelope FROM membership_log WHERE revision=?",
                    [sequence(revision)],
                    |r| r.get(0),
                )
                .optional()?
                .ok_or(super::super::Fault::ResyncRequired)?;
            if bytes.is_empty() || bytes.len() > MEMBERSHIP_PIECES * crate::limits::CHUNK_BYTES {
                return Err(OwnerFault::Capacity.into());
            }
            out.push(bytes);
        }
        Ok(out)
    }
    /// The signed key wraps addressed to the owner member for the page, exactly as stored:
    /// what the owner's own other machine reads with the owner's keys.
    pub(crate) fn owner_member_wraps(
        &self,
        page: &str,
        epoch: u64,
        target: &statement::Head,
    ) -> Result<Vec<Vec<u8>>> {
        let mut query = self.tx.prepare(
            "SELECT envelope FROM wraps WHERE page=?1 AND epoch<=?2 AND revision<=?3
             AND kind='member' AND recipient=?4 ORDER BY epoch,revision LIMIT 65",
        )?;
        let rows = query.query_map(
            params![
                page,
                sequence(epoch),
                sequence(target.revision),
                target.owner_member.id
            ],
            |r| r.get::<_, Vec<u8>>(0),
        )?;
        let mut out = Vec::new();
        for row in rows {
            let bytes = row?;
            if bytes.is_empty() || bytes.len() > 2048 {
                return Err(OwnerFault::Capacity.into());
            }
            out.push(bytes);
        }
        if out.len() > 64 {
            return Err(OwnerFault::Capacity.into());
        }
        Ok(out)
    }
    /// The signed key wraps addressed to a link for the page, exactly as stored, oldest epoch first:
    /// what a holder of the link reads, beside the owner's own.
    pub(crate) fn owner_link_wraps(
        &self,
        page: &str,
        epoch: u64,
        target: &statement::Head,
        link: &str,
    ) -> Result<Vec<Vec<u8>>> {
        let mut query = self.tx.prepare(
            "SELECT envelope FROM wraps WHERE page=?1 AND epoch<=?2 AND revision<=?3
             AND kind='link' AND recipient=?4 ORDER BY epoch,revision LIMIT 65",
        )?;
        let rows = query.query_map(
            params![page, sequence(epoch), sequence(target.revision), link],
            |r| r.get::<_, Vec<u8>>(0),
        )?;
        let mut out = Vec::new();
        for row in rows {
            let bytes = row?;
            if bytes.is_empty() || bytes.len() > 2048 {
                return Err(OwnerFault::Capacity.into());
            }
            out.push(bytes);
        }
        if out.len() > 64 {
            return Err(OwnerFault::Capacity.into());
        }
        Ok(out)
    }
    /// Whether this machine writes the stream of device `id`, which is what makes it ours to
    /// publish and not the cloud's to hand back. `writer` is the root-local writer this
    /// machine's keyring names (it seals every CLI write and every browser Save and has no
    /// registration row); any other local device is one this machine registered. A device whose
    /// chain was imported from the cloud has neither, and stays foreign.
    pub(crate) fn is_local_device(&self, writer: &str, id: &str) -> Result<bool> {
        if id == writer {
            return Ok(self.device(id)?.is_some_and(|device| !device.revoked));
        }
        Ok(self.registration(id)?.is_some_and(|row| !row.revoked))
    }
}

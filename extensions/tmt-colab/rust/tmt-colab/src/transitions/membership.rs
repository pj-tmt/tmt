//! Root-local membership and known-device changes. Remote event admission stays in Registration.
use super::{Engine, TransitionError, epoch};
use crate::{
    Result,
    decoder::Decoder,
    fold::{self, Authority, Issuer, Snapshot},
    keyring::Keyring,
    store::{
        Store,
        owner::{Cut, Mutation, OwnerFault, OwnerTransaction, Recipient},
    },
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use tmt_colab_model::{certificate, crypto, framing, payload, statement, values, wrap};

pub enum MemberAction {
    Add(Recipient),
    Remove { member_id: String },
    Role { member_id: String, role: String },
}
pub struct MemberRequest<'a> {
    pub operation_id: &'a str,
    pub expected_revision: u64,
    pub action: MemberAction,
}
pub struct DeviceRevoke<'a> {
    pub operation_id: &'a str,
    pub expected_revision: u64,
    pub device_id: &'a str,
    /// Trusted remote grant revision; replay fencing is independent of operation ID.
    pub grant_revision: u64,
}
enum Action<'a> {
    Member(MemberAction),
    Device { id: &'a str, grant: u64 },
}
impl Action<'_> {
    fn reduction(&self, target: &Recipient) -> bool {
        match self {
            Self::Member(MemberAction::Add(_)) => false,
            Self::Member(MemberAction::Role { role, .. }) => {
                rank(role) < rank(target.role.as_deref().unwrap_or(""))
            }
            _ => true,
        }
    }
    fn selected(&self) -> (&'static str, Value) {
        match self {
            Self::Member(MemberAction::Add(r)) => (
                "member.add",
                json!({"memberId":r.id,"role":r.role,
                "signKey":values::encode_binary(&r.signing_key),"encKey":values::encode_binary(&r.encryption_key),"pages":r.pages}),
            ),
            Self::Member(MemberAction::Remove { member_id }) => {
                ("member.remove", json!({"memberId":member_id}))
            }
            Self::Member(MemberAction::Role { member_id, role }) => {
                ("member.role", json!({"memberId":member_id,"role":role}))
            }
            Self::Device { id, grant } => (
                "device.revoke",
                json!({"deviceId":id,"grantRevision":grant}),
            ),
        }
    }
}
struct Plan {
    authority: Authority,
    pages: Vec<String>,
    target: Recipient,
    devices: BTreeSet<String>,
    noop: bool,
    catalog: Vec<String>,
    device_digest: [u8; 32],
}
impl Engine {
    pub fn member(
        &mut self,
        store: &mut Store,
        key: &Keyring,
        request: MemberRequest<'_>,
        now: u64,
    ) -> std::result::Result<Vec<u8>, TransitionError> {
        self.change(
            store,
            key,
            request.operation_id,
            request.expected_revision,
            Action::Member(request.action),
            now,
        )
        .map(|(outcome, _)| outcome)
        .map_err(TransitionError::from_error)
    }
    pub fn revoke_device(
        &mut self,
        store: &mut Store,
        key: &Keyring,
        request: DeviceRevoke<'_>,
        now: u64,
    ) -> std::result::Result<Vec<u8>, TransitionError> {
        self.change(
            store,
            key,
            request.operation_id,
            request.expected_revision,
            Action::Device {
                id: request.device_id,
                grant: request.grant_revision,
            },
            now,
        )
        .map(|(outcome, _)| outcome)
        .map_err(TransitionError::from_error)
    }
    pub(crate) fn remote_revoke(
        &mut self,
        store: &mut Store,
        key: &Keyring,
        id: &str,
        grant: u64,
        now: u64,
    ) -> Result<bool> {
        let digest = crypto::digest(&framing::frame(&[
            b"tmt-colab-remote-revoke-operation-v1",
            key.space_id.as_bytes(),
            id.as_bytes(),
            grant.to_string().as_bytes(),
        ])?);
        let mut bytes = digest[..16].to_vec();
        bytes[6] = (bytes[6] & 15) | 64;
        bytes[8] = (bytes[8] & 63) | 128;
        let hex = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
        let op = format!(
            "{}-{}-{}-{}-{}",
            &hex[..8],
            &hex[8..12],
            &hex[12..16],
            &hex[16..20],
            &hex[20..]
        );
        for _ in 0..3 {
            let revision = store
                .owner_head(&key.space_id, &key.owner_public())?
                .ok_or(OwnerFault::Invalid)?
                .revision;
            match self.change(store, key, &op, revision, Action::Device { id, grant }, now) {
                Err(e) if e.downcast_ref::<OwnerFault>() == Some(&OwnerFault::StaleHead) => {
                    continue;
                }
                other => return other.map(|(_, changed)| changed),
            }
        }
        Err(OwnerFault::StaleHead.into())
    }
    fn change(
        &mut self,
        store: &mut Store,
        key: &Keyring,
        id: &str,
        expected: u64,
        action: Action<'_>,
        now: u64,
    ) -> Result<(Vec<u8>, bool)> {
        values::generated_id(id)?;
        values::time(now)?;
        let (operation, selected) = action.selected();
        let digest = crypto::digest(&framing::frame(&[
            b"tmt-colab-local-transition-v1",
            key.space_id.as_bytes(),
            id.as_bytes(),
            expected.to_string().as_bytes(),
            operation.as_bytes(),
            &serde_json::to_vec(&selected)?,
        ])?);
        if let Some(saved) = store.owner_read(&key.space_id, &key.owner_public(), |tx| {
            tx.saved_operation(id, &digest)
        })? {
            return Ok((saved, false));
        }
        for _ in 0..3 {
            let plan = store.owner_read(&key.space_id, &key.owner_public(), |tx| {
                plan(tx, key, &action)
            })?;
            // No-op events never write a receipt or require a matching stale log revision.
            if plan.noop {
                return Ok((
                    serde_json::to_vec(&json!({"statements":[],"wrapLists":[]}))?,
                    false,
                ));
            }
            if plan.authority.head.revision != expected {
                return Err(OwnerFault::StaleHead.into());
            }
            let mut snapshots = BTreeMap::new();
            for page in &plan.pages {
                let snapshot = Snapshot::capture(store, key, page)?;
                if snapshot.authority.head != plan.authority.head {
                    return Err(OwnerFault::StaleHead.into());
                }
                snapshots.insert(page.clone(), snapshot);
            }
            let mut prepared = BTreeMap::new();
            let mut next = expected.checked_add(1).ok_or(OwnerFault::Capacity)?;
            for (page, snapshot) in snapshots {
                let rotate = match &action {
                    Action::Member(MemberAction::Add(_)) => snapshot.authority.history_current,
                    Action::Member(MemberAction::Role { .. }) => false,
                    _ => true,
                };
                if rotate {
                    next = next.checked_add(1).ok_or(OwnerFault::Capacity)?;
                    if !self.decoders.contains_key(&page) {
                        self.decoders
                            .insert(page.clone(), Decoder::new(self.program.clone())?);
                    }
                    let decoder = self.decoders.get_mut(&page).ok_or(OwnerFault::Invalid)?;
                    let rotation = epoch::Prepared::new(snapshot, key, &page, next, decoder)?;
                    prepared.insert(page, Page::Rotate(Box::new(rotation)));
                } else {
                    if matches!(action, Action::Member(MemberAction::Role { .. }))
                        && action.reduction(&plan.target)
                    {
                        if !self.decoders.contains_key(&page) {
                            self.decoders
                                .insert(page.clone(), Decoder::new(self.program.clone())?);
                        }
                        snapshot.materialize(
                            key,
                            &page,
                            self.decoders.get_mut(&page).ok_or(OwnerFault::Invalid)?,
                        )?;
                    }
                    prepared.insert(page, Page::Keep(Box::new(snapshot)));
                }
            }
            let mut changed = false;
            let committed = store.owner_transaction(
                &key.space_id,
                &key.owner_public(),
                Mutation {
                    operation_id: id,
                    digest,
                    expected_revision: expected,
                },
                |tx| {
                    if tx.head() != Some(&plan.authority.head)
                        || tx.pages()? != plan.catalog
                        || crypto::digest(&serde_json::to_vec(&tx.devices()?)?)
                            != plan.device_digest
                    {
                        return Err(OwnerFault::StaleHead.into());
                    }
                    for (page, p) in &prepared {
                        epoch::recheck(tx, p.snapshot(), page)?;
                    }
                    if let Action::Device { id, grant } = &action
                        && (tx.device(id)?.is_none_or(|d| d.revoked)
                            || tx
                                .registration(id)?
                                .is_some_and(|r| r.revoked || r.grant_revision >= *grant))
                    {
                        return Err(OwnerFault::StaleHead.into());
                    }
                    let reduction = action.reduction(&plan.target);
                    let mut cuts = Vec::<Cut>::new();
                    if reduction {
                        for p in prepared.values() {
                            cuts.extend(
                                p.snapshot()
                                    .cuts
                                    .iter()
                                    .filter(|c| plan.devices.contains(&c.stream))
                                    .cloned(),
                            );
                        }
                    }
                    if cuts.len() > 512 {
                        return Err(OwnerFault::Capacity.into());
                    }
                    tx.pin_cuts(&cuts)?;
                    let mut payload = selected.clone();
                    if operation != "member.add" {
                        payload["cuts"] =
                            Value::Array(cuts.iter().map(|c| c.payload()).collect::<Result<_>>()?);
                    }
                    if operation == "device.revoke" {
                        payload
                            .as_object_mut()
                            .ok_or(OwnerFault::Invalid)?
                            .remove("grantRevision");
                    }
                    let statement =
                        key.sign_statement(tx.head(), operation, &serde_json::to_vec(&payload)?)?;
                    tx.append_statement(&statement)?;
                    let mut statements = vec![statement];
                    let mut wraps = Vec::new();
                    for (page, p) in &prepared {
                        let mut authority = p.snapshot().authority.clone();
                        project(&mut authority, key, &action, &plan.target, &statements[0])?;
                        match p {
                            Page::Rotate(p) => {
                                let (s, w) = p.commit(tx, key, &authority, now)?;
                                statements.push(s);
                                wraps.extend(w);
                            }
                            Page::Keep(snapshot) if operation == "member.add" => {
                                let r = &plan.target;
                                if snapshot.authority.history_current {
                                    return Err(OwnerFault::Invalid.into());
                                }
                                for (epoch, secret) in tx.recent_secrets(page, snapshot.epoch)? {
                                    let w = key.seal_wrap(
                                        &wrap::Header {
                                            space: key.space_id.clone(),
                                            page: page.clone(),
                                            epoch: epoch.to_string(),
                                            recipient_kind: "member".into(),
                                            recipient_id: r.id.clone(),
                                            recipient_key: r.encryption_key,
                                            signer_key: key.owner_public(),
                                            membership_revision: (expected + 1).to_string(),
                                        },
                                        &secret,
                                    )?;
                                    tx.put_wrap(&w)?;
                                    wraps.push(w);
                                }
                            }
                            _ => {}
                        }
                    }
                    if let Action::Member(_) = &action {
                        let mut r = plan.target.clone();
                        match &action {
                            Action::Member(MemberAction::Remove { .. }) => r.revoked = true,
                            Action::Member(MemberAction::Role { role, .. }) => {
                                r.role = Some(role.clone())
                            }
                            _ => {}
                        }
                        tx.put_recipient(&r)?;
                    }
                    if operation == "member.remove" {
                        for device in &plan.devices {
                            if let Some(mut d) = tx.device(device)? {
                                d.revoked = true;
                                tx.put_device(&d)?;
                            }
                        }
                    }
                    if let Action::Device { id, grant } = &action {
                        tx.tombstone(id, *grant)?;
                    }
                    changed = true;
                    outcome(&statements, wraps)
                },
            );
            match committed {
                Err(e) if e.downcast_ref::<OwnerFault>() == Some(&OwnerFault::StaleHead) => {
                    continue;
                }
                other => return other.map(|outcome| (outcome, changed)),
            }
        }
        Err(OwnerFault::StaleHead.into())
    }
}
enum Page {
    Rotate(Box<epoch::Prepared>),
    Keep(Box<Snapshot>),
}
impl Page {
    fn snapshot(&self) -> &Snapshot {
        match self {
            Self::Rotate(p) => &p.snapshot,
            Self::Keep(s) => s,
        }
    }
}
fn rank(role: &str) -> u8 {
    match role {
        "viewer" => 0,
        "commenter" => 1,
        "editor" => 2,
        _ => 3,
    }
}
fn plan(tx: &OwnerTransaction<'_>, key: &Keyring, action: &Action<'_>) -> Result<Plan> {
    let log = tx.log()?;
    let (states, _) = fold::verify_log(&log, key, "")?;
    let authority = states.last().ok_or(OwnerFault::Invalid)?.clone();
    let catalog = tx.pages()?;
    let device_digest = crypto::digest(&serde_json::to_vec(&tx.devices()?)?);
    let mut devices = BTreeSet::new();
    let mut noop = false;
    let target = match action {
        Action::Member(MemberAction::Add(r)) => {
            if r.kind != "member" || r.revoked {
                return Err(OwnerFault::Invalid.into());
            }
            let (op, payload) = action.selected();
            payload::decode(op, &serde_json::to_vec(&payload)?)?;
            if authority
                .recipients
                .contains_key(&("member".into(), r.id.clone()))
            {
                return Err(OwnerFault::Conflict.into());
            }
            r.clone()
        }
        Action::Member(MemberAction::Remove { member_id })
        | Action::Member(MemberAction::Role { member_id, .. }) => {
            values::generated_id(member_id)?;
            if member_id == &authority.head.owner_member.id {
                return Err(OwnerFault::WrongOwner.into());
            }
            let r = authority
                .recipients
                .get(&("member".into(), member_id.clone()))
                .ok_or(OwnerFault::Invalid)?
                .recipient
                .clone();
            if r.revoked {
                return Err(OwnerFault::WrongOwner.into());
            }
            if let Action::Member(MemberAction::Role { role, .. }) = action {
                payload::decode(
                    "member.role",
                    &serde_json::to_vec(&json!({"memberId":member_id,"role":role,"cuts":[]}))?,
                )?;
            }
            for d in tx.devices()? {
                let chain = certificate::Chain::from_json(&d.chain)?;
                let c = chain.certificate()?;
                if c.issuer_kind == "member" && c.issuer_id == member_id {
                    fold::verify_chain(
                        &chain,
                        authority
                            .recipients
                            .get(&("member".into(), member_id.clone()))
                            .ok_or(OwnerFault::Invalid)?,
                        key,
                        authority.head.revision,
                    )?;
                    devices.insert(c.device_id.into());
                }
            }
            r
        }
        Action::Device { id, grant } => {
            values::generated_id(id)?;
            if *grant == 0 {
                return Err(OwnerFault::Invalid.into());
            }
            let d = tx.device(id)?.ok_or(OwnerFault::Invalid)?;
            noop = d.revoked
                || authority.revoked_devices.contains(*id)
                || tx
                    .registration(id)?
                    .is_some_and(|r| r.revoked || r.grant_revision >= *grant);
            let chain = certificate::Chain::from_json(&d.chain)?;
            let c = chain.certificate()?;
            let issuer = authority
                .recipients
                .get(&(c.issuer_kind.into(), c.issuer_id.into()))
                .ok_or(OwnerFault::Invalid)?;
            fold::verify_chain(&chain, issuer, key, authority.head.revision)?;
            devices.insert((*id).into());
            issuer.recipient.clone()
        }
    };
    let pages = match action {
        Action::Member(MemberAction::Add(r)) => r.pages.clone(),
        _ => {
            let r = &target;
            let pages = if r.id == authority.head.owner_member.id && r.kind == "member" {
                tx.pages()?
            } else {
                r.pages.clone()
            };
            let mut active = Vec::new();
            for page in pages {
                let (states, _) = fold::verify_log(&log, key, &page)?;
                if states.last().is_some_and(|a| fold::eligible(r, a, &page)) {
                    active.push(page);
                }
            }
            active
        }
    };
    if pages.len() > 256 {
        return Err(OwnerFault::Capacity.into());
    }
    Ok(Plan {
        authority,
        pages,
        target,
        devices,
        noop,
        catalog,
        device_digest,
    })
}
fn project(
    authority: &mut Authority,
    key: &Keyring,
    action: &Action<'_>,
    target: &Recipient,
    statement: &statement::Envelope,
) -> Result<()> {
    match action {
        Action::Member(MemberAction::Add(r)) => {
            let verified =
                statement.verify_next(&key.space_id, &key.owner_public(), Some(&authority.head))?;
            Arc::make_mut(&mut authority.recipients).insert(
                ("member".into(), r.id.clone()),
                Issuer {
                    recipient: r.clone(),
                    statement_hash: statement.hash()?,
                    revision: verified.head.revision,
                },
            );
        }
        Action::Member(MemberAction::Remove { .. }) => {
            Arc::make_mut(&mut authority.recipients)
                .get_mut(&("member".into(), target.id.clone()))
                .ok_or(OwnerFault::Invalid)?
                .recipient
                .revoked = true;
        }
        Action::Member(MemberAction::Role { role, .. }) => {
            Arc::make_mut(&mut authority.recipients)
                .get_mut(&("member".into(), target.id.clone()))
                .ok_or(OwnerFault::Invalid)?
                .recipient
                .role = Some(role.clone());
        }
        Action::Device { id, .. } => {
            Arc::make_mut(&mut authority.revoked_devices).insert((*id).into());
        }
    }
    Ok(())
}
fn outcome(statements: &[statement::Envelope], wraps: Vec<wrap::Envelope>) -> Result<Vec<u8>> {
    let mut ordered = BTreeMap::new();
    for w in wraps {
        let h = w.header()?;
        if ordered
            .insert(
                (
                    h.page.clone(),
                    values::decimal(&h.epoch, false)?,
                    h.recipient_kind.clone(),
                    h.recipient_id.clone(),
                ),
                w,
            )
            .is_some()
        {
            return Err(OwnerFault::Invalid.into());
        }
    }
    let wraps = ordered.into_values().collect::<Vec<_>>();
    let lists = wraps
        .chunks(512)
        .map(|chunk| chunk.to_vec())
        .collect::<Vec<_>>();
    Ok(serde_json::to_vec(
        &json!({"statements":statements.iter().map(|s|Ok(serde_json::from_slice::<Value>(&s.to_json()?)?)).collect::<Result<Vec<_>>>()?,"wrapLists":lists}),
    )?)
}

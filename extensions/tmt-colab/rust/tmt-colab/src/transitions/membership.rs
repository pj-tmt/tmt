//! Atomic recipient and known-device changes. Remote event admission stays in Registration.
use super::{Engine, OwnerAction, OwnerRequest, TransitionError, epoch, request::OwnerContext};
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
pub(super) enum Action<'a> {
    Member(MemberAction),
    Device {
        id: &'a str,
        grant: u64,
    },
    Link {
        remove: Option<&'a str>,
        add: Option<Recipient>,
        seed: Option<&'a [u8; 32]>,
    },
}
impl Action<'_> {
    fn reduction(&self, target: &Recipient) -> bool {
        match self {
            Self::Member(MemberAction::Add(_)) | Self::Link { remove: None, .. } => false,
            Self::Member(MemberAction::Role { role, .. }) => {
                rank(role) < rank(target.role.as_deref().unwrap_or(""))
            }
            _ => true,
        }
    }
    pub(super) fn selected(&self) -> (&'static str, Value) {
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
            Self::Link {
                remove: Some(id),
                add,
                ..
            } => (
                "link.remove",
                json!({"linkId":id,
                "replacement":add.as_ref().map(link_payload)}),
            ),
            Self::Link {
                remove: None,
                add: Some(r),
                ..
            } => ("link.add", link_payload(r)),
            Self::Link { .. } => ("link.add", Value::Null),
            Self::Device { id, grant } => (
                "device.revoke",
                json!({"deviceId":id,"grantRevision":grant}),
            ),
        }
    }
}
fn link_payload(r: &Recipient) -> Value {
    json!({"linkId":r.id,"role":r.role,"linkSignKey":values::encode_binary(&r.signing_key),
        "linkEncKey":values::encode_binary(&r.encryption_key),"pages":r.pages})
}
struct Plan {
    authority: Authority,
    pages: Vec<String>,
    rotate_pages: BTreeSet<String>,
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
        self.apply(
            store,
            key,
            OwnerRequest {
                operation_id: request.operation_id,
                expected_revision: request.expected_revision,
                action: OwnerAction::Member(request.action),
                transport_digest: None,
                scope: None,
            },
            now,
        )
        .map(|applied| applied.outcome)
    }
    pub fn revoke_device(
        &mut self,
        store: &mut Store,
        key: &Keyring,
        request: DeviceRevoke<'_>,
        now: u64,
    ) -> std::result::Result<Vec<u8>, TransitionError> {
        self.apply(
            store,
            key,
            OwnerRequest {
                operation_id: request.operation_id,
                expected_revision: request.expected_revision,
                action: OwnerAction::DeviceRevoke {
                    device_id: request.device_id,
                    grant_revision: request.grant_revision,
                },
                transport_digest: None,
                scope: None,
            },
            now,
        )
        .map(|applied| applied.outcome)
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
            match self.change(
                store,
                key,
                OwnerContext {
                    id: &op,
                    expected: revision,
                    transport: None,
                    scope: None,
                },
                Action::Device { id, grant },
                now,
            ) {
                Err(e) if e.downcast_ref::<OwnerFault>() == Some(&OwnerFault::StaleHead) => {
                    continue;
                }
                other => return other.map(|(_, changed)| changed),
            }
        }
        Err(OwnerFault::StaleHead.into())
    }
    pub(super) fn change(
        &mut self,
        store: &mut Store,
        key: &Keyring,
        request: OwnerContext<'_>,
        action: Action<'_>,
        now: u64,
    ) -> Result<(Vec<u8>, bool)> {
        let id = request.id;
        let expected = request.expected;
        values::generated_id(id)?;
        values::time(now)?;
        let (operation, selected) = action.selected();
        let digest = request.digest(key, operation, &selected)?;
        self.run_transition(
            store,
            key,
            Mutation {
                operation_id: id,
                digest,
                expected_revision: expected,
            },
            |engine, store| {
                let plan = store.owner_read(&key.space_id, &key.owner_public(), |tx| {
                    plan(tx, key, &action, request)
                })?;
                // No-op events never write a receipt or require a matching stale log revision.
                if plan.noop {
                    return Ok(None);
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
                    if let Action::Link { add: Some(r), .. } = &action
                        && r.pages.contains(page)
                        && !fold::eligible(r, &snapshot.authority, page)
                    {
                        return Err(OwnerFault::WrongOwner.into());
                    }
                    snapshots.insert(page.clone(), snapshot);
                }
                let mut prepared = BTreeMap::new();
                let mut next = expected.checked_add(1).ok_or(OwnerFault::Capacity)?;
                for (page, snapshot) in snapshots {
                    let rotate = match &action {
                        Action::Member(MemberAction::Add(_)) => snapshot.authority.history_current,
                        Action::Member(MemberAction::Role { .. }) => false,
                        Action::Link { .. } => plan.rotate_pages.contains(&page),
                        _ => true,
                    };
                    if rotate {
                        next = next.checked_add(1).ok_or(OwnerFault::Capacity)?;
                        if !engine.decoders.contains_key(&page) {
                            engine
                                .decoders
                                .insert(page.clone(), Decoder::new(engine.program.clone())?);
                        }
                        let decoder = engine.decoders.get_mut(&page).ok_or(OwnerFault::Invalid)?;
                        let rotation = epoch::Prepared::new(snapshot, key, &page, next, decoder)?;
                        prepared.insert(page, Page::Rotate(Box::new(rotation)));
                    } else {
                        // Authenticate tails/checkpoints before signing cuts without a new baseline.
                        if matches!(action, Action::Member(MemberAction::Role { .. }))
                            && action.reduction(&plan.target)
                        {
                            if !engine.decoders.contains_key(&page) {
                                engine
                                    .decoders
                                    .insert(page.clone(), Decoder::new(engine.program.clone())?);
                            }
                            snapshot.materialize(
                                key,
                                &page,
                                engine.decoders.get_mut(&page).ok_or(OwnerFault::Invalid)?,
                            )?;
                        }
                        prepared.insert(page, Page::Keep(Box::new(snapshot)));
                    }
                }
                Ok(Some((plan, prepared)))
            },
            |tx, (plan, prepared)| {
                if tx.head() != Some(&plan.authority.head)
                    || tx.pages()? != plan.catalog
                    || crypto::digest(&serde_json::to_vec(&tx.devices()?)?) != plan.device_digest
                {
                    return Err(OwnerFault::StaleHead.into());
                }
                check_scope(tx, request, &action, &plan.target)?;
                for (page, p) in prepared {
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
                if operation == "link.remove" {
                    payload
                        .as_object_mut()
                        .ok_or(OwnerFault::Invalid)?
                        .remove("replacement");
                }
                if operation != "member.add" && operation != "link.add" {
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
                for (page, p) in prepared {
                    let mut authority = p.snapshot().authority.clone();
                    project(&mut authority, key, &action, &plan.target, &statements[0])?;
                    match p {
                        Page::Rotate(p) => {
                            let (s, w) = p.commit(tx, key, &authority, now)?;
                            statements.push(s);
                            wraps.extend(w);
                        }
                        Page::Keep(snapshot)
                            if operation == "member.add" || operation == "link.add" =>
                        {
                            wraps.extend(join_wraps(
                                tx,
                                key,
                                &plan.target,
                                page,
                                snapshot.epoch,
                                snapshot.authority.history_current,
                                expected + 1,
                            )?);
                        }
                        _ => {}
                    }
                }
                // Reset's replacement follows every removal/advance, in this same transaction.
                if let Action::Link {
                    remove: Some(_),
                    add: Some(r),
                    ..
                } = &action
                {
                    let added = key.sign_statement(
                        tx.head(),
                        "link.add",
                        &serde_json::to_vec(&link_payload(r))?,
                    )?;
                    tx.append_statement(&added)?;
                    let revision = tx.head().ok_or(OwnerFault::Invalid)?.revision;
                    for page in &r.pages {
                        let snapshot = prepared.get(page).ok_or(OwnerFault::Invalid)?.snapshot();
                        let epoch = tx.current_epoch(page)?;
                        wraps.extend(join_wraps(
                            tx,
                            key,
                            r,
                            page,
                            epoch,
                            snapshot.authority.history_current,
                            revision,
                        )?);
                    }
                    tx.put_recipient(r)?;
                    statements.push(added);
                }
                if matches!(&action, Action::Member(_) | Action::Link { .. }) {
                    let mut r = plan.target.clone();
                    match &action {
                        Action::Member(MemberAction::Remove { .. })
                        | Action::Link {
                            remove: Some(_), ..
                        } => r.revoked = true,
                        Action::Member(MemberAction::Role { role, .. }) => {
                            r.role = Some(role.clone())
                        }
                        _ => {}
                    }
                    tx.put_recipient(&r)?;
                }
                if operation == "member.remove" || operation == "link.remove" {
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
                outcome(&statements, wraps)
            },
        )
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
fn check_scope(
    tx: &OwnerTransaction<'_>,
    request: OwnerContext<'_>,
    action: &Action<'_>,
    target: &Recipient,
) -> Result<()> {
    if request.scope.is_none() {
        return Ok(());
    }
    let pages = match action {
        Action::Member(MemberAction::Add(_)) | Action::Link { remove: None, .. } => {
            target.pages.clone()
        }
        _ => {
            tx.recipient(&target.kind, &target.id)?
                .ok_or(OwnerFault::StaleHead)?
                .pages
        }
    };
    request.check_scope(&pages)
}
fn plan(
    tx: &OwnerTransaction<'_>,
    key: &Keyring,
    action: &Action<'_>,
    request: OwnerContext<'_>,
) -> Result<Plan> {
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
        Action::Link { remove, add, seed } => {
            if let Some(r) = add {
                payload::decode("link.add", &serde_json::to_vec(&link_payload(r))?)?;
                if authority
                    .recipients
                    .contains_key(&("link".into(), r.id.clone()))
                {
                    return Err(OwnerFault::Conflict.into());
                }
            }
            if let Some(id) = remove {
                values::generated_id(id)?;
                let issuer = authority
                    .recipients
                    .get(&("link".into(), (*id).into()))
                    .ok_or(OwnerFault::Invalid)?;
                let r = &issuer.recipient;
                if r.revoked {
                    return Err(OwnerFault::WrongOwner.into());
                }
                if let Some(seed) = seed {
                    let old = tmt_colab_model::link::Keys::derive(seed, &key.space_id, id)?;
                    if old.signing_public() == r.signing_key
                        || old.recipient().public_key() == r.encryption_key
                    {
                        return Err(OwnerFault::WrongOwner.into());
                    }
                }
                for device in tx.devices()? {
                    let chain = certificate::Chain::from_json(&device.chain)?;
                    let c = chain.certificate()?;
                    if c.issuer_kind == "link" && c.issuer_id == *id {
                        fold::verify_chain(&chain, issuer, key, authority.head.revision)?;
                        devices.insert(c.device_id.into());
                    }
                }
                r.clone()
            } else {
                add.clone().ok_or(OwnerFault::Invalid)?
            }
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
    check_scope(tx, request, action, &target)?;
    let pages = match action {
        Action::Member(MemberAction::Add(r))
        | Action::Link {
            remove: None,
            add: Some(r),
            ..
        } => r.pages.clone(),
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
    let rotate_pages = if matches!(
        action,
        Action::Link {
            remove: Some(_),
            ..
        }
    ) {
        pages.iter().cloned().collect()
    } else {
        BTreeSet::new()
    };
    let mut pages = pages;
    if let Action::Link {
        remove: Some(_),
        add: Some(r),
        ..
    } = action
    {
        pages.extend(r.pages.iter().cloned());
        pages.sort();
        pages.dedup();
    }
    if pages.len() > 256 {
        return Err(OwnerFault::Capacity.into());
    }
    Ok(Plan {
        authority,
        pages,
        rotate_pages,
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
        Action::Link {
            remove: Some(_), ..
        } => {
            Arc::make_mut(&mut authority.recipients)
                .get_mut(&("link".into(), target.id.clone()))
                .ok_or(OwnerFault::Invalid)?
                .recipient
                .revoked = true;
        }
        Action::Link {
            remove: None,
            add: Some(r),
            ..
        } => {
            let verified =
                statement.verify_next(&key.space_id, &key.owner_public(), Some(&authority.head))?;
            Arc::make_mut(&mut authority.recipients).insert(
                ("link".into(), r.id.clone()),
                Issuer {
                    recipient: r.clone(),
                    statement_hash: statement.hash()?,
                    revision: verified.head.revision,
                },
            );
        }
        Action::Link { .. } => return Err(OwnerFault::Invalid.into()),
        Action::Device { id, .. } => {
            Arc::make_mut(&mut authority.revoked_devices).insert((*id).into());
        }
    }
    Ok(())
}
fn join_wraps(
    tx: &mut OwnerTransaction<'_>,
    key: &Keyring,
    r: &Recipient,
    page: &str,
    epoch: u64,
    current: bool,
    revision: u64,
) -> Result<Vec<wrap::Envelope>> {
    let mut wraps = Vec::new();
    let secrets = if current {
        vec![(
            epoch,
            tx.epoch_secret(page, epoch)?.ok_or(OwnerFault::Invalid)?,
        )]
    } else {
        tx.recent_secrets(page, epoch)?
    };
    for (retained, mut secret) in secrets {
        let result: Result<()> = (|| {
            let wrapped = key.seal_wrap(
                &wrap::Header {
                    space: key.space_id.clone(),
                    page: page.into(),
                    epoch: retained.to_string(),
                    recipient_kind: r.kind.clone(),
                    recipient_id: r.id.clone(),
                    recipient_key: r.encryption_key,
                    signer_key: key.owner_public(),
                    membership_revision: revision.to_string(),
                },
                &secret,
            )?;
            tx.put_wrap(&wrapped)?;
            wraps.push(wrapped);
            Ok(())
        })();
        secret.fill(0);
        result?;
    }
    Ok(wraps)
}
pub(super) fn outcome(
    statements: &[statement::Envelope],
    wraps: Vec<wrap::Envelope>,
) -> Result<Vec<u8>> {
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

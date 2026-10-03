//! Root-local page policy; transport/session authorization remains caller-owned.
use super::{
    Engine, HistoryMode, Publication, ShareMode, epoch, membership, request::OwnerContext,
};
use crate::{
    Result,
    fold::{self, Authority, Snapshot},
    keyring::Keyring,
    store::{
        Store,
        owner::{Mutation, OwnerFault, OwnerTransaction, Recipient},
    },
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use tmt_colab_model::{certificate, crypto, payload, values};

impl ShareMode {
    fn text(self) -> &'static str {
        match self {
            Self::Private => "private",
            Self::Link => "link",
            Self::Public => "public",
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub(super) enum PageAction {
    Share {
        mode: ShareMode,
        publication: Publication,
    },
    History(HistoryMode),
    Retention(Option<u64>),
    Archive,
    Delete,
}
impl PageAction {
    fn selected(self, page: &str) -> (&'static str, Value) {
        match self {
            Self::Share { mode, publication } => (
                "page.share",
                json!({"pageId":page,"mode":mode.text(),
                "publication":match publication { Publication::Loopback => "loopback", Publication::Cloud => "cloud" }}),
            ),
            Self::History(mode) => (
                "page.history",
                json!({"pageId":page,"mode":match mode { HistoryMode::Shared => "shared", HistoryMode::Current => "current" }}),
            ),
            Self::Retention(days) => ("retention.set", json!({"pageId":page,"days":days})),
            Self::Archive => ("page.archive", json!({"pageId":page})),
            Self::Delete => ("page.delete", json!({"pageId":page})),
        }
    }
}
pub(super) struct PageRequest<'a> {
    pub operation_id: &'a str,
    pub expected_revision: u64,
    pub page: &'a str,
    pub action: PageAction,
}
struct Plan {
    authority: Authority,
    catalog: Vec<String>,
    device_digest: [u8; 32],
    links: Vec<Recipient>,
    devices: BTreeMap<String, BTreeSet<String>>,
    rotations: BTreeSet<String>,
}
impl Engine {
    pub(super) fn page_change(
        &mut self,
        store: &mut Store,
        key: &Keyring,
        request: PageRequest<'_>,
        now: u64,
        context: OwnerContext<'_>,
    ) -> Result<(Vec<u8>, bool)> {
        values::generated_id(request.operation_id)?;
        values::generated_id(request.page)?;
        values::time(now)?;
        let (operation, selected) = request.action.selected(request.page);
        if operation != "page.share" {
            payload::decode(operation, &serde_json::to_vec(&selected)?)?;
        }
        let digest = context.digest(key, operation, &selected)?;
        self.run_transition(
            store,
            key,
            Mutation {
                operation_id: request.operation_id,
                digest,
                expected_revision: request.expected_revision,
            },
            |engine, store| {
                let plan = store.owner_read(&key.space_id, &key.owner_public(), |tx| {
                    plan(tx, key, &request, context)
                })?;
                if plan.authority.head.revision != request.expected_revision {
                    return Err(OwnerFault::StaleHead.into());
                }
                let mut rotations = BTreeMap::new();
                let mut revision = request
                    .expected_revision
                    .checked_add(plan.links.len() as u64)
                    .ok_or(OwnerFault::Capacity)?;
                for page in &plan.rotations {
                    let snapshot = Snapshot::capture(store, key, page)?;
                    if snapshot.authority.head != plan.authority.head {
                        return Err(OwnerFault::StaleHead.into());
                    }
                    let public = snapshot.authority.policy.public_mode && page != request.page;
                    revision = revision.checked_add(1).ok_or(OwnerFault::Capacity)?;
                    let prepared =
                        epoch::Prepared::new(snapshot, key, page, revision, engine.decoder(page)?)?;
                    rotations.insert(page.clone(), prepared);
                    if public {
                        revision = revision.checked_add(1).ok_or(OwnerFault::Capacity)?;
                    }
                }
                Ok(Some((plan, rotations)))
            },
            |tx, (plan, rotations)| {
                if tx.head() != Some(&plan.authority.head)
                    || tx.pages()? != plan.catalog
                    || crypto::digest(&serde_json::to_vec(&tx.devices()?)?) != plan.device_digest
                    || tx.current_epoch(request.page)? != plan.authority.policy.epoch
                {
                    return Err(OwnerFault::StaleHead.into());
                }
                context.check_scope(&[request.page.into()])?;
                for prepared in rotations.values() {
                    prepared.recheck(tx)?;
                }
                let mut statements = Vec::new();
                let mut wraps = Vec::new();
                for link in &plan.links {
                    let devices = plan.devices.get(&link.id).ok_or(OwnerFault::Invalid)?;
                    let cuts = rotations
                        .values()
                        .flat_map(|p| p.snapshot.cuts.iter())
                        .filter(|c| devices.contains(&c.stream))
                        .cloned()
                        .collect::<Vec<_>>();
                    if cuts.len() > 512 {
                        return Err(OwnerFault::Capacity.into());
                    }
                    tx.pin_cuts(&cuts)?;
                    let removed = key.sign_statement(
                        tx.head(),
                        "link.remove",
                        &serde_json::to_vec(&json!({"linkId":link.id,
                        "cuts":cuts.iter().map(|c| c.payload()).collect::<Result<Vec<_>>>()?}))?,
                    )?;
                    tx.append_statement(&removed)?;
                    statements.push(removed);
                    let mut recipient = link.clone();
                    recipient.revoked = true;
                    tx.put_recipient(&recipient)?;
                    for id in devices {
                        if let Some(mut d) = tx.device(id)? {
                            d.revoked = true;
                            tx.put_device(&d)?;
                        }
                    }
                }
                for (page, prepared) in rotations {
                    let mut authority = prepared.snapshot.authority.clone();
                    for link in &plan.links {
                        Arc::make_mut(&mut authority.recipients)
                            .get_mut(&("link".into(), link.id.clone()))
                            .ok_or(OwnerFault::Invalid)?
                            .recipient
                            .revoked = true;
                    }
                    if page == request.page
                        && let PageAction::Share { mode, .. } = request.action
                    {
                        authority.policy.link_mode = mode == ShareMode::Link;
                        // The final share statement publishes the requested mode/key once.
                        authority.policy.public_mode = false;
                    }
                    let (s, w) = prepared.commit(tx, key, &authority, now)?;
                    statements.extend(s);
                    wraps.extend(w);
                }
                let payload = if let PageAction::Share { mode, .. } = request.action {
                    share_payload(
                        tx,
                        request.page,
                        mode,
                        plan.authority.policy.history_current,
                    )?
                } else {
                    selected.clone()
                };
                let statement =
                    key.sign_statement(tx.head(), operation, &serde_json::to_vec(&payload)?)?;
                tx.append_statement(&statement)?;
                statements.push(statement);
                if matches!(request.action, PageAction::Delete) {
                    tx.delete_page_data(request.page)?;
                }
                membership::outcome(&statements, wraps)
            },
        )
    }
}
fn plan(
    tx: &OwnerTransaction<'_>,
    key: &Keyring,
    request: &PageRequest<'_>,
    context: OwnerContext<'_>,
) -> Result<Plan> {
    context.check_scope(&[request.page.into()])?;
    if matches!(
        request.action,
        PageAction::Share {
            mode: ShareMode::Public,
            publication: Publication::Cloud
        }
    ) {
        return Err(OwnerFault::WrongOwner.into());
    }
    let log = tx.log()?;
    let (states, _) = fold::verify_log(&log, key, request.page)?;
    let authority = states.last().ok_or(OwnerFault::Invalid)?.clone();
    if tx.page_epoch(request.page)?.is_none() {
        return Err(OwnerFault::Invalid.into());
    }
    if authority.policy.deleted
        || (authority.policy.archived
            && !matches!(
                request.action,
                PageAction::Delete | PageAction::Retention(_)
            ))
    {
        return Err(OwnerFault::WrongOwner.into());
    }
    let mut links = Vec::new();
    let mut rotations = BTreeSet::new();
    if let PageAction::Share { mode, .. } = request.action {
        let old = if authority.policy.public_mode {
            ShareMode::Public
        } else if authority.policy.link_mode {
            ShareMode::Link
        } else {
            ShareMode::Private
        };
        let narrowing = matches!(
            (old, mode),
            (ShareMode::Link, ShareMode::Private)
                | (ShareMode::Public, ShareMode::Private | ShareMode::Link)
        );
        if mode == ShareMode::Public || narrowing {
            rotations.insert(request.page.into());
        }
        if narrowing {
            for issuer in authority.recipients.values().filter(|i| {
                i.recipient.kind == "link"
                    && !i.recipient.revoked
                    && i.recipient.pages.iter().any(|p| p == request.page)
            }) {
                links.push(issuer.recipient.clone());
                for page in &issuer.recipient.pages {
                    let (states, _) = fold::verify_log(&log, key, page)?;
                    if states.last().is_some_and(|a| a.policy.writable()) {
                        rotations.insert(page.clone());
                    }
                }
            }
        }
    }
    if rotations.len() > 256 {
        return Err(OwnerFault::Capacity.into());
    }
    let mut devices = links
        .iter()
        .map(|r| (r.id.clone(), BTreeSet::new()))
        .collect::<BTreeMap<_, _>>();
    let known = tx.devices()?;
    for device in &known {
        let chain = certificate::Chain::from_json(&device.chain)?;
        let c = chain.certificate()?;
        if c.issuer_kind == "link"
            && let Some(ids) = devices.get_mut(c.issuer_id)
        {
            fold::verify_chain(
                &chain,
                authority
                    .recipients
                    .get(&("link".into(), c.issuer_id.into()))
                    .ok_or(OwnerFault::Invalid)?,
                key,
                authority.head.revision,
            )?;
            ids.insert(c.device_id.into());
        }
    }
    Ok(Plan {
        authority,
        catalog: tx.pages()?,
        device_digest: crypto::digest(&serde_json::to_vec(&known)?),
        links,
        devices,
        rotations,
    })
}
pub(super) fn share_payload(
    tx: &OwnerTransaction<'_>,
    page: &str,
    mode: ShareMode,
    current: bool,
) -> Result<Value> {
    let epoch = tx.current_epoch(page)?;
    let mut keys = Vec::new();
    if mode == ShareMode::Public {
        let secrets = if current {
            vec![(
                epoch,
                tx.epoch_secret(page, epoch)?.ok_or(OwnerFault::Invalid)?,
            )]
        } else {
            tx.recent_secrets(page, epoch)?
        };
        for (epoch, mut secret) in secrets {
            keys.push(json!({"epoch":epoch.to_string(),"key":values::encode_binary(&secret)}));
            secret.fill(0);
        }
    }
    Ok(json!({"pageId":page,"mode":mode.text(),"epoch":epoch.to_string(),"publishedKeys":keys}))
}

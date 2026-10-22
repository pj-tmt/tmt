//! Owner-browser extension-key registration. Remote authenticates the device;
//! this boundary verifies both extension-key certificates before any signing.
use crate::{
    Result,
    keyring::Keyring,
    store::{
        Store,
        owner::{Device, Mutation, OwnerTransaction},
    },
    transitions::Engine,
};
use serde::{Deserialize, Serialize};
use tmt_colab_model::{certificate, crypto, framing, statement, values};

pub const PATH: &str = "/api/devices/register";
pub const FRESHNESS_MS: u64 = 10 * 60 * 1000;
pub const CERTIFICATE_MS: u64 = 365 * 24 * 60 * 60 * 1000;
pub const RENEWAL_MS: u64 = 30 * 24 * 60 * 60 * 1000;
const GENESIS: &str = "00000000-0000-4000-8000-000000000001";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Code {
    Invalid,
    Denied,
    Expired,
    Conflict,
    Unavailable,
    Capacity,
}
impl Code {
    pub fn status(self) -> u16 {
        match self {
            Self::Invalid => 400,
            Self::Denied | Self::Expired => 403,
            Self::Conflict => 409,
            Self::Unavailable | Self::Capacity => 503,
        }
    }
    pub fn text(self) -> &'static str {
        match self {
            Self::Invalid => "INVALID",
            Self::Denied => "DENIED",
            Self::Expired => "EXPIRED",
            Self::Conflict => "CONFLICT",
            Self::Unavailable => "UNAVAILABLE",
            Self::Capacity => "CAPACITY",
        }
    }
}
impl std::fmt::Display for Code {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.text())
    }
}
impl std::error::Error for Code {}
impl From<tmt_colab_model::Invalid> for Code {
    fn from(_: tmt_colab_model::Invalid) -> Self {
        Self::Invalid
    }
}

/// Trusted only when read from remote's header on the owner-only mount socket.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Context {
    device_id: String,
    kind: String,
    origin: String,
    name: String,
    public_key: String,
    owner: bool,
    grant_revision: u64,
}
impl Context {
    fn parse(raw: Option<&str>) -> std::result::Result<Self, Code> {
        let c: Self = serde_json::from_str(raw.ok_or(Code::Denied)?).map_err(|_| Code::Denied)?;
        if !c.owner
            || c.grant_revision == 0
            || !matches!(c.kind.as_str(), "browser" | "addon" | "cli")
        {
            return Err(Code::Denied);
        }
        values::generated_id(&c.device_id).map_err(|_| Code::Denied)?;
        c.key()?;
        Ok(c)
    }
    fn key(&self) -> std::result::Result<[u8; 32], Code> {
        let key = binary::<32>(&self.public_key).map_err(|_| Code::Denied)?;
        crypto::public_key(&key).map_err(|_| Code::Denied)?;
        Ok(key)
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Request {
    device_id: String,
    sign: KeyCertificate,
    enc: KeyCertificate,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct KeyCertificate {
    public_key: String,
    issued_at_ms: u64,
    signature: String,
}
fn binary<const N: usize>(text: &str) -> std::result::Result<[u8; N], Code> {
    values::binary(text, N)?
        .try_into()
        .map_err(|_| Code::Invalid)
}
/// Remote owns this layout (canonical::ext_cert); the shared model LP primitive
/// implements its byte rules without importing a Remote behavior dependency.
fn ext_cert(purpose: &str, key: &[u8; 32], issued: u64) -> std::result::Result<Vec<u8>, Code> {
    values::time(issued)?;
    Ok(framing::frame(&[
        b"tmt-ext-cert-v1",
        b"colab",
        purpose.as_bytes(),
        key,
        issued.to_string().as_bytes(),
    ])?)
}
impl KeyCertificate {
    fn verify(
        &self,
        purpose: &str,
        context: &[u8; 32],
        now: u64,
    ) -> std::result::Result<[u8; 32], Code> {
        let key = binary(&self.public_key)?;
        let signature = binary::<64>(&self.signature)?;
        let input = ext_cert(purpose, &key, self.issued_at_ms)?;
        if self.issued_at_ms > now || now - self.issued_at_ms > FRESHNESS_MS {
            return Err(Code::Expired);
        }
        crypto::verify_signature(context, &input, &signature).map_err(|_| Code::Denied)?;
        if purpose == "sign" {
            crypto::public_key(&key)?;
        }
        if purpose == "enc" && key == [0; 32] {
            return Err(Code::Invalid);
        }
        Ok(key)
    }
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DeviceRegistration {
    context: Context,
    signing: [u8; 32],
    encryption: [u8; 32],
    outcome: Vec<u8>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Revocation {
    Unchanged,
    Tombstoned,
    MembershipChanged,
}

/// What one browser catalog read holds, read in one store transaction.
struct CatalogRead {
    catalog: serde_json::Value,
    fingerprints: std::collections::BTreeMap<String, String>,
    /// The local titles still valid for their page (`page_titles`).
    titles: std::collections::BTreeMap<String, String>,
}

pub struct Registration {
    store: Store,
    keyring: Keyring,
    engine: Engine,
    readers: crate::readers::Sessions,
    reader_clock: std::sync::Arc<dyn Fn() -> std::result::Result<u64, Code> + Send + Sync>,
    save_source: Option<crate::page::save::SourceOpener>,
    /// The owner list's display-only page summaries; filled in the background, never signed.
    summaries: std::sync::Arc<crate::summary::SummaryCache>,
    /// How the background refresh opens its own read-only source; without it no summary is made.
    summary_source: Option<crate::page::save::SourceOpener>,
}
impl Registration {
    pub fn new(
        store: Store,
        keyring: Keyring,
        decoder_program: std::path::PathBuf,
    ) -> Result<Self> {
        Self::with_decoder_config(store, keyring, crate::decoder::Config::new(decoder_program))
    }
    pub fn with_decoder_config(
        store: Store,
        keyring: Keyring,
        decoder_config: crate::decoder::Config,
    ) -> Result<Self> {
        Ok(Self {
            store,
            keyring,
            engine: Engine::with_decoder_config(decoder_config)?,
            readers: Default::default(),
            reader_clock: std::sync::Arc::new(now_ms),
            save_source: None,
            summaries: Default::default(),
            summary_source: None,
        })
    }
    /// The page's current epoch, to name it when the cloud worker is asked to look at it again.
    pub fn page_epoch(&self, page: &str) -> Option<u64> {
        self.store
            .owner_read(&self.keyring.space_id, &self.keyring.owner_public(), |tx| {
                tx.page_epoch(page)
            })
            .ok()?
            .and_then(|epoch| epoch.parse().ok())
    }
    /// Server-owned reader clock, injected for deterministic expiry verification.
    pub fn with_reader_clock(
        mut self,
        clock: impl Fn() -> std::result::Result<u64, Code> + Send + Sync + 'static,
    ) -> Self {
        self.reader_clock = std::sync::Arc::new(clock);
        self
    }
    pub(crate) fn reader_request(
        &mut self,
        path: &str,
        body: &[u8],
    ) -> std::result::Result<Vec<u8>, Code> {
        let now = (self.reader_clock)()?;
        if path == crate::readers::CHALLENGE_PATH {
            self.readers
                .challenge(&self.store, &self.keyring, body, now)
        } else {
            self.readers
                .exchange(&mut self.store, &self.keyring, body, now)
        }
    }
    pub(crate) fn reader_upgrade(
        &mut self,
        token: &[u8; 32],
    ) -> std::result::Result<(String, String), Code> {
        self.readers
            .upgrade(&self.store, &self.keyring, token, (self.reader_clock)()?)
    }
    pub(crate) fn release_reader(&mut self, id: &str) {
        self.readers.release(id);
    }
    pub fn close(self) -> Result<()> {
        self.store.close()
    }
    /// Read-only owner discovery also works before extension-key registration.
    pub fn session(context: Option<&str>) -> std::result::Result<Vec<u8>, Code> {
        let c = Context::parse(context)?;
        serde_json::to_vec(
            &serde_json::json!({"deviceId":c.device_id,"publicKey":c.public_key,
            "grantRevision":c.grant_revision.to_string(),"name":c.name}),
        )
        .map_err(|_| Code::Unavailable)
    }
    /// The owner's page list. Each page carries the summary the cache holds for its current
    /// fingerprint, and pages without one are scheduled for a bounded background refresh: this
    /// read replies at once with whatever is fresh and never waits for the fold. A page's work and
    /// its parent are resolved here across the fresh pages, never cached: they change when another
    /// page does.
    pub fn pages(&self, context: Option<&str>) -> std::result::Result<Vec<u8>, Code> {
        let CatalogRead {
            mut catalog,
            fingerprints,
            titles,
        } = self.page_catalog_fingerprinted(context)?;
        let fresh: std::collections::BTreeMap<String, crate::summary::Summary> = catalog["pages"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|page| {
                let id = page["pageId"].as_str()?;
                let summary = self.summaries.get(id, fingerprints.get(id)?)?;
                Some((id.to_owned(), summary))
            })
            .collect();
        let found: crate::work_index::Found = fresh
            .iter()
            .map(|(id, summary)| (id.clone(), summary.work.as_deref().cloned()))
            .collect();
        let graph = crate::work_links::Graph(&found);
        let parents = graph.parents();
        for page in catalog["pages"].as_array_mut().into_iter().flatten() {
            let Some(id) = page["pageId"].as_str().map(str::to_owned) else {
                continue;
            };
            let Some(summary) = fresh.get(&id) else {
                continue;
            };
            page["summary"] = summary.json();
            if let Some(work) = graph.row(&id) {
                page["summary"]["work"] = work;
            }
            if let Some((parent, task)) = parents.get(&id) {
                let mut value = serde_json::json!({"page": parent, "task": task});
                // Additive: the parent task's own title, so the crumb names it without reading
                // the parent page.
                if let Some(title) = graph.task_title(parent, task) {
                    value["title"] = serde_json::json!(title);
                }
                page["summary"]["parent"] = value;
            }
        }
        if let Some(open) = &self.summary_source {
            // Every page without a fresh summary is folded in the background, newest first, so the
            // next read has more of them; this read never waits for it.
            let order = crate::summary::recent_first(&catalog, &fingerprints);
            if !self.summaries.stale(&order, 1).is_empty() {
                crate::summary::drain(&self.summaries, open);
            }
        }
        crate::page_titles::attach_to_catalog(&mut catalog, &titles);
        serde_json::to_vec(&catalog).map_err(|_| Code::Unavailable)
    }
    /// Complete cached chapter metadata. The owner read checks authority and the current
    /// fingerprint without subscribing, touching activity or decoding on this request path.
    pub fn page_summary_details(
        &self,
        context: Option<&str>,
        page: &str,
    ) -> std::result::Result<Vec<u8>, Code> {
        Context::parse(context)?;
        values::generated_id(page).map_err(|_| Code::Invalid)?;
        let fingerprint = self
            .store
            .owner_read(&self.keyring.space_id, &self.keyring.owner_public(), |tx| {
                tx.page_fingerprint(page)
            })
            .map_err(map_error)?
            .ok_or(Code::Denied)?;
        let value = match self.summaries.details(page, &fingerprint) {
            crate::summary::CachedDetails::Ready(projection) => {
                let digest: String = crypto::digest(fingerprint.as_bytes())
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect();
                let mut ready = serde_json::json!({"pageId":page,"state":"ready","fingerprint":digest,
                    "summary":projection.summary.json(),"readings":projection.readings,
                    "needs":projection.needs});
                if let Some(latest) = &projection.latest {
                    ready["latest"] = serde_json::json!(latest);
                }
                ready
            }
            crate::summary::CachedDetails::Unavailable => {
                serde_json::json!({"pageId":page,"state":"unavailable"})
            }
            crate::summary::CachedDetails::Pending if self.summary_source.is_some() => {
                // Do not call stale() on a single-page map: it also prunes the full cache.
                crate::summary::refresh(
                    &self.summaries,
                    self.summary_source.as_ref().expect("source checked"),
                    vec![(page.to_owned(), fingerprint)],
                );
                serde_json::json!({"pageId":page,"state":"pending"})
            }
            crate::summary::CachedDetails::Pending => {
                serde_json::json!({"pageId":page,"state":"unavailable"})
            }
        };
        let bytes = serde_json::to_vec(&value).map_err(|_| Code::Unavailable)?;
        if bytes.len() > crate::limits::SUMMARY_DETAILS_BYTES {
            return serde_json::to_vec(&serde_json::json!({"pageId":page,"state":"unavailable"}))
                .map_err(|_| Code::Unavailable);
        }
        Ok(bytes)
    }
    /// The browser's catalog read: the page list, each page's fingerprint and the local titles still
    /// valid for their page, all in one store transaction.
    fn page_catalog_fingerprinted(
        &self,
        context: Option<&str>,
    ) -> std::result::Result<CatalogRead, Code> {
        Context::parse(context)?;
        self.store
            .owner_read(&self.keyring.space_id, &self.keyring.owner_public(), |tx| {
                Ok(CatalogRead {
                    catalog: tx.page_list()?,
                    fingerprints: tx.page_fingerprints()?,
                    titles: tx.page_titles()?,
                })
            })
            .map_err(map_error)
    }
    fn owner_catalog_fingerprinted(
        &self,
    ) -> std::result::Result<
        (
            serde_json::Value,
            std::collections::BTreeMap<String, String>,
        ),
        Code,
    > {
        self.store
            .owner_read(&self.keyring.space_id, &self.keyring.owner_public(), |tx| {
                Ok((tx.page_list()?, tx.page_fingerprints()?))
            })
            .map_err(map_error)
    }
    /// Display-only owner-socket callback. Unlike pages(), this never schedules a refresh.
    pub(crate) fn describe(&self) -> std::result::Result<Vec<u8>, Code> {
        let (catalog, fingerprints) = self.owner_catalog_fingerprinted()?;
        let pages: Vec<_> = catalog["pages"]
            .as_array()
            .ok_or(Code::Unavailable)?
            .iter()
            .filter(|page| page["archived"] != true)
            .collect();
        if pages.len() > crate::limits::DESCRIBE_COUNT {
            return Err(Code::Capacity);
        }
        // Match the page list's waitingCount: one waiting page, regardless of its item count.
        // Its count cannot exceed the already-bounded active page count.
        let needs_you = pages.iter().try_fold(0usize, |count, page| {
            let id = page["pageId"].as_str()?;
            let fingerprint = fingerprints.get(id)?;
            let summary = self.summaries.get(id, fingerprint)?;
            Some(count + usize::from(summary.waiting))
        });
        let mut answer = serde_json::json!({
            "version":1,
            "rail":{"label":"Colab","icon":"file-text","path":"/","order":100},
            "menu":[{"id":"pages","label":"Pages","path":"/","count":{"value":pages.len(),"kind":"plain"}}],
            "settings":[]
        });
        if let Some(needs_you) = needs_you {
            answer["needsYou"] = serde_json::json!(needs_you);
        }
        serde_json::to_vec(&answer).map_err(|_| Code::Unavailable)
    }
    /// The caller serializes management with sync before taking this mutex.
    pub fn apply_owner(
        &mut self,
        request: crate::transitions::OwnerRequest<'_>,
        now: u64,
    ) -> std::result::Result<crate::transitions::Applied, crate::transitions::TransitionError> {
        self.engine
            .apply(&mut self.store, &self.keyring, request, now)
    }
    /// Display-only project writes: the caller serializes management with sync before this mutex.
    pub fn set_projects(
        &mut self,
        changes: &[(String, Vec<String>)],
    ) -> std::result::Result<tmt_colab_model::statement::Head, crate::transitions::TransitionError>
    {
        self.engine
            .set_projects(&mut self.store, &self.keyring, changes)
    }
    pub fn set_pinned(
        &mut self,
        page: &str,
        pinned: bool,
    ) -> std::result::Result<tmt_colab_model::statement::Head, crate::transitions::TransitionError>
    {
        self.engine
            .set_pinned(&mut self.store, &self.keyring, page, pinned)
    }
    pub fn manage_project(
        &mut self,
        change: &crate::projects::Change,
    ) -> std::result::Result<tmt_colab_model::statement::Head, crate::transitions::TransitionError>
    {
        self.engine
            .manage_project(&mut self.store, &self.keyring, change)
    }
    /// The public key that must have signed a root-local batch; never request-selected.
    pub(crate) fn author_key(&self) -> crate::Result<[u8; 32]> {
        Ok(self.keyring.local_writer()?.1)
    }
    /// Root-local sealed batch commit; caller holds the sync mutex before this service.
    pub(crate) fn publish(
        &mut self,
        job: &crate::publication::SignedJob,
        packet: &[u8],
        chain: &[u8],
        now: u64,
        combine_until: std::time::Instant,
    ) -> crate::Result<crate::sync::Commit> {
        let committed = crate::page::commit_publication(
            &mut self.store,
            &self.keyring,
            job,
            packet,
            chain,
            now,
        )?;
        let mut combined = None;
        if committed.accepted == crate::store::Accepted::New
            && matches!(
                committed.record.outcome,
                crate::publication::Outcome::Committed { .. }
            )
        {
            // Combining this device's own tail is best effort: the write is already durable, and
            // the next write tries again.
            let page = &job.manifest.page_id;
            combined = self
                .engine
                .decoder(page)
                .and_then(|decoder| {
                    crate::page::compact::compact(
                        &mut self.store,
                        &self.keyring,
                        page,
                        decoder,
                        crate::page::compact::Trigger {
                            until: Some(combine_until),
                            ..Default::default()
                        },
                    )
                })
                .ok()
                .flatten()
                .zip(self.keyring.local_writer().ok())
                .map(|(done, (stream, ..))| crate::sync::Combined {
                    stream,
                    through: done.through,
                });
        }
        // The sync mutex is held, so no other writer can move the page between the combine and
        // this read: the revision is the one this write's reply reports.
        let revision = matches!(
            committed.record.outcome,
            crate::publication::Outcome::Committed { .. }
        )
        .then(|| crate::page::revision(&self.store, &self.keyring, &job.manifest.page_id).ok())
        .flatten();
        Ok(crate::sync::Commit {
            committed,
            revision,
            combined,
        })
    }
    /// A requested combine of this device's own stream, reported to whoever asked. The caller
    /// holds the sync mutex first, so no write moves the page between its reads and its combine.
    pub(crate) fn compact(
        &mut self,
        page: &str,
        until: std::time::Instant,
    ) -> crate::Result<crate::page::compact::Report> {
        let decoder = self.engine.decoder(page)?;
        crate::page::compact::request(&mut self.store, &self.keyring, page, decoder, Some(until))
    }
    /// How full the page is. The caller holds the sync mutex first.
    pub(crate) fn usage(&mut self, page: &str) -> crate::Result<crate::page::PageUsage> {
        let decoder = self.engine.decoder(page)?;
        crate::page::usage(&self.store, &self.keyring, page, decoder)
    }
    /// What the root-local stream recorded for an operation a browser Save chose. Read-only. A
    /// convert operation also names the page it made, if that page exists, so a lost reply says
    /// both whether the page was made and whether the link landed.
    pub(crate) fn save_status(
        &self,
        page: &str,
        operation_id: &str,
    ) -> crate::Result<crate::page::save::SaveResult> {
        let recorded = crate::page::save::SaveResult::recorded(
            operation_id,
            crate::page::publication_status_by_operation(
                &self.store,
                &self.keyring,
                page,
                operation_id,
            )?,
        )?;
        let child = crate::work_convert::child_page_id(&self.keyring.space_id, operation_id);
        Ok(recorded.with_child(self.page_epoch(&child).map(|_| child)))
    }
    /// Make the page a browser's convert operation names, as the owner's own `page.create`: a
    /// column page with this title and first text, no publisher agent and nothing the browser
    /// chose. The caller holds the sync lock and has authorized the owner device.
    pub(crate) fn create_child(
        &mut self,
        page: &str,
        operation_id: &str,
        title: &str,
        source: &str,
        now: u64,
    ) -> crate::Result<()> {
        let space = self.keyring.space_id.clone();
        let revision = self
            .store
            .owner_head(&space, &self.keyring.owner_public())?
            .map_or(0, |head| head.revision)
            .to_string();
        let payload = serde_json::json!({
            "pageId": page, "title": title, "source": source,
            "layout": crate::decoder::PageLayout {
                kind: crate::decoder::LayoutKind::Column,
                reason: None,
            },
        });
        let body = serde_json::to_vec(&serde_json::json!({
            "space": space, "page": page, "expectedRevision": revision,
            "operationId": crate::work_convert::create_operation_id(&space, operation_id),
            "operation": "page.create",
            "payload": tmt_colab_model::values::encode_binary(&serde_json::to_vec(&payload)?),
        }))?;
        crate::management::local(self, &space, &body, now)
            .map(|_| ())
            .map_err(|code| {
                crate::work_admission::refused(
                    match code {
                        crate::management::Code::Capacity => "COLAB_CAPACITY",
                        _ => "COLAB_UNAVAILABLE",
                    },
                    "The page for this task could not be made.",
                )
            })
    }
    /// Where a browser Save gets a snapshot of its own to prepare from, outside every lock.
    pub(crate) fn save_source(&self) -> Option<crate::page::save::SourceOpener> {
        self.save_source.clone()
    }
    /// The serve supplies how to open a fresh read-only store, keyring and decoder, so that a
    /// large preparation never holds the writer or the sync mutex.
    /// The source the page summary refresh reads through; the serve supplies it, so a registration
    /// built without one (a test, an offline tool) lists pages without summaries.
    pub fn with_summary_source(mut self, open: crate::page::save::SourceOpener) -> Self {
        self.summary_source = Some(open);
        self
    }
    /// Folds every page that has no fresh summary or title yet, once, on the background worker: the
    /// first list read does the same, so the serve starts it, and a store that gained its title
    /// projection by an upgrade fills without anyone opening the list. Never blocks; without a
    /// summary source (a test, an offline tool) it does nothing.
    pub fn start_summaries(&self) {
        if let Some(open) = &self.summary_source {
            crate::summary::drain(&self.summaries, open);
        }
    }
    pub fn with_save_source(mut self, open: crate::page::save::SourceOpener) -> Self {
        self.save_source = Some(open);
        self
    }
    pub(crate) fn management_device(
        &mut self,
        context: Option<&str>,
        now: u64,
    ) -> std::result::Result<(String, [u8; 32]), Code> {
        let device = Context::parse(context)?.device_id;
        let signing = self.active_device(context, now)?;
        Ok((device, signing))
    }

    /// The caller supplies server time, never a request-selected clock.
    pub fn register(
        &mut self,
        context: Option<&str>,
        body: &[u8],
        now: u64,
    ) -> std::result::Result<Vec<u8>, Code> {
        values::time(now)?;
        if body.len() > crate::limits::HTTP_BODY_BYTES {
            return Err(Code::Invalid);
        }
        let context = Context::parse(context)?;
        let request: Request = serde_json::from_slice(body).map_err(|_| Code::Invalid)?;
        values::generated_id(&request.device_id)?;
        if request.device_id != context.device_id {
            return Err(Code::Denied);
        }
        let remote_key = context.key()?;
        let signing = request.sign.verify("sign", &remote_key, now)?;
        let encryption = request.enc.verify("enc", &remote_key, now)?;
        self.genesis().map_err(map_error)?;
        let keyring = &self.keyring;
        let apply = |tx: &mut OwnerTransaction<'_>| -> Result<Vec<u8>> {
            let (initial, member, current_revision) = issuer(tx, keyring)?;
            let old_device = tx.device(&context.device_id)?;
            if old_device.as_ref().is_some_and(|d| d.revoked) {
                return Err(Code::Denied.into());
            }
            if let Some(row) = tx.registration(&context.device_id)? {
                if row.revoked || context.grant_revision < row.grant_revision {
                    return Err(Code::Denied.into());
                }
                let mut saved: DeviceRegistration =
                    serde_json::from_slice(&row.binding.ok_or(Code::Unavailable)?)?;
                if saved.context.public_key != context.public_key
                    || saved.signing != signing
                    || saved.encryption != encryption
                {
                    return Err(Code::Conflict.into());
                }
                let device = old_device.as_ref().ok_or(Code::Unavailable)?;
                let chain = certificate::Chain::from_json(&device.chain)?;
                let cert = chain.certificate()?;
                verify_device(
                    &chain,
                    &cert,
                    &initial,
                    &member,
                    &saved,
                    current_revision,
                    &keyring.space_id,
                )?;
                if now < cert.issued_at {
                    return Err(Code::Expired.into());
                }
                if cert.membership_revision == "1"
                    && cert.expires_at.saturating_sub(now) >= RENEWAL_MS
                {
                    saved.context = context;
                    tx.put_registration(
                        &saved.context.device_id,
                        &serde_json::to_vec(&saved)?,
                        saved.context.grant_revision,
                    )?;
                    forward_owner_wraps(tx, keyring, &saved.context.device_id, &encryption)?;
                    return Ok(saved.outcome);
                }
            } else if old_device.is_some() {
                return Err(Code::Conflict.into());
            }
            let cert = certificate::Certificate {
                space: &keyring.space_id,
                issuer_kind: "member",
                issuer_id: &member.id,
                device_id: &context.device_id,
                signing_key: &signing,
                encryption_key: &encryption,
                // The chain resolves the management member's revision-1 member.add,
                // while current_revision still fences live admission above.
                membership_revision: "1",
                issued_at: now,
                expires_at: now.checked_add(CERTIFICATE_MS).ok_or(Code::Invalid)?,
            };
            let chain = serde_json::json!({
                "version":1,
                "issuerStatement":values::encode_binary(&initial.hash()?),
                "deviceCertificate":values::encode_binary(&certificate::input(&cert)?),
                "issuerSignature":values::encode_binary(&keyring.sign_device_certificate(&cert)?)
            });
            let chain_bytes = serde_json::to_vec(&chain)?;
            certificate::Chain::from_json(&chain_bytes)?.verify(
                &initial.hash()?,
                &cert,
                &member.signing_key,
            )?;
            let statement = serde_json::from_slice::<serde_json::Value>(&initial.to_json()?)?;
            let outcome = serde_json::to_vec(
                &serde_json::json!({"chain":chain,"issuerStatement":statement}),
            )?;
            tx.put_device(&Device {
                chain: chain_bytes,
                revoked: false,
            })?;
            let binding = DeviceRegistration {
                context,
                signing,
                encryption,
                outcome: outcome.clone(),
            };
            tx.put_registration(
                &binding.context.device_id,
                &serde_json::to_vec(&binding)?,
                binding.context.grant_revision,
            )?;
            forward_owner_wraps(tx, keyring, &binding.context.device_id, &encryption)?;
            Ok(outcome)
        };
        self.store
            .device_transaction(&keyring.space_id, &keyring.owner_public(), apply)
            .map_err(map_error)
    }
    fn genesis(&mut self) -> Result<()> {
        let key = &self.keyring;
        if self
            .store
            .owner_head(&key.space_id, &key.owner_public())?
            .is_some()
        {
            return Ok(());
        }
        let (_, payload) = crate::transitions::owner_genesis(key)?;
        self.store.owner_transaction(
            &key.space_id,
            &key.owner_public(),
            Mutation {
                operation_id: GENESIS,
                digest: crypto::digest(&payload),
                expected_revision: 0,
            },
            |tx| {
                Ok(crate::transitions::initialize_owner(tx, key)?
                    .ok_or(Code::Unavailable)?
                    .to_json()?)
            },
        )?;
        Ok(())
    }
    /// Trusted remote consumer: known devices commit cuts/rotation and a tombstone;
    /// unknown IDs only tombstone. Equal/older or already-revoked events return false.
    pub fn revoke(&mut self, device_id: &str, grant_revision: u64) -> Result<bool> {
        self.revoke_event(device_id, grant_revision)
            .map(|effect| effect != Revocation::Unchanged)
    }
    /// Preserve the distinction between a registration tombstone and an owner transition:
    /// only the latter can advance page epochs and leave attachments waiting for rekey.
    pub(crate) fn revoke_event(
        &mut self,
        device_id: &str,
        grant_revision: u64,
    ) -> Result<Revocation> {
        values::generated_id(device_id)?;
        if grant_revision == 0 {
            return Err(crate::store::owner::OwnerFault::Invalid.into());
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis()
            .try_into()?;
        for _ in 0..3 {
            let (known, noop) = self.store.owner_read(
                &self.keyring.space_id,
                &self.keyring.owner_public(),
                |tx| {
                    let device = tx.device(device_id)?;
                    let row = tx.registration(device_id)?;
                    Ok((
                        device.is_some(),
                        device.is_some_and(|d| d.revoked)
                            || row.is_some_and(|r| r.revoked || r.grant_revision >= grant_revision),
                    ))
                },
            )?;
            if noop {
                return Ok(Revocation::Unchanged);
            }
            if known {
                return self
                    .engine
                    .remote_revoke(
                        &mut self.store,
                        &self.keyring,
                        device_id,
                        grant_revision,
                        now,
                    )
                    .map(|changed| {
                        if changed {
                            Revocation::MembershipChanged
                        } else {
                            Revocation::Unchanged
                        }
                    });
            }
            match self.store.revoke_remote_device(
                &self.keyring.space_id,
                &self.keyring.owner_public(),
                device_id,
                grant_revision,
            ) {
                Err(e)
                    if e.downcast_ref::<crate::store::owner::OwnerFault>()
                        == Some(&crate::store::owner::OwnerFault::StaleHead) =>
                {
                    continue;
                }
                other => {
                    return other.map(|changed| {
                        if changed {
                            Revocation::Tombstoned
                        } else {
                            Revocation::Unchanged
                        }
                    });
                }
            }
        }
        Err(crate::store::owner::OwnerFault::StaleHead.into())
    }
    /// Admission for socket/sync composition, checked against durable
    /// revocation every time. A forwarded cookie/session alone is insufficient.
    pub fn active_device(
        &mut self,
        context: Option<&str>,
        now: u64,
    ) -> std::result::Result<[u8; 32], Code> {
        values::time(now)?;
        let context = Context::parse(context)?;
        let key = &self.keyring;
        if self
            .store
            .owner_head(&key.space_id, &key.owner_public())
            .map_err(map_error)?
            .is_none()
        {
            return Err(Code::Denied);
        }
        let bytes = self
            .store
            .device_transaction(&key.space_id, &key.owner_public(), |tx| {
                let row = tx.registration(&context.device_id)?.ok_or(Code::Denied)?;
                if row.revoked || context.grant_revision < row.grant_revision {
                    return Err(Code::Denied.into());
                }
                let mut binding: DeviceRegistration =
                    serde_json::from_slice(&row.binding.ok_or(Code::Denied)?)?;
                if binding.context.public_key != context.public_key {
                    return Err(Code::Denied.into());
                }
                let device = tx.device(&context.device_id)?.ok_or(Code::Denied)?;
                if device.revoked {
                    return Err(Code::Denied.into());
                }
                let chain = certificate::Chain::from_json(&device.chain)?;
                let cert = chain.certificate()?;
                let (initial, member, revision) = issuer(tx, key)?;
                verify_device(
                    &chain,
                    &cert,
                    &initial,
                    &member,
                    &binding,
                    revision,
                    &key.space_id,
                )?;
                if now < cert.issued_at || now >= cert.expires_at {
                    return Err(Code::Expired.into());
                }
                if context.grant_revision > row.grant_revision {
                    binding.context = context;
                    tx.put_registration(
                        &binding.context.device_id,
                        &serde_json::to_vec(&binding)?,
                        binding.context.grant_revision,
                    )?;
                }
                Ok(binding.signing.to_vec())
            })
            .map_err(map_error)?;
        bytes.try_into().map_err(|_| Code::Unavailable)
    }
}
/// Mounted owner and read-only reader admission. Upgrade verifies owner binding
/// through active_device or consumes a reader ticket; sync reads live authority.
/// The pinned local management member represents the owner across local pages.
#[derive(Clone)]
pub struct OwnerAdmission(pub std::sync::Arc<std::sync::Mutex<Registration>>);
impl OwnerAdmission {
    /// The socket supplies this forwarded context only for its actual upgraded
    /// peer. This parses the existing Remote context; it does not grant Colab
    /// write permission or turn a reader ticket into an owner.
    pub(crate) fn object_remote_binding(raw: &str) -> Result<(String, u64)> {
        let context = Context::parse(Some(raw))?;
        Ok((context.device_id, context.grant_revision))
    }
    pub(crate) fn recheck_object_remote(&self, raw: &str) -> Result<()> {
        self.0
            .lock()
            .map_err(|_| crate::sync::Code::Denied)?
            .active_device(Some(raw), now_ms()?)?;
        Ok(())
    }
    /// Actual owner/reader Session and historical recipient entitlement. The
    /// private peer owner supplies the principal; policy JSON cannot select it.
    pub(crate) fn attachment_context(
        &self,
        principal: &str,
        scope: &crate::sync::SyncScope,
        epoch: u64,
    ) -> Result<[u8; 32]> {
        use crate::sync::{Access, Admission, WrapRecipients};
        self.authorize(principal, scope, Access::Read)?;
        let service = self.0.lock().map_err(|_| crate::sync::Code::Denied)?;
        let current = values::decimal(&scope.epoch, false)?;
        if epoch == 0 || epoch > current || current - epoch >= 64 {
            return Err(crate::sync::Code::Denied.into());
        }
        let reader = service.readers.contains(principal);
        let recipients = if reader {
            service.readers.recipients(principal)?
        } else {
            WrapRecipients::Owner
        };
        let reader_context = if reader {
            Some(service.readers.attachment_context(principal)?)
        } else {
            None
        };
        service
            .store
            .owner_read(&scope.space, &service.keyring.owner_public(), |tx| {
                let head = tx.head().ok_or(crate::sync::Code::Denied)?;
                if matches!(recipients, WrapRecipients::None) {
                    let (_, log) =
                        crate::fold::verify_log(&tx.log()?, &service.keyring, &scope.page)?;
                    let published = log
                        .iter()
                        .rev()
                        .find_map(|p| match p {
                            tmt_colab_model::payload::Payload::PageShare(p)
                                if p.page_id == scope.page =>
                            {
                                Some(p)
                            }
                            _ => None,
                        })
                        .ok_or(crate::sync::Code::Denied)?;
                    let key = published
                        .published_keys
                        .as_ref()
                        .and_then(|keys| {
                            keys.as_slice()
                                .iter()
                                .find(|k| k.epoch == epoch.to_string())
                        })
                        .ok_or(crate::sync::Code::Denied)?;
                    if !matches!(published.mode, tmt_colab_model::payload::ShareMode::Public)
                        || values::binary(&key.key, 32)?.as_slice()
                            != tx
                                .epoch_secret(&scope.page, epoch)?
                                .ok_or(crate::sync::Code::Denied)?
                    {
                        return Err(crate::sync::Code::Denied.into());
                    }
                } else {
                    let recipient = match &recipients {
                        WrapRecipients::Owner => ("device", principal),
                        WrapRecipients::Link(id) => ("link", id.as_str()),
                        WrapRecipients::None => unreachable!(),
                    };
                    let recipient_key = match &recipients {
                        WrapRecipients::Owner => {
                            *certificate::Chain::from_json(
                                &tx.device(principal)?
                                    .ok_or(crate::sync::Code::Denied)?
                                    .chain,
                            )?
                            .certificate()?
                            .encryption_key
                        }
                        WrapRecipients::Link(id) => {
                            tx.recipient("link", id)?
                                .ok_or(crate::sync::Code::Denied)?
                                .encryption_key
                        }
                        WrapRecipients::None => unreachable!(),
                    };
                    let mut offset = 0;
                    let mut entitled = false;
                    loop {
                        let (wraps, more) = tx.wrap_page(
                            &scope.page,
                            current,
                            principal,
                            head,
                            offset,
                            &recipients,
                        )?;
                        if wraps.is_empty() && more {
                            return Err(crate::sync::Code::Denied.into());
                        }
                        offset += wraps.len();
                        if offset > 512 {
                            return Err(crate::sync::Code::Capacity.into());
                        }
                        for raw in wraps {
                            let envelope = tmt_colab_model::wrap::Envelope::from_json(
                                &values::binary(&raw, 2048)?,
                            )?;
                            envelope.verify_owner(&service.keyring.owner_public())?;
                            let h = envelope.header()?;
                            if h.space == scope.space
                                && h.page == scope.page
                                && h.epoch == epoch.to_string()
                                && h.recipient_kind == recipient.0
                                && h.recipient_id == recipient.1
                                && h.recipient_key == recipient_key
                                && values::decimal(&h.membership_revision, false)? <= head.revision
                            {
                                entitled = true;
                            }
                        }
                        if !more {
                            break;
                        }
                    }
                    if !entitled {
                        return Err(crate::sync::Code::Denied.into());
                    }
                }
                if let Some(context) = reader_context {
                    return Ok(context);
                }
                let row = tx
                    .registration(principal)?
                    .ok_or(crate::sync::Code::Denied)?;
                let device = tx.device(principal)?.ok_or(crate::sync::Code::Denied)?;
                Ok(crypto::digest(&framing::frame(&[
                    b"tmt-colab-attachment-owner-context-v1",
                    principal.as_bytes(),
                    &serde_json::to_vec(scope)?,
                    row.grant_revision.to_string().as_bytes(),
                    row.binding.as_deref().ok_or(crate::sync::Code::Denied)?,
                    &device.chain,
                ])?))
            })
    }
}
impl crate::sync::Admission for OwnerAdmission {
    fn device(&self, principal: &str) -> String {
        self.0
            .lock()
            .ok()
            .and_then(|service| service.readers.device(principal))
            .unwrap_or_else(|| principal.to_owned())
    }
    fn authorize_link_save(
        &self,
        device: &str,
        scope: &crate::sync::SyncScope,
    ) -> std::result::Result<(), crate::sync::Code> {
        use crate::sync::Code as SyncCode;
        // Only the link's own answer is a verdict. A lock, a clock or a store that fails says
        // nothing about the link, so the request is not refused for it.
        let service = self.0.lock().map_err(|_| SyncCode::ResyncRequired)?;
        if scope.space != service.keyring.space_id {
            return Err(SyncCode::Denied);
        }
        let now = (service.reader_clock)().map_err(|_| SyncCode::ResyncRequired)?;
        crate::readers::device_writer(&service.store, &service.keyring, scope, device, now)
            .map_err(|code| match code {
                Code::Expired => SyncCode::Expired,
                Code::Denied | Code::Invalid => SyncCode::Denied,
                Code::Conflict | Code::Unavailable | Code::Capacity => SyncCode::ResyncRequired,
            })
            .and_then(|writer| writer.edits())
    }
    fn save_source(&self) -> Option<crate::page::save::SourceOpener> {
        self.0.lock().ok()?.save_source()
    }
    fn save_author(&self) -> crate::Result<[u8; 32]> {
        self.0
            .lock()
            .map_err(|_| crate::page::Fault::Unavailable)?
            .author_key()
    }
    fn commit_save(
        &mut self,
        job: &crate::publication::SignedJob,
        packet: &[u8],
        chain: &[u8],
        now: u64,
        combine_until: std::time::Instant,
    ) -> crate::Result<crate::sync::Commit> {
        self.0
            .lock()
            .map_err(|_| crate::page::Fault::Unavailable)?
            .publish(job, packet, chain, now, combine_until)
    }
    fn save_status(
        &self,
        page: &str,
        operation_id: &str,
    ) -> crate::Result<crate::page::save::SaveResult> {
        self.0
            .lock()
            .map_err(|_| crate::page::Fault::Unavailable)?
            .save_status(page, operation_id)
    }
    fn create_child(
        &mut self,
        page: &str,
        operation_id: &str,
        title: &str,
        source: &str,
        now: u64,
    ) -> crate::Result<()> {
        self.0
            .lock()
            .map_err(|_| crate::page::Fault::Unavailable)?
            .create_child(page, operation_id, title, source, now)
    }
    fn usage(&mut self, page: &str) -> crate::Result<crate::page::PageUsage> {
        self.0
            .lock()
            .map_err(|_| crate::page::Fault::Unavailable)?
            .usage(page)
    }
    fn compact(
        &mut self,
        page: &str,
        until: std::time::Instant,
    ) -> crate::Result<crate::page::compact::Report> {
        self.0
            .lock()
            .map_err(|_| crate::page::Fault::Unavailable)?
            .compact(page, until)
    }

    fn alive(&self, principal: &str) -> std::result::Result<(), crate::sync::Code> {
        let service = self.0.lock().map_err(|_| crate::sync::Code::Denied)?;
        if service.readers.contains(principal) {
            service
                .readers
                .check(
                    principal,
                    None,
                    &service.store,
                    &service.keyring,
                    (service.reader_clock)().map_err(|_| crate::sync::Code::Denied)?,
                )
                .map_err(reader_sync_error)?;
        }
        Ok(())
    }

    fn catchup_context(
        &self,
        principal: &str,
        scope: &crate::sync::SyncScope,
        store: &Store,
    ) -> std::result::Result<crate::sync::CatchupContext, crate::sync::Code> {
        let service = self.0.lock().map_err(|_| crate::sync::Code::Denied)?;
        service
            .store
            .owner_read(&scope.space, &service.keyring.owner_public(), |tx| {
                let head = tx.head().ok_or(crate::sync::Code::Denied)?;
                if tx.page_policy_at(&scope.page, head.revision)?.deleted {
                    return Err(crate::sync::Code::Denied.into());
                }
                Ok(())
            })
            .map_err(|error| sync_fault(error.as_ref()))?;
        Ok(crate::sync::CatchupContext {
            membership_head: store
                .owner_head(&scope.space, &service.keyring.owner_public())
                .map_err(|error| sync_fault(error.as_ref()))?
                .ok_or(crate::sync::Code::Denied)?,
            baseline: store
                .baseline(
                    &scope.page,
                    values::decimal(&scope.epoch, false).map_err(|_| crate::sync::Code::Denied)?,
                )
                .map_err(|error| sync_fault(error.as_ref()))?
                .map(|baseline| baseline.descriptor),
            owner_key: service.keyring.owner_public(),
            recipients: if service.readers.contains(principal) {
                service
                    .readers
                    .check(
                        principal,
                        Some(scope),
                        &service.store,
                        &service.keyring,
                        (service.reader_clock)().map_err(|_| crate::sync::Code::Denied)?,
                    )
                    .map_err(reader_sync_error)?;
                service
                    .readers
                    .recipients(principal)
                    .map_err(reader_sync_error)?
            } else {
                crate::sync::WrapRecipients::Owner
            },
        })
    }

    fn authorize(
        &self,
        principal: &str,
        scope: &crate::sync::SyncScope,
        access: crate::sync::Access<'_>,
    ) -> std::result::Result<[u8; 32], crate::sync::Code> {
        use crate::sync::Code as SyncCode;
        let service = self.0.lock().map_err(|_| SyncCode::Denied)?;
        if scope.space != service.keyring.space_id {
            return Err(SyncCode::Denied);
        }
        let now = now_ms().map_err(|_| SyncCode::Denied)?;
        if service.readers.contains(principal) {
            service
                .readers
                .check(
                    principal,
                    Some(scope),
                    &service.store,
                    &service.keyring,
                    (service.reader_clock)().map_err(|_| SyncCode::Denied)?,
                )
                .map_err(reader_sync_error)?;
            // A link session reads; it stores its own originals only while the link's role may
            // write them. Everything else a writer does (`Save`, `Publish`) is denied to every
            // link: an editor link asks for a text change with an `edit-request` record.
            return match access {
                crate::sync::Access::Read => Ok([0; 32]),
                crate::sync::Access::Save | crate::sync::Access::Publish => Err(SyncCode::Denied),
                access => service
                    .readers
                    .writer(
                        principal,
                        scope,
                        &service.store,
                        &service.keyring,
                        (service.reader_clock)().map_err(|_| SyncCode::Denied)?,
                    )
                    .map_err(reader_sync_error)
                    .and_then(|writer| writer.admit(&access)),
            };
        }
        service
            .store
            .owner_read(&scope.space, &service.keyring.owner_public(), |tx| {
                let row = tx.registration(principal)?.ok_or(SyncCode::Denied)?;
                if row.revoked {
                    return Err(SyncCode::Denied.into());
                }
                let binding: DeviceRegistration =
                    serde_json::from_slice(&row.binding.ok_or(SyncCode::Denied)?)?;
                let device = tx.device(principal)?.ok_or(SyncCode::Denied)?;
                if device.revoked {
                    return Err(SyncCode::Denied.into());
                }
                let chain = certificate::Chain::from_json(&device.chain)?;
                let cert = chain.certificate()?;
                let head = tx.head().ok_or(SyncCode::Denied)?;
                let member = &head.owner_member;
                let issuer = tx
                    .recipient("member", &member.id)?
                    .ok_or(SyncCode::Denied)?;
                if issuer.revoked
                    || issuer.role.as_deref() != Some("editor")
                    || issuer.signing_key != member.signing_key
                    || issuer.encryption_key != member.encryption_key
                    || cert.device_id != principal
                    || cert.space != scope.space
                    || cert.issuer_kind != "member"
                    || cert.issuer_id != member.id
                    || cert.signing_key != &binding.signing
                    || cert.encryption_key != &binding.encryption
                    || values::decimal(cert.membership_revision, false)? > head.revision
                {
                    return Err(SyncCode::Denied.into());
                }
                if now < cert.issued_at || now >= cert.expires_at {
                    return Err(SyncCode::Expired.into());
                }
                let policy = tx.page_policy_at(&scope.page, head.revision)?;
                if policy.deleted
                    || (policy.archived
                        && matches!(
                            access,
                            crate::sync::Access::Append(_)
                                | crate::sync::Access::Write
                                | crate::sync::Access::Save
                                | crate::sync::Access::Publish
                        ))
                {
                    return Err(SyncCode::Denied.into());
                }
                if tx.page_epoch(&scope.page)?.as_deref() != Some(scope.epoch.as_str()) {
                    return Err(SyncCode::StaleEpoch.into());
                }
                if let crate::sync::Access::Append(c) = access {
                    let cited = values::decimal(&c.membership_revision, false)?;
                    if cited > head.revision
                        || cited < values::decimal(cert.membership_revision, false)?
                        || !matches!(c.namespace.as_str(), "content" | "own")
                    {
                        return Err(SyncCode::Denied.into());
                    }
                    // A session admitted before the head moved seals at the head it knew. Every
                    // authority fact above is read at the head, so the append stands when the
                    // statements it missed are all scoped to other pages. Any other change, while
                    // the device is still authorised, is a stale view and no verdict on the device:
                    // ask it to catch up, never `DENIED`, which the browser treats as lost access.
                    if cited < head.revision
                        && !tx.unrelated_since(&scope.page, cited, head.revision)?
                    {
                        return Err(SyncCode::ResyncRequired.into());
                    }
                }
                Ok(binding.signing)
            })
            .map_err(|error| sync_fault(error.as_ref()))
    }
}

/// Registration admission already authenticated the owner device and its pinned keys.
/// Wrap additions share its device transaction; failed capacity/signing publishes none.
fn forward_owner_wraps(
    tx: &mut OwnerTransaction<'_>,
    key: &Keyring,
    device: &str,
    recipient_key: &[u8; 32],
) -> Result<()> {
    let head = tx.head().ok_or(Code::Unavailable)?.clone();
    let pages = tx.pages()?;
    if pages.len() > crate::limits::PAGES {
        return Err(Code::Capacity.into());
    }
    let mut added = 0;
    for page in pages {
        let policy = tx.page_policy_at(&page, head.revision)?;
        if !policy.writable() {
            continue;
        }
        let epochs = tx.recent_secrets(&page, policy.epoch)?;
        for (epoch, mut secret) in epochs {
            let result = (|| -> Result<()> {
                if (policy.history_current && epoch != policy.epoch)
                    || tx.device_wrap_exists(&page, epoch, device, recipient_key)?
                {
                    return Ok(());
                }
                added += 1;
                if added > crate::limits::OWNER_WRAPS {
                    return Err(Code::Capacity.into());
                }
                tx.put_wrap(&key.seal_wrap(
                    &tmt_colab_model::wrap::Header {
                        space: key.space_id.clone(),
                        page: page.clone(),
                        epoch: epoch.to_string(),
                        recipient_kind: "device".into(),
                        recipient_id: device.into(),
                        recipient_key: *recipient_key,
                        signer_key: key.owner_public(),
                        membership_revision: head.revision.to_string(),
                    },
                    &secret,
                )?)?;
                Ok(())
            })();
            secret.fill(0);
            result?;
        }
    }
    Ok(())
}

fn issuer(
    tx: &OwnerTransaction<'_>,
    keyring: &Keyring,
) -> Result<(statement::Envelope, statement::OwnerMember, u64)> {
    let head = tx.head().ok_or(Code::Unavailable)?;
    let member = keyring.management_member()?;
    if head.owner_member != member {
        return Err(Code::Unavailable.into());
    }
    let initial = tx.statement(1)?.ok_or(Code::Unavailable)?;
    let verified = initial.verify_next(&keyring.space_id, &keyring.owner_public(), None)?;
    if verified.head.owner_member != member {
        return Err(Code::Unavailable.into());
    }
    let issuer = tx
        .recipient("member", &member.id)?
        .ok_or(Code::Unavailable)?;
    if issuer.revoked
        || issuer.signing_key != member.signing_key
        || issuer.encryption_key != member.encryption_key
        || issuer.role.as_deref() != Some("editor")
    {
        return Err(Code::Denied.into());
    }
    Ok((initial, member, head.revision))
}
fn verify_device(
    chain: &certificate::Chain,
    cert: &certificate::Certificate<'_>,
    initial: &statement::Envelope,
    member: &statement::OwnerMember,
    binding: &DeviceRegistration,
    revision: u64,
    space: &str,
) -> Result<()> {
    if cert.device_id != binding.context.device_id || cert.space != space {
        return Err(Code::Denied.into());
    }
    if cert.issuer_kind != "member"
        || cert.issuer_id != member.id
        || cert.signing_key != &binding.signing
        || cert.encryption_key != &binding.encryption
        || values::decimal(cert.membership_revision, false)? > revision
    {
        return Err(Code::Denied.into());
    }
    chain.verify(&initial.hash()?, cert, &member.signing_key)?;
    Ok(())
}
fn map_error(error: Box<dyn std::error::Error + Send + Sync>) -> Code {
    error
        .downcast_ref::<Code>()
        .copied()
        .unwrap_or(Code::Unavailable)
}

pub(crate) fn now_ms() -> std::result::Result<u64, Code> {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Code::Unavailable)?
        .as_millis()
        .try_into()
        .map_err(|_| Code::Unavailable)
}

/// What a peer is told when an admission read fails. A refusal the store answered keeps its code;
/// a store that could not be read (another connection held it past the busy timeout) is no
/// verdict on the device, so the peer resyncs instead of being denied (#2956).
fn sync_fault(error: &(dyn std::error::Error + 'static)) -> crate::sync::Code {
    if let Some(code) = error.downcast_ref::<crate::sync::Code>() {
        return *code;
    }
    if crate::store::contended(error) {
        crate::sync::Code::ResyncRequired
    } else {
        crate::sync::Code::Denied
    }
}

fn reader_sync_error(code: Code) -> crate::sync::Code {
    match code {
        Code::Expired => crate::sync::Code::Expired,
        _ => crate::sync::Code::Denied,
    }
}

#[cfg(test)]
mod describe_tests {
    use super::*;
    use crate::summary::{Summary, SummaryProjection};

    /// A page's work row and parent are resolved across the pages the cache holds fresh.
    #[test]
    fn the_owner_list_resolves_work_and_parent_across_fresh_pages_only() {
        use serde_json::json;
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "colab-list-work-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                std::fs::remove_dir_all(&self.0).unwrap();
            }
        }
        let _cleanup = Cleanup(root.clone());
        let layout = crate::keyring::Layout::open(&root).unwrap();
        let key = Keyring::open(&layout).unwrap();
        let store = Store::open(&layout).unwrap();
        let (top, leaf, bare, unread) = (
            "00000000-0000-4000-8000-000000000002",
            "00000000-0000-4000-8000-000000000003",
            "00000000-0000-4000-8000-000000000004",
            "00000000-0000-4000-8000-000000000005",
        );
        for page in [top, leaf, bare, unread] {
            store.create_page(page).unwrap();
        }
        let mut service =
            Registration::new(store, key, std::path::PathBuf::from("/unused-decoder")).unwrap();
        service.genesis().unwrap();
        let context = json!({"deviceId":"00000000-0000-4000-8000-000000000009","kind":"browser",
            "origin":"http://127.0.0.1:1","name":"Laptop",
            "publicKey":values::encode_binary(&service.keyring.owner_public()),
            "owner":true,"grantRevision":1})
        .to_string();
        let (_, fingerprints) = service.owner_catalog_fingerprinted().unwrap();
        // `top` has a finished task of its own and one that follows `leaf`; `leaf` is done.
        let work = |tasks: serde_json::Value, order: serde_json::Value, next: u32| json!({"work": {"version":1,"next":next,"tasks":tasks,"order":order}});
        let top_work = work(
            json!({
                "T-1":{"title":"Leaf","state":"todo","child":leaf},
                "T-2":{"title":"Own","state":"done"}
            }),
            json!(["T-1", "T-2"]),
            3,
        );
        let leaf_work = work(
            json!({"T-1":{"title":"Do","state":"done"}}),
            json!(["T-1"]),
            2,
        );
        let put = |page: &str, fingerprint: &str, meta: Option<&serde_json::Value>| {
            service.summaries.put(
                page,
                fingerprint,
                Some(SummaryProjection::from(Summary {
                    comments: 0,
                    waiting: false,
                    needs_you: 0,
                    author: None,
                    work: meta
                        .and_then(|meta| crate::work_index::page_work(page, meta))
                        .map(Box::new),
                })),
            )
        };
        put(top, &fingerprints[top], Some(&top_work));
        put(leaf, &fingerprints[leaf], Some(&leaf_work));
        put(bare, &fingerprints[bare], None);
        let rows =
            |service: &Registration| -> std::collections::BTreeMap<String, serde_json::Value> {
                let list: serde_json::Value =
                    serde_json::from_slice(&service.pages(Some(&context)).unwrap()).unwrap();
                list["pages"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|page| (page["pageId"].as_str().unwrap().to_owned(), page.clone()))
                    .collect()
            };
        let list = rows(&service);
        // `leaf` is done, so the task that follows it is done and `top` is done: the stage crosses
        // pages. The row names no per-task state and no assignee id.
        assert_eq!(
            list[top]["summary"]["work"],
            json!({"stage":"done","done":2,"total":2,"signoff":0,"working":[]})
        );
        assert_eq!(
            list[leaf]["summary"]["work"],
            json!({"stage":"done","done":1,"total":1,"signoff":0,"working":[]})
        );
        assert_eq!(
            list[leaf]["summary"]["parent"],
            json!({"page":top,"task":"T-1","title":"Leaf"})
        );
        // A fresh page without work has a summary and neither field; a page without a fresh summary
        // has no summary at all.
        assert!(list[bare]["summary"].get("work").is_none());
        assert!(list[bare]["summary"].get("parent").is_none());
        assert!(list[top]["summary"].get("parent").is_none());
        assert!(list[unread].get("summary").is_none());

        // The followed page goes stale: its stage is unknown, so the task keeps its own state.
        put(leaf, "stale fingerprint", Some(&leaf_work));
        let list = rows(&service);
        assert_eq!(list[top]["summary"]["work"]["stage"], "in-progress");
        assert_eq!(list[top]["summary"]["work"]["done"], 1);
        assert!(list[leaf].get("summary").is_none());
    }

    #[test]
    fn describe_matches_active_waiting_pages_and_omits_unknown_counts() {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "colab-describe-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                std::fs::remove_dir_all(&self.0).unwrap();
            }
        }
        let _cleanup = Cleanup(root.clone());
        let layout = crate::keyring::Layout::open(&root).unwrap();
        let key = Keyring::open(&layout).unwrap();
        let store = Store::open(&layout).unwrap();
        let active = "00000000-0000-4000-8000-000000000002";
        let archived = "00000000-0000-4000-8000-000000000003";
        store.create_page(active).unwrap();
        store.create_page(archived).unwrap();
        let mut service =
            Registration::new(store, key, std::path::PathBuf::from("/unused-decoder")).unwrap();
        service.genesis().unwrap();
        let key = &service.keyring;
        let payload = serde_json::to_vec(&serde_json::json!({"pageId":archived})).unwrap();
        service
            .store
            .owner_transaction(
                &key.space_id,
                &key.owner_public(),
                Mutation {
                    operation_id: "00000000-0000-4000-8000-000000000004",
                    digest: crypto::digest(&payload),
                    expected_revision: 1,
                },
                |tx| {
                    let statement = key.sign_statement(tx.head(), "page.archive", &payload)?;
                    tx.append_statement(&statement)?;
                    Ok(statement.to_json()?)
                },
            )
            .unwrap();
        let (catalog, fingerprints) = service.owner_catalog_fingerprinted().unwrap();
        assert!(
            catalog["pages"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p["pageId"] == archived && p["archived"] == true)
        );
        let put = |page: &str, fingerprint: &str, count| {
            service.summaries.put(
                page,
                fingerprint,
                Some(SummaryProjection::from(Summary {
                    comments: 0,
                    waiting: count > 0,
                    needs_you: count,
                    author: None,
                    work: None,
                })),
            )
        };
        put(active, &fingerprints[active], 2);
        put(archived, "stale fingerprint", 2);
        let answer: serde_json::Value = serde_json::from_slice(
            &service
                .describe()
                .expect("archived stale summaries are irrelevant"),
        )
        .unwrap();
        assert_eq!(answer["menu"][0]["count"]["value"], 1);
        assert_eq!(
            answer["needsYou"], 1,
            "two items on one waiting page count once"
        );
        put(archived, &fingerprints[archived], 2);
        let answer: serde_json::Value =
            serde_json::from_slice(&service.describe().unwrap()).unwrap();
        assert_eq!(answer["menu"][0]["count"]["value"], 1);
        assert_eq!(
            answer["needsYou"], 1,
            "fresh archived waiting pages are excluded too"
        );
        for fingerprint in [fingerprints[active].as_str(), "old fingerprint"] {
            service.summaries.put(active, fingerprint, None);
            let answer: serde_json::Value =
                serde_json::from_slice(&service.describe().unwrap()).unwrap();
            assert_eq!(answer["rail"]["label"], "Colab");
            assert!(answer.get("needsYou").is_none());
            assert_eq!(
                answer["menu"][0]["count"]["value"], 1,
                "catalog count stays known"
            );
            assert!(
                service
                    .summaries
                    .get(active, &fingerprints[active])
                    .is_none(),
                "describe cannot refresh the cache"
            );
        }
        put(active, &fingerprints[active], 0);
        let answer: serde_json::Value =
            serde_json::from_slice(&service.describe().unwrap()).unwrap();
        assert_eq!(answer["needsYou"], 0, "known zero remains a number");
        let oracle = rusqlite::Connection::open(layout.directory.join("space.db")).unwrap();
        oracle
            .execute("UPDATE membership_log SET envelope=x'00'", [])
            .unwrap();
        assert_eq!(
            service.describe(),
            Err(Code::Unavailable),
            "a broken catalog still refuses"
        );
        drop(oracle);
        service.close().unwrap();
    }
}

//! The owner's serve applies the edit requests that link devices left in their own streams.
//!
//! A hosted editor link cannot seal page text, so it writes an `edit-request` own record (see
//! `edit_request.rs`) and this machine, while it runs, saves it as the root-local writer would
//! save an owner's browser edit: through the same preparation, merge and commit, signed by
//! no link. Each request is settled once and the decision is kept, so a restart never applies one
//! twice and never evaluates a refused one again.
use super::{Admission, Code, SaveJob, Server, SyncScope};
use crate::{
    fold::Snapshot,
    page::{patch::Hunk, save::Save},
    store::{EDIT_DECISIONS, EditState},
};
use serde_json::Value;
use tmt_colab_model::values;

/// The most requests one call settles; the next call continues.
const BATCH: usize = 16;

/// Keep one consumer responsible through identity resolution, durable settlement and receipt.
/// The nested SaveJob retains its existing preparation count on the same operation.
struct MetadataClaim<'a, A: Admission> {
    server: &'a Server<A>,
    key: (String, String),
}
impl<A: Admission> Drop for MetadataClaim<'_, A> {
    fn drop(&mut self) {
        if let Ok(mut state) = self.server.0.lock() {
            state.release_save(&self.key);
        }
    }
}

/// The pages where a link session stored an `own` original that may carry an edit request, until
/// a connection settles them. Every page stays marked however many sessions mark pages meanwhile.
#[derive(Default)]
pub(super) struct Due(Vec<SyncScope>);
impl Due {
    pub(super) fn mark(&mut self, scope: &SyncScope) {
        if !self.0.contains(scope) {
            self.0.push(scope.clone());
        }
    }
    /// All the marked pages, which are no longer marked.
    pub(super) fn take(&mut self) -> Vec<SyncScope> {
        std::mem::take(&mut self.0)
    }
}

/// One request as its writer's `own` document holds it, already decoder-validated.
struct Request {
    writer: String,
    operation: String,
    save: Save,
    hunks: Option<Vec<Hunk>>,
    metadata: Option<crate::edit_request::metadata::Request>,
    record: Value,
}
impl Request {
    fn read(page: &str, writer: &str, key: &str, record: &Value) -> Option<Self> {
        crate::edit_request::validate_record("intents", key, record).ok()?;
        if record.get("kind")?.as_str()? != "edit-request" {
            return None;
        }
        if record["version"] == 2 {
            return Some(Self {
                writer: writer.into(),
                operation: key.into(),
                save: Save {
                    page: page.into(),
                    operation_id: key.into(),
                    base_sha256: [0; 32],
                    source: String::new(),
                    attachments: None,
                },
                hunks: None,
                metadata: Some(
                    crate::edit_request::metadata::Request::read("intents", key, record).ok()?,
                ),
                record: record.clone(),
            });
        }
        let hunks = match record.get("hunks") {
            None => None,
            Some(hunks) => Some(
                hunks
                    .as_array()?
                    .iter()
                    .map(|hunk| {
                        Some(Hunk {
                            old_start: usize::try_from(hunk["oldStart"].as_u64()?).ok()?,
                            old: hunk["old"].as_str()?.to_owned(),
                            new: hunk["new"].as_str()?.to_owned(),
                        })
                    })
                    .collect::<Option<Vec<_>>>()?,
            ),
        };
        Some(Self {
            writer: writer.to_owned(),
            operation: key.to_owned(),
            save: Save {
                page: page.to_owned(),
                operation_id: key.to_owned(),
                base_sha256: values::binary(record["baseSha256"].as_str()?, 32)
                    .ok()?
                    .try_into()
                    .ok()?,
                source: record["source"].as_str()?.to_owned(),
                attachments: None,
            },
            hunks,
            metadata: None,
            record: record.clone(),
        })
    }
}

impl<A: Admission> Server<A> {
    /// Settle the page's undecided edit requests: refuse those whose link no longer allows them,
    /// save the rest through the ordinary Save path, and keep each decision. Returns how many
    /// were settled; a request whose attempt ended without an answer a later try could change
    /// stays undecided. An admission with no native writer settles nothing.
    pub fn apply_edit_requests(&self, scope: &SyncScope) -> Result<usize, Code> {
        let epoch = values::decimal(&scope.epoch, false)?;
        let (open, decided) = {
            let state = self.0.lock().map_err(|_| Code::Denied)?;
            let Some(open) = state.admission.save_source() else {
                return Ok(0);
            };
            let decided = state
                .store
                .edit_decisions(&scope.page, epoch)
                .map_err(|_| Code::Denied)?;
            (open, decided)
        };
        // A full page settles nothing more: a request saved without room for its decision would be
        // saved again by every later trigger.
        let room = EDIT_DECISIONS.saturating_sub(decided.len());
        // One read-only view of the page, outside every lock.
        let pending = {
            let mut source = open().map_err(|_| Code::Denied)?;
            let snapshot = Snapshot::capture(&source.store, &source.keyring, &scope.page)
                .map_err(|_| Code::Denied)?;
            if snapshot.epoch != epoch {
                return Ok(0);
            }
            let view = snapshot
                .materialize(&source.keyring, &scope.page, &mut source.decoder)
                .map_err(|_| Code::Denied)?;
            let mut pending = Vec::new();
            let mut new_requests = 0;
            for (writer, roots) in &view.own {
                let Some(intents) = roots.get("intents").and_then(Value::as_object) else {
                    continue;
                };
                for (key, record) in intents {
                    if record.get("kind").and_then(Value::as_str) != Some("edit-request") {
                        continue;
                    }
                    let settled = decided.contains(&(writer.clone(), key.clone()));
                    if settled && record["version"] != 2 {
                        continue;
                    }
                    let result_key = crate::edit_request::metadata::outcome_key(writer, key);
                    if record["version"] == 2
                        && view.status_writers.iter().any(|owner| {
                            view.own
                                .get(owner)
                                .and_then(|roots| roots.get("messages"))
                                .and_then(|messages| messages.get(&result_key))
                                .is_some()
                        })
                    {
                        continue;
                    }
                    if !settled && new_requests >= room {
                        continue;
                    }
                    if let Some(request) = Request::read(&scope.page, writer, key, record) {
                        if !settled {
                            new_requests += 1;
                        }
                        pending.push(request);
                    }
                }
            }
            pending.sort_by(|a, b| (&a.writer, &a.operation).cmp(&(&b.writer, &b.operation)));
            pending.truncate(BATCH);
            pending
        };
        let mut settled = 0;
        for request in pending {
            let state = self.settle(scope, &open, request)?;
            settled += usize::from(state);
        }
        Ok(settled)
    }
    /// Decide one request. `Ok(false)` when it stays undecided.
    fn settle(
        &self,
        scope: &SyncScope,
        open: &crate::page::save::SourceOpener,
        request: Request,
    ) -> Result<bool, Code> {
        if request.metadata.is_some() {
            return self.settle_metadata(scope, open, request);
        }
        let Request {
            writer,
            operation,
            save,
            hunks,
            ..
        } = request;
        let allowed = self
            .0
            .lock()
            .map_err(|_| Code::Denied)?
            .admission
            .authorize_link_save(&writer, scope);
        let decision = match allowed {
            // The link's own answer: it ends the request for good.
            Err(Code::Denied | Code::Expired) => Some(EditState::Refused),
            // No answer could be had now (a lock, a clock or the store): try again later.
            Err(_) => None,
            Ok(()) => {
                {
                    let mut state = self.0.lock().map_err(|_| Code::Denied)?;
                    *state
                        .saving
                        .entry((scope.page.clone(), operation.clone()))
                        .or_default() += 1;
                }
                let job = SaveJob {
                    open: open.clone(),
                    scope: scope.clone(),
                    principal: writer.clone(),
                    for_request: true,
                    save,
                    hunks,
                    subpage: None,
                    metadata: None,
                };
                self.run_save_job(None, &job)?.settles()
            }
        };
        let Some(decision) = decision else {
            return Ok(false);
        };
        let epoch = values::decimal(&scope.epoch, false)?;
        self.0
            .lock()
            .map_err(|_| Code::Denied)?
            .store
            .decide_edit(&scope.page, epoch, &writer, &operation, decision)
            .map_err(|_| Code::Denied)?;
        Ok(true)
    }

    fn settle_metadata(
        &self,
        scope: &SyncScope,
        open: &crate::page::save::SourceOpener,
        request: Request,
    ) -> Result<bool, Code> {
        use crate::edit_request::metadata;
        use crate::page::save::SaveResult;
        let epoch = values::decimal(&scope.epoch, false)?;
        let operation = metadata::operation(
            &scope.page,
            &scope.epoch,
            &request.writer,
            &request.operation,
        );
        let key = (scope.page.clone(), operation.clone());
        {
            let mut state = self.0.lock().map_err(|_| Code::Denied)?;
            if state.saving.contains_key(&key) {
                return Ok(false);
            }
            state.saving.insert(key.clone(), 1);
        }
        let _claim = MetadataClaim { server: self, key };
        let stored = self
            .0
            .lock()
            .map_err(|_| Code::Denied)?
            .store
            .edit_result(&scope.page, epoch, &request.writer, &request.operation)
            .map_err(|_| Code::Denied)?;
        let record = if let Some(stored) = stored {
            stored
        } else {
            // A committed operation wins over a later stale target or grant loss after a crash.
            let recovered = self
                .0
                .lock()
                .map_err(|_| Code::Denied)?
                .admission
                .save_status(&scope.page, &operation)
                .map_err(|_| Code::Denied)?;
            let result = if matches!(recovered.state, "committed" | "rejected") {
                recovered
            } else {
                let allowed = self
                    .0
                    .lock()
                    .map_err(|_| Code::Denied)?
                    .admission
                    .authorize_link_save(&request.writer, scope);
                match allowed {
                    Err(Code::Denied | Code::Expired) => {
                        SaveResult::rejected(&operation, &crate::page::Fault::Denied)
                    }
                    Err(_) => return Ok(false),
                    Ok(()) => match request
                        .metadata
                        .as_ref()
                        .expect("metadata request")
                        .prepare(open, &scope.page, &operation)
                    {
                        Err(error) => SaveResult::rejected(&operation, error.as_ref()),
                        Ok(edit) => {
                            let mut save = request.save;
                            save.operation_id = operation;
                            let job = SaveJob {
                                open: open.clone(),
                                scope: scope.clone(),
                                principal: request.writer.clone(),
                                for_request: true,
                                save,
                                hunks: None,
                                subpage: None,
                                metadata: Some(edit),
                            };
                            *self
                                .0
                                .lock()
                                .map_err(|_| Code::Denied)?
                                .saving
                                .entry((scope.page.clone(), job.save.operation_id.clone()))
                                .or_default() += 1;
                            self.run_save_job(None, &job)?
                        }
                    },
                }
            };
            let Some(decision) = result.settles() else {
                return Ok(false);
            };
            // Project the closed terminal metadata shape, not unrelated Save fields such as
            // source findings, task IDs or child-page receipts.
            let terminal = if matches!(result.state, "committed" | "unchanged") {
                serde_json::json!({"operationId":request.operation,"state":result.state,"revision":result.revision})
            } else {
                serde_json::json!({"operationId":request.operation,"state":result.state,"code":result.code,"message":result.message})
            };
            let record = serde_json::json!({"version":1,"kind":"edit-result","writer":request.writer,
                "operationId":request.operation,"requestSha256":values::encode_binary(&tmt_colab_model::crypto::digest(
                    &crate::closed::canonical(&request.record).map_err(|_|Code::Invalid)?)),"result":terminal});
            self.0
                .lock()
                .map_err(|_| Code::Denied)?
                .store
                .decide_metadata(
                    &scope.page,
                    epoch,
                    &request.writer,
                    &request.operation,
                    decision,
                    &record,
                )
                .map_err(|_| Code::Denied)?;
            record
        };
        self.publish_metadata_result(scope, open, &record)?;
        Ok(true)
    }

    /// Only the owner signs this receipt. Lost receipt publication recovers the same durable
    /// result without resolving the identity, rechecking its old target or publishing again.
    fn publish_metadata_result(
        &self,
        scope: &SyncScope,
        open: &crate::page::save::SourceOpener,
        record: &Value,
    ) -> Result<(), Code> {
        let key = crate::edit_request::metadata::outcome_key(
            record["writer"].as_str().ok_or(Code::Invalid)?,
            record["operationId"].as_str().ok_or(Code::Invalid)?,
        );
        let mut source = open().map_err(|_| Code::Denied)?;
        let now = crate::registration::now_ms().map_err(|_| Code::Denied)?;
        let frozen = crate::page::prepare_own_records(
            &source.store,
            &source.keyring,
            &scope.page,
            &[crate::decoder::OwnRecord {
                root: "messages".into(),
                key,
                value: record.clone(),
            }],
            &mut source.decoder,
            now,
        )
        .map_err(|_| Code::Denied)?;
        if frozen.job().manifest.space_id != scope.space
            || frozen.job().manifest.page_id != scope.page
            || frozen.job().manifest.epoch != scope.epoch
        {
            return Err(Code::Expired);
        }
        let mut state = self.0.lock().map_err(|_| Code::Denied)?;
        let author = state.admission.save_author().map_err(|_| Code::Denied)?;
        let outputs = super::broadcasts(
            frozen.job(),
            frozen.packet(),
            &values::encode_binary(frozen.chain()),
            &author,
        )
        .map_err(|_| Code::Denied)?;
        let committed = state
            .admission
            .commit_save(
                frozen.job(),
                frozen.packet(),
                frozen.chain(),
                now,
                std::time::Instant::now() + crate::limits::PUBLISH_COMBINE,
            )
            .map_err(|_| Code::Denied)?;
        crate::page::publication_receipt(frozen.job(), &committed.committed.record)
            .map_err(|_| Code::Denied)?;
        state.fanout_delivery(&outputs.0, outputs.1);
        if let Some(combined) = &committed.combined {
            state.announce_combined(scope, combined);
        }
        state.touch(scope);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(page: &str) -> SyncScope {
        SyncScope {
            space: "space".into(),
            page: page.into(),
            epoch: "1".into(),
        }
    }

    #[test]
    fn two_pages_marked_before_any_is_taken_are_both_taken_once() {
        let mut due = Due::default();
        due.mark(&scope("first"));
        due.mark(&scope("second"));
        due.mark(&scope("first"));
        assert_eq!(due.take(), [scope("first"), scope("second")]);
        assert!(due.take().is_empty(), "taking marks nothing again");
    }
}

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
}
impl Request {
    fn read(page: &str, writer: &str, key: &str, record: &Value) -> Option<Self> {
        crate::edit_request::validate_record("intents", key, record).ok()?;
        if record.get("kind")?.as_str()? != "edit-request" {
            return None;
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
        if room == 0 {
            return Ok(0);
        }
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
            for (writer, roots) in &view.own {
                let Some(intents) = roots.get("intents").and_then(Value::as_object) else {
                    continue;
                };
                for (key, record) in intents {
                    if record.get("kind").and_then(Value::as_str) != Some("edit-request")
                        || decided.contains(&(writer.clone(), key.clone()))
                    {
                        continue;
                    }
                    if let Some(request) = Request::read(&scope.page, writer, key, record) {
                        pending.push(request);
                    }
                }
            }
            pending.sort_by(|a, b| (&a.writer, &a.operation).cmp(&(&b.writer, &b.operation)));
            pending.truncate(BATCH.min(room));
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
        let Request {
            writer,
            operation,
            save,
            hunks,
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
                    work: None,
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

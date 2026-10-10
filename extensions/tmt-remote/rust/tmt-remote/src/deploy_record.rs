//! Durable deployment identity and run outcomes. Writers retain deploy.lock; readers
//! never take it and see one complete record through atomic rename. No remote.db opener.
use crate::{
    canonical,
    deploy_run::{self, DeployRecord, DeploySink, DeploySinkError, RunState, StepState},
    error::RemoteError,
    limits,
    readiness::{self, FirestoreEvidence, FirestoreEvidenceSource},
    state::Layout,
    store::uuid_v4,
    wire,
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::PathBuf,
};

const FILE: &str = "deploy.json";
const PREFIX: &str = ".deploy-";
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeployDocument {
    version: u8,
    record: DeployRecord,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    target: Option<DeployTarget>,
}

/// The immutable owner-home target, separate from the last usable binding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeployTarget {
    project: String,
    region: String,
}
impl DeployTarget {
    fn valid(&self) -> bool {
        deploy_run::project_ok(&self.project) && deploy_run::location_ok(&self.region)
    }
}
fn unavailable() -> RemoteError {
    RemoteError::new(
        "REMOTE_STATE_UNAVAILABLE",
        "Deployment record could not be persisted or read.",
    )
}
fn invalid() -> RemoteError {
    RemoteError::new(
        "REMOTE_STATE_UNSAFE",
        "Deployment record is invalid or unsafe; it was not reset.",
    )
}
fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn valid(record: &DeployRecord) -> bool {
    if canonical::uuid(&record.deployment_id).is_err() {
        return false;
    }
    if let Some(binding) = &record.binding
        && (!digest(&binding.plan_digest)
            || binding.project.is_empty()
            || binding.project.len() > 30)
    {
        return false;
    }
    if let Some(run) = &record.run {
        let mut ids = std::collections::BTreeSet::new();
        if !digest(&run.plan_digest)
            || run.account.is_empty()
            || run.account.len() > 1024
            || run.account.chars().any(char::is_control)
            || run.steps.is_empty()
            || run
                .steps
                .iter()
                .any(|step| step.id.is_empty() || step.id.len() > 1024 || !ids.insert(&step.id))
        {
            return false;
        }
        if let Some(hosting) = &run.hosting {
            let Ok(view) =
                serde_json::from_value::<deploy_run::DeployView>(hosting.envelope.clone())
            else {
                return false;
            };
            if serde_json::to_value(&view).expect("view serializes") != hosting.envelope
                || crate::deploy_plan::sha256_hex(
                    &serde_json::to_vec(&view).expect("view serializes"),
                ) != run.plan_digest
                || view.deployment_id != record.deployment_id
                || view.account != run.account
                || view.hosting.is_none()
                || view.steps != run.steps.iter().map(|s| s.id.clone()).collect::<Vec<_>>()
                || hosting
                    .publication
                    .as_ref()
                    .is_some_and(|p| !p.valid_for(&view, hosting, &run.plan_digest))
                || [&hosting.app_id, &hosting.operation, &hosting.version]
                    .iter()
                    .any(|s| {
                        s.as_ref().is_some_and(|s| {
                            s.is_empty() || s.len() > 256 || s.chars().any(char::is_control)
                        })
                    })
                || (run.state == RunState::Complete && hosting.publication.is_none())
            {
                return false;
            }
        }
        if run.state == RunState::Complete
            && (!run
                .steps
                .iter()
                .all(|s| matches!(s.state, StepState::Done | StepState::Adopted))
                || run.steps.last().is_none_or(|s| s.id != "verify")
                || record
                    .binding
                    .as_ref()
                    .is_none_or(|b| b.plan_digest != run.plan_digest))
        {
            return false;
        }
    } else if record.binding.is_some() {
        return false;
    }
    true
}

/// A read-only snapshot, including while another process holds the writer lock.
/// Missing is not damaged; malformed, oversized and unsafe state never becomes a draft.
pub fn read(layout: &Layout) -> Result<Option<DeployRecord>, RemoteError> {
    Ok(read_document(layout)?.map(|document| document.record))
}
fn read_document(layout: &Layout) -> Result<Option<DeployDocument>, RemoteError> {
    let Some(file) = layout.read_file(FILE)? else {
        return Ok(None);
    };
    let mut bytes = Vec::new();
    file.take((limits::DEPLOY_RECORD_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| unavailable())?;
    if bytes.len() > limits::DEPLOY_RECORD_BYTES {
        return Err(invalid());
    }
    let value = wire::strict_json(&bytes).ok_or_else(invalid)?;
    let document: DeployDocument = serde_json::from_value(value.clone()).map_err(|_| invalid())?;
    // Nested engine records accept ordinary serde defaults; require their full exact
    // schema here so unknown fields or missing optional fields are not silently lost.
    if !matches!(
        (document.version, &document.target),
        (1, None) | (2 | 3, Some(_))
    ) || (document.version == 3)
        != document
            .record
            .run
            .as_ref()
            .is_some_and(|r| r.hosting.is_some())
        || document
            .target
            .as_ref()
            .is_some_and(|target| !target.valid())
        || !valid(&document.record)
        || serde_json::to_value(&document).map_err(|_| invalid())? != value
    {
        return Err(invalid());
    }
    Ok(Some(document))
}

/// One bounded, lock-free snapshot per status request. No provider calls or repair;
/// atomic publication allows this reader to coexist with a long-running deployment.
pub struct DeployRecordEvidence {
    layout: Layout,
}
impl DeployRecordEvidence {
    pub fn new(layout: &Layout) -> Self {
        Self {
            layout: layout.clone(),
        }
    }
}
impl FirestoreEvidenceSource for DeployRecordEvidence {
    fn evidence(&self) -> Option<FirestoreEvidence> {
        match read(&self.layout) {
            Ok(Some(record)) => readiness::from_record(&record),
            Ok(None) => None,
            Err(_) => Some(FirestoreEvidence::unknown()),
        }
    }
}

/// One writer for the whole plan/run. Drop releases only the writer lock, not identity.
/// It never holds the serving lease; status readers do not wait for this owner.
pub struct DeployRecordStore<'a> {
    layout: &'a Layout,
    _lock: File,
    target: Option<DeployTarget>,
}
impl<'a> DeployRecordStore<'a> {
    pub fn open(layout: &'a Layout) -> Result<Self, RemoteError> {
        Self::open_checked(layout, None)
    }
    /// Check a retained target under the writer lock, before cleanup or provider setup.
    pub fn open_for_target(
        layout: &'a Layout,
        project: &str,
        region: &str,
    ) -> Result<Self, RemoteError> {
        Self::open_checked(layout, Some((project, region)))
    }
    fn open_checked(
        layout: &'a Layout,
        requested: Option<(&str, &str)>,
    ) -> Result<Self, RemoteError> {
        let lock = layout.file("deploy.lock")?;
        lock.try_lock().map_err(|error| match error {
            std::fs::TryLockError::WouldBlock => RemoteError::new(
                "REMOTE_DEPLOY_BUSY",
                "Another deployment owns the writer lock.",
            ),
            std::fs::TryLockError::Error(_) => unavailable(),
        })?;
        // Every v1 record is unbound, even when its old binding names a project.
        // Never infer a target or convert a record on a read.
        let target = read_document(layout)?.and_then(|document| document.target);
        let owner = Self {
            layout,
            _lock: lock,
            target,
        };
        if let Some((project, region)) = requested {
            owner.check_target(project, region)?;
        }
        // Refuse damaged or conflicting published state before cleaning staging files.
        for entry in fs::read_dir(&layout.directory).map_err(|_| unavailable())? {
            let entry = entry.map_err(|_| unavailable())?;
            if entry
                .file_name()
                .to_str()
                .and_then(|n| n.strip_prefix(PREFIX))
                .is_some_and(|n| canonical::uuid(n).is_ok())
            {
                let file = OpenOptions::new()
                    .read(true)
                    .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
                    .open(entry.path())
                    .map_err(|_| invalid())?;
                admit_stage(&file)?;
                fs::remove_file(entry.path()).map_err(|_| unavailable())?;
                layout.sync().map_err(|_| unavailable())?;
            }
        }
        Ok(owner)
    }
    /// No provider call is needed to reject a different project or region.
    pub fn check_target(&self, project: &str, region: &str) -> Result<(), RemoteError> {
        if let Some(target) = self
            .target
            .as_ref()
            .filter(|target| target.project != project || target.region != region)
        {
            return Err(RemoteError::new(
                "REMOTE_DEPLOY_PROJECT_CONFLICT",
                &format!(
                    "This Remote home deploys to {} ({}), not {} ({}). To move it, remove '{}'; nothing else is deleted. The old project's Rules stay until a new plan, authorized from any home, replaces them.",
                    target.project,
                    target.region,
                    project,
                    region,
                    self.layout.directory.join("deploy.json").display()
                ),
            ));
        }
        Ok(())
    }
    /// Select only an authorized explicit plan's target; the next atomic save binds it
    /// together with the plan identity, before any provider effect.
    pub fn bind_target(&mut self, project: &str, region: &str) -> Result<(), RemoteError> {
        self.check_target(project, region)?;
        let target = DeployTarget {
            project: project.into(),
            region: region.into(),
        };
        if !target.valid() {
            return Err(invalid());
        }
        self.target = Some(target);
        Ok(())
    }
    pub fn load_or_draft(&self) -> Result<DeployRecord, RemoteError> {
        read(self.layout)?.map_or_else(|| uuid_v4().map(|id| DeployRecord::new(&id)), Ok)
    }
    pub fn persist(&mut self, record: &DeployRecord) -> Result<(), RemoteError> {
        if !valid(record) {
            return Err(invalid());
        }
        let bytes = serde_json::to_vec(&DeployDocument {
            version: if record.run.as_ref().is_some_and(|r| r.hosting.is_some()) {
                3
            } else if self.target.is_some() {
                2
            } else {
                1
            },
            record: record.clone(),
            target: self.target.clone(),
        })
        .map_err(|_| invalid())?;
        if bytes.len() > limits::DEPLOY_RECORD_BYTES {
            return Err(invalid());
        }
        // Validate the destination through the admitted no-follow reader before replacing.
        if read(self.layout)?.is_some_and(|old| old.deployment_id != record.deployment_id) {
            return Err(invalid());
        }
        let path = self
            .layout
            .directory
            .join(format!("{PREFIX}{}", uuid_v4()?));
        let mut staged = DeployStaged {
            path,
            published: false,
        };
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
            .open(&staged.path)
            .map_err(|_| unavailable())?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| unavailable())?;
        fs::rename(&staged.path, self.layout.directory.join(FILE)).map_err(|_| unavailable())?;
        staged.published = true;
        // A failure here stops effects but does not claim the visible file rolled back.
        self.layout.sync().map_err(|_| unavailable())
    }
}
impl DeploySink for DeployRecordStore<'_> {
    fn save(&mut self, record: &DeployRecord) -> Result<(), DeploySinkError> {
        self.persist(record).map_err(|_| DeploySinkError)
    }
}
fn admit_stage(file: &File) -> Result<(), RemoteError> {
    use std::os::unix::fs::MetadataExt;
    let m = file.metadata().map_err(|_| invalid())?;
    if !m.is_file()
        || m.uid() != nix::unistd::Uid::effective().as_raw()
        || m.mode() & 0o777 != 0o600
        || m.len() > limits::DEPLOY_RECORD_BYTES as u64
    {
        return Err(invalid());
    }
    Ok(())
}
struct DeployStaged {
    path: PathBuf,
    published: bool,
}
impl Drop for DeployStaged {
    fn drop(&mut self) {
        if !self.published {
            let _ = fs::remove_file(&self.path);
        }
    }
}

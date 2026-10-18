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
    if record
        .run
        .as_ref()
        .is_some_and(|r| r.state == RunState::Superseded)
        || record
            .superseded_runs
            .iter()
            .any(|r| r.state != RunState::Superseded)
    {
        return false;
    }
    for run in record.run.iter().chain(&record.superseded_runs) {
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
            let identifiers = [
                view.project.as_str(),
                view.account.as_str(),
                record.deployment_id.as_str(),
                view.hosting.as_ref().map_or("", |h| h.site.as_str()),
                hosting.app_id.as_deref().unwrap_or(""),
                hosting.version.as_deref().unwrap_or(""),
                hosting
                    .version
                    .as_deref()
                    .and_then(|v| v.rsplit('/').next())
                    .unwrap_or(""),
            ];
            if run
                .steps
                .iter()
                .flat_map(|s| s.cause.iter())
                .flat_map(|c| c.file_samples.iter().flatten())
                .any(|sample| {
                    sample.path != "<redacted>"
                        && identifiers
                            .iter()
                            .any(|id| !id.is_empty() && sample.path.contains(id))
                })
            {
                return false;
            }
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
    }
    if record.run.is_none() && record.binding.is_some() {
        return false;
    }
    true
}

/// A read-only snapshot, including while another process holds the writer lock.
/// Missing is not damaged; malformed, oversized and unsafe state never becomes a draft.
pub fn read_project(layout: &Layout, project: &str) -> Result<Option<DeployRecord>, RemoteError> {
    Ok(snapshot(layout, project)?.map(|snapshot| snapshot.record))
}
/// Evidence and selected identity come from the same atomic file snapshot.
pub struct OwnedSnapshot {
    pub record: DeployRecord,
    pub project: String,
    pub region: Option<String>,
}
impl OwnedSnapshot {
    pub fn identity(&self) -> Option<serde_json::Value> {
        self.region.as_ref().map(|region| serde_json::json!({"project":self.project,"region":region,"deploymentId":self.record.deployment_id}))
    }
}
pub fn snapshot(layout: &Layout, project: &str) -> Result<Option<OwnedSnapshot>, RemoteError> {
    refuse_legacy(layout)?;
    let Some(selected) = layout.project(project, false)? else {
        return Ok(None);
    };
    let document = read_document(&selected)?;
    if let Some(target) = document
        .as_ref()
        .and_then(|d| d.target.as_ref())
        .filter(|t| t.project != project)
    {
        return Err(copy_conflict(layout, project, &target.project));
    }
    Ok(document.map(|d| OwnedSnapshot {
        record: d.record,
        project: project.into(),
        region: d.target.map(|t| t.region),
    }))
}
fn copy_conflict(layout: &Layout, selected: &str, retained: &str) -> RemoteError {
    RemoteError::new(
        "REMOTE_SELECTOR_CONFLICT",
        &format!(
            "Conflicting copies: '{}' and '{}'.",
            layout
                .directory
                .join("deployments")
                .join(selected)
                .join(FILE)
                .display(),
            layout
                .directory
                .join("deployments")
                .join(retained)
                .join(FILE)
                .display()
        ),
    )
    .with_hint("Run tmt remote ls")
}
/// No legacy path is inferred, moved, converted or silently skipped.
pub fn refuse_legacy(layout: &Layout) -> Result<(), RemoteError> {
    if layout.directory.join(FILE).symlink_metadata().is_ok() {
        return Err(RemoteError::new(
            "REMOTE_SELECTOR_LEGACY",
            "An older single-project Firestore setup file must be moved aside before choosing a project",
        )
        .with_hint(&format!(
            "Run mv '{}' '{}.old'",
            layout.directory.join(FILE).display(),
            layout.directory.join(FILE).display()
        )));
    }
    Ok(())
}
/// Bounded local directory inventory; this is an admission bound, not a project quota.
pub fn projects(layout: &Layout) -> Result<Vec<String>, RemoteError> {
    refuse_legacy(layout)?;
    let Some(parent) = tmt_extension_state::Layout::existing(&layout.directory, "deployments", &[])
        .map_err(crate::state::state_error)?
    else {
        return Ok(vec![]);
    };
    let entries = match fs::read_dir(&parent.directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(_) => return Err(unavailable()),
    };
    let mut projects = vec![];
    for (index, entry) in entries.enumerate() {
        if index >= limits::OWNED_PROJECT_SCAN {
            return Err(invalid());
        }
        let entry = entry.map_err(|_| unavailable())?;
        let name = entry.file_name().into_string().map_err(|_| invalid())?;
        let Some(selected) = layout.project(&name, false)? else {
            return Err(invalid());
        };
        if selected.read_file(FILE)?.is_some() {
            projects.push(name);
        }
    }
    projects.sort();
    Ok(projects)
}
pub fn read(layout: &Layout) -> Result<Option<DeployRecord>, RemoteError> {
    match crate::remote_selector::owned(layout, None, None)? {
        Some(project) => read_project(layout, &project),
        None => Ok(None),
    }
}
/// Metadata is local and opt-in; it does not claim live provider readiness.
pub fn metadata(layout: &Layout, project: &str) -> Result<Option<serde_json::Value>, RemoteError> {
    Ok(snapshot(layout, project)?.and_then(|snapshot| snapshot.identity()))
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

/// Derive the public carrier from one lock-free snapshot; never save or repair.
pub fn read_link(layout: &Layout) -> Result<String, RemoteError> {
    let record = read(layout)?.ok_or_else(crate::remote_link::unavailable)?;
    crate::remote_link::from_record(&record)
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
        match crate::remote_selector::owned(&self.layout, None, None).and_then(|project| {
            project.map_or(Ok(None), |project| read_project(&self.layout, &project))
        }) {
            Ok(Some(record)) => readiness::from_record(&record),
            Ok(None) => None,
            Err(_) => Some(FirestoreEvidence::unknown()),
        }
    }
}

/// One writer for the whole plan/run. Drop releases only the writer lock, not identity.
/// It never holds the serving lease; status readers do not wait for this owner.
pub struct DeployRecordStore {
    layout: Layout,
    project: String,
    _lock: File,
    target: Option<DeployTarget>,
}
impl DeployRecordStore {
    pub(crate) fn record_path(&self) -> PathBuf {
        self.layout.directory.join(FILE)
    }
    pub fn open(layout: &Layout, project: &str) -> Result<Self, RemoteError> {
        Self::open_checked(layout, project, None)
    }
    /// Check a retained target under the writer lock, before cleanup or provider setup.
    pub fn open_for_target(
        layout: &Layout,
        project: &str,
        region: &str,
    ) -> Result<Self, RemoteError> {
        Self::open_checked(layout, project, Some((project, region)))
    }
    fn open_checked(
        layout: &Layout,
        project: &str,
        requested: Option<(&str, &str)>,
    ) -> Result<Self, RemoteError> {
        refuse_legacy(layout)?;
        let lock = layout.file("deploy.lock")?;
        lock.try_lock().map_err(|error| match error {
            std::fs::TryLockError::WouldBlock => RemoteError::new(
                "REMOTE_DEPLOY_BUSY",
                "Another deployment owns the writer lock.",
            ),
            std::fs::TryLockError::Error(_) => unavailable(),
        })?;
        let home = layout;
        let layout = layout
            .project(project, true)?
            .expect("created project directory");
        // A preview stays unbound until authorization; its directory already identifies the project.
        let target = read_document(&layout)?.and_then(|document| document.target);
        let owner = Self {
            layout: layout.clone(),
            project: project.into(),
            _lock: lock,
            target,
        };
        if let Some(target) = owner
            .target
            .as_ref()
            .filter(|target| target.project != project)
        {
            return Err(copy_conflict(home, project, &target.project));
        }
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
                    "Project {} uses region {}, not {}.",
                    target.project, target.region, region
                ),
            )
            .with_hint(&format!(
                "Run tmt remote deploy firestore --project {} --region {} --sign-in anonymous",
                target.project, target.region
            )));
        }
        Ok(())
    }
    /// Select only an authorized explicit plan's target; the next atomic save binds it
    /// together with the plan identity, before any provider effect.
    pub fn bind_target(&mut self, project: &str, region: &str) -> Result<(), RemoteError> {
        self.check_target(project, region)?;
        if project != self.project {
            return Err(invalid());
        }
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
        read_document(&self.layout)?
            .map(|document| document.record)
            .map_or_else(|| uuid_v4().map(|id| DeployRecord::new(&id)), Ok)
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
        if read_document(&self.layout)?
            .map(|document| document.record)
            .is_some_and(|old| old.deployment_id != record.deployment_id)
        {
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
impl DeploySink for DeployRecordStore {
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

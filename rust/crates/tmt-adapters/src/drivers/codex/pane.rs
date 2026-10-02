//! Read-only pane evidence survives binding retirement without a second registry.
use super::record::{Foreground, Store, recovery};
use crate::{
    process::{
        UnixCommandRunner,
        runtime::{ProcessObservation, observe_runtime_process},
    },
    runtime::channel::{ChannelFault, EvidenceError, PaneAddress, PaneEvidence},
};
use std::{cell::RefCell, collections::HashMap, fs, path::Path, time::Instant};
use tmt_core::{binding::session::RuntimeLiveness, endpoint::ProcessIncarnation};

pub fn enrolled(
    directory: &Path,
    pane: &PaneAddress<'_>,
    binding: Option<&str>,
    deadline: Instant,
) -> Result<PaneEvidence, EvidenceError> {
    enrolled_with_runner(&UnixCommandRunner, directory, pane, binding, deadline)
}
fn enrolled_with_runner(
    runner: &impl crate::process::CommandRunner,
    directory: &Path,
    pane: &PaneAddress<'_>,
    binding: Option<&str>,
    deadline: Instant,
) -> Result<PaneEvidence, EvidenceError> {
    let cache = RefCell::new(HashMap::<u64, ProcessObservation>::new());
    inspect(directory, pane, binding, deadline, |process| {
        let mut cache = cache.borrow_mut();
        let observation = cache.entry(process.pid()).or_insert_with(|| {
            observe_runtime_process(runner, process.pid(), deadline)
                .unwrap_or(ProcessObservation::Unknown)
        });
        observation.matches(process)
    })
}
fn inspect(
    directory: &Path,
    pane: &PaneAddress<'_>,
    binding: Option<&str>,
    deadline: Instant,
    observe: impl Fn(&ProcessIncarnation) -> RuntimeLiveness,
) -> Result<PaneEvidence, EvidenceError> {
    let root = directory.join("codex");
    let mut result = PaneEvidence::default();
    let entries = match fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(result),
        Err(_) => return Err(EvidenceError::at(ChannelFault::Unverifiable, &root)),
    };
    let store = Store::at(directory);
    for entry in entries {
        if Instant::now() >= deadline {
            return Err(EvidenceError::at(ChannelFault::Unverifiable, &root));
        }
        let path = entry
            .map_err(|_| EvidenceError::at(ChannelFault::Unverifiable, &root))?
            .path();
        if path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }
        let id = path.file_stem().and_then(|id| id.to_str());
        let own_binding = id.is_some() && id == binding;
        let record = match id.and_then(|id| store.read(id).ok().flatten()) {
            Some(record) => record,
            None if own_binding => return Err(EvidenceError::at(ChannelFault::InvalidRecord, &path)
                .with_detail(format!("Record {path:?} belongs to this binding. Verify pane {:?} on server {:?} no longer runs the original foreground or owned app-server before removing only that named stale record. Recovery pastes nothing.", pane.pane_id, pane.server.server_id))),
            None => { result.skipped.push(path); continue; }
        };
        let Some(attribution) = &record.attribution else {
            if own_binding {
                return Err(EvidenceError::at(ChannelFault::Unverifiable, &path)
                    .with_detail(recovery(&path, &record)));
            }
            result.skipped.push(path);
            continue;
        };
        if !attribution.matches(pane.server, pane.pane_id, pane.pane_pid) {
            continue;
        }
        let mut processes = vec![&record.launch_owner];
        if let Foreground::Known(foreground) = &record.foreground {
            processes.push(foreground);
        }
        if let Some(ready) = &record.ready {
            processes.push(&ready.server);
        }
        let mut unknown = false;
        for process in processes {
            let process = process
                .incarnation()
                .ok_or_else(|| EvidenceError::at(ChannelFault::InvalidRecord, &path))?;
            match observe(&process) {
                RuntimeLiveness::Alive => {
                    result.enrolled = true;
                    return Ok(result);
                }
                RuntimeLiveness::Unknown => unknown = true,
                RuntimeLiveness::Gone => {}
            }
        }
        if unknown || matches!(record.foreground, Foreground::Unknown) {
            return Err(EvidenceError::at(ChannelFault::Unverifiable, &path)
                .with_detail(recovery(&path, &record)));
        }
        // Positively ended records do not consume an active-record cap. The
        // single absolute deadline bounds the scan, including process queries.
    }
    Ok(result)
}

#[cfg(test)]
mod tests;

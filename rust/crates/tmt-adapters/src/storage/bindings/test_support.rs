use std::path::PathBuf;

use crate::test_support::TestDirectory;
use tmt_core::{
    binding::{Binding, BindingEndpoint},
    endpoint::{EndpointProbe, EndpointSnapshot, PaneObservation, ServerEvidence},
    host::HostKind,
    identity::Identity,
};

use super::super::{Storage, StorageError};

pub(super) struct Fixture {
    _directory: TestDirectory,
    pub(super) database: PathBuf,
}

impl Fixture {
    pub(super) fn new() -> Self {
        let directory = TestDirectory::new();
        Self {
            database: directory.path.join("state").join("tmux-team.db"),
            _directory: directory,
        }
    }

    pub(super) fn open(&self) -> Storage {
        Storage::open(&self.database).unwrap()
    }
}

pub(super) fn pane(id: &str, pid: u64) -> PaneObservation {
    PaneObservation {
        id: id.into(),
        target: None,
        cwd: None,
        command: "agent".into(),
        pane_pid: pid,
        pane_incarnation: None,
        suggested_name: None,
        marker: None,
    }
}

/// A concurrent storage operation runs after evidence acquisition but before
/// the presence reader receives it. No threads or timing are needed.
pub(super) struct DuringProbe<F> {
    pub(super) snapshot: EndpointSnapshot,
    pub(super) on_probe: F,
}

impl<F: FnMut() -> Result<(), StorageError>> BindingEndpoint for DuringProbe<F> {
    type Error = StorageError;

    fn begin_coordination(&mut self) {}
    fn budget_available(&self) -> bool {
        true
    }
    fn current_host(&self) -> HostKind {
        HostKind::Tmux
    }
    fn current_snapshot(&mut self, _: &[String]) -> Result<EndpointSnapshot, Self::Error> {
        let snapshot = self.snapshot.clone();
        (self.on_probe)()?;
        Ok(snapshot)
    }
    fn probe_binding(
        &mut self,
        _: &ServerEvidence,
        panes: &[String],
    ) -> Result<EndpointProbe, Self::Error> {
        self.current_snapshot(panes).map(EndpointProbe::Live)
    }
    fn publish(&mut self, _: &Binding, _: &Identity) -> Result<(), Self::Error> {
        unreachable!("presence reads never publish markers")
    }
    fn clear(&mut self, _: &Binding) -> Result<bool, Self::Error> {
        unreachable!("presence reads never clear markers")
    }
    fn pane_incarnation(
        &mut self,
        _: &ServerEvidence,
        _: u64,
    ) -> Result<Option<String>, Self::Error> {
        unreachable!("presence reads never inspect pane starts")
    }
}

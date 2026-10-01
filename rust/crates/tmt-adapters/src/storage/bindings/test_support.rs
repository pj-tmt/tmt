use std::path::PathBuf;

use crate::test_support::TestDirectory;
use tmt_core::endpoint::PaneObservation;

use super::super::Storage;

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

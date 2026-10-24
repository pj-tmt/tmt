//! A guarded add whose agent-name call fails cleanup after the bind
//! committed: a real script driver approved in a private directory, a
//! private store, and a runner that reports the `name-agent` driver process
//! as not cleaned up. No environment variable or production path changes.

use super::{GuardedRequest, present_guarded, run_guarded};
use crate::invocation::OutputMode;
use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tmt_adapters::{
    config::ConfigPaths,
    driver_protocol::registry,
    host::Host,
    process::{
        CommandError, CommandOutput, CommandRequest, CommandRunner, UnixCommandRunner, ancestry,
        runtime::ProcessObservation,
    },
    storage::Storage,
};
use tmt_core::host::{HostKind, HostName};

/// Answers one pane on `/tmp/fake.sock` whose shell, like its server, is
/// the pid in `pid`; keeps the marker `publish` sent; logs every operation.
const DRIVER: &str = r#"#!/bin/sh
d=$(dirname "$0")
echo "$3" >> "$d/calls"
input=$(cat)
case "$3" in
  capabilities) printf '%s' '{"ok":{"protocols":[1],"kind":"host","name":"fake","version":"0.0.0-test","ops":["server","snapshot","publish","clear","name-agent"],"paneId":{"prefix":"fake-"},"target":"f{n}","callerEnv":[]}}' ;;
  server) printf '{"ok":{"server":{"socket":"/tmp/fake.sock","pid":%s,"startTime":"driver-says"}}}' "$(cat "$d/pid")" ;;
  publish) printf '%s' "$input" | sed -n 's/.*"marker":\({[^}]*}\).*/\1/p' > "$d/marker"; printf '{"ok":{}}' ;;
  name-agent) printf '{"ok":{}}' ;;
  snapshot)
    m=$(cat "$d/marker" 2>/dev/null); [ -n "$m" ] || m=null
    printf '{"ok":{"panes":[{"id":"fake-1","target":"f1","cwd":"/src","command":"sh","panePid":%s,"suggestedName":null,"marker":%s}]}}' "$(cat "$d/pid")" "$m" ;;
  *) printf '{"error":{"code":"unsupported","message":"no %s"}}' "$3" ;;
esac
"#;

/// The production runner, except that a `name-agent` driver call runs and
/// then reports its process as not cleaned up.
#[derive(Clone)]
struct UnreapedNaming;

impl CommandRunner for UnreapedNaming {
    fn process_observation(&self, pid: u64, deadline: Instant) -> Option<ProcessObservation> {
        UnixCommandRunner.process_observation(pid, deadline)
    }

    fn process_parent(&self, pid: u64, deadline: Instant) -> Option<u64> {
        UnixCommandRunner.process_parent(pid, deadline)
    }

    fn process_link(&self, pid: u64, deadline: Instant) -> Option<ancestry::ProcessLink> {
        UnixCommandRunner.process_link(pid, deadline)
    }

    fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
        if !request.args.iter().any(|arg| arg == "name-agent") {
            return UnixCommandRunner.execute(request);
        }
        let _ = UnixCommandRunner.execute(request);
        let mut error = UnixCommandRunner
            .execute(CommandRequest {
                program: OsStr::new("/nonexistent/tmt-guarded-test"),
                args: &[],
                input: b"",
                deadline: Instant::now() + Duration::from_secs(1),
                max_output_bytes: 16,
            })
            .unwrap_err();
        error.cleanup_error = Some(std::io::Error::other("not reaped"));
        Err(error)
    }
}

/// A private directory, removed when dropped.
struct Private(PathBuf);

impl Drop for Private {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn paths(root: &Path) -> ConfigPaths {
    ConfigPaths {
        global_dir: root.join("global"),
        global_config: root.join("global/config.json"),
        local_config: root.join("local.json"),
        database: root.join("global/tmt.db"),
    }
}

fn calls(root: &Path, op: &str) -> usize {
    fs::read_to_string(root.join("driver/calls"))
        .unwrap_or_default()
        .lines()
        .filter(|line| *line == op)
        .count()
}

#[test]
fn a_naming_cleanup_failure_after_the_bind_is_partial_and_never_replayed() {
    let root =
        Private(std::env::temp_dir().join(format!("tmt-guarded-naming-{}", std::process::id())));
    let _ = fs::remove_dir_all(&root.0);
    fs::create_dir_all(root.0.join("driver")).unwrap();
    fs::create_dir_all(root.0.join("global")).unwrap();
    let driver = root.0.join("driver/tmt-driver-fake");
    tmt_test_support::write_executable(&driver, DRIVER.as_bytes(), 0o755).unwrap();
    fs::write(root.0.join("driver/pid"), std::process::id().to_string()).unwrap();
    let paths = paths(&root.0);
    let record = registry::approve(&paths.global_dir, &driver, &UnixCommandRunner).unwrap();
    let admitted = registry::Admitted {
        records: vec![record],
        ..registry::Admitted::default()
    };
    let fake = HostKind::External(HostName::new("fake").unwrap());
    let scan = |runner: UnixCommandRunner| {
        let host = Host::for_host(fake, runner, admitted.records.clone());
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut observation = host
            .discover(Some("/tmp/fake.sock"), None, deadline)
            .unwrap();
        tmt_adapters::discovery::annotate(&mut observation, &paths.database, deadline).unwrap();
        observation.candidates.remove(0)
    };
    let candidate = scan(UnixCommandRunner);
    let expect = candidate.fence.as_ref().unwrap().digest();
    let request = GuardedRequest {
        pane: "fake-1",
        name: Some("worker"),
        identity: None,
        expect: &expect,
        socket: Some("/tmp/fake.sock"),
        save: false,
    };
    let mut partial = false;
    let reads = tmt_adapters::host::external::approval_reads();
    let outcome = run_guarded(
        UnreapedNaming,
        fake,
        &admitted,
        &paths,
        &request,
        &mut partial,
    );
    // Naming used the drivers the command read; it read no approval again.
    assert_eq!(tmt_adapters::host::external::approval_reads(), reads);
    let failure = outcome.as_ref().err().expect("a cleanup failure");
    assert_eq!(failure.code, "CLEANUP_ERROR");
    assert!(partial);
    // One publication and one naming call: nothing was replayed.
    assert_eq!(calls(&root.0, "publish"), 1);
    assert_eq!(calls(&root.0, "name-agent"), 1);
    // The JSON a caller sees, through the command's own presentation.
    let mut printed = Vec::new();
    let mode = OutputMode { json: true };
    let status = present_guarded(&mut printed, mode, "fake-1", outcome, partial).unwrap();
    assert_eq!(status, 1);
    let document: serde_json::Value = serde_json::from_slice(&printed).unwrap();
    assert_eq!(document["error"]["code"], "CLEANUP_ERROR");
    assert_eq!(document["status"], "partial");
    assert_eq!(document["pane"], "fake-1");
    // The binding is durable.
    let mut storage = Storage::open(&paths.database).unwrap();
    let identities = tmt_core::identity::IdentityReader::list_identities(&storage).unwrap();
    assert_eq!(identities.len(), 1);
    storage.close().unwrap();
    // A read-only scan observes the committed identity; only an explicit,
    // freshly fenced UUID request repeats, and it names the agent again.
    let known = scan(UnixCommandRunner);
    assert_eq!(known.state, "known");
    let id = known.identity.as_ref().unwrap().identity.id.clone();
    assert_eq!(id, identities[0].id);
    let fresh = known.fence.as_ref().unwrap().digest();
    let repeat = GuardedRequest {
        name: None,
        identity: Some(&id),
        expect: &fresh,
        ..request
    };
    let mut partial = false;
    let (bound, named) = run_guarded(
        UnixCommandRunner,
        fake,
        &admitted,
        &paths,
        &repeat,
        &mut partial,
    )
    .unwrap();
    assert!(bound.already_bound);
    assert!(named.is_some_and(|outcome| outcome.result.is_ok()));
    assert_eq!(calls(&root.0, "publish"), 1);
    assert_eq!(calls(&root.0, "name-agent"), 2);
}

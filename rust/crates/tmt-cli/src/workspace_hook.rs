//! Private worker observation transport; provider context is published first.

use serde_json::{Value, json};
use std::{io::Write, time::Instant};
use tmt_adapters::{config::ConfigPaths, host::Host, workspace};
use tmt_core::{endpoint::ServerEvidence, host::HostKind};

pub(crate) struct Observation {
    pub context: String,
    /// Only an admitted SessionStart/SessionEnd sets this. It remains recovery
    /// input and cannot bypass the capture adapter's fresh native server fence.
    pub workspace: Option<ServerEvidence>,
}

impl Observation {
    pub fn encode(&self) -> Vec<u8> {
        json!({"version": 1, "context": self.context, "workspace": self.workspace.as_ref().map(|server| json!({"host": server.host.as_str(), "id": server.server_id, "socket": server.socket_path, "pid": server.server_pid, "start": server.server_start_time}))}).to_string().into_bytes()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ()> {
        let value: Value = serde_json::from_slice(bytes).map_err(|_| ())?;
        if value["version"].as_u64() != Some(1) {
            return Err(());
        }
        let context = value["context"].as_str().ok_or(())?.to_owned();
        let workspace = match value.get("workspace") {
            Some(Value::Null) => None,
            Some(server) => Some(ServerEvidence {
                host: HostKind::parse(server["host"].as_str().ok_or(())?).ok_or(())?,
                server_id: server["id"].as_str().ok_or(())?.into(),
                socket_path: server["socket"].as_str().ok_or(())?.into(),
                server_pid: server["pid"].as_u64().ok_or(())?,
                server_start_time: server["start"].as_str().ok_or(())?.into(),
            }),
            None => return Err(()),
        };
        Ok(Self { context, workspace })
    }
}

pub(crate) fn publish(output: &mut impl Write, observation: Observation, deadline: Instant) {
    let workspace = observation.workspace;
    publish_then_capture(output, &observation.context, deadline, || {
        if let Some(server) = workspace
            && let Ok(paths) = ConfigPaths::discover()
        {
            let host = Host::for_server(&server);
            let bound = deadline.min(Instant::now() + workspace::CAPTURE_BUDGET);
            let _ = workspace::capture_event(&paths, &host, Some(&server), None, bound);
        }
    });
}

fn publish_then_capture(
    output: &mut impl Write,
    context: &str,
    deadline: Instant,
    capture: impl FnOnce(),
) {
    // Optional capture can neither withhold already available context nor turn
    // its failure into the lifecycle-unavailable diagnostic.
    if output
        .write_all(context.as_bytes())
        .and_then(|()| output.flush())
        .is_err()
    {
        return;
    }
    let remaining = deadline
        .saturating_duration_since(Instant::now())
        .as_millis();
    if tmt_core::workspace::hook_capture_allowed(
        remaining.try_into().unwrap_or(u64::MAX),
        workspace::HOOK_CAPTURE_BOUND.as_millis() as u64,
        workspace::HOOK_RESERVE.as_millis() as u64,
    ) {
        capture();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{Arc, Mutex, mpsc},
        time::Duration,
    };

    struct Output(Arc<Mutex<(Vec<u8>, bool)>>);
    impl Write for Output {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().0.extend(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.0.lock().unwrap().1 = true;
            Ok(())
        }
    }

    #[test]
    fn blocked_optional_capture_cannot_delay_context_publication() {
        let bytes = Arc::new(Mutex::new((Vec::new(), false)));
        let output = bytes.clone();
        let (entered, ready) = mpsc::channel();
        let (release, gate) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            publish_then_capture(
                &mut Output(output),
                "exact hook context",
                Instant::now() + Duration::from_secs(2),
                || {
                    entered.send(()).unwrap();
                    gate.recv().unwrap();
                },
            )
        });
        ready.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(
            *bytes.lock().unwrap(),
            (b"exact hook context".to_vec(), true)
        );
        release.send(()).unwrap();
        worker.join().unwrap();
    }

    #[test]
    fn insufficient_headroom_skips_capture_and_keeps_context() {
        let mut output = Vec::new();
        publish_then_capture(
            &mut output,
            "context",
            Instant::now() + Duration::from_millis(400),
            || panic!("reserve consumed"),
        );
        assert_eq!(output, b"context");
    }
}

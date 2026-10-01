//! Launcher-only socket lifetime; a separate owner reaps its original server
//! child on launcher EOF, including SIGKILL of the launcher. No PID recovery kill.
use super::{
    attachment::LaunchOptions,
    lease::Lease,
    record::{Attribution, Record, Store},
};
use crate::{
    process::{UnixCommandRunner, runtime::observe_runtime_process},
    runtime::{
        RuntimeCommand,
        channel::{ChannelEnrollment, ChannelPlan, ServeRequest},
    },
};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    fs::File,
    io::{self, BufRead, Read, Write},
    os::{
        fd::{AsFd, OwnedFd},
        unix::{
            ffi::{OsStrExt, OsStringExt},
            net::UnixStream,
        },
    },
    path::PathBuf,
    time::{Duration, Instant},
};
use subprocess::{Exec, ExecExt, Job};
use tmt_core::binding::session::{ProviderSessionId, RuntimeLiveness};
const LIMIT: usize = 64 * 1024;
const START: Duration = Duration::from_secs(15);

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Start {
    executable: Vec<u8>,
    args: Vec<Vec<u8>>,
    cwd: Vec<u8>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Ready {
    executable: Vec<u8>,
    args: Vec<Vec<u8>>,
    environment: Vec<(Vec<u8>, Vec<u8>)>,
    session: String,
}

pub struct Supervisor {
    store: Store,
    record: Record,
    control: Option<UnixStream>,
    job: Option<Job>,
    command: RuntimeCommand,
    environment: Vec<(OsString, OsString)>,
    session: Option<ProviderSessionId>,
}
impl Supervisor {
    pub fn start(plan: &ChannelPlan<'_>) -> io::Result<Self> {
        LaunchOptions::parse(plan.command, plan.working_directory).map_err(|_| invalid())?;
        let deadline = Instant::now() + START;
        let attribution = Attribution::new(
            plan.identity_id,
            plan.pane.server,
            plan.pane.pane_id,
            plan.pane.pane_pid,
        )?;
        let store = Store::open(plan.directory)?;
        store.prune(deadline, |process| {
            observe_runtime_process(&UnixCommandRunner, process.pid(), deadline)
                .map(|observed| observed.matches(process))
                .unwrap_or(RuntimeLiveness::Unknown)
        })?;
        let mut record = Record::new(plan.binding_id, plan.owner)?;
        record.attribution = Some(attribution);
        store.create(&record, |owner| {
            observe_runtime_process(&UnixCommandRunner, owner.pid(), deadline)
                .map(|p| p.matches(owner))
                .unwrap_or(RuntimeLiveness::Unknown)
        })?;
        let start = Start {
            executable: plan.command.executable.as_bytes().to_vec(),
            args: plan
                .command
                .args
                .iter()
                .map(|a| a.as_bytes().to_vec())
                .collect(),
            cwd: plan.working_directory.as_os_str().as_bytes().to_vec(),
        };
        let bytes = serde_json::to_vec(&start).map_err(io::Error::other)?;
        if bytes.len() > LIMIT {
            store.withdraw(&record)?;
            return Err(invalid());
        }
        let launched = (|| {
            let (control, child) = UnixStream::pair()?;
            // std's Unix sockets are CLOEXEC; assert the property before any
            // helper/app-server/foreground exec can inherit the launcher end.
            let flags = nix::fcntl::fcntl(control.as_fd(), nix::fcntl::FcntlArg::F_GETFD)
                .map_err(io::Error::other)?;
            if flags & nix::libc::FD_CLOEXEC == 0 {
                return Err(invalid());
            }
            let input = File::from(OwnedFd::from(child.try_clone()?));
            let output = File::from(OwnedFd::from(child));
            let job = Exec::cmd(plan.tmt)
                .args([
                    OsString::from("__channel-server"),
                    "codex".into(),
                    record.binding_id.clone().into(),
                    record.generation.clone().into(),
                    plan.directory.as_os_str().into(),
                ])
                .setpgid()
                .stdin(input)
                .stdout(output)
                .start()?;
            Ok((control, job))
        })();
        let (control, job) = match launched {
            Ok(value) => value,
            Err(error) => {
                store.withdraw(&record)?;
                return Err(error);
            }
        };
        let mut result = Self {
            store,
            record: record.clone(),
            control: Some(control),
            job: Some(job),
            command: plan.command.clone(),
            environment: vec![],
            session: None,
        };
        let control = result.control.as_mut().expect("owned socket assigned");
        write_frame(control, &bytes, deadline)?;
        let ready: Ready =
            serde_json::from_slice(&read_frame(control, deadline)?).map_err(|_| invalid())?;
        let session = ProviderSessionId::new(&ready.session).map_err(|_| invalid())?;
        let persisted = result.store.read(plan.binding_id)?.ok_or_else(invalid)?;
        if persisted.generation != record.generation
            || persisted.launch_owner != record.launch_owner
            || persisted.ready.as_ref().map(|r| r.thread.as_str()) != Some(session.as_str())
        {
            return Err(invalid());
        }
        result.command = RuntimeCommand {
            executable: OsString::from_vec(ready.executable),
            args: ready.args.into_iter().map(OsString::from_vec).collect(),
        };
        result.environment = ready
            .environment
            .into_iter()
            .map(|(k, v)| (OsString::from_vec(k), OsString::from_vec(v)))
            .collect();
        result.session = Some(session);
        Ok(result)
    }
    pub fn foreground_started(
        &mut self,
        process: &tmt_core::endpoint::ProcessIncarnation,
    ) -> io::Result<()> {
        self.record = self.store.foreground(&self.record, process)?;
        Ok(())
    }
    fn stop(&mut self, retire: bool) -> io::Result<()> {
        if retire && let Some(control) = &mut self.control {
            // This explicit event is allowed only after no-child/confirmed reap.
            control.set_write_timeout(Some(Duration::from_secs(1)))?;
            control.write_all(b"W")?;
        }
        self.control.take(); // sole launcher endpoint: EOF is the teardown request.
        let Some(job) = self.job.take() else {
            return Ok(());
        };
        match job.wait_timeout(START + Duration::from_secs(5)) {
            Ok(Some(status)) if status.success() => Ok(()),
            Ok(Some(_)) => Err(io::Error::other(
                "Codex supervisor failed; enrollment evidence is retained unless cleanup was confirmed",
            )),
            _ => {
                job.detach();
                Err(io::Error::other(
                    "Codex supervisor cleanup unconfirmed; enrollment retained",
                ))
            }
        }
    }
}
impl ChannelEnrollment for Supervisor {
    fn foreground_started(
        &mut self,
        foreground: &tmt_core::endpoint::ProcessIncarnation,
    ) -> Result<(), crate::runtime::channel::ChannelError> {
        Supervisor::foreground_started(self, foreground)
            .map_err(|_| crate::runtime::channel::ChannelError::Enrollment)
    }
    fn command(&self) -> &RuntimeCommand {
        &self.command
    }
    fn environment(&self) -> &[(OsString, OsString)] {
        &self.environment
    }
    fn provider_session(&self) -> Option<&ProviderSessionId> {
        self.session.as_ref()
    }
    fn withdraw(mut self: Box<Self>) {
        if let Err(error) = self.stop(true) {
            eprintln!("Codex supervisor: {error}");
        }
    }
}
impl Drop for Supervisor {
    fn drop(&mut self) {
        if let Err(error) = self.stop(false) {
            eprintln!("Codex supervisor: {error}");
        }
    }
}

pub fn serve(
    request: &ServeRequest<'_>,
    mut input: Box<dyn BufRead + Send>,
    output: &mut dyn Write,
) -> io::Result<()> {
    let store = Store::open(request.directory)?;
    let record = store.read(request.binding_id)?.ok_or_else(invalid)?;
    if record.generation != request.generation || record.ready.is_some() {
        return Err(invalid());
    }
    let config = (|| {
        let mut line = Vec::new();
        input
            .by_ref()
            .take((LIMIT + 1) as u64)
            .read_until(b'\n', &mut line)?;
        if line.len() > LIMIT || line.last() != Some(&b'\n') {
            return Err(invalid());
        }
        serde_json::from_slice::<Start>(&line).map_err(|_| invalid())
    })();
    let start = match config {
        Ok(value) => value,
        Err(error) => {
            store.withdraw(&record)?;
            return Err(error);
        }
    };
    let command = RuntimeCommand {
        executable: OsString::from_vec(start.executable),
        args: start.args.into_iter().map(OsString::from_vec).collect(),
    };
    let cwd = PathBuf::from(OsString::from_vec(start.cwd));
    let mut lease = Lease::from_record(store, record, &command, &cwd, Instant::now() + START)?;
    let ready = Ready {
        executable: lease.command().executable.as_bytes().to_vec(),
        args: lease
            .command()
            .args
            .iter()
            .map(|a| a.as_bytes().to_vec())
            .collect(),
        environment: lease
            .environment()
            .iter()
            .map(|(k, v)| (k.as_bytes().to_vec(), v.as_bytes().to_vec()))
            .collect(),
        session: lease.session().ok_or_else(invalid)?.as_str().into(),
    };
    let bytes = serde_json::to_vec(&ready).map_err(io::Error::other)?;
    if bytes.len() > LIMIT {
        return Err(invalid());
    }
    output.write_all(&bytes)?;
    output.write_all(b"\n")?;
    output.flush()?;
    // No model/queue command travels on this lifetime channel. EOF from the
    // launcher, not app-server status or a later PID observation, owns shutdown.
    let mut extra = [0u8; 1];
    match input.read(&mut extra) {
        Ok(0) => Ok(()), // Drop reaps the server but retains foreground evidence.
        Ok(1) if extra[0] == b'W' => lease.withdraw(),
        Ok(_) => Err(invalid()),
        Err(error) => Err(error),
    }
}
fn write_frame(stream: &mut UnixStream, bytes: &[u8], deadline: Instant) -> io::Result<()> {
    let mut frame = bytes.to_vec();
    frame.push(b'\n');
    let mut remaining = frame.as_slice();
    while !remaining.is_empty() {
        stream.set_write_timeout(Some(budget(deadline)?))?;
        let n = stream.write(remaining)?;
        if n == 0 {
            return Err(io::ErrorKind::WriteZero.into());
        }
        remaining = &remaining[n..];
    }
    Ok(())
}
fn read_frame(stream: &mut UnixStream, deadline: Instant) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut byte = [0u8; 1];
    while bytes.len() <= LIMIT {
        stream.set_read_timeout(Some(budget(deadline)?))?;
        if stream.read(&mut byte)? == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        if byte[0] == b'\n' {
            return Ok(bytes);
        }
        bytes.push(byte[0]);
    }
    Err(invalid())
}
fn budget(deadline: Instant) -> io::Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|v| !v.is_zero())
        .ok_or_else(|| io::ErrorKind::TimedOut.into())
}
fn invalid() -> io::Error {
    io::Error::other("Invalid Codex supervisor handshake")
}

#[cfg(test)]
mod tests;

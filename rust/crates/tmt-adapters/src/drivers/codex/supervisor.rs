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
    resume_session: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Ready {
    executable: Vec<u8>,
    args: Vec<Vec<u8>>,
    environment: Vec<(Vec<u8>, Vec<u8>)>,
    session: Option<String>,
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
        LaunchOptions::for_launch(plan.command, plan.working_directory, plan.resume_session)
            .map_err(|_| invalid())?;
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
        let start = Start {
            executable: plan.command.executable.as_bytes().to_vec(),
            args: plan
                .command
                .args
                .iter()
                .map(|a| a.as_bytes().to_vec())
                .collect(),
            cwd: plan.working_directory.as_os_str().as_bytes().to_vec(),
            resume_session: plan
                .resume_session
                .map(|session| session.as_str().to_owned()),
        };
        let bytes = serde_json::to_vec(&start).map_err(io::Error::other)?;
        if bytes.len() > LIMIT {
            return Err(invalid());
        }
        store.create(&record, |owner| {
            observe_runtime_process(&UnixCommandRunner, owner.pid(), deadline)
                .map(|p| p.matches(owner))
                .unwrap_or(RuntimeLiveness::Unknown)
        })?;
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
        result.initialize(&bytes, deadline)?;
        Ok(result)
    }

    fn initialize(&mut self, bytes: &[u8], deadline: Instant) -> io::Result<()> {
        let initialized = (|| {
            let control = self.control.as_mut().expect("owned socket assigned");
            write_frame(control, bytes, deadline)?;
            let ready: Ready =
                serde_json::from_slice(&read_frame(control, deadline)?).map_err(|_| invalid())?;
            let session = ready
                .session
                .as_deref()
                .map(ProviderSessionId::new)
                .transpose()
                .map_err(|_| invalid())?;
            let persisted = self
                .store
                .read(&self.record.binding_id)?
                .ok_or_else(invalid)?;
            if persisted.generation != self.record.generation
                || persisted.launch_owner != self.record.launch_owner
                || persisted.ready.as_ref().map(|r| r.thread.as_str())
                    != session.as_ref().map(ProviderSessionId::as_str)
                || (session.is_none() && persisted.fresh.is_none())
            {
                return Err(invalid());
            }
            self.command = RuntimeCommand {
                executable: OsString::from_vec(ready.executable),
                args: ready.args.into_iter().map(OsString::from_vec).collect(),
            };
            self.environment = ready
                .environment
                .into_iter()
                .map(|(k, v)| (OsString::from_vec(k), OsString::from_vec(v)))
                .collect();
            self.session = session;
            Ok(())
        })();
        if initialized.is_err() {
            // No foreground command has escaped enrollment. Explicitly retire;
            // EOF alone is reserved for a potentially surviving foreground.
            if let Err(error) = self.stop(true) {
                eprintln!("Codex supervisor startup cleanup: {error}");
            }
        }
        initialized
    }
    pub fn foreground_started(
        &mut self,
        process: &tmt_core::endpoint::ProcessIncarnation,
    ) -> io::Result<()> {
        self.record = self.store.foreground(&self.record, process)?;
        if self.record.fresh.is_some() {
            let session = discover_fresh(
                &self.store,
                &self.record,
                Instant::now() + super::MAXIMUM_FRESH_DISCOVERY_DURATION,
            )?;
            self.record = self.store.fresh_thread(&self.record, session.as_str())?;
            self.session = Some(session);
        }
        Ok(())
    }
    fn stop(&mut self, retire: bool) -> io::Result<()> {
        let retirement = if retire && let Some(control) = &mut self.control {
            // A failed write is not retirement acknowledgement. Still close
            // the socket and wait for our own supervisor to finish cleanup.
            control
                .set_write_timeout(Some(Duration::from_secs(1)))
                .and_then(|()| control.write_all(b"W"))
        } else {
            Ok(())
        };
        self.control.take(); // sole launcher endpoint: EOF is the teardown request.
        let Some(job) = self.job.take() else {
            return retirement;
        };
        match job.wait_timeout(START + Duration::from_secs(5)) {
            Ok(Some(status)) if status.success() => retirement,
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
    fn foreground_admitted(
        &mut self,
        foreground: &tmt_core::endpoint::ProcessIncarnation,
    ) -> Result<(), crate::runtime::channel::ChannelError> {
        if self.record.fresh.is_some() {
            let deadline = Instant::now() + super::MAXIMUM_FRESH_DISCOVERY_DURATION;
            self.record = self
                .store
                .admit_fresh(&self.record, foreground, |p| {
                    observe_runtime_process(&UnixCommandRunner, p.pid(), deadline)
                        .map(|value| value.matches(p))
                        .unwrap_or(RuntimeLiveness::Unknown)
                })
                .map_err(|_| crate::runtime::channel::ChannelError::Enrollment)?;
        }
        Ok(())
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
    let resume_session = match start
        .resume_session
        .as_deref()
        .map(ProviderSessionId::new)
        .transpose()
    {
        Ok(session) => session,
        Err(_) => {
            store.withdraw(&record)?;
            return Err(invalid());
        }
    };
    let mut lease = Lease::from_record(
        store,
        record,
        &command,
        &cwd,
        resume_session.as_ref(),
        Instant::now() + START,
    )?;
    let published = (|| {
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
            session: lease.session().map(|s| s.as_str().to_owned()),
        };
        let bytes = serde_json::to_vec(&ready).map_err(io::Error::other)?;
        if bytes.len() > LIMIT {
            return Err(invalid());
        }
        output.write_all(&bytes)?;
        output.write_all(b"\n")?;
        Ok(())
    })();
    if let Err(error) = published {
        // A failed Ready publication cannot have returned a foreground command.
        lease.withdraw()?;
        return Err(error);
    }
    // Once the complete frame is written, it may already have reached the
    // launcher. A later flush error cannot prove that no foreground exists.
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
/// The one-time pre-first-turn gate. Never called again after binding, even
/// when the provider creates auxiliary ephemeral threads later.
fn discover_fresh(
    store: &Store,
    record: &Record,
    deadline: Instant,
) -> io::Result<ProviderSessionId> {
    use super::transport::{Client, Endpoint};
    use serde_json::json;
    let fresh = record.fresh.as_ref().ok_or_else(invalid)?;
    let token = super::delivery::capability(store, record).map_err(|_| invalid())?;
    let endpoint = Endpoint::new(
        std::net::SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, fresh.port),
        token,
    )
    .map_err(|_| invalid())?;
    let mut client = Client::connect(&endpoint, deadline).map_err(|_| invalid())?;
    loop {
        let id = uuid::Uuid::new_v4().to_string();
        let response = client
            .call(
                &json!({"id":id,"method":"thread/loaded/list","params":{}}),
                &id,
            )
            .map_err(|_| invalid())?;
        if let Some(session) = loaded_thread(&response)? {
            let id = uuid::Uuid::new_v4().to_string();
            let response = client.call(&json!({"id":id,"method":"thread/read","params":{"threadId":session.as_str(),"includeTurns":false}}), &id).map_err(|_| invalid())?;
            verify_thread(&response, &session, &fresh.cwd)?;
            return Ok(session);
        }
        let remaining = budget(deadline)?;
        std::thread::sleep(remaining.min(Duration::from_millis(10)));
    }
}
fn loaded_thread(response: &serde_json::Value) -> io::Result<Option<ProviderSessionId>> {
    if response.get("error").is_some() {
        return Err(invalid());
    }
    let data = response
        .pointer("/result/data")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(invalid)?;
    match data.as_slice() {
        [] => Ok(None),
        [id] => {
            let id = id.as_str().ok_or_else(invalid)?;
            uuid::Uuid::parse_str(id).map_err(|_| invalid())?;
            ProviderSessionId::new(id).map(Some).map_err(|_| invalid())
        }
        _ => Err(invalid()),
    }
}
fn verify_thread(
    response: &serde_json::Value,
    session: &ProviderSessionId,
    cwd: &std::path::Path,
) -> io::Result<()> {
    use serde_json::Value;
    if response.get("error").is_some() {
        return Err(invalid());
    }
    let thread = response.pointer("/result/thread").ok_or_else(invalid)?;
    if thread.get("id").and_then(Value::as_str) != Some(session.as_str())
        || thread.get("ephemeral").and_then(Value::as_bool) != Some(false)
        || thread
            .get("cwd")
            .and_then(Value::as_str)
            .map(std::path::Path::new)
            .is_none_or(|observed| observed != cwd)
    {
        return Err(invalid());
    }
    Ok(())
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

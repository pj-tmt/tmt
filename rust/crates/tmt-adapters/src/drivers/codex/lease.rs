//! Compose one opt-in record, owned endpoint and exact foreground thread.
//! The launcher remains the sole owner of binding admission authority.

use super::{
    attachment::{LaunchOptions, TOKEN_ENV},
    record::{Process, Ready, Record, Store},
    server::{OwnedServer, StartError},
    transport::Client,
};
use crate::{
    process::{UnixCommandRunner, runtime::observe_runtime_process},
    runtime::RuntimeCommand,
};
use serde_json::{Value, json};
use std::{ffi::OsString, io, path::Path, time::Instant};
use tmt_core::{
    binding::session::{ProviderSessionId, RuntimeLiveness},
    endpoint::ProcessIncarnation,
};

pub struct Lease {
    store: Store,
    record: Record,
    server: Option<OwnedServer>,
    retain_enrollment: bool,
    command: RuntimeCommand,
    environment: Vec<(OsString, OsString)>,
    session: Option<ProviderSessionId>,
}

impl Lease {
    /// Caller must validate the claimed binding/session and this live launch
    /// incarnation through the existing atomic launcher authority before entry.
    pub fn start(
        binding: &str,
        owner: &ProcessIncarnation,
        command: &RuntimeCommand,
        cwd: &Path,
        directory: &Path,
        deadline: Instant,
    ) -> io::Result<Self> {
        let options = LaunchOptions::parse(command, cwd).map_err(|_| invalid())?;
        let store = Store::open(directory)?;
        let record = Record::new(binding, owner)?;
        store.create(&record, |process| {
            observe_runtime_process(&UnixCommandRunner, process.pid(), deadline)
                .map(|observation| observation.matches(process))
                .unwrap_or(RuntimeLiveness::Unknown)
        })?;
        // From the first durable opt-in write, every later failure is owned by
        // this guard. Nothing depends on preferences.launched or thread readiness.
        let mut lease = Self {
            store,
            record,
            server: None,
            retain_enrollment: false,
            command: command.clone(),
            environment: Vec::new(),
            session: None,
        };
        let generation = lease.store.generation_directory(&lease.record)?;
        lease.accept_start(OwnedServer::start(command, &options, &generation, deadline))?;
        let server = lease.server.as_ref().expect("owned server just assigned");
        let mut client = Client::connect(&server.endpoint, deadline).map_err(|_| invalid())?;
        let id = uuid::Uuid::new_v4().to_string();
        let response = client
            .call(
                &json!({"id":id,"method":"thread/start","params":{
                    "cwd":options.working_directory(), "allowProviderModelFallback":false
                }}),
                &id,
            )
            .map_err(|_| invalid())?;
        let session = created_thread(&response, options.working_directory())?;
        lease.command = options
            .foreground(command, &server.endpoint.url(), &session)
            .map_err(|_| invalid())?;
        lease
            .environment
            .push((TOKEN_ENV.into(), server.capability().into()));
        lease.record = lease.store.ready(
            &lease.record,
            Ready {
                server: Process::of(&server.incarnation),
                port: server.endpoint.port(),
                thread: session.as_str().into(),
            },
        )?;
        lease.session = Some(session);
        Ok(lease)
    }

    fn accept_start(&mut self, result: Result<OwnedServer, StartError>) -> io::Result<()> {
        match result {
            Ok(server) => {
                self.server = Some(server);
                Ok(())
            }
            Err(error) => {
                self.retain_enrollment = !error.cleanup_confirmed();
                Err(io::Error::other(error))
            }
        }
    }

    pub fn command(&self) -> &RuntimeCommand {
        &self.command
    }
    pub fn environment(&self) -> &[(OsString, OsString)] {
        &self.environment
    }
    pub fn session(&self) -> Option<&ProviderSessionId> {
        self.session.as_ref()
    }

    pub fn withdraw(&mut self) -> io::Result<()> {
        // Retain enrollment if process cleanup cannot be proven. A missing
        // record must never disguise an orphaned opted-in endpoint as baseline.
        if self.retain_enrollment {
            return Err(io::Error::other(
                "Codex enrollment retained after unconfirmed startup cleanup",
            ));
        }
        if let Some(server) = &mut self.server {
            server.stop()?;
        }
        self.server = None;
        self.store.withdraw(&self.record)?;
        Ok(())
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        if let Err(error) = self.withdraw() {
            eprintln!("Codex enrollment cleanup failed: {error}");
        }
    }
}

fn created_thread(response: &Value, cwd: &Path) -> io::Result<ProviderSessionId> {
    if response.get("error").is_some() {
        return Err(invalid());
    }
    let result = response.get("result").ok_or_else(invalid)?;
    let returned_cwd = result
        .get("cwd")
        .and_then(Value::as_str)
        .ok_or_else(invalid)?;
    if Path::new(returned_cwd).canonicalize()? != cwd.canonicalize()? {
        return Err(invalid());
    }
    let id = result
        .pointer("/thread/id")
        .and_then(Value::as_str)
        .ok_or_else(invalid)?;
    uuid::Uuid::parse_str(id).map_err(|_| invalid())?;
    ProviderSessionId::new(id).map_err(|_| invalid())
}
fn invalid() -> io::Error {
    io::Error::other("Codex owned thread enrollment could not be established")
}

#[cfg(test)]
mod tests;

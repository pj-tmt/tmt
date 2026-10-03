//! Compose one opt-in record, owned endpoint and exact foreground thread.
//! The launcher remains the sole owner of binding admission authority.

use super::{
    attachment::{LaunchOptions, TOKEN_ENV},
    record::{FreshEndpoint, Process, Ready, Record, Store},
    server::{OwnedServer, StartError},
    transport::Client,
};
use crate::runtime::RuntimeCommand;
use serde_json::{Value, json};
use std::{ffi::OsString, io, path::Path, time::Instant};
use tmt_core::binding::session::ProviderSessionId;

enum EnrollmentCleanup {
    Retire,
    PreserveForeground,
    UnconfirmedStartup,
}

pub struct Lease {
    store: Store,
    record: Record,
    server: Option<OwnedServer>,
    cleanup: EnrollmentCleanup,
    command: RuntimeCommand,
    environment: Vec<(OsString, OsString)>,
    session: Option<ProviderSessionId>,
}

impl Lease {
    pub(super) fn from_record(
        store: Store,
        record: Record,
        command: &RuntimeCommand,
        cwd: &Path,
        resume_session: Option<&ProviderSessionId>,
        deadline: Instant,
    ) -> io::Result<Self> {
        // From the first durable opt-in write, every later failure is owned by
        // this guard. Nothing depends on preferences.launched or thread readiness.
        let mut lease = Self {
            store,
            record,
            server: None,
            cleanup: EnrollmentCleanup::Retire,
            command: command.clone(),
            environment: Vec::new(),
            session: None,
        };
        let options =
            LaunchOptions::for_launch(command, cwd, resume_session).map_err(|_| invalid())?;
        let generation = lease.store.generation_directory(&lease.record)?;
        lease.accept_start(OwnedServer::start_with_environment(
            command,
            &options,
            &generation,
            deadline,
            &super::channel_context::environment(&lease.record),
        ))?;
        let server = lease.server.as_ref().expect("owned server just assigned");
        let mut client = Client::connect(&server.endpoint, deadline).map_err(|_| invalid())?;
        if let Some(expected) = resume_session {
            let id = uuid::Uuid::new_v4().to_string();
            let response = client.call(&json!({"id":id,"method":"thread/resume", "params":options.thread_resume_params(expected)}), &id).map_err(|_| invalid())?;
            let session = created_thread(&response, options.working_directory())?;
            if expected != &session {
                return Err(invalid());
            }
            lease.command = options
                .foreground(command, &server.endpoint.url(), &session)
                .map_err(|_| invalid())?;
            lease.record = lease.store.ready(
                &lease.record,
                Ready {
                    server: Process::of(&server.incarnation),
                    port: server.endpoint.port(),
                    thread: session.as_str().into(),
                },
            )?;
            lease.session = Some(session);
        } else {
            let id = uuid::Uuid::new_v4().to_string();
            let response = client
                .call(
                    &json!({"id":id,"method":"thread/loaded/list","params":{}}),
                    &id,
                )
                .map_err(|_| invalid())?;
            if response.get("error").is_some()
                || response
                    .pointer("/result/data")
                    .and_then(Value::as_array)
                    .is_none_or(|data| !data.is_empty())
            {
                return Err(invalid());
            }
            lease.command = options
                .fresh_foreground(command, &server.endpoint.url())
                .map_err(|_| invalid())?;
            lease.record = lease.store.fresh_endpoint(
                &lease.record,
                FreshEndpoint {
                    server: Process::of(&server.incarnation),
                    port: server.endpoint.port(),
                    cwd: options.working_directory().canonicalize()?,
                    thread: None,
                },
            )?;
        }
        lease.environment = super::channel_context::environment(&lease.record);
        lease
            .environment
            .push((TOKEN_ENV.into(), server.capability().into()));
        // A planned foreground may exist as soon as the command is returned.
        // EOF/withdraw is not evidence that it ended.
        lease.cleanup = EnrollmentCleanup::PreserveForeground;
        Ok(lease)
    }

    fn accept_start(&mut self, result: Result<OwnedServer, StartError>) -> io::Result<()> {
        match result {
            Ok(server) => {
                self.server = Some(server);
                Ok(())
            }
            Err(error) => {
                if !error.cleanup_confirmed() {
                    self.cleanup = EnrollmentCleanup::UnconfirmedStartup;
                }
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
        self.stop(true)
    }

    fn stop(&mut self, retire: bool) -> io::Result<()> {
        // Retain enrollment if process cleanup cannot be proven. A missing
        // record must never disguise an orphaned opted-in endpoint as baseline.
        if matches!(self.cleanup, EnrollmentCleanup::UnconfirmedStartup) {
            return Err(io::Error::other(
                "Codex enrollment retained after unconfirmed startup cleanup",
            ));
        }
        if let Some(server) = &mut self.server {
            server.stop()?;
        }
        self.server = None;
        if retire || matches!(self.cleanup, EnrollmentCleanup::Retire) {
            self.store.withdraw(&self.record)?;
        }
        Ok(())
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        if let Err(error) = self.stop(false) {
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

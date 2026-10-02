//! The Herdr host driver: Herdr's CLI behind the driver protocol
//! (contracts/driver-protocol-v1.md). It reports what Herdr and `ps` show;
//! core decides what any of it means.

mod herdr;
pub mod marker;
mod panes;
pub mod run;

use herdr::{Herdr, HerdrError};
use panes::{Listed, valid_pid};
use run::{Runner, deadline};
use tmt_driver_protocol::{
    CallerPane, CallerRequest, CallerResponse, Capabilities, CaptureRequest, CaptureResponse,
    ClearRequest, ClearResponse, DriverError, ErrorCode, Grammar, Handler, InputRequest, Pane,
    PaneIdSyntax, PromptRequest, PublishRequest, Request, ResolveTargetRequest,
    ResolveTargetResponse, ServerIncarnation, ServerRequest, ServerResponse, SnapshotRequest,
    SnapshotResponse,
};

/// The most panes one snapshot reports, as the contract bounds it.
const MAX_PANES: usize = 4096;

/// What the driver declares. `input` takes one line: Herdr types text raw,
/// so a line break would submit early, and such text is refused before any
/// effect. Messages reach agent panes through `prompt`, which Herdr submits
/// whole. `focus` is not
/// declared: Herdr has no command that focuses a pane by its ID.
pub fn capabilities() -> Capabilities {
    Capabilities {
        protocols: vec![tmt_driver_protocol::PROTOCOL],
        kind: "host".into(),
        name: "herdr".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        ops: [
            "caller",
            "server",
            "resolve-target",
            "snapshot",
            "publish",
            "clear",
            "capture",
            "input",
            "prompt",
        ]
        .map(String::from)
        .into(),
        pane_id: PaneIdSyntax {
            prefix: "term_".into(),
        },
        target: Some("w{n}:p{n}".into()),
        caller_env: vec!["HERDR_PANE_ID".into(), "HERDR_SOCKET_PATH".into()],
    }
}

pub struct HerdrDriver<R> {
    herdr: Herdr<R>,
    grammar: Grammar,
}

impl<R: Runner> HerdrDriver<R> {
    pub fn new(runner: R) -> Self {
        Self {
            herdr: Herdr::new(runner),
            grammar: Grammar::from_capabilities(&capabilities())
                .expect("the Herdr driver declares a valid grammar"),
        }
    }

    fn socket<'a>(&self, socket: &'a str) -> Result<&'a str, DriverError> {
        if socket.starts_with('/') && !socket.chars().any(char::is_control) {
            Ok(socket)
        } else {
            Err(bad_request("the socket is not an absolute path"))
        }
    }

    fn pane_id<'a>(&self, pane_id: &'a str) -> Result<&'a str, DriverError> {
        if self.grammar.is_pane_id(pane_id) {
            Ok(pane_id)
        } else {
            Err(bad_request("the pane ID is not a Herdr terminal ID"))
        }
    }

    /// The listed pane with this terminal ID; Herdr's commands take its
    /// target, which may change as panes move.
    fn find(
        &self,
        socket: &str,
        pane_id: &str,
        deadline: std::time::Instant,
    ) -> Result<Option<Listed>, HerdrError> {
        Ok(self
            .herdr
            .list(&self.grammar, socket, deadline)?
            .into_iter()
            .find(|pane| pane.id == pane_id))
    }
}

fn bad_request(message: &str) -> DriverError {
    DriverError::new(ErrorCode::BadRequest, message)
}

impl<R: Runner> Handler for HerdrDriver<R> {
    fn capabilities(&mut self) -> Capabilities {
        capabilities()
    }

    fn caller(&mut self, request: Request<CallerRequest>) -> Result<CallerResponse, DriverError> {
        let deadline = deadline(request.deadline_ms);
        let env = &request.body.env;
        let (Some(target), Some(socket)) = (env.get("HERDR_PANE_ID"), env.get("HERDR_SOCKET_PATH"))
        else {
            return Ok(CallerResponse { pane: None });
        };
        if !self.grammar.is_target(target) || self.socket(socket).is_err() {
            return Ok(CallerResponse { pane: None });
        }
        let Some(socket) = self
            .herdr
            .running(Some(socket), deadline)
            .map_err(HerdrError::into_driver)?
        else {
            return Ok(CallerResponse { pane: None });
        };
        let pane = (|| {
            let Some(pane) = self.herdr.get(&self.grammar, &socket, target, deadline)? else {
                return Ok(None);
            };
            Ok(self
                .herdr
                .process(&socket, &pane.target, deadline)?
                .map(|process| CallerPane {
                    id: pane.id,
                    socket: socket.clone(),
                    shell_pid: process.shell_pid,
                }))
        })()
        .map_err(HerdrError::into_driver)?;
        Ok(CallerResponse { pane })
    }

    fn server(&mut self, request: Request<ServerRequest>) -> Result<ServerResponse, DriverError> {
        let deadline = deadline(request.deadline_ms);
        if let Some(socket) = &request.body.socket {
            self.socket(socket)?;
        }
        let server = (|| {
            let Some(socket) = self
                .herdr
                .running(request.body.socket.as_deref(), deadline)?
            else {
                return Ok(None);
            };
            // Herdr does not report its pid; any pane's shell is its child.
            // A server without a pane has nothing to prove its process with,
            // and nothing to bind.
            for pane in self.herdr.list(&self.grammar, &socket, deadline)? {
                let Some(process) = self.herdr.process(&socket, &pane.target, deadline)? else {
                    continue;
                };
                if let Some((pid, start_time)) =
                    self.herdr.server_process(process.shell_pid, deadline)?
                {
                    return Ok(Some(ServerIncarnation {
                        socket,
                        pid,
                        start_time,
                    }));
                }
            }
            Ok(None)
        })()
        .map_err(HerdrError::into_driver)?;
        Ok(ServerResponse { server })
    }

    fn resolve_target(
        &mut self,
        request: Request<ResolveTargetRequest>,
    ) -> Result<ResolveTargetResponse, DriverError> {
        let deadline = deadline(request.deadline_ms);
        let socket = self.socket(&request.body.socket)?;
        if !self.grammar.is_target(&request.body.target) {
            return Err(bad_request("the target is not a Herdr pane name"));
        }
        let pane = self
            .herdr
            .get(&self.grammar, socket, &request.body.target, deadline)
            .map_err(HerdrError::into_driver)?;
        Ok(ResolveTargetResponse {
            pane_id: pane.map(|pane| pane.id),
        })
    }

    fn snapshot(
        &mut self,
        request: Request<SnapshotRequest>,
    ) -> Result<SnapshotResponse, DriverError> {
        let deadline = deadline(request.deadline_ms);
        let socket = self.socket(&request.body.socket)?;
        if let Some(scope) = &request.body.panes
            && (scope.len() > MAX_PANES || scope.iter().any(|id| self.pane_id(id).is_err()))
        {
            return Err(bad_request("the pane scope is invalid"));
        }
        let panes = (|| {
            let listed = self.herdr.list(&self.grammar, socket, deadline)?;
            let selected: Vec<Listed> = listed
                .into_iter()
                .filter(|pane| {
                    request
                        .body
                        .panes
                        .as_ref()
                        .is_none_or(|scope| scope.contains(&pane.id))
                })
                .collect();
            if selected.len() > MAX_PANES {
                return Err(HerdrError::Malformed(
                    "Herdr lists more panes than a snapshot holds",
                ));
            }
            let mut panes = Vec::with_capacity(selected.len());
            for pane in selected {
                // A pane that closed after the list is simply absent.
                let Some(process) = self.herdr.process(socket, &pane.target, deadline)? else {
                    continue;
                };
                panes.push(Pane {
                    marker: marker::decode(pane.tokens.as_ref()),
                    id: pane.id,
                    target: Some(pane.target),
                    cwd: pane.cwd,
                    command: process.command,
                    pane_pid: process.shell_pid,
                    // Core suggests names from the command itself.
                    suggested_name: None,
                });
            }
            Ok(panes)
        })()
        .map_err(HerdrError::into_driver)?;
        Ok(SnapshotResponse { panes })
    }

    fn publish(&mut self, request: Request<PublishRequest>) -> Result<(), DriverError> {
        let deadline = deadline(request.deadline_ms);
        let PublishRequest {
            socket,
            pane_id,
            pane_pid,
            marker: expected,
        } = &request.body;
        let socket = self.socket(socket)?;
        let pane_id = self.pane_id(pane_id)?;
        if !valid_pid(*pane_pid) {
            return Err(bad_request("the pane pid is invalid"));
        }
        let tokens = marker::encode(expected)
            .ok_or_else(|| bad_request("the name is too long to mark a Herdr pane"))?;
        let gone = || DriverError::new(ErrorCode::NotFound, "the pane no longer runs that shell");
        let pane = self
            .find(socket, pane_id, deadline)
            .map_err(HerdrError::into_driver)?
            .ok_or_else(gone)?;
        let process = self
            .herdr
            .process(socket, &pane.target, deadline)
            .map_err(HerdrError::into_driver)?;
        if process.map(|process| process.shell_pid) != Some(*pane_pid) {
            return Err(gone());
        }
        let mut args = vec![
            "pane",
            "report-metadata",
            pane.target.as_str(),
            "--source",
            marker::SOURCE,
        ];
        for token in &tokens {
            args.extend(["--token", token.as_str()]);
        }
        // One report sets this marker and clears the parts it does not use.
        let unset = marker::unset_keys(&tokens);
        for key in &unset {
            args.extend(["--clear-token", key.as_str()]);
        }
        self.herdr
            .act(socket, &args, deadline)
            .map_err(HerdrError::into_driver)?;
        // Herdr drops a report it considers stale without an error; read back.
        let written = self
            .find(socket, pane_id, deadline)
            .map_err(HerdrError::into_driver)?
            .and_then(|pane| marker::decode(pane.tokens.as_ref()));
        if written.as_ref() != Some(expected) {
            return Err(DriverError::new(
                ErrorCode::Failed,
                "Herdr did not keep the TMT marker",
            ));
        }
        Ok(())
    }

    fn clear(&mut self, request: Request<ClearRequest>) -> Result<ClearResponse, DriverError> {
        let deadline = deadline(request.deadline_ms);
        let socket = self.socket(&request.body.socket)?;
        let pane_id = self.pane_id(&request.body.pane_id)?;
        let pane = match self.find(socket, pane_id, deadline) {
            Ok(Some(pane)) => pane,
            Ok(None) => return Ok(ClearResponse { cleared: false }),
            Err(error) => return Err(error.into_driver()),
        };
        if marker::binding_id(pane.tokens.as_ref()) != Some(request.body.binding_id.as_str()) {
            return Ok(ClearResponse { cleared: false });
        }
        let present: Vec<String> = marker::keys()
            .into_iter()
            .filter(|key| {
                pane.tokens
                    .as_ref()
                    .is_some_and(|tokens| tokens.contains_key(key))
            })
            .collect();
        let mut args = vec![
            "pane",
            "report-metadata",
            pane.target.as_str(),
            "--source",
            marker::SOURCE,
        ];
        for key in &present {
            args.extend(["--clear-token", key.as_str()]);
        }
        self.herdr
            .act(socket, &args, deadline)
            .map_err(HerdrError::into_driver)?;
        Ok(ClearResponse { cleared: true })
    }

    fn capture(
        &mut self,
        request: Request<CaptureRequest>,
    ) -> Result<CaptureResponse, DriverError> {
        let deadline = deadline(request.deadline_ms);
        let socket = self.socket(&request.body.socket)?;
        let pane_id = self.pane_id(&request.body.pane_id)?;
        if request.body.lines == 0 {
            return Err(bad_request("capture needs at least one line"));
        }
        let gone = || DriverError::new(ErrorCode::NotFound, "the pane is gone");
        let pane = self
            .find(socket, pane_id, deadline)
            .map_err(HerdrError::into_driver)?
            .ok_or_else(gone)?;
        let lines = request.body.lines.to_string();
        // `recent` is the scrollback's tail, as plain text without styling.
        let args = [
            "pane",
            "read",
            pane.target.as_str(),
            "--source",
            "recent",
            "--lines",
            lines.as_str(),
            "--format",
            "text",
        ];
        let text = self
            .herdr
            .text(socket, &args, deadline)
            .map_err(HerdrError::into_driver)?;
        Ok(CaptureResponse { text })
    }

    /// Herdr types `send-text` raw, so a line break would submit the text
    /// before core asks for Enter. Such text is refused before any effect
    /// (the contract's `bad_request`, definitely not sent): an agent pane
    /// takes a message through `prompt`, and a plain pane only one line.
    fn input(&mut self, request: Request<InputRequest>) -> Result<(), DriverError> {
        let deadline = deadline(request.deadline_ms);
        let InputRequest {
            socket,
            pane_id,
            text,
            enter,
        } = &request.body;
        let socket = self.socket(socket)?;
        let pane_id = self.pane_id(pane_id)?;
        if text.contains(['\r', '\n']) {
            return Err(bad_request("Herdr input takes one line of text"));
        }
        let pane = self
            .find(socket, pane_id, deadline)
            .map_err(HerdrError::into_driver)?
            .ok_or_else(|| DriverError::new(ErrorCode::NotFound, "the pane is gone"))?;
        // The text is the last argument, as is: Herdr reads it literally even
        // when it looks like an option, and has no `--` separator.
        if !text.is_empty() {
            self.herdr
                .act(
                    socket,
                    &["pane", "send-text", pane.target.as_str(), text.as_str()],
                    deadline,
                )
                .map_err(HerdrError::into_driver)?;
        }
        if *enter {
            self.herdr
                .act(
                    socket,
                    &["pane", "send-keys", pane.target.as_str(), "Enter"],
                    deadline,
                )
                .map_err(|error| {
                    let mut error = error.into_driver();
                    // Text already typed is not "nothing sent".
                    if !text.is_empty() && error.code == ErrorCode::NotFound {
                        error.code = ErrorCode::Failed;
                    }
                    error
                })?;
        }
        Ok(())
    }

    /// Herdr's `agent prompt`: the agent it recognizes in the pane takes the
    /// text and Herdr submits it. One attempt, no waiting; a pane with no
    /// agent answers `no_agent`, and core decides whether to type instead.
    fn prompt(&mut self, request: Request<PromptRequest>) -> Result<(), DriverError> {
        let deadline = deadline(request.deadline_ms);
        let socket = self.socket(&request.body.socket)?;
        let pane_id = self.pane_id(&request.body.pane_id)?;
        let pane = self
            .find(socket, pane_id, deadline)
            .map_err(HerdrError::into_driver)?
            .ok_or_else(|| DriverError::new(ErrorCode::NotFound, "the pane is gone"))?;
        self.herdr
            .act(
                socket,
                &[
                    "agent",
                    "prompt",
                    pane.target.as_str(),
                    request.body.text.as_str(),
                ],
                deadline,
            )
            .map_err(|error| {
                // These codes are prompt answers alone (the protocol's
                // prompt-only codes), so only this call maps them.
                let code = match error.code() {
                    Some("agent_not_found") => Some(ErrorCode::NoAgent),
                    Some("agent_blocked") => Some(ErrorCode::Blocked),
                    Some("agent_not_ready") => Some(ErrorCode::NotReady),
                    _ => None,
                };
                let mut error = error.into_driver();
                if let Some(code) = code {
                    error.code = code;
                }
                error
            })
    }
}

#[cfg(test)]
mod tests;

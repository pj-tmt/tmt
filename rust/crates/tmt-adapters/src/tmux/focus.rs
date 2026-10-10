//! Bringing a pane to the invoking user's tmux client. Only the view changes:
//! no buffer, paste or key input, and never a guessed client.

use super::{
    CallerEnvironment, CommandRunner, OperationOptions, Tmux, TmuxError, socket_args, valid_pane_id,
};

// tmux sanitizes control characters in formats; share the evidence separator.
const FIELD: &str = super::evidence::SEPARATOR;

/// The invoker as its own environment reports it: the server socket and
/// session ID from `TMUX`, and its pane from `TMUX_PANE`. Observed with tmux
/// 3.7: a shell in a display-popup inherits a `TMUX_PANE` for the popup's own
/// pane, which has no session, and key-binding jobs (`run-shell`,
/// `display-popup`) get no `TMUX_PANE` at all; in both, `TMUX` names the
/// session of the client that invoked them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invoker {
    pub socket: String,
    pub pane: Option<String>,
    pub session: Option<String>,
}

impl Invoker {
    /// Only what the environment reports; no client is guessed. Key-binding
    /// jobs have `TMUX` but no `TMUX_PANE`; `TMUX`'s session serves.
    pub fn from_environment(environment: &CallerEnvironment) -> Option<Self> {
        let socket = environment.selected_server_socket().ok().flatten()?;
        let pane = environment
            .pane
            .as_ref()
            .and_then(|pane| pane.to_str())
            .filter(|pane| !pane.is_empty())
            .map(str::to_owned);
        let session = environment.selected_session();
        if pane.is_none() && session.is_none() {
            return None;
        }
        Some(Self {
            socket: socket.to_owned(),
            pane,
            session,
        })
    }
}

/// The invoker's tmux client and the pane it shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientView {
    pub client: String,
    pub pane: Option<String>,
}

#[derive(Debug)]
pub enum FocusError {
    /// No tmux client can be identified for the invoker; nothing was changed.
    HostUnsupported,
    /// The requested pane does not exist on the invoker's server.
    PaneNotFound,
    Evidence(TmuxError),
}

impl std::fmt::Display for FocusError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::HostUnsupported => {
                formatter.write_str("No tmux client for this invocation can be focused.")
            }
            Self::PaneNotFound => formatter.write_str("The pane was not found."),
            Self::Evidence(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for FocusError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Evidence(error) => Some(error),
            _ => None,
        }
    }
}

/// A tmux command that ran and exited nonzero (for example "can't find pane")
/// means the thing asked about is absent; other failures stay tmux errors.
fn absent_or(error: TmuxError, absent: FocusError) -> FocusError {
    if error.exited() && !error.socket_permission_denied() {
        absent
    } else {
        FocusError::Evidence(error)
    }
}

impl<R: CommandRunner> Tmux<R> {
    fn query(
        &self,
        socket: &str,
        args: &[&str],
        options: OperationOptions<'_>,
    ) -> Result<String, TmuxError> {
        let mut argv = socket_args(Some(socket));
        argv.extend(args.iter().map(|arg| (*arg).to_owned()));
        self.execute(argv, options, super::TmuxFailure::Command)
    }

    /// The invoker's client: among clients showing the invoker's session (its
    /// pane's session, or `TMUX`'s session without a pane or for a popup pane
    /// that has none),
    /// the most recently active, with its current pane. Read-only. A bare
    /// "current client" is never used: with several clients it can be another
    /// user view.
    pub fn invoker_client(
        &self,
        invoker: &Invoker,
        options: OperationOptions<'_>,
    ) -> Result<ClientView, FocusError> {
        let session = match invoker.pane.as_deref() {
            None => String::new(),
            Some(pane) if valid_pane_id(pane) => self
                .query(
                    &invoker.socket,
                    &["display-message", "-p", "-t", pane, "#{session_id}"],
                    options,
                )
                .map_err(|error| absent_or(error, FocusError::HostUnsupported))?,
            Some(_) => return Err(FocusError::HostUnsupported),
        };
        let session = match session.trim() {
            "" => invoker
                .session
                .clone()
                .filter(|session| {
                    session.len() > 1 && session[1..].bytes().all(|b| b.is_ascii_digit())
                })
                .ok_or(FocusError::HostUnsupported)?,
            session => session.to_owned(),
        };
        let format = [
            "#{client_name}",
            "#{session_id}",
            "#{client_activity}",
            "#{pane_id}",
        ]
        .join(FIELD);
        let clients = self
            .query(&invoker.socket, &["list-clients", "-F", &format], options)
            .map_err(FocusError::Evidence)?;
        clients
            .lines()
            .filter_map(|line| {
                let fields: Vec<&str> = line.split(FIELD).collect();
                match fields.as_slice() {
                    [name, client_session, activity, pane]
                        if *client_session == session.as_str() =>
                    {
                        Some((
                            activity.parse::<u64>().unwrap_or(0),
                            (*name).to_owned(),
                            Some((*pane).to_owned()).filter(|pane| valid_pane_id(pane)),
                        ))
                    }
                    _ => None,
                }
            })
            .max_by_key(|(activity, _, _)| *activity)
            .map(|(_, client, pane)| ClientView { client, pane })
            .ok_or(FocusError::HostUnsupported)
    }

    /// Switches the invoker's client to `pane` (its session, window and pane)
    /// and returns that client with the pane it showed before.
    pub fn focus_pane(
        &self,
        invoker: &Invoker,
        pane: &str,
        options: OperationOptions<'_>,
    ) -> Result<ClientView, FocusError> {
        if !valid_pane_id(pane) {
            return Err(FocusError::PaneNotFound);
        }
        self.query(
            &invoker.socket,
            &["display-message", "-p", "-t", pane, "#{pane_id}"],
            options,
        )
        .map_err(|error| absent_or(error, FocusError::PaneNotFound))?;
        let before = self.invoker_client(invoker, options)?;
        self.query(
            &invoker.socket,
            &[
                "switch-client",
                "-c",
                &before.client,
                "-t",
                pane,
                ";",
                "select-window",
                "-t",
                pane,
                ";",
                "select-pane",
                "-t",
                pane,
            ],
            options,
        )
        .map_err(|error| absent_or(error, FocusError::PaneNotFound))?;
        Ok(before)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        process::CommandFailure,
        scripted_runner::{ScriptedRunner, failure_with_kind},
    };

    fn invoker() -> Invoker {
        Invoker {
            socket: "/tmp/tmt-focus.sock".into(),
            pane: Some("%2".into()),
            session: Some("$9".into()),
        }
    }

    fn clients(rows: &[[&str; 4]]) -> Vec<u8> {
        rows.iter()
            .map(|row| row.join(FIELD))
            .collect::<Vec<_>>()
            .join("\n")
            .into_bytes()
    }

    #[test]
    fn the_invokers_most_recent_client_is_switched_and_nothing_is_typed() {
        let runner = ScriptedRunner::default();
        runner.push_output(b"%5\n".to_vec(), Vec::new());
        runner.push_output(b"$1\n".to_vec(), Vec::new());
        runner.push_output(
            clients(&[
                ["/dev/ttys001", "$2", "900", "%7"],
                ["/dev/ttys002", "$1", "100", "%2"],
                ["client-3", "$1", "500", "%3"],
            ]),
            Vec::new(),
        );
        runner.push_output(Vec::new(), Vec::new());
        let tmux = Tmux::new(runner);
        let before = tmux
            .focus_pane(&invoker(), "%5", OperationOptions::default())
            .unwrap();
        assert_eq!(
            before,
            ClientView {
                client: "client-3".into(),
                pane: Some("%3".into())
            },
            "most recent client on the invoker's session"
        );
        let calls = tmux.runner.calls.borrow();
        let args: Vec<Vec<String>> = calls.iter().map(|call| call.args.clone()).collect();
        assert_eq!(args[1][3..7], ["display-message", "-p", "-t", "%2"]);
        assert_eq!(
            args[3][3..],
            [
                "switch-client",
                "-c",
                "client-3",
                "-t",
                "%5",
                ";",
                "select-window",
                "-t",
                "%5",
                ";",
                "select-pane",
                "-t",
                "%5"
            ]
        );
        assert!(
            args.iter()
                .all(|call| call[..3] == ["-u", "-S", "/tmp/tmt-focus.sock"])
        );
        for forbidden in ["send-keys", "paste-buffer", "set-buffer"] {
            assert!(!args.iter().flatten().any(|arg| arg == forbidden));
        }
    }

    #[test]
    fn the_client_query_reads_only_and_uses_the_same_resolution() {
        let runner = ScriptedRunner::default();
        runner.push_output(b"$1\n".to_vec(), Vec::new());
        runner.push_output(
            clients(&[
                ["/dev/ttys001", "$2", "900", "%7"],
                ["/dev/ttys002", "$1", "500", "%4"],
            ]),
            Vec::new(),
        );
        let tmux = Tmux::new(runner);
        assert_eq!(
            tmux.invoker_client(&invoker(), OperationOptions::default())
                .unwrap(),
            ClientView {
                client: "/dev/ttys002".into(),
                pane: Some("%4".into())
            }
        );
        let calls = tmux.runner.calls.borrow();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].args[3], "display-message");
        assert_eq!(calls[1].args[3], "list-clients");

        let runner = ScriptedRunner::default();
        runner.push_output(b"$1\n".to_vec(), Vec::new());
        runner.push_output(clients(&[["other", "$2", "900", "%7"]]), Vec::new());
        let tmux = Tmux::new(runner);
        assert!(matches!(
            tmux.invoker_client(&invoker(), OperationOptions::default()),
            Err(FocusError::HostUnsupported)
        ));
    }

    #[test]
    fn a_key_binding_job_without_a_pane_uses_the_session_in_tmux() {
        // tmux 3.7c: run-shell and display-popup started by a key binding have
        // TMUX but no TMUX_PANE.
        let runner = ScriptedRunner::default();
        runner.push_output(
            clients(&[
                ["/dev/ttys001", "$2", "900", "%7"],
                ["/dev/ttys002", "$9", "100", "%4"],
            ]),
            Vec::new(),
        );
        let tmux = Tmux::new(runner);
        let job = Invoker {
            pane: None,
            ..invoker()
        };
        assert_eq!(
            tmux.invoker_client(&job, OperationOptions::default())
                .unwrap()
                .client,
            "/dev/ttys002"
        );
        assert_eq!(tmux.runner.calls.borrow().len(), 1, "only list-clients");

        let tmux = Tmux::new(ScriptedRunner::default());
        let nothing = Invoker {
            pane: None,
            session: None,
            ..invoker()
        };
        assert!(matches!(
            tmux.invoker_client(&nothing, OperationOptions::default()),
            Err(FocusError::HostUnsupported)
        ));
        assert!(tmux.runner.calls.borrow().is_empty());
    }

    #[test]
    fn no_client_on_the_invokers_session_or_no_pane_is_host_unsupported() {
        let runner = ScriptedRunner::default();
        runner.push_output(b"%5\n".to_vec(), Vec::new());
        runner.push_output(b"$1\n".to_vec(), Vec::new());
        runner.push_output(clients(&[["other", "$2", "900", "%7"]]), Vec::new());
        let tmux = Tmux::new(runner);
        assert!(matches!(
            tmux.focus_pane(&invoker(), "%5", OperationOptions::default()),
            Err(FocusError::HostUnsupported)
        ));
        assert_eq!(
            tmux.runner.calls.borrow().len(),
            3,
            "no switch was attempted"
        );

        let tmux = Tmux::new(ScriptedRunner::default());
        tmux.runner.push_output(b"%5\n".to_vec(), Vec::new());
        let malformed_pane = Invoker {
            pane: Some("pane".into()),
            ..invoker()
        };
        assert!(matches!(
            tmux.focus_pane(&malformed_pane, "%5", OperationOptions::default()),
            Err(FocusError::HostUnsupported)
        ));
    }

    #[test]
    fn a_missing_target_pane_is_reported_before_any_switch() {
        let runner = ScriptedRunner::default();
        runner.results.borrow_mut().push_back(Err(failure_with_kind(
            CommandFailure::Exit {
                code: Some(1),
                signal: None,
            },
            false,
        )));
        let tmux = Tmux::new(runner);
        assert!(matches!(
            tmux.focus_pane(&invoker(), "%99", OperationOptions::default()),
            Err(FocusError::PaneNotFound)
        ));
        assert!(matches!(
            tmux.focus_pane(&invoker(), "main:1.0", OperationOptions::default()),
            Err(FocusError::PaneNotFound)
        ));
        assert_eq!(tmux.runner.calls.borrow().len(), 1);
    }

    #[test]
    fn a_popup_pane_without_a_session_uses_the_popup_clients_session() {
        // tmux 3.7: in display-popup, TMUX_PANE is the popup's own pane; asking
        // for its session prints nothing, and a bare display-message would name
        // the most recently active client, possibly another user's view.
        let runner = ScriptedRunner::default();
        runner.push_output(b"%5\n".to_vec(), Vec::new());
        runner.push_output(b"\n".to_vec(), Vec::new());
        runner.push_output(
            clients(&[
                ["/dev/ttys024", "$1", "900", "%1"],
                ["/dev/ttys023", "$9", "100", "%0"],
            ]),
            Vec::new(),
        );
        runner.push_output(Vec::new(), Vec::new());
        let tmux = Tmux::new(runner);
        let popup = Invoker {
            pane: Some("%23".into()),
            ..invoker()
        };
        let before = tmux
            .focus_pane(&popup, "%5", OperationOptions::default())
            .unwrap();
        assert_eq!(before.pane.as_deref(), Some("%0"));
        assert_eq!(tmux.runner.calls.borrow()[3].args[5], "/dev/ttys023");

        // Without a usable TMUX session either, nothing is guessed.
        let runner = ScriptedRunner::default();
        runner.push_output(b"%5\n".to_vec(), Vec::new());
        runner.push_output(b"\n".to_vec(), Vec::new());
        let tmux = Tmux::new(runner);
        let unknown = Invoker {
            session: None,
            ..popup
        };
        assert!(matches!(
            tmux.focus_pane(&unknown, "%5", OperationOptions::default()),
            Err(FocusError::HostUnsupported)
        ));
        assert_eq!(tmux.runner.calls.borrow().len(), 2);
    }
}

//! The terminal hosts that hold agent panes. The CLI and the other adapters
//! reach a host only through [`Host`], never by naming one.
//!
//! A handle has a primary host, chosen for a reason: the caller's own host
//! ([`Host::for_caller`]), the host that runs a known server
//! ([`Host::for_server`]), or the host whose syntax an explicit pane target
//! uses ([`Host::for_target`]). Its binding session observes new panes on the
//! primary host, and probes, marks and clears every stored binding on that
//! binding's own host, so presence is complete from either host (#479).
//! Endpoints travel as core values that carry their host.
//!
//! A host without a server-level store (Herdr) gets TMT's own server UUID
//! through core's `HostServerIds` port, resolved by [`Host::resolve_servers`] before any
//! binding transaction opens.

use crate::{
    herdr::{self, Herdr, HerdrError},
    process::{CommandError, CommandRunner, UnixCommandRunner},
    tmux::{self, Tmux, TmuxError},
};
use std::{
    ffi::OsString,
    fmt,
    time::{Duration, Instant},
};
use tmt_core::{
    binding::{
        Binding, BindingEndpoint, BindingEntry, BindingTargetEvidence, session::RuntimeState,
    },
    driver::{ActionResult, DeliveryAcceptance, Driver, Focused, InterfaceStatus, SendFailure},
    endpoint::{EndpointProbe, EndpointSnapshot, ServerEvidence},
    host::{HostKind, HostServerIds, HostServerIncarnation, ServerSelector},
    identity::Identity,
    request::RequestEndpoint,
};

pub use crate::tmux::{
    ClientView, DeliveryError, DeliveryStage, FocusError, Invoker, OperationOptions, PaneCosmetics,
    PaneRefresh,
};

/// Invocation-owned observations; tests never mutate process-global variables.
pub struct CallerEnvironment {
    /// `TMUX` and `TMUX_PANE`.
    pub tmux: Option<OsString>,
    pub pane: Option<OsString>,
    /// `HERDR_PANE_ID` and `HERDR_SOCKET_PATH`.
    pub herdr_pane: Option<OsString>,
    pub herdr_socket: Option<OsString>,
    pub process_id: u64,
}

impl CallerEnvironment {
    pub fn current() -> Self {
        Self {
            tmux: std::env::var_os("TMUX"),
            pane: std::env::var_os("TMUX_PANE"),
            herdr_pane: std::env::var_os("HERDR_PANE_ID"),
            herdr_socket: std::env::var_os("HERDR_SOCKET_PATH"),
            process_id: u64::from(std::process::id()),
        }
    }

    fn names_tmux(&self) -> bool {
        [&self.tmux, &self.pane]
            .into_iter()
            .any(|value| value.as_ref().is_some_and(|value| !value.is_empty()))
    }

    fn names_herdr(&self) -> bool {
        [&self.herdr_pane, &self.herdr_socket]
            .into_iter()
            .all(|value| value.as_ref().is_some_and(|value| !value.is_empty()))
    }
}

#[derive(Debug)]
pub enum HostError {
    Tmux(TmuxError),
    Herdr(HerdrError),
}

impl HostError {
    pub fn cleanup_failed(&self) -> bool {
        match self {
            Self::Tmux(error) => error.cleanup_failed(),
            Self::Herdr(error) => error.cleanup_failed(),
        }
    }

    pub fn socket_permission_denied(&self) -> bool {
        match self {
            Self::Tmux(error) => error.socket_permission_denied(),
            Self::Herdr(_) => false,
        }
    }
}

impl From<TmuxError> for HostError {
    fn from(error: TmuxError) -> Self {
        Self::Tmux(error)
    }
}

impl From<HerdrError> for HostError {
    fn from(error: HerdrError) -> Self {
        Self::Herdr(error)
    }
}

// Transparent: a host error reads exactly as its host's own error.
impl fmt::Display for HostError {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tmux(error) => error.fmt(output),
            Self::Herdr(error) => error.fmt(output),
        }
    }
}

impl std::error::Error for HostError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Tmux(error) => error.source(),
            Self::Herdr(error) => error.source(),
        }
    }
}

#[derive(Debug)]
pub enum ActionError {
    Evidence(HostError),
    Unverified,
    Offline,
    /// No client of the invoking user can be focused; nothing changed.
    HostUnsupported,
    Delivery(DeliveryError),
    Process(CommandError),
}

impl From<tmux::ActionError> for ActionError {
    fn from(error: tmux::ActionError) -> Self {
        match error {
            tmux::ActionError::Evidence(error) => Self::Evidence(HostError::Tmux(error)),
            tmux::ActionError::Unverified => Self::Unverified,
            tmux::ActionError::Offline => Self::Offline,
            tmux::ActionError::HostUnsupported => Self::HostUnsupported,
            tmux::ActionError::Delivery(error) => Self::Delivery(error),
            tmux::ActionError::Process(error) => Self::Process(error),
        }
    }
}

impl fmt::Display for ActionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Evidence(error) => error.fmt(f),
            Self::Unverified => f.write_str("Could not verify the identity binding."),
            Self::Offline => f.write_str("The agent runtime has ended; no pane input was sent."),
            Self::HostUnsupported => {
                f.write_str("No tmux client for this invocation can be focused.")
            }
            Self::Delivery(error) => error.fmt(f),
            Self::Process(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for ActionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Evidence(error) => Some(error),
            Self::Delivery(error) => Some(error),
            Self::Process(error) => Some(error),
            Self::Unverified | Self::Offline | Self::HostUnsupported => None,
        }
    }
}

pub struct Host<R = UnixCommandRunner> {
    primary: HostKind,
    tmux: Tmux<R>,
    herdr: Herdr<R>,
}

impl Host {
    /// The host a caller-scoped command runs in: the host whose pane shell is
    /// the caller's nearest ancestor. Without a host in the environment,
    /// tmux's default server serves, as it always has.
    pub fn for_caller(environment: &CallerEnvironment) -> Self {
        Self::for_caller_with(environment, UnixCommandRunner)
    }

    /// The host that runs a known server.
    pub fn for_server(server: &ServerEvidence) -> Self {
        Self::for_server_with(server, UnixCommandRunner)
    }

    /// The host whose syntax an explicit pane target uses; text that no host
    /// reads as a target is resolved by name elsewhere.
    pub fn for_target(target: &str) -> Self {
        let canonical = tmt_core::names::normalize_name(target);
        let host = if HostKind::Herdr.is_target(&canonical) {
            HostKind::Herdr
        } else {
            HostKind::Tmux
        };
        Self::of(host, UnixCommandRunner, None)
    }
}

impl<R: CommandRunner + Clone> Host<R> {
    pub fn for_caller_with(environment: &CallerEnvironment, runner: R) -> Self {
        let socket = environment
            .herdr_socket
            .as_ref()
            .and_then(|socket| socket.to_str())
            .map(str::to_owned);
        let host = match (environment.names_tmux(), environment.names_herdr()) {
            (false, true) => HostKind::Herdr,
            (true, true) => {
                Self::of(HostKind::Tmux, runner.clone(), socket.clone()).nearest(environment)
            }
            _ => HostKind::Tmux,
        };
        Self::of(host, runner, socket)
    }

    pub fn for_server_with(server: &ServerEvidence, runner: R) -> Self {
        let host = Self::of(server.host, runner, Some(server.socket_path.clone()));
        if server.host == HostKind::Herdr {
            host.herdr.set_resolved(server.clone());
        }
        host
    }

    fn of(primary: HostKind, runner: R, herdr_socket: Option<String>) -> Self {
        Self {
            primary,
            tmux: Tmux::new(runner.clone()),
            herdr: Herdr::new(runner, herdr_socket),
        }
    }
}

impl<R: CommandRunner> Host<R> {
    pub fn kind(&self) -> HostKind {
        self.primary
    }

    /// Inside both hosts (tmux in a Herdr pane, or the reverse), the caller's
    /// own pane is the nearer ancestor. Unverifiable Herdr evidence keeps
    /// tmux, as before Herdr.
    fn nearest(&self, environment: &CallerEnvironment) -> HostKind {
        let deadline = Instant::now() + Duration::from_secs(1);
        let Ok(Some(herdr)) = self.herdr.caller(environment, deadline) else {
            return HostKind::Tmux;
        };
        match self.tmux.caller_depth(environment) {
            Ok(Some(tmux)) if tmux < herdr.depth => HostKind::Tmux,
            _ => HostKind::Herdr,
        }
    }

    /// The server a caller's environment selects on this handle's host, for
    /// presentation priority only.
    pub fn selected_server<'a>(
        &self,
        environment: &'a CallerEnvironment,
    ) -> Option<ServerSelector<'a>> {
        match self.primary {
            HostKind::Tmux => environment.selected_server(),
            HostKind::Herdr => {
                environment
                    .herdr_socket
                    .as_ref()?
                    .to_str()
                    .map(|socket| ServerSelector {
                        host: HostKind::Herdr,
                        socket,
                    })
            }
        }
    }

    /// Resolve TMT's UUID for this handle's Herdr server before any binding
    /// transaction; tmux keeps its own in a server option. A server without
    /// panes stays unresolved: there is nothing on it to bind.
    pub fn resolve_servers<I: HostServerIds>(&self, ids: &mut I) -> Result<(), HostError> {
        if self.primary != HostKind::Herdr || self.herdr.resolved().is_some() {
            return Ok(());
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        let socket = self.herdr.server_socket(None, deadline)?;
        let Some(incarnation) = self.herdr.incarnation(&socket, deadline)? else {
            return Ok(());
        };
        let server_id = ids
            .server_id(&HostServerIncarnation {
                host: HostKind::Herdr,
                socket_path: &incarnation.socket,
                server_pid: incarnation.pid,
                server_start_time: &incarnation.start,
            })
            .map_err(HerdrError::server_ids)?;
        if !tmt_core::endpoint::valid_server_id(&server_id) {
            return Err(HerdrError::server_ids(std::io::Error::other("invalid server ID")).into());
        }
        self.herdr.set_resolved(ServerEvidence {
            host: HostKind::Herdr,
            server_id,
            socket_path: incarnation.socket,
            server_pid: incarnation.pid,
            server_start_time: incarnation.start,
        });
        Ok(())
    }

    /// The core binding and driver ports.
    pub fn session(&self) -> BindingSession<'_, R> {
        BindingSession {
            primary: self.primary,
            tmux: tmux::BindingSession::new(&self.tmux),
            herdr: herdr::Session::new(&self.herdr),
        }
    }

    pub fn caller_pane(
        &self,
        environment: &CallerEnvironment,
    ) -> Result<Option<String>, HostError> {
        match self.primary {
            HostKind::Tmux => Ok(self.tmux.caller_pane(environment)?),
            HostKind::Herdr => Ok(self
                .herdr
                .caller(environment, Instant::now() + Duration::from_secs(1))?
                .map(|pane| pane.terminal_id)),
        }
    }

    /// tmux's explicit mark; Herdr has none.
    pub fn marked_pane(
        &self,
        environment: &CallerEnvironment,
        options: OperationOptions<'_>,
    ) -> Result<Option<BindingTargetEvidence>, HostError> {
        match self.primary {
            HostKind::Tmux => Ok(self.tmux.marked_pane(environment, options)?),
            HostKind::Herdr => Ok(None),
        }
    }

    pub fn resolve_target(
        &self,
        target: &str,
        options: OperationOptions<'_>,
    ) -> Result<Option<String>, HostError> {
        match self.primary {
            HostKind::Tmux => Ok(self.tmux.resolve_target(target, options)?),
            HostKind::Herdr => Ok(self.herdr.resolve_target(target, options.deadline)?),
        }
    }

    pub fn snapshot(&self, options: OperationOptions<'_>) -> Result<EndpointSnapshot, HostError> {
        match self.primary {
            HostKind::Tmux => Ok(self.tmux.snapshot(options)?),
            HostKind::Herdr => Ok(self.herdr_session(options).snapshot(options.pane_ids)?),
        }
    }

    pub fn observe_snapshot(
        &self,
        options: OperationOptions<'_>,
    ) -> Result<EndpointSnapshot, HostError> {
        match self.primary {
            HostKind::Tmux => Ok(self.tmux.observe_snapshot(options)?),
            HostKind::Herdr => Ok(self.herdr_session(options).snapshot(options.pane_ids)?),
        }
    }

    fn herdr_session(&self, options: OperationOptions<'_>) -> herdr::Session<'_, R> {
        let mut session = herdr::Session::new(&self.herdr);
        session.begin_coordination();
        session.limit(options.deadline);
        session
    }

    pub fn probe(
        &self,
        server: &ServerEvidence,
        options: OperationOptions<'_>,
    ) -> Result<EndpointProbe, HostError> {
        match server.host {
            HostKind::Tmux => {
                Ok(self
                    .tmux
                    .probe(&server.socket_path, server.server_pid, options)?)
            }
            HostKind::Herdr => Ok(self
                .herdr_session(options)
                .probe(server, options.pane_ids.unwrap_or_default())?),
        }
    }

    pub fn capture(&self, endpoint: &RequestEndpoint, lines: u64) -> Result<String, HostError> {
        match endpoint.server.host {
            HostKind::Tmux => {
                Ok(self
                    .tmux
                    .capture_on(&endpoint.server.socket_path, &endpoint.pane_id, lines)?)
            }
            HostKind::Herdr => Err(HerdrError::unsupported("Reading a Herdr pane").into()),
        }
    }

    pub fn send(
        &self,
        endpoint: &RequestEndpoint,
        message: &str,
        enter_delay: Duration,
    ) -> Result<(), DeliveryError> {
        match endpoint.server.host {
            HostKind::Tmux => self.tmux.send_on(
                &endpoint.server.socket_path,
                &endpoint.pane_id,
                message,
                enter_delay,
            ),
            HostKind::Herdr => Err(DeliveryError::unsupported()),
        }
    }

    pub fn invoker_client(
        &self,
        invoker: &Invoker,
        options: OperationOptions<'_>,
    ) -> Result<ClientView, FocusError> {
        self.tmux.invoker_client(invoker, options)
    }

    pub fn focus_pane(
        &self,
        invoker: &Invoker,
        pane: &str,
        options: OperationOptions<'_>,
    ) -> Result<ClientView, FocusError> {
        self.tmux.focus_pane(invoker, pane, options)
    }

    /// Post-commit pane cosmetics. On Herdr only the marker's name is kept
    /// current; its badge arrives with `run` (#479 H5).
    pub fn update_binding_cosmetics(
        &self,
        binding: &Binding,
        cosmetics: PaneCosmetics<'_>,
    ) -> Result<PaneRefresh, HostError> {
        self.update_binding_cosmetics_until(
            binding,
            cosmetics,
            Instant::now() + Duration::from_secs(1),
        )
    }

    pub fn update_binding_cosmetics_until(
        &self,
        binding: &Binding,
        cosmetics: PaneCosmetics<'_>,
        deadline: Instant,
    ) -> Result<PaneRefresh, HostError> {
        match (binding.server.host, cosmetics) {
            (HostKind::Tmux, _) => Ok(self
                .tmux
                .update_binding_cosmetics_until(binding, cosmetics, deadline)?),
            (HostKind::Herdr, PaneCosmetics::Bound { identity, .. }) => {
                let mut session = herdr::Session::new(&self.herdr);
                session.begin_coordination();
                session.limit(Some(deadline));
                match session.refresh_name(binding, identity) {
                    Ok(refresh) => Ok(refresh),
                    Err(error) if error.cleanup_failed() => Err(error.into()),
                    Err(_) => Ok(PaneRefresh::Failed),
                }
            }
            (HostKind::Herdr, PaneCosmetics::Ended) => Ok(PaneRefresh::Absent),
        }
    }
}

/// The core binding and driver ports over every host. New panes are observed
/// on the primary host; a stored binding is always handled by its own host.
pub struct BindingSession<'a, R> {
    primary: HostKind,
    tmux: tmux::BindingSession<'a, R>,
    herdr: herdr::Session<'a, R>,
}

impl<R: CommandRunner> BindingSession<'_, R> {
    /// The user's own tmux client location, required only by `focus`.
    pub fn with_invoker(mut self, invoker: Invoker) -> Self {
        self.tmux = self.tmux.with_invoker(invoker);
        self
    }

    /// Preserve the caller's configured transport delay; it is not routing policy.
    pub fn with_enter_delay(mut self, delay: Duration) -> Self {
        self.tmux = self.tmux.with_enter_delay(delay);
        self
    }

    /// Runtime liveness after the caller verified this binding's endpoint.
    pub fn observed_runtime(&self, binding: &Binding) -> Result<RuntimeState, ActionError> {
        match binding.server.host {
            HostKind::Tmux => Ok(self.tmux.observed_runtime(binding)?),
            HostKind::Herdr => self
                .herdr
                .observed_runtime(binding)
                .map_err(ActionError::Process),
        }
    }
}

impl<R: CommandRunner> BindingEndpoint for BindingSession<'_, R> {
    type Error = HostError;

    fn begin_coordination(&mut self) {
        self.tmux.begin_coordination();
        self.herdr.begin_coordination();
    }

    fn budget_available(&self) -> bool {
        match self.primary {
            HostKind::Tmux => self.tmux.budget_available(),
            HostKind::Herdr => self.herdr.budget_available(),
        }
    }

    fn current_snapshot(&mut self, panes: &[String]) -> Result<EndpointSnapshot, Self::Error> {
        match self.primary {
            HostKind::Tmux => Ok(self.tmux.current_snapshot(panes)?),
            HostKind::Herdr => Ok(self.herdr.snapshot(Some(panes))?),
        }
    }

    fn probe_binding(
        &mut self,
        server: &ServerEvidence,
        panes: &[String],
    ) -> Result<EndpointProbe, Self::Error> {
        match server.host {
            HostKind::Tmux => Ok(self.tmux.probe_binding(server, panes)?),
            HostKind::Herdr => Ok(self.herdr.probe(server, panes)?),
        }
    }

    fn publish(&mut self, binding: &Binding, identity: &Identity) -> Result<(), Self::Error> {
        match binding.server.host {
            HostKind::Tmux => Ok(self.tmux.publish(binding, identity)?),
            HostKind::Herdr => Ok(self.herdr.publish(binding, identity)?),
        }
    }

    fn clear(&mut self, binding: &Binding) -> Result<bool, Self::Error> {
        match binding.server.host {
            HostKind::Tmux => Ok(self.tmux.clear(binding)?),
            HostKind::Herdr => Ok(self.herdr.clear(binding)?),
        }
    }
}

fn binding_host(entry: &BindingEntry, primary: HostKind) -> HostKind {
    entry
        .binding
        .as_ref()
        .map_or(primary, |binding| binding.server.host)
}

fn lift<T>(result: ActionResult<T, tmux::ActionError>) -> ActionResult<T, ActionError> {
    match result {
        ActionResult::Unsupported => ActionResult::Unsupported,
        ActionResult::Completed(value) => ActionResult::Completed(value),
        ActionResult::Failed(error) => ActionResult::Failed(error.into()),
    }
}

impl<R: CommandRunner> Driver for BindingSession<'_, R> {
    type Target = BindingEntry;
    type Error = ActionError;
    type Launch = ();

    fn status(&mut self, entry: &BindingEntry) -> ActionResult<InterfaceStatus, ActionError> {
        match binding_host(entry, self.primary) {
            HostKind::Tmux => lift(self.tmux.status(entry)),
            HostKind::Herdr => match self.herdr.status(entry) {
                Ok(status) => ActionResult::Completed(status),
                Err(error) => ActionResult::Failed(error),
            },
        }
    }

    /// Herdr delivery arrives in #479 H4; until then the core falls through
    /// to the Inbox, as for any host without pane input.
    fn send(
        &mut self,
        entry: &BindingEntry,
        message: &str,
    ) -> ActionResult<DeliveryAcceptance, SendFailure<ActionError>> {
        match binding_host(entry, self.primary) {
            HostKind::Tmux => match self.tmux.send(entry, message) {
                ActionResult::Unsupported => ActionResult::Unsupported,
                ActionResult::Completed(value) => ActionResult::Completed(value),
                ActionResult::Failed(failure) => ActionResult::Failed(match failure {
                    SendFailure::NotSent(error) => SendFailure::NotSent(error.into()),
                    SendFailure::Uncertain(error) => SendFailure::Uncertain(error.into()),
                    SendFailure::Denied(error) => SendFailure::Denied(error.into()),
                    SendFailure::AwaitingApproval(error) => {
                        SendFailure::AwaitingApproval(error.into())
                    }
                }),
            },
            HostKind::Herdr => ActionResult::Unsupported,
        }
    }

    fn focus(&mut self, entry: &BindingEntry) -> ActionResult<Focused, ActionError> {
        match binding_host(entry, self.primary) {
            HostKind::Tmux => lift(self.tmux.focus(entry)),
            HostKind::Herdr => ActionResult::Failed(ActionError::HostUnsupported),
        }
    }
}

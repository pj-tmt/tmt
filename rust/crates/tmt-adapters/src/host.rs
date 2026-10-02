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

mod delivery;
pub(crate) mod driver;
pub mod external;

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
    ClientView, FocusError, Invoker, OperationOptions, PaneCosmetics, PaneRefresh,
};
pub(crate) use delivery::DeliveryCause;
pub use delivery::{DeliveryError, DeliveryStage};

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
    /// A host no installed driver serves; nothing was attempted.
    Unavailable(String),
    /// An external host's driver could not be run or broke the protocol.
    Driver(external::CallError),
    /// An external host's driver answered with an error.
    Refused {
        driver: String,
        code: tmt_driver_protocol::ErrorCode,
        message: String,
    },
    /// Core's own evidence about an external host contradicts the request.
    Evidence {
        driver: String,
        reason: &'static str,
    },
    /// TMT could not record an external server's incarnation.
    ServerIds(Box<dyn std::error::Error + Send + Sync>),
}

impl HostError {
    pub fn cleanup_failed(&self) -> bool {
        match self {
            Self::Tmux(error) => error.cleanup_failed(),
            Self::Herdr(error) => error.cleanup_failed(),
            Self::Driver(external::CallError::Process { error, .. }) => error.cleanup_failed(),
            Self::Unavailable(_)
            | Self::Driver(_)
            | Self::Refused { .. }
            | Self::Evidence { .. }
            | Self::ServerIds(_) => false,
        }
    }

    pub fn socket_permission_denied(&self) -> bool {
        match self {
            Self::Tmux(error) => error.socket_permission_denied(),
            Self::Herdr(_)
            | Self::Unavailable(_)
            | Self::Driver(_)
            | Self::Refused { .. }
            | Self::Evidence { .. }
            | Self::ServerIds(_) => false,
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
            Self::Unavailable(name) => write!(output, "Host driver {name} is not installed."),
            Self::Driver(error) => error.fmt(output),
            // The driver's message is untrusted text, bounded and free of
            // control characters by the protocol.
            Self::Refused {
                driver, message, ..
            } => write!(output, "Host driver {driver}: {message}"),
            Self::Evidence { driver, reason } => write!(output, "Host driver {driver}: {reason}."),
            Self::ServerIds(error) => write!(output, "Could not record the host server: {error}"),
        }
    }
}

impl std::error::Error for HostError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Tmux(error) => error.source(),
            Self::Herdr(error) => error.source(),
            Self::Driver(error) => Some(error),
            Self::ServerIds(error) => Some(error.as_ref()),
            Self::Unavailable(_) | Self::Refused { .. } | Self::Evidence { .. } => None,
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
    external: external::Drivers<R>,
}

impl Host {
    /// The host a caller-scoped command runs in: the host whose pane shell is
    /// the caller's nearest ancestor. Without a host in the environment,
    /// tmux's default server serves, as it always has.
    pub fn for_caller(environment: &CallerEnvironment) -> Self {
        Self::for_caller_with(environment, UnixCommandRunner).with_drivers(external::approved())
    }

    /// The host that runs a known server.
    pub fn for_server(server: &ServerEvidence) -> Self {
        Self::for_server_with(server, UnixCommandRunner).with_drivers(external::approved())
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
        Self::of(host, UnixCommandRunner, None).with_drivers(external::approved())
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
        match server.host {
            HostKind::Tmux => {}
            HostKind::Herdr => host.herdr.set_resolved(server.clone()),
            HostKind::External(_) => host.external.set_resolved(server.clone()),
        }
        host
    }

    /// The drivers this handle may run for external hosts. The `_with`
    /// constructors approve none, so tests never read the user's registry.
    pub fn with_drivers(self, records: Vec<external::registry::DriverRecord>) -> Self {
        let runner = self.herdr.runner().clone();
        let resolved = self.external.resolved().cloned();
        let external = external::Drivers::new(runner, records);
        if let Some(server) = resolved {
            external.set_resolved(server);
        }
        Self { external, ..self }
    }

    fn of(primary: HostKind, runner: R, herdr_socket: Option<String>) -> Self {
        Self {
            primary,
            tmux: Tmux::new(runner.clone()),
            herdr: Herdr::new(runner.clone(), herdr_socket),
            external: external::Drivers::new(runner, Vec::new()),
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
            HostKind::External(_) => None,
        }
    }

    /// Resolve TMT's UUID for this handle's Herdr server before any binding
    /// transaction; tmux keeps its own in a server option. A server without
    /// panes stays unresolved: there is nothing on it to bind.
    pub fn resolve_servers<I: HostServerIds>(&self, ids: &mut I) -> Result<(), HostError> {
        if let HostKind::External(host) = self.primary {
            return self.external.resolve_server(host, ids);
        }
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
                server_pid: incarnation.process.pid(),
                server_start_time: incarnation.process.start_identity(),
            })
            .map_err(HerdrError::server_ids)?;
        if !tmt_core::endpoint::valid_server_id(&server_id) {
            return Err(HerdrError::server_ids(std::io::Error::other("invalid server ID")).into());
        }
        self.herdr.set_resolved(ServerEvidence {
            host: HostKind::Herdr,
            server_id,
            socket_path: incarnation.socket,
            server_pid: incarnation.process.pid(),
            server_start_time: incarnation.process.start_identity().to_owned(),
        });
        Ok(())
    }

    /// The core binding and driver ports.
    pub fn session(&self) -> BindingSession<'_, R> {
        BindingSession {
            primary: self.primary,
            tmux: tmux::BindingSession::new(&self.tmux),
            herdr: herdr::Session::new(&self.herdr),
            external: external::Session::new(&self.external),
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
            HostKind::External(_) => Ok(None),
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
            HostKind::Herdr | HostKind::External(_) => Ok(None),
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
            HostKind::External(_) => Ok(None),
        }
    }

    pub fn snapshot(&self, options: OperationOptions<'_>) -> Result<EndpointSnapshot, HostError> {
        match self.primary {
            HostKind::Tmux => Ok(self.tmux.snapshot(options)?),
            HostKind::Herdr => Ok(self.herdr_session(options).snapshot(options.pane_ids)?),
            HostKind::External(host) => self
                .external_session(options)
                .driver(host)
                .snapshot(options.pane_ids.unwrap_or_default()),
        }
    }

    pub fn observe_snapshot(
        &self,
        options: OperationOptions<'_>,
    ) -> Result<EndpointSnapshot, HostError> {
        match self.primary {
            HostKind::Tmux => Ok(self.tmux.observe_snapshot(options)?),
            HostKind::Herdr => Ok(self.herdr_session(options).snapshot(options.pane_ids)?),
            HostKind::External(host) => self
                .external_session(options)
                .driver(host)
                .snapshot(options.pane_ids.unwrap_or_default()),
        }
    }

    fn external_session(&self, options: OperationOptions<'_>) -> external::Session<'_, R> {
        let mut session = external::Session::new(&self.external);
        session.begin_coordination();
        session.limit(options.deadline);
        session
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
            // Without its driver nothing is known, and unknown never retires.
            HostKind::External(host) => self
                .external_session(options)
                .driver(host)
                .probe(server, options.pane_ids.unwrap_or_default()),
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
            // Capture through a driver arrives in slice 3b-2b.
            HostKind::External(host) => Err(HostError::Unavailable(host.as_str().to_owned())),
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
                &tmt_core::driver::pane_input_text(message),
                enter_delay,
            ),
            HostKind::Herdr | HostKind::External(_) => Err(DeliveryError::unsupported()),
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
            (HostKind::Herdr, PaneCosmetics::Ended) | (HostKind::External(_), _) => {
                Ok(PaneRefresh::Absent)
            }
        }
    }
}

/// The core binding and driver ports over every host. New panes are observed
/// on the primary host; a stored binding is always handled by its own host.
pub struct BindingSession<'a, R> {
    primary: HostKind,
    tmux: tmux::BindingSession<'a, R>,
    herdr: herdr::Session<'a, R>,
    /// Every other host, through its approved driver, one per host.
    external: external::Session<'a, R>,
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

    /// The driver of a host this session reaches.
    fn driver(&mut self, host: HostKind) -> &mut dyn driver::HostDriver {
        match host {
            HostKind::Tmux => &mut self.tmux,
            HostKind::Herdr => &mut self.herdr,
            HostKind::External(host) => self.external.driver(host),
        }
    }

    /// Runtime liveness after the caller verified this binding's endpoint.
    pub fn observed_runtime(&self, binding: &Binding) -> Result<RuntimeState, ActionError> {
        match binding.server.host {
            HostKind::Tmux => driver::HostDriver::observed_runtime(&self.tmux, binding),
            HostKind::Herdr => driver::HostDriver::observed_runtime(&self.herdr, binding),
            HostKind::External(_) => self.external.observed_runtime(binding),
        }
        .map_err(ActionError::Process)
    }
}

impl<R: CommandRunner> BindingEndpoint for BindingSession<'_, R> {
    type Error = HostError;

    fn current_host(&self) -> HostKind {
        self.primary
    }

    fn begin_coordination(&mut self) {
        self.tmux.begin_coordination();
        self.herdr.begin_coordination();
        self.external.begin_coordination();
    }

    fn budget_available(&self) -> bool {
        match self.primary {
            HostKind::Tmux => driver::HostDriver::budget_available(&self.tmux),
            HostKind::Herdr => driver::HostDriver::budget_available(&self.herdr),
            HostKind::External(_) => self.external.budget_available(),
        }
    }

    fn current_snapshot(&mut self, panes: &[String]) -> Result<EndpointSnapshot, Self::Error> {
        self.driver(self.primary).snapshot(panes)
    }

    fn probe_binding(
        &mut self,
        server: &ServerEvidence,
        panes: &[String],
    ) -> Result<EndpointProbe, Self::Error> {
        self.driver(server.host).probe(server, panes)
    }

    fn publish(&mut self, binding: &Binding, identity: &Identity) -> Result<(), Self::Error> {
        self.driver(binding.server.host).publish(binding, identity)
    }

    fn clear(&mut self, binding: &Binding) -> Result<bool, Self::Error> {
        self.driver(binding.server.host).clear(binding)
    }

    fn pane_incarnation(
        &mut self,
        server: &ServerEvidence,
        pane_pid: u64,
    ) -> Result<Option<String>, Self::Error> {
        self.driver(server.host).pane_incarnation(pane_pid)
    }
}

/// A binding's own host; an entry without one belongs to the primary host.
fn binding_host(entry: &BindingEntry, primary: HostKind) -> HostKind {
    entry
        .binding
        .as_ref()
        .map_or(primary, |binding| binding.server.host)
}

impl<R: CommandRunner> Driver for BindingSession<'_, R> {
    type Target = BindingEntry;
    type Error = ActionError;
    type Launch = ();

    fn status(&mut self, entry: &BindingEntry) -> ActionResult<InterfaceStatus, ActionError> {
        driver::status(self.driver(binding_host(entry, self.primary)), entry)
    }

    /// A host without pane input (Herdr until #479 H4) is `Unsupported`, and
    /// the core falls through to the Inbox.
    fn send(
        &mut self,
        entry: &BindingEntry,
        message: &str,
    ) -> ActionResult<DeliveryAcceptance, SendFailure<ActionError>> {
        driver::send(
            self.driver(binding_host(entry, self.primary)),
            entry,
            message,
        )
    }

    fn focus(&mut self, entry: &BindingEntry) -> ActionResult<Focused, ActionError> {
        driver::focus(self.driver(binding_host(entry, self.primary)), entry)
    }
}

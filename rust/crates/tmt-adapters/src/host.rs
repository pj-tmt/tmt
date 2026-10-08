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
//! A host without a server-level store (every external host) gets TMT's own
//! server UUID through core's `HostServerIds` port, resolved by
//! [`Host::resolve_servers`] before any binding transaction opens.

mod delivery;
pub(crate) mod driver;
pub mod external;

use external::ExternalCaller;

use crate::{
    process::{CommandError, CommandRunner, UnixCommandRunner},
    tmux::{self, Tmux, TmuxError},
};
use std::{
    collections::BTreeMap,
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
    host::{HostKind, HostServerIds, ServerSelector},
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
    pub process_id: u64,
    /// The variables approved drivers declared for `caller`, those set and
    /// non-empty; nothing else of the environment reaches a driver.
    pub driver_env: BTreeMap<String, OsString>,
}

impl CallerEnvironment {
    /// Cheap scheduling locators only. No variable proves a current caller;
    /// effects still require the host's native ancestry/endpoint verification.
    pub fn pane_locators(&self) -> Vec<(HostKind, &str, Option<&str>)> {
        let mut locators = Vec::new();
        if let Some(pane) = self.pane.as_ref().and_then(|value| value.to_str())
            && HostKind::Tmux.is_pane_id(pane)
            && let Ok(socket) = self.selected_server_socket()
        {
            locators.push((HostKind::Tmux, pane, socket));
        }
        for value in self.driver_env.values().filter_map(|value| value.to_str()) {
            for host in HostKind::all().filter(|host| *host != HostKind::Tmux) {
                if host.is_pane_id(value) && !locators.contains(&(host, value, None)) {
                    locators.push((host, value, None));
                }
            }
        }
        locators
    }

    pub fn current() -> Self {
        let driver_env = external::approved()
            .iter()
            .flat_map(|record| {
                record
                    .capabilities
                    .host()
                    .into_iter()
                    .flat_map(|value| value.caller_env.iter())
            })
            .filter_map(|name| {
                let value = std::env::var_os(name).filter(|value| !value.is_empty())?;
                Some((name.clone(), value))
            })
            .collect();
        Self {
            tmux: std::env::var_os("TMUX"),
            pane: std::env::var_os("TMUX_PANE"),
            process_id: u64::from(std::process::id()),
            driver_env,
        }
    }

    fn names_tmux(&self) -> bool {
        [&self.tmux, &self.pane]
            .into_iter()
            .any(|value| value.as_ref().is_some_and(|value| !value.is_empty()))
    }
}

#[derive(Debug)]
pub enum HostError {
    Tmux(TmuxError),
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
            Self::Unavailable(_)
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

// Transparent: a host error reads exactly as its host's own error.
impl fmt::Display for HostError {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tmux(error) => error.fmt(output),
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

/// A host error as the cause of a failed pane input (a driver's `input`).
impl DeliveryCause for HostError {
    fn socket_permission_denied(&self) -> bool {
        HostError::socket_permission_denied(self)
    }

    fn cleanup_failed(&self) -> bool {
        HostError::cleanup_failed(self)
    }
}

impl std::error::Error for HostError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Tmux(error) => error.source(),
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
    external: external::Drivers<R>,
}

impl Host {
    /// The host a caller-scoped command runs in: the host whose pane shell is
    /// the caller's nearest ancestor. Without a host in the environment,
    /// tmux's default server serves, as it always has.
    pub fn for_caller(environment: &CallerEnvironment) -> Self {
        Self::for_caller_with_drivers(environment, UnixCommandRunner, external::approved())
    }

    /// The host that runs a known server.
    pub fn for_server(server: &ServerEvidence) -> Self {
        Self::for_server_with(server, UnixCommandRunner).with_drivers(external::approved())
    }

    /// The host whose syntax an explicit pane target uses; text that no host
    /// reads as a target is resolved by name elsewhere.
    pub fn for_target(target: &str) -> Self {
        let canonical = tmt_core::names::normalize_name(target);
        // The syntaxes are disjoint (approval refuses an overlap); tmux, the
        // broadest, is the default.
        let host = HostKind::all()
            .filter(|host| *host != HostKind::Tmux)
            .find(|host| host.is_target(&canonical))
            .unwrap_or(HostKind::Tmux);
        Self::of(host, UnixCommandRunner).with_drivers(external::approved())
    }
}

impl<R: CommandRunner + Clone> Host<R> {
    /// The caller's host with no driver approved: tmux. The `_with`
    /// constructors approve no driver.
    pub fn for_caller_with(environment: &CallerEnvironment, runner: R) -> Self {
        Self::for_caller_with_drivers(environment, runner, Vec::new())
    }

    /// The caller's host, external ones included: the nearest verified pane
    /// wins. Without a verified external pane the host is tmux.
    pub fn for_caller_with_drivers(
        environment: &CallerEnvironment,
        runner: R,
        records: Vec<crate::driver_protocol::registry::DriverRecord>,
    ) -> Self {
        let builtin = Self::of(HostKind::Tmux, runner.clone());
        if environment.driver_env.is_empty() || records.is_empty() {
            return builtin.with_drivers(records);
        }
        let probe = builtin.with_drivers(records.clone());
        match probe.nearest_external(environment) {
            Some(caller) => {
                let host = Self::of(HostKind::External(caller.host), runner).with_drivers(records);
                host.external.set_caller(caller);
                host
            }
            None => probe,
        }
    }

    pub fn for_server_with(server: &ServerEvidence, runner: R) -> Self {
        let host = Self::of(server.host, runner);
        if let HostKind::External(_) = server.host {
            host.external.set_resolved(server.clone());
        }
        host
    }

    /// The drivers this handle may run for external hosts. The `_with`
    /// constructors approve none, so tests never read the user's registry.
    pub fn with_drivers(
        self,
        records: Vec<crate::driver_protocol::registry::DriverRecord>,
    ) -> Self {
        let runner = self.external.runner().clone();
        let resolved = self.external.resolved().cloned();
        let caller = self.external.caller().cloned();
        let external = external::Drivers::new(runner, records);
        if let Some(server) = resolved {
            external.set_resolved(server);
        }
        if let Some(caller) = caller {
            external.set_caller(caller);
        }
        Self { external, ..self }
    }

    fn of(primary: HostKind, runner: R) -> Self {
        Self {
            primary,
            tmux: Tmux::new(runner.clone()),
            external: external::Drivers::new(runner, Vec::new()),
        }
    }
}

impl<R: CommandRunner> Host<R> {
    pub fn kind(&self) -> HostKind {
        self.primary
    }

    /// The nearest verified external pane of the caller, unless the caller's
    /// tmux pane is nearer (tmux in a Herdr pane, or the reverse). Failures
    /// prove nothing and keep tmux.
    fn nearest_external(&self, environment: &CallerEnvironment) -> Option<ExternalCaller> {
        let deadline = Instant::now() + Duration::from_secs(1);
        let external = self
            .external
            .callers(environment, deadline)
            .ok()?
            .into_iter()
            .next()?;
        let tmux = environment
            .names_tmux()
            .then(|| self.tmux.caller_depth(environment).ok().flatten())
            .flatten();
        tmux.is_none_or(|depth| external.depth < depth)
            .then_some(external)
    }

    /// The server a caller's environment selects on this handle's host, for
    /// presentation priority only: tmux's from its variables, an external
    /// host's from the caller's verified pane.
    pub fn selected_server<'a>(
        &'a self,
        environment: &'a CallerEnvironment,
    ) -> Option<ServerSelector<'a>> {
        match self.primary {
            HostKind::Tmux => environment.selected_server(),
            HostKind::External(_) => self
                .external
                .caller()
                .filter(|caller| HostKind::External(caller.host) == self.primary)
                .map(|caller| ServerSelector {
                    host: self.primary,
                    socket: &caller.socket,
                }),
        }
    }

    /// Resolve TMT's UUID for this handle's external server before any
    /// binding transaction; tmux keeps its own in a server option. A server
    /// without panes stays unresolved: there is nothing on it to bind.
    pub fn resolve_servers<I: HostServerIds>(&self, ids: &mut I) -> Result<(), HostError> {
        match self.primary {
            HostKind::Tmux => Ok(()),
            HostKind::External(host) => self.external.resolve_server(host, ids),
        }
    }
    /// The core binding and driver ports.
    pub fn session(&self) -> BindingSession<'_, R> {
        BindingSession {
            primary: self.primary,
            tmux: tmux::BindingSession::new(&self.tmux),
            external: external::Session::new(&self.external),
        }
    }

    pub fn caller_pane(
        &self,
        environment: &CallerEnvironment,
    ) -> Result<Option<String>, HostError> {
        match self.primary {
            HostKind::Tmux => Ok(self.tmux.caller_pane(environment)?),
            HostKind::External(host) => Ok(self
                .external
                .caller()
                .filter(|caller| caller.host == host)
                .map(|caller| caller.pane_id.clone())),
        }
    }

    /// How the user names the caller's own pane `id`: its ID on tmux, its
    /// public target on an external host, as the driver reports it. A pane
    /// the driver can't describe keeps its ID.
    pub fn caller_label(&self, id: &str) -> Result<String, HostError> {
        let HostKind::External(_) = self.primary else {
            return Ok(id.to_owned());
        };
        let target = self
            .external
            .caller_target(Instant::now() + Duration::from_secs(1))?;
        Ok(self.primary.pane_address(id, target.as_deref()).to_owned())
    }

    /// tmux's explicit mark; external hosts have none.
    pub fn marked_pane(
        &self,
        environment: &CallerEnvironment,
        options: OperationOptions<'_>,
    ) -> Result<Option<BindingTargetEvidence>, HostError> {
        match self.primary {
            HostKind::Tmux => Ok(self.tmux.marked_pane(environment, options)?),
            HostKind::External(_) => Ok(None),
        }
    }

    pub fn resolve_target(
        &self,
        target: &str,
        options: OperationOptions<'_>,
    ) -> Result<Option<String>, HostError> {
        match self.primary {
            HostKind::Tmux => Ok(self.tmux.resolve_target(target, options)?),
            HostKind::External(host) => {
                // `server` (when no caller names the socket), then `resolve-target`.
                let bound = Instant::now() + Duration::from_secs(1);
                let deadline = options
                    .deadline
                    .map_or(bound, |deadline| deadline.min(bound));
                self.external.resolve_target(host, target, deadline)
            }
        }
    }

    pub fn snapshot(&self, options: OperationOptions<'_>) -> Result<EndpointSnapshot, HostError> {
        match self.primary {
            HostKind::Tmux => Ok(self.tmux.snapshot(options)?),
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
            HostKind::External(host) => self.external.capture(
                host,
                &endpoint.server.socket_path,
                &endpoint.pane_id,
                u32::try_from(lines).unwrap_or(u32::MAX),
            ),
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
            HostKind::External(host) => self.external.send(
                host,
                &endpoint.server.socket_path,
                &endpoint.pane_id,
                &tmt_core::driver::pane_input_text(message),
                enter_delay,
            ),
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

    /// Post-commit pane cosmetics. An external host's pane has no badge;
    /// only its marker's name is kept current.
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
            (HostKind::External(host), PaneCosmetics::Bound { identity, .. }) => {
                let mut session = external::Session::new(&self.external);
                session.begin_coordination();
                session.limit(Some(deadline));
                match driver::refresh_marker_name(session.driver(host), binding, identity) {
                    Ok(refresh) => Ok(refresh),
                    Err(error) if error.cleanup_failed() => Err(error),
                    Err(_) => Ok(PaneRefresh::Failed),
                }
            }
            (HostKind::External(_), PaneCosmetics::Ended) => Ok(PaneRefresh::Absent),
        }
    }
}

/// The core binding and driver ports over every host. New panes are observed
/// on the primary host; a stored binding is always handled by its own host.
pub struct BindingSession<'a, R> {
    primary: HostKind,
    tmux: tmux::BindingSession<'a, R>,
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
        self.external = self.external.with_enter_delay(delay);
        self
    }

    /// The driver of a host this session reaches.
    fn driver(&mut self, host: HostKind) -> &mut dyn driver::HostDriver {
        match host {
            HostKind::Tmux => &mut self.tmux,
            HostKind::External(host) => self.external.driver(host),
        }
    }

    /// Recent user keys in a verified binding, without provider buffer guesses.
    pub fn input_activity(
        &mut self,
        entry: &BindingEntry,
    ) -> Result<tmt_core::driver::InputActivity, ActionError> {
        driver::input_activity(self.driver(binding_host(entry, self.primary)), entry)
    }

    /// Runtime liveness after the caller verified this binding's endpoint.
    pub fn observed_runtime(&self, binding: &Binding) -> Result<RuntimeState, ActionError> {
        match binding.server.host {
            HostKind::Tmux => driver::HostDriver::observed_runtime(&self.tmux, binding),
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
        self.external.begin_coordination();
    }

    fn budget_available(&self) -> bool {
        match self.primary {
            HostKind::Tmux => driver::HostDriver::budget_available(&self.tmux),
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

    /// A host without pane input is `Unsupported`, and the core falls
    /// through to the Inbox.
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

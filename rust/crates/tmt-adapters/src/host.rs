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
pub mod launch;
mod nested;
mod pty;

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
pub use driver::{AgentNameRefusal, AgentNaming};
pub use driver::{Kept, PaneClose, PaneSwap};
pub use tmt_core::discovery::DiscoveryObservation;
pub use tmt_core::host::SwapDirection;

/// The default launch template of every host: tmux's own, then each approved host driver's
/// (see [`launch::HostLaunch`]).
pub fn launch_templates(
    drivers: &[(
        crate::driver_protocol::registry::DriverRecord,
        crate::driver_protocol::registry::ApprovalState,
    )],
) -> Vec<launch::HostLaunch> {
    launch::compose(
        launch::HostLaunch {
            host: HostKind::Tmux.as_str().into(),
            source: launch::HostSource::BuiltIn,
            launch: Some(tmux::launch_template()),
        },
        drivers,
    )
}

/// Recovery topology and evidence returned by the optional host capture port.
pub struct WorkspaceCapture {
    pub snapshot: tmt_core::workspace::WorkspaceSnapshot,
    pub evidence: Vec<(String, u64, Option<tmt_core::endpoint::BindingMarker>)>,
    pub binding_server: Option<ServerEvidence>,
    pub starts: std::collections::HashMap<u64, tmt_core::endpoint::ProcessIncarnation>,
    pub terminals: BTreeMap<String, String>,
}

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
        Self::approved(&external::approved())
    }

    /// The current environment, with the caller variables that `records`
    /// declare.
    pub fn approved(records: &[crate::driver_protocol::registry::DriverRecord]) -> Self {
        Self::declared(records.iter().flat_map(|record| {
            record
                .capabilities
                .host()
                .into_iter()
                .flat_map(|value| value.caller_env.iter())
        }))
    }

    /// The current environment, with the caller variables of a command's
    /// admitted drivers and of its gaps: a gap's variables still say which
    /// host the caller is in, though its driver never runs.
    pub fn admitted(admitted: &crate::driver_protocol::registry::Admitted) -> Self {
        let mut environment = Self::approved(&admitted.records);
        let gaps = Self::declared(admitted.gaps.iter().flat_map(|gap| gap.caller_env.iter()));
        environment.driver_env.extend(gaps.driver_env);
        environment
    }

    fn declared<'a>(names: impl Iterator<Item = &'a String>) -> Self {
        let mut driver_env: BTreeMap<String, OsString> = names
            .filter_map(|name| {
                let value = std::env::var_os(name).filter(|value| !value.is_empty())?;
                Some((name.clone(), value))
            })
            .collect();
        for name in [pty::PANE_ENV, pty::SOCKET_ENV] {
            if let Some(value) = std::env::var_os(name).filter(|value| !value.is_empty()) {
                driver_env.insert(name.into(), value);
            }
        }
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

    /// Which host the caller's locators name, among the `records` approved.
    /// A set variable names its host even when it is not valid text; only a
    /// nonempty tmux variable names tmux. It runs no driver.
    pub fn named_host<'a>(
        &self,
        records: &'a [crate::driver_protocol::registry::DriverRecord],
    ) -> NamedHost<'a> {
        let mut named: Vec<_> = records
            .iter()
            .filter(|record| {
                record.capabilities.host().is_some_and(|host| {
                    host.caller_env
                        .iter()
                        .any(|name| self.driver_env.contains_key(name))
                })
            })
            .collect();
        named.dedup_by(|a, b| a.name == b.name);
        if self.driver_env.contains_key(pty::PANE_ENV)
            || self.driver_env.contains_key(pty::SOCKET_ENV)
        {
            return if !named.is_empty() {
                NamedHost::Ambiguous
            } else if self.names_tmux() {
                NamedHost::PtyNested
            } else {
                NamedHost::Pty
            };
        }
        match (self.names_tmux(), named.as_slice()) {
            (_, []) => NamedHost::Builtin,
            (false, [record]) => NamedHost::External(record),
            (true, [record]) => NamedHost::Nested(record),
            _ => NamedHost::Ambiguous,
        }
    }
}

/// Why implicit selection cannot choose the caller's host for a command that
/// read its drivers once ([`CallerEnvironment::unselectable`]).
#[derive(Debug, PartialEq, Eq)]
pub enum Unselectable<'a> {
    /// The locators name a host whose approval is a gap, alone or with tmux:
    /// without running that driver, no pane is proven nearest.
    Gap(&'a crate::driver_protocol::registry::Gap),
    /// The locators name more than one external host, listed by name;
    /// registry order never chooses.
    Ambiguous(Vec<&'a str>),
    /// No variable is known to name a host.
    RegistryUnreadable,
}

impl CallerEnvironment {
    /// Whether implicit selection must refuse before any driver or tmux call,
    /// and why. `None` keeps the ordinary caller selection. It runs nothing.
    pub fn unselectable<'a>(
        &self,
        admitted: &'a crate::driver_protocol::registry::Admitted,
    ) -> Option<Unselectable<'a>> {
        if admitted.unreadable.is_some() {
            return Some(Unselectable::RegistryUnreadable);
        }
        let names = |declared: &[String]| {
            declared
                .iter()
                .any(|name| self.driver_env.contains_key(name))
        };
        let gaps: Vec<_> = admitted
            .gaps
            .iter()
            .filter(|gap| names(&gap.caller_env))
            .collect();
        let mut hosts: Vec<&str> = admitted
            .records
            .iter()
            .filter(|record| {
                record
                    .capabilities
                    .host()
                    .is_some_and(|host| names(&host.caller_env))
            })
            .map(|record| record.name.as_str())
            .chain(gaps.iter().map(|gap| gap.name.as_str()))
            .collect();
        hosts.sort_unstable();
        hosts.dedup();
        match (hosts.len(), gaps.as_slice()) {
            (0 | 1, []) => None,
            (1, [gap]) => Some(Unselectable::Gap(gap)),
            _ => Some(Unselectable::Ambiguous(hosts)),
        }
    }
}

/// The host a caller's locators name: none of the approved external hosts
/// (tmux, or outside any host), exactly one external host and no tmux, tmux
/// and exactly one external host (nested, in either order), or more than one
/// external host (unsupported).
#[derive(Debug)]
pub enum NamedHost<'a> {
    Pty,
    PtyNested,
    Builtin,
    External(&'a crate::driver_protocol::registry::DriverRecord),
    Nested(&'a crate::driver_protocol::registry::DriverRecord),
    Ambiguous,
}

/// The most a visible-screen read returns; larger output fails whole.
pub const SCREEN_MAX_BYTES: usize = 1024 * 1024;

/// How a host encodes a pane's visible screen: tmux keeps its SGR attributes,
/// an external host's driver answers plain text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenFormat {
    Ansi,
    Plain,
}

#[derive(Debug)]
pub enum HostError {
    Tmux(TmuxError),
    /// Built-in supervisor transport failure; never proof of process loss.
    Pty(std::io::Error),
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
    /// TMT's own read of the caller's processes could not clean up.
    Process(CommandError),
}

impl HostError {
    pub fn cleanup_failed(&self) -> bool {
        match self {
            Self::Tmux(error) => error.cleanup_failed(),
            Self::Driver(external::CallError::Process { error, .. }) => error.cleanup_failed(),
            Self::Process(error) => error.cleanup_failed(),
            Self::Pty(_)
            | Self::Unavailable(_)
            | Self::Driver(_)
            | Self::Refused { .. }
            | Self::Evidence { .. }
            | Self::ServerIds(_) => false,
        }
    }

    pub fn socket_permission_denied(&self) -> bool {
        match self {
            Self::Tmux(error) => error.socket_permission_denied(),
            Self::Pty(_)
            | Self::Unavailable(_)
            | Self::Driver(_)
            | Self::Refused { .. }
            | Self::Evidence { .. }
            | Self::ServerIds(_)
            | Self::Process(_) => false,
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
            Self::Pty(error) => write!(output, "Built-in host: {error}"),
            Self::Unavailable(name) => write!(output, "Host driver {name} is not installed."),
            Self::Driver(error) => error.fmt(output),
            // The driver's message is untrusted text, bounded and free of
            // control characters by the protocol.
            Self::Refused {
                driver, message, ..
            } => write!(output, "Host driver {driver}: {message}"),
            Self::Evidence { driver, reason } => write!(output, "Host driver {driver}: {reason}."),
            Self::ServerIds(error) => write!(output, "Could not record the host server: {error}"),
            Self::Process(error) => error.fmt(output),
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
            Self::Pty(error) => Some(error),
            Self::Driver(error) => Some(error),
            Self::ServerIds(error) => Some(error.as_ref()),
            Self::Process(error) => Some(error),
            Self::Unavailable(_) | Self::Refused { .. } | Self::Evidence { .. } => None,
        }
    }
}

#[derive(Debug)]
pub enum ActionError {
    Evidence(HostError),
    Unverified,
    NoNeighbour,
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
            Self::NoNeighbour => f.write_str("No visible neighbour in that direction."),
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
            Self::Unverified | Self::NoNeighbour | Self::Offline | Self::HostUnsupported => None,
        }
    }
}

/// One-release option-cutover proof; the host facade keeps native tmux details
/// out of launch and upgrade command owners.
pub use crate::tmux::rename::OptionRename;

pub struct Host<R = UnixCommandRunner> {
    primary: HostKind,
    tmux: Tmux<R>,
    external: external::Drivers<R>,
    /// The caller's pane shell pid on the pinned ancestry that chose a nested
    /// caller's host; `None` for every other handle.
    pinned_pane: Option<u64>,
}

impl<R: CommandRunner> Host<R> {
    /// One-release upgrade operation on the captured binding's selected host.
    /// External hosts have no retired tmux options and are never probed here.
    pub fn prepare_option_rename(
        &self,
        entry: &BindingEntry,
        deadline: Instant,
    ) -> Result<Option<OptionRename>, HostError> {
        if self.primary != HostKind::Tmux
            || entry
                .binding
                .as_ref()
                .is_none_or(|binding| binding.server.host != HostKind::Tmux)
        {
            return Err(HostError::Unavailable("option cutover is tmux-only".into()));
        }
        self.tmux
            .prepare_option_rename(entry, deadline)
            .map_err(Into::into)
    }

    /// Caller holds the full-record storage fence while committing this proof.
    pub fn commit_option_rename(
        &self,
        proof: &OptionRename,
        deadline: Instant,
    ) -> Result<bool, HostError> {
        if self.primary != HostKind::Tmux {
            return Ok(false);
        }
        self.tmux
            .commit_option_rename(proof, deadline)
            .map_err(Into::into)
    }
}

impl Host {
    /// A newly created PTY replaces every inherited host locator in its child
    /// only. The resulting locators still require native ancestry verification.
    pub fn pty_environment(
        directory: &std::path::Path,
        pane: &str,
    ) -> Vec<(OsString, Option<OsString>)> {
        let mut names = vec!["TMUX".to_owned(), "TMUX_PANE".to_owned()];
        for record in external::approved() {
            if let Some(host) = record.capabilities.host() {
                names.extend(host.caller_env.iter().cloned());
            }
        }
        let mut values: Vec<_> = names
            .into_iter()
            .map(|name| (OsString::from(name), None))
            .collect();
        values.push((pty::PANE_ENV.into(), Some(pane.into())));
        values.push((
            pty::SOCKET_ENV.into(),
            Some(crate::pty::socket_path(directory, pane).into_os_string()),
        ));
        values
    }

    /// Before any managed preparation or stop: make sure the launcher is
    /// listening, including for a command that fails or exits immediately.
    pub fn wait_for_launcher(&self, environment: &CallerEnvironment) -> Result<(), HostError> {
        if self.primary != HostKind::pty() {
            return Ok(());
        }
        pty::wait_for_launcher(environment)
    }

    /// A receipt to the background launcher after ordinary managed admission.
    /// This is not a new session observation and changes no binding state.
    pub fn notify_launch_ready(&self, binding: &Binding) -> Result<(), HostError> {
        if self.primary != HostKind::pty() {
            return Ok(());
        }
        pty::launch_ready(binding)
    }

    /// The host a caller-scoped command runs in: the host whose pane shell is
    /// the caller's nearest ancestor. Without a host in the environment,
    /// tmux's default server serves, as it always has.
    pub fn for_caller(environment: &CallerEnvironment) -> Self {
        Self::for_caller_with_drivers(environment, UnixCommandRunner, external::approved())
    }

    /// A recorded or explicit recovery socket uses only the built-in reader,
    /// without discovering caller environment, external approvals or state paths.
    pub fn for_workspace_socket() -> Self {
        Self::of(HostKind::Tmux, UnixCommandRunner)
    }

    /// An explicit recovery socket selects the built-in host even outside its pane.
    /// Implicit recovery keeps the caller's ordinary host selection.
    pub fn for_workspace(environment: &CallerEnvironment, explicit_socket: bool) -> Self {
        if explicit_socket {
            Self::for_workspace_socket()
        } else {
            Self::for_caller(environment)
        }
    }

    /// The host that runs a known server.
    pub fn for_server(server: &ServerEvidence) -> Self {
        Self::for_server_with(server, UnixCommandRunner).with_drivers(external::approved())
    }

    /// The host whose syntax an explicit pane target uses; text that no host
    /// reads as a target is resolved by name elsewhere.
    pub fn for_target(target: &str) -> Self {
        let text = tmt_core::names::target_text(target);
        // The syntaxes are disjoint (approval refuses an overlap); tmux, the
        // broadest, is the default.
        let host = HostKind::all()
            .filter(|host| *host != HostKind::Tmux)
            .find(|host| host.is_target(&text))
            .unwrap_or(HostKind::Tmux);
        let handle = Self::of(host, UnixCommandRunner).with_drivers(external::approved());
        if host == HostKind::pty() {
            handle.external.set_target(text);
        }
        handle
    }
}

impl<R: CommandRunner + Clone> Host<R> {
    /// The caller's host with no driver approved: tmux. The `_with`
    /// constructors approve no driver.
    pub fn for_caller_with(environment: &CallerEnvironment, runner: R) -> Self {
        Self::for_caller_with_drivers(environment, runner, Vec::new())
    }

    /// The caller's host, external ones included: the nearest verified pane
    /// wins. Without a verified external pane the host is tmux. An ordinary
    /// command keeps tmux even when a selection read fails to clean up.
    pub fn for_caller_with_drivers(
        environment: &CallerEnvironment,
        runner: R,
        records: Vec<crate::driver_protocol::registry::DriverRecord>,
    ) -> Self {
        let deadline = Instant::now() + Duration::from_secs(1);
        Self::for_caller_within(environment, runner.clone(), records.clone(), deadline)
            .unwrap_or_else(|_| Self::of(HostKind::Tmux, runner).with_drivers(records))
    }

    /// [`Host::for_caller_with_drivers`] for discovery, spending at most
    /// until `deadline` on finding the caller's pane. Doubt keeps tmux; only
    /// a failed cleanup is an error, and it chooses no host.
    pub fn for_caller_within(
        environment: &CallerEnvironment,
        runner: R,
        records: Vec<crate::driver_protocol::registry::DriverRecord>,
        deadline: Instant,
    ) -> Result<Self, HostError> {
        let builtin = Self::of(HostKind::Tmux, runner.clone());
        if environment.driver_env.is_empty()
            || (records.is_empty() && !environment.driver_env.contains_key(pty::PANE_ENV))
        {
            return Ok(builtin.with_drivers(records));
        }
        let probe = builtin.with_drivers(records.clone());
        Ok(match probe.nearest_external(environment, deadline)? {
            Some(caller) => {
                let host = Self::of(HostKind::External(caller.host), runner).with_drivers(records);
                host.external.set_caller(caller);
                host
            }
            None => probe,
        })
    }

    /// A hook or discovery caller's host: the one host its locators name,
    /// verified, never a fallback. No locator keeps tmux (outside any host);
    /// tmux variables alone keep the tmux caller check. One named external
    /// host must answer conclusively with the caller's pane. tmux and one
    /// external host (nested) admit only the owner that one pinned ancestry
    /// proves ([`Host::for_nested_caller`]). Locators naming more than one
    /// external host refuse whatever the pane checks say. The external server
    /// ID must be one a bind recorded (`ids` never records one).
    pub fn for_verified_caller<I: HostServerIds>(
        environment: &CallerEnvironment,
        runner: R,
        records: Vec<crate::driver_protocol::registry::DriverRecord>,
        ids: &mut I,
    ) -> Result<Self, HostError> {
        let unverified =
            |reason: &str| HostError::Unavailable(format!("caller host unverified: {reason}"));
        let deadline = Instant::now() + Duration::from_secs(1);
        let builtin = Self::of(HostKind::Tmux, runner.clone()).with_drivers(records.clone());
        match environment.named_host(&records) {
            NamedHost::Builtin => return Ok(builtin),
            NamedHost::External(_) | NamedHost::Pty => {}
            NamedHost::Nested(_) | NamedHost::PtyNested => {
                return Self::for_nested_caller(
                    environment,
                    builtin,
                    runner,
                    records,
                    ids,
                    deadline,
                );
            }
            NamedHost::Ambiguous => {
                return Err(unverified("the caller's locators name more than one host"));
            }
        }
        let callers = builtin.external.callers(environment, None, deadline)?;
        let [external] = <[ExternalCaller; 1]>::try_from(callers.found)
            .ok()
            .filter(|_| callers.conclusive)
            .ok_or_else(|| unverified("no approved driver verified the caller's pane"))?;
        Self::for_external_caller(external, runner, records, ids)
    }

    /// The handle for a verified external caller pane, whose server must be
    /// one a bind recorded.
    fn for_external_caller<I: HostServerIds>(
        external: ExternalCaller,
        runner: R,
        records: Vec<crate::driver_protocol::registry::DriverRecord>,
        ids: &mut I,
    ) -> Result<Self, HostError> {
        let name = external.host;
        let host = Self::of(HostKind::External(name), runner).with_drivers(records);
        host.external.set_caller(external);
        host.resolve_servers(ids)?;
        match host.external.resolved() {
            Some(_) => Ok(host),
            None => Err(HostError::Evidence {
                driver: name.as_str().to_owned(),
                reason: "the caller's server was not resolved",
            }),
        }
    }

    /// An explicitly selected host, with the drivers a command read once.
    pub fn for_host(
        host: HostKind,
        runner: R,
        records: Vec<crate::driver_protocol::registry::DriverRecord>,
    ) -> Self {
        Self::of(host, runner).with_drivers(records)
    }

    /// [`Host::for_host`] for a scan with no socket: on an external host, the
    /// caller's own verified pane of that host names its server, found by
    /// that host's driver alone until `deadline`. Without one, or in doubt,
    /// the driver's default server serves. Only a failed cleanup is an error.
    pub fn for_selected_host(
        host: HostKind,
        environment: &CallerEnvironment,
        runner: R,
        records: Vec<crate::driver_protocol::registry::DriverRecord>,
        deadline: Instant,
    ) -> Result<Self, HostError> {
        let HostKind::External(name) = host else {
            return Ok(Self::for_host(host, runner, records));
        };
        let own = records
            .iter()
            .filter(|record| record.name == name.as_str())
            .cloned()
            .collect();
        let probe = Self::for_host(host, runner.clone(), own);
        let caller = probe.external.callers(environment, None, deadline)?.found;
        let selected = Self::for_host(host, runner, records);
        if let Some(caller) = caller.into_iter().find(|caller| caller.host == name) {
            selected.external.set_caller(caller);
        }
        Ok(selected)
    }

    pub fn for_server_with(server: &ServerEvidence, runner: R) -> Self {
        let mut host = Self::of(server.host, runner);
        match server.host {
            HostKind::Tmux => host.tmux = host.tmux.on_socket(&server.socket_path),
            HostKind::External(_) => host.external.set_resolved(server.clone()),
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
        let target = self.external.target().map(str::to_owned);
        let external = external::Drivers::new(runner, records);
        if let Some(target) = target {
            external.set_target(target);
        }
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
            pinned_pane: None,
        }
    }
}

impl<R: CommandRunner> Host<R> {
    /// First-run inventory never initializes host metadata or records a server.
    pub fn discover(
        &self,
        socket: Option<&str>,
        pane: Option<&str>,
        deadline: Instant,
    ) -> Result<DiscoveryObservation, HostError> {
        match self.primary {
            HostKind::Tmux => self
                .tmux
                .discover(socket, pane, deadline)
                .map_err(HostError::from),
            HostKind::External(host) => self.external.discover(host, socket, pane, deadline),
        }
    }

    /// Before a guarded bind's transaction: records TMT's UUID for the
    /// server of `fence` and resolves this handle to it, only once the host
    /// still names that socket and core sees that very server process. tmux
    /// keeps its own server ID and records nothing here.
    pub fn resolve_fenced_server<I: HostServerIds>(
        &self,
        fence: &tmt_core::discovery::DiscoveryTarget,
        ids: &mut I,
    ) -> Result<Option<String>, HostError> {
        match self.primary {
            HostKind::Tmux => Ok(None),
            HostKind::External(host) => self
                .external
                .resolve_fenced_server(host, fence, ids, Instant::now() + Duration::from_secs(3))
                .map(Some),
        }
    }

    pub fn kind(&self) -> HostKind {
        self.primary
    }

    pub fn workspace_supported(&self) -> bool {
        self.primary == HostKind::Tmux
    }

    /// The nearest verified external pane of the caller. With tmux variables
    /// too (tmux in a Herdr pane, or the reverse), one pinned ancestry orders
    /// both panes ([`Host::nearest_nested_external`]). Failures prove
    /// nothing and keep tmux; only a failed cleanup is an error.
    fn nearest_external(
        &self,
        environment: &CallerEnvironment,
        deadline: Instant,
    ) -> Result<Option<ExternalCaller>, HostError> {
        if environment.names_tmux() {
            return self.nearest_nested_external(environment, deadline);
        }
        Ok(self
            .external
            .callers(environment, None, deadline)?
            .found
            .into_iter()
            .next())
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

    /// The caller's pane for a command that cannot do without it; see
    /// [`tmux::Tmux::caller_pane_required`]. An external host answers as [`Host::caller_pane`].
    pub fn caller_pane_required(
        &self,
        environment: &CallerEnvironment,
    ) -> Result<Option<String>, HostError> {
        match self.primary {
            HostKind::Tmux => Ok(self.tmux.caller_pane_required(environment)?),
            HostKind::External(_) => self.caller_pane(environment),
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

    pub fn workspace_pane_tty(
        &self,
        socket: &str,
        pane: &str,
        deadline: Instant,
    ) -> Result<Option<String>, HostError> {
        match self.primary {
            HostKind::Tmux => Ok(Some(self.tmux.workspace_pane_tty(socket, pane, deadline)?)),
            HostKind::External(_) => Ok(None),
        }
    }

    /// Recovery preview reads only current names, on the explicitly selected socket.
    /// Unsupported hosts leave observation unknown rather than guessing absence.
    pub fn workspace_session_names(
        &self,
        socket: &str,
        deadline: Instant,
    ) -> Result<Option<Vec<String>>, HostError> {
        match self.primary {
            HostKind::Tmux => Ok(Some(self.tmux.workspace_session_names(socket, deadline)?)),
            HostKind::External(_) => Ok(None),
        }
    }

    /// Layout creation owns only newly returned resources on the selected socket.
    pub fn workspace_restore_layout(
        &self,
        snapshot: &tmt_core::workspace::WorkspaceSnapshot,
        deadline: Instant,
        revival_panes: &[String],
    ) -> Result<Option<crate::workspace::restore::LayoutRestore>, HostError> {
        match self.primary {
            HostKind::Tmux => Ok(Some(self.tmux.workspace_restore_layout(
                snapshot,
                deadline,
                revival_panes,
            )?)),
            HostKind::External(_) => Ok(None),
        }
    }

    pub fn workspace_start_pane(
        &self,
        snapshot: &tmt_core::workspace::WorkspaceSnapshot,
        layout: &crate::workspace::restore::LayoutRestore,
        pane: &crate::workspace::restore::RestoredPane,
        argv: &[String],
        deadline: Instant,
    ) -> Result<bool, HostError> {
        match self.primary {
            HostKind::Tmux => {
                self.tmux
                    .workspace_start_pane(snapshot, layout, pane, argv, deadline)?;
                Ok(true)
            }
            HostKind::External(_) => Ok(false),
        }
    }

    /// Recovery capture is optional; external drivers have no workspace port yet.
    pub fn workspace_capture(
        &self,
        socket: &str,
        expected: Option<&ServerEvidence>,
        caller: Option<&CallerEnvironment>,
        deadline: Instant,
    ) -> Result<Option<WorkspaceCapture>, HostError> {
        match self.primary {
            HostKind::Tmux => Ok(Some(
                self.tmux
                    .workspace_capture(socket, expected, caller, deadline)?,
            )),
            HostKind::External(_) => Ok(None),
        }
    }

    pub fn workspace_command_marker(
        &self,
        socket: &str,
        pane: &str,
        document: Option<&str>,
        deadline: Instant,
    ) -> Result<(), HostError> {
        match self.primary {
            HostKind::Tmux => Ok(self
                .tmux
                .workspace_command_marker(socket, pane, document, deadline)?),
            HostKind::External(_) => Ok(()),
        }
    }

    pub fn clear_workspace_command(
        &self,
        socket: &str,
        pane: &str,
        expected: &str,
        deadline: Instant,
    ) -> Result<(), HostError> {
        match self.primary {
            HostKind::Tmux => Ok(self
                .tmux
                .clear_workspace_command(socket, pane, expected, deadline)?),
            HostKind::External(_) => Ok(()),
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

    /// The pane's visible screen as its host renders it, for a read-only view.
    pub fn screen(&self, endpoint: &RequestEndpoint) -> Result<(ScreenFormat, String), HostError> {
        match endpoint.server.host {
            HostKind::Tmux => Ok((
                ScreenFormat::Ansi,
                self.tmux
                    .screen_on(&endpoint.server.socket_path, &endpoint.pane_id)?,
            )),
            HostKind::External(host) => self
                .external
                .capture(host, &endpoint.server.socket_path, &endpoint.pane_id, 0)
                .map(|text| (ScreenFormat::Plain, text)),
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

    pub fn swap_pane(
        &self,
        invoker: &Invoker,
        pane: &str,
        direction: SwapDirection,
        options: OperationOptions<'_>,
    ) -> Result<PaneSwap, FocusError> {
        match self.primary {
            HostKind::Tmux => self.tmux.swap_pane(invoker, pane, direction, options),
            HostKind::External(_) => Err(FocusError::HostUnsupported),
        }
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

    /// Whether the binding's host gives agents names TMT can set: never on
    /// tmux, and on an external host only when its approved driver declares
    /// `name-agent`. It runs no driver.
    pub fn offers_agent_names(&self, binding: &Binding) -> bool {
        match binding.server.host {
            HostKind::Tmux => false,
            HostKind::External(host) => self
                .external
                .offers(host, tmt_driver_protocol::Op::NameAgent),
        }
    }

    /// After a committed bind or rename: gives the agent in the binding's
    /// pane the identity's name as its agent name, on a host that has agent
    /// names. Never a pane label, and never during a read command. A
    /// refusal is the driver's [`HostError::Refused`]; the binding stands
    /// either way. A caller's `deadline` caps the whole call, the driver's
    /// start included, when it is sooner than the operation's own.
    pub fn name_agent(
        &self,
        binding: &Binding,
        identity: &Identity,
        deadline: Option<Instant>,
    ) -> Result<driver::AgentNaming, HostError> {
        if identity.id != binding.identity_id {
            return Ok(driver::AgentNaming::NotLinked);
        }
        match binding.server.host {
            HostKind::Tmux => Ok(driver::AgentNaming::NotLinked),
            HostKind::External(host) => {
                let mut session = external::Session::new(&self.external);
                session.begin_coordination();
                session.limit(deadline);
                driver::HostDriver::name_agent(session.driver(host), binding, &identity.name)
            }
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
    pub fn with_discovery_target(mut self, target: tmt_core::discovery::DiscoveryTarget) -> Self {
        self.external = self.external.with_discovery_target(target.clone());
        self.tmux = self.tmux.with_discovery_target(target);
        self
    }

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

    /// Closes the member's pane only when the host says tmt opened it; see
    /// [`driver::close_opened_pane`].
    pub fn close_opened_pane(&mut self, entry: &BindingEntry) -> Result<PaneClose, ActionError> {
        driver::close_opened_pane(self.driver(binding_host(entry, self.primary)), entry)
    }

    pub fn swap_into(
        &mut self,
        entry: &BindingEntry,
        direction: SwapDirection,
    ) -> Result<PaneSwap, ActionError> {
        driver::swap_into(
            self.driver(binding_host(entry, self.primary)),
            entry,
            direction,
        )
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

    fn clear(&mut self, binding: &Binding, identity: &Identity) -> Result<bool, Self::Error> {
        self.driver(binding.server.host).clear(binding, identity)
    }

    fn pane_incarnation(
        &mut self,
        server: &ServerEvidence,
        pane_pid: u64,
    ) -> Result<Option<String>, Self::Error> {
        self.driver(server.host).pane_incarnation(pane_pid)
    }
}

impl<R: CommandRunner> tmt_core::binding::GuardedBindingEndpoint for BindingSession<'_, R> {
    fn discovery_target(
        &mut self,
        pane: &str,
    ) -> Result<Option<tmt_core::discovery::DiscoveryTarget>, Self::Error> {
        match self.primary {
            HostKind::Tmux => self.tmux.discovery_target(pane).map_err(HostError::from),
            HostKind::External(host) => self.external.discovery_target(host, pane),
        }
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

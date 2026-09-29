//! The terminal host that holds agent panes. The CLI and the other adapters
//! reach a host only through [`Host`], never by naming one, so a second host
//! (Herdr, #479) extends this port instead of every caller.
//!
//! tmux is the only host today. The port forwards to [`crate::tmux`] and
//! re-exports its error and value types under host names; they become
//! per-host variants when a second host exists.
//!
//! Every handle states why its host was chosen: the caller's own host
//! ([`Host::for_caller`]), the host that runs a known server
//! ([`Host::for_server`]), or the host whose syntax an explicit pane target
//! uses ([`Host::for_target`]). Endpoints travel as core values that carry
//! their host, never as loose socket or pane strings.

use crate::{
    process::{CommandRunner, UnixCommandRunner},
    tmux::Tmux,
};
use std::time::{Duration, Instant};
use tmt_core::{
    binding::{Binding, BindingTargetEvidence},
    endpoint::{EndpointProbe, EndpointSnapshot, ServerEvidence},
    host::HostKind,
    request::RequestEndpoint,
};

pub use crate::tmux::{
    ActionError, BindingSession, CallerEnvironment, ClientView, DeliveryError, DeliveryStage,
    FocusError, Invoker, OperationOptions, PaneCosmetics, PaneRefresh, TmuxError as HostError,
};

pub struct Host<R = UnixCommandRunner> {
    tmux: Tmux<R>,
}

impl Host {
    /// The host a caller-scoped command runs in. Without a host in the
    /// environment, tmux's default server serves, as it always has.
    pub fn for_caller(environment: &CallerEnvironment) -> Self {
        Self::for_caller_with(environment, UnixCommandRunner)
    }

    /// The host that runs a known server.
    pub fn for_server(server: &ServerEvidence) -> Self {
        Self::for_server_with(server, UnixCommandRunner)
    }

    /// The host whose syntax an explicit pane target uses; text that no host
    /// reads as a target is resolved by name elsewhere.
    pub fn for_target(_target: &str) -> Self {
        Self::of(HostKind::Tmux, UnixCommandRunner)
    }
}

impl<R: CommandRunner> Host<R> {
    pub fn for_caller_with(_environment: &CallerEnvironment, runner: R) -> Self {
        Self::of(HostKind::Tmux, runner)
    }

    pub fn for_server_with(server: &ServerEvidence, runner: R) -> Self {
        Self::of(server.host, runner)
    }

    fn of(host: HostKind, runner: R) -> Self {
        match host {
            HostKind::Tmux => Self {
                tmux: Tmux::new(runner),
            },
        }
    }

    /// The core binding and driver ports. The session observes panes on this
    /// host; a stored server of another host is never evidence here.
    pub fn session(&self) -> BindingSession<'_, R> {
        BindingSession::new(&self.tmux)
    }

    pub fn caller_pane(
        &self,
        environment: &CallerEnvironment,
    ) -> Result<Option<String>, HostError> {
        self.tmux.caller_pane(environment)
    }

    pub fn marked_pane(
        &self,
        environment: &CallerEnvironment,
        options: OperationOptions<'_>,
    ) -> Result<Option<BindingTargetEvidence>, HostError> {
        self.tmux.marked_pane(environment, options)
    }

    pub fn resolve_target(
        &self,
        target: &str,
        options: OperationOptions<'_>,
    ) -> Result<Option<String>, HostError> {
        self.tmux.resolve_target(target, options)
    }

    pub fn snapshot(&self, options: OperationOptions<'_>) -> Result<EndpointSnapshot, HostError> {
        self.tmux.snapshot(options)
    }

    pub fn observe_snapshot(
        &self,
        options: OperationOptions<'_>,
    ) -> Result<EndpointSnapshot, HostError> {
        self.tmux.observe_snapshot(options)
    }

    pub fn probe(
        &self,
        server: &ServerEvidence,
        options: OperationOptions<'_>,
    ) -> Result<EndpointProbe, HostError> {
        if server.host != HostKind::Tmux {
            return Ok(EndpointProbe::Unknown);
        }
        self.tmux
            .probe(&server.socket_path, server.server_pid, options)
    }

    pub fn capture(&self, endpoint: &RequestEndpoint, lines: u64) -> Result<String, HostError> {
        self.tmux
            .capture_on(&endpoint.server.socket_path, &endpoint.pane_id, lines)
    }

    pub fn send(
        &self,
        endpoint: &RequestEndpoint,
        message: &str,
        enter_delay: Duration,
    ) -> Result<(), DeliveryError> {
        self.tmux.send_on(
            &endpoint.server.socket_path,
            &endpoint.pane_id,
            message,
            enter_delay,
        )
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

    pub fn update_binding_cosmetics(
        &self,
        binding: &Binding,
        cosmetics: PaneCosmetics<'_>,
    ) -> Result<PaneRefresh, HostError> {
        self.tmux.update_binding_cosmetics(binding, cosmetics)
    }

    pub fn update_binding_cosmetics_until(
        &self,
        binding: &Binding,
        cosmetics: PaneCosmetics<'_>,
        deadline: Instant,
    ) -> Result<PaneRefresh, HostError> {
        self.tmux
            .update_binding_cosmetics_until(binding, cosmetics, deadline)
    }
}

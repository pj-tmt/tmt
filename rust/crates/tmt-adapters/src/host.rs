//! The terminal host that holds agent panes. The CLI and the other adapters
//! reach a host only through [`Host`], never by naming one, so a second host
//! (Herdr, #479) extends this port instead of every caller.
//!
//! tmux is the only host today. The port forwards to [`crate::tmux`] and
//! re-exports its error and value types under host names; they become
//! per-host variants when a second host exists.

use crate::{
    process::{CommandRunner, UnixCommandRunner},
    tmux::Tmux,
};
use std::time::{Duration, Instant};
use tmt_core::{
    binding::{Binding, BindingTargetEvidence},
    endpoint::{EndpointProbe, EndpointSnapshot},
};

pub use crate::tmux::{
    ActionError, BindingSession, CallerEnvironment, ClientView, DeliveryError, DeliveryStage,
    FocusError, Invoker, OperationOptions, PaneCosmetics, PaneRefresh, TmuxError as HostError,
};

pub struct Host<R = UnixCommandRunner> {
    tmux: Tmux<R>,
}

impl Default for Host {
    fn default() -> Self {
        Self {
            tmux: Tmux::default(),
        }
    }
}

impl<R: CommandRunner> Host<R> {
    pub fn new(runner: R) -> Self {
        Self {
            tmux: Tmux::new(runner),
        }
    }

    /// The core binding and driver ports over this host.
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
        socket: &str,
        recorded_pid: u64,
        options: OperationOptions<'_>,
    ) -> Result<EndpointProbe, HostError> {
        self.tmux.probe(socket, recorded_pid, options)
    }

    pub fn capture(&self, socket: &str, pane: &str, lines: u64) -> Result<String, HostError> {
        self.tmux.capture_on(socket, pane, lines)
    }

    pub fn send(
        &self,
        socket: &str,
        pane: &str,
        message: &str,
        enter_delay: Duration,
    ) -> Result<(), DeliveryError> {
        self.tmux.send_on(socket, pane, message, enter_delay)
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

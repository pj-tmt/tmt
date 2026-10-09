//! Application-port composition over the existing tmux runner and wire parser.

use super::{CommandRunner, OperationOptions, Tmux, TmuxError, TmuxFailure, socket_args};
use std::time::{Duration, Instant};
use tmt_core::{
    binding::{Binding, BindingEndpoint, session::RuntimeState},
    endpoint::{EndpointProbe, EndpointSnapshot, ServerEvidence},
    identity::Identity,
};

pub struct BindingSession<'a, R> {
    tmux: &'a Tmux<R>,
    deadline: Instant,
    enter_delay: Duration,
    invoker: Option<super::Invoker>,
}

impl<R: CommandRunner> BindingSession<'_, R> {
    fn observe_pane_start(&self, pane_pid: u64) -> Result<Option<String>, TmuxError> {
        crate::process::runtime::observe_start(&self.tmux.runner, pane_pid, self.deadline)
            .map_err(|error| TmuxError::command(TmuxFailure::Command, error))
    }
}

impl<'a, R: CommandRunner> BindingSession<'a, R> {
    pub fn new(tmux: &'a Tmux<R>) -> Self {
        Self {
            tmux,
            deadline: Instant::now(),
            enter_delay: Duration::ZERO,
            invoker: None,
        }
    }

    /// The user's own tmux client location, required only by `focus`.
    pub fn with_invoker(mut self, invoker: super::Invoker) -> Self {
        self.invoker = Some(invoker);
        self
    }

    /// Preserve the caller's configured transport delay; it is not routing policy.
    pub fn with_enter_delay(mut self, delay: Duration) -> Self {
        self.enter_delay = delay;
        self
    }

    fn options<'b>(&self, panes: Option<&'b [String]>) -> OperationOptions<'b> {
        OperationOptions {
            deadline: Some(self.deadline),
            pane_ids: panes,
        }
    }
}

mod actions;
mod border;

impl<R: CommandRunner> BindingEndpoint for BindingSession<'_, R> {
    type Error = TmuxError;

    fn current_host(&self) -> tmt_core::host::HostKind {
        tmt_core::host::HostKind::Tmux
    }

    fn begin_coordination(&mut self) {
        self.deadline = Instant::now() + Duration::from_secs(3);
    }
    fn budget_available(&self) -> bool {
        Instant::now() < self.deadline
    }

    fn current_snapshot(&mut self, panes: &[String]) -> Result<EndpointSnapshot, Self::Error> {
        self.tmux.snapshot(self.options(Some(panes)))
    }

    fn probe_binding(
        &mut self,
        server: &ServerEvidence,
        panes: &[String],
    ) -> Result<EndpointProbe, Self::Error> {
        // Another host's server is never evidence on this one.
        if !self.budget_available() || server.host != tmt_core::host::HostKind::Tmux {
            return Ok(EndpointProbe::Unknown);
        }
        let probe = self.tmux.probe(
            &server.socket_path,
            server.server_pid,
            self.options(Some(panes)),
        )?;
        // A spent coordination budget does not authorize loss/retirement.
        Ok(if self.budget_available() {
            probe
        } else {
            EndpointProbe::Unknown
        })
    }

    fn publish(&mut self, binding: &Binding, identity: &Identity) -> Result<(), Self::Error> {
        self.tmux.set_marker_on(
            Some(&binding.server.socket_path),
            &binding.pane_id,
            &binding.marker(identity),
            self.options(None),
        )
    }

    fn pane_incarnation(
        &mut self,
        _: &ServerEvidence,
        pane_pid: u64,
    ) -> Result<Option<String>, Self::Error> {
        self.observe_pane_start(pane_pid)
    }

    fn clear(&mut self, binding: &Binding) -> Result<bool, Self::Error> {
        self.tmux.clear_marker_on(
            Some(&binding.server.socket_path),
            &binding.pane_id,
            Some(&binding.id),
            self.options(None),
        )
    }
}

/// What a post-commit pane refresh shows for a binding.
#[derive(Debug, Clone, Copy)]
pub enum PaneCosmetics<'a> {
    /// The bound identity: a marker still carrying an earlier name is
    /// rewritten to the stored one, and the badge is shown when enabled.
    Bound {
        identity: &'a Identity,
        badge: bool,
        /// Only an explicit user binding/launch command requests guidance.
        border_hint: bool,
    },
    /// The binding ended: the badge is hidden.
    Ended,
}

/// Whether a pane refresh reached the binding's pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneRefresh {
    Updated,
    /// The binding is not on a live pane it still owns; nothing to show.
    Absent,
    /// tmux did not complete the update within its budget.
    Failed,
}

impl<R: CommandRunner> Tmux<R> {
    /// Cosmetic work is bounded and post-commit. Recheck the recorded endpoint
    /// and marker before touching the pane-local options; never change a title,
    /// window theme or another binding. Only failed child cleanup propagates.
    pub fn update_binding_cosmetics(
        &self,
        binding: &Binding,
        cosmetics: PaneCosmetics<'_>,
    ) -> Result<PaneRefresh, TmuxError> {
        self.update_binding_cosmetics_until(
            binding,
            cosmetics,
            Instant::now() + Duration::from_secs(1),
        )
    }

    /// Hook callers share their existing deadline; cosmetic work gets no extra
    /// budget and cannot turn a successful session write into a retry.
    pub fn update_binding_cosmetics_until(
        &self,
        binding: &Binding,
        cosmetics: PaneCosmetics<'_>,
        deadline: Instant,
    ) -> Result<PaneRefresh, TmuxError> {
        if Instant::now() >= deadline {
            return Ok(PaneRefresh::Failed);
        }
        let result: Result<PaneRefresh, TmuxError> = (|| {
            let panes = [binding.pane_id.clone()];
            let options = OperationOptions {
                deadline: Some(deadline),
                pane_ids: Some(&panes),
            };
            let EndpointProbe::Live(snapshot) = self.probe(
                &binding.server.socket_path,
                binding.server.server_pid,
                options,
            )?
            else {
                return Ok(PaneRefresh::Absent);
            };
            if snapshot.server != binding.server {
                return Ok(PaneRefresh::Absent);
            }
            let Some(pane) = snapshot
                .panes
                .iter()
                .find(|p| p.id == binding.pane_id && p.pane_pid == binding.pane_pid)
            else {
                return Ok(PaneRefresh::Absent);
            };
            let owned = pane
                .marker
                .as_ref()
                .filter(|marker| marker.binding_id == binding.id);
            if pane.marker.is_some() && owned.is_none() {
                return Ok(PaneRefresh::Absent);
            }
            let (badge, border_hint) = match cosmetics {
                PaneCosmetics::Bound {
                    identity,
                    badge,
                    border_hint,
                } => {
                    let Some(marker) = owned.filter(|_| identity.id == binding.identity_id) else {
                        return Ok(PaneRefresh::Absent);
                    };
                    if (marker.name != identity.name
                        || marker.canonical_name != identity.canonical_name)
                        && !self.refresh_marker_on(
                            Some(&binding.server.socket_path),
                            &binding.pane_id,
                            &binding.marker(identity),
                            options,
                        )?
                    {
                        return Ok(PaneRefresh::Absent);
                    }
                    (badge.then_some(identity.name.as_str()), border_hint)
                }
                PaneCosmetics::Ended => (None, false),
            };
            let mut args = socket_args(Some(&binding.server.socket_path));
            args.extend(["set-option".into(), "-p".into()]);
            if badge.is_none() {
                args.push("-u".into());
            }
            args.extend([
                "-t".into(),
                binding.pane_id.clone(),
                "@tmux-team.badge".into(),
            ]);
            if let Some(name) = badge {
                args.push(badge_label(name, binding.session.state));
            }
            self.execute(args, options, TmuxFailure::Command)?;
            self.update_badge_border(binding, badge.is_some(), border_hint, options)?;
            Ok(PaneRefresh::Updated)
        })();
        match result {
            Err(error) if error.cleanup_failed() => Err(error),
            Err(_) => Ok(PaneRefresh::Failed),
            Ok(refresh) => Ok(refresh),
        }
    }
}

fn badge_label(name: &str, state: RuntimeState) -> String {
    let mut characters = name.chars();
    let mut label: String = characters
        .by_ref()
        .take(48)
        .map(|c| {
            if c == '#' {
                '＃'
            } else if c.is_control() {
                ' '
            } else {
                c
            }
        })
        .collect();
    if characters.next().is_some() {
        label.push('…');
    }
    label.push_str(" (tmt)");
    match state {
        RuntimeState::Running => {
            format!("#[push-default]#[fg=green]●#[default]#[pop-default] {label}")
        }
        RuntimeState::Ended => format!("#[push-default]#[dim]○ {label}#[default]#[pop-default]"),
        RuntimeState::Unknown => label,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_badges_restore_styles_and_keep_names_literal() {
        assert_eq!(badge_label("alice", RuntimeState::Unknown), "alice (tmt)");
        assert_eq!(
            badge_label("alice", RuntimeState::Running),
            "#[push-default]#[fg=green]●#[default]#[pop-default] alice (tmt)"
        );
        assert_eq!(
            badge_label("alice", RuntimeState::Ended),
            "#[push-default]#[dim]○ alice (tmt)#[default]#[pop-default]"
        );
        assert_eq!(
            badge_label("#[fg=red]\n", RuntimeState::Running),
            "#[push-default]#[fg=green]●#[default]#[pop-default] ＃[fg=red]  (tmt)"
        );
        assert_eq!(
            badge_label(&"x".repeat(49), RuntimeState::Unknown),
            format!("{}… (tmt)", "x".repeat(48))
        );
    }
}

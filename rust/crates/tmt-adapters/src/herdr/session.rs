//! The binding and driver operations on Herdr panes. Delivery (H4), focus
//! (H6) and pane cosmetics (H5) are not implemented yet: they are
//! unsupported, never a silent substitute.

use super::{Herdr, HerdrError, marker};
use crate::{
    host::{ActionError, DeliveryError, HostError, driver::HostDriver},
    process::{CommandError, CommandRunner, runtime},
};
use std::time::{Duration, Instant};
use tmt_core::{
    binding::{Binding, session::RuntimeState},
    driver::Focused,
    endpoint::{EndpointProbe, EndpointSnapshot, ServerEvidence},
    host::HostKind,
    identity::Identity,
};

pub struct Session<'a, R> {
    herdr: &'a Herdr<R>,
    deadline: Instant,
}

impl<'a, R: CommandRunner> Session<'a, R> {
    pub fn new(herdr: &'a Herdr<R>) -> Self {
        Self {
            herdr,
            deadline: Instant::now(),
        }
    }

    pub fn begin_coordination(&mut self) {
        self.deadline = Instant::now() + Duration::from_secs(3);
    }

    /// A caller's own deadline, when it is sooner.
    pub fn limit(&mut self, deadline: Option<Instant>) {
        if let Some(deadline) = deadline {
            self.deadline = self.deadline.min(deadline);
        }
    }

    pub fn budget_available(&self) -> bool {
        Instant::now() < self.deadline
    }

    /// Panes on this handle's resolved server. The transaction only ever sees
    /// that resolution: a server that changed since is an error, never a
    /// second registration.
    pub fn snapshot(&self, panes: Option<&[String]>) -> Result<EndpointSnapshot, HerdrError> {
        let server = self
            .herdr
            .resolved()
            .ok_or_else(|| HerdrError::evidence("The Herdr server was not resolved"))?;
        let Some(observed) = self
            .herdr
            .observe(&server.socket_path, panes, self.deadline)?
        else {
            return Ok(EndpointSnapshot {
                server: server.clone(),
                panes: Vec::new(),
            });
        };
        if !observed.incarnation.matches(server) {
            return Err(HerdrError::evidence(
                "The Herdr server restarted during this command",
            ));
        }
        Ok(EndpointSnapshot {
            server: server.clone(),
            panes: observed.panes,
        })
    }

    /// A stored server, observed at its socket. The same process and start is
    /// `Live` with the stored evidence; another incarnation or no server is
    /// loss only when the recorded process is conclusively gone.
    pub fn probe(
        &self,
        server: &ServerEvidence,
        panes: &[String],
    ) -> Result<EndpointProbe, HerdrError> {
        if server.host != HostKind::Herdr || !self.budget_available() {
            return Ok(EndpointProbe::Unknown);
        }
        let death = || {
            if runtime::recorded_process_gone(server.server_pid) {
                EndpointProbe::Dead
            } else {
                EndpointProbe::Unknown
            }
        };
        let not_running = |error: &HerdrError| error.code() == Some("server_not_running");
        let observed = self
            .herdr
            .status(Some(&server.socket_path), self.deadline)
            .and_then(|status| {
                if status.running {
                    self.herdr
                        .observe(&server.socket_path, Some(panes), self.deadline)
                } else {
                    Err(HerdrError::api(
                        "server_not_running".into(),
                        "no Herdr server is running",
                    ))
                }
            });
        let probe = match observed {
            Ok(Some(observed)) if observed.incarnation.matches(server) => {
                EndpointProbe::Live(EndpointSnapshot {
                    server: server.clone(),
                    panes: observed.panes,
                })
            }
            Ok(_) => death(),
            Err(error) if error.cleanup_failed() => return Err(error),
            Err(error) if not_running(&error) => death(),
            Err(_) => EndpointProbe::Unknown,
        };
        // A spent coordination budget does not authorize loss or retirement.
        Ok(if self.budget_available() {
            probe
        } else {
            EndpointProbe::Unknown
        })
    }

    fn pane_id(
        &self,
        binding: &Binding,
    ) -> Result<Option<super::evidence::ListedPane>, HerdrError> {
        Ok(self
            .herdr
            .list(&binding.server.socket_path, self.deadline)?
            .into_iter()
            .find(|pane| pane.terminal_id == binding.pane_id))
    }

    pub fn publish(&self, binding: &Binding, identity: &Identity) -> Result<(), HerdrError> {
        let expected = binding.marker(identity);
        let tokens = marker::encode(&expected).ok_or_else(|| {
            HerdrError::evidence("The identity name is too long to mark a Herdr pane")
        })?;
        let pane = self
            .pane_id(binding)?
            .ok_or_else(|| HerdrError::evidence("The Herdr pane closed before binding"))?;
        let mut args = vec![
            "pane",
            "report-metadata",
            pane.pane_id.as_str(),
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
            .act(&binding.server.socket_path, &args, self.deadline)?;
        // Herdr drops a report it considers stale without an error; read back.
        let written = self
            .pane_id(binding)?
            .and_then(|pane| marker::decode(pane.tokens.as_ref()));
        if written.as_ref() != Some(&expected) {
            return Err(HerdrError::evidence("Herdr did not keep the TMT marker"));
        }
        Ok(())
    }

    /// Clears this binding's marker; a marker another binding owns stays.
    pub fn clear(&self, binding: &Binding) -> Result<bool, HerdrError> {
        let Some(pane) = self.pane_id(binding)? else {
            return Ok(false);
        };
        if marker::binding_id(pane.tokens.as_ref()) != Some(binding.id.as_str()) {
            return Ok(false);
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
            pane.pane_id.as_str(),
            "--source",
            marker::SOURCE,
        ];
        for key in &present {
            args.extend(["--clear-token", key.as_str()]);
        }
        self.herdr
            .act(&binding.server.socket_path, &args, self.deadline)?;
        Ok(true)
    }

    /// After a committed rename, rewrite this binding's own marker so the
    /// pane shows the stored name. Herdr has no TMT badge yet (#479 H5);
    /// a pane another binding owns is left alone.
    pub fn refresh_name(
        &self,
        binding: &Binding,
        identity: &Identity,
    ) -> Result<crate::host::PaneRefresh, HerdrError> {
        use crate::host::PaneRefresh;
        let Some(pane) = self.pane_id(binding)? else {
            return Ok(PaneRefresh::Absent);
        };
        let Some(current) = marker::decode(pane.tokens.as_ref()) else {
            return Ok(PaneRefresh::Absent);
        };
        if current.binding_id != binding.id || identity.id != binding.identity_id {
            return Ok(PaneRefresh::Absent);
        }
        if current.name != identity.name || current.canonical_name != identity.canonical_name {
            self.publish(binding, identity)?;
        }
        Ok(PaneRefresh::Updated)
    }

    pub fn observed_runtime(
        &self,
        binding: &Binding,
    ) -> Result<RuntimeState, crate::process::CommandError> {
        runtime::binding_runtime(self.herdr.runner(), binding, self.deadline)
    }
}

/// Herdr behind the host-driver trait. Input (#479 H4) and focus (H6) are
/// not implemented yet: a send is `Unsupported`, so core uses the inbox, and
/// focus is refused before any evidence is read.
impl<R: CommandRunner> HostDriver for Session<'_, R> {
    fn begin_coordination(&mut self) {
        Session::begin_coordination(self);
    }

    fn budget_available(&self) -> bool {
        Session::budget_available(self)
    }

    fn snapshot(&mut self, panes: &[String]) -> Result<EndpointSnapshot, HostError> {
        Ok(Session::snapshot(self, Some(panes))?)
    }

    fn probe(
        &mut self,
        server: &ServerEvidence,
        panes: &[String],
    ) -> Result<EndpointProbe, HostError> {
        Ok(Session::probe(self, server, panes)?)
    }

    fn publish(&mut self, binding: &Binding, identity: &Identity) -> Result<(), HostError> {
        Ok(Session::publish(self, binding, identity)?)
    }

    fn clear(&mut self, binding: &Binding) -> Result<bool, HostError> {
        Ok(Session::clear(self, binding)?)
    }

    fn observed_runtime(&self, binding: &Binding) -> Result<RuntimeState, CommandError> {
        Session::observed_runtime(self, binding)
    }

    fn has_input(&self) -> bool {
        false
    }

    fn input(&mut self, _: &Binding, _: &str) -> Result<(), DeliveryError> {
        Err(DeliveryError::unsupported())
    }

    fn focus_preflight(&self, _: Option<&Binding>) -> Result<(), ActionError> {
        Err(ActionError::HostUnsupported)
    }

    fn focus(&mut self, _: &Binding) -> Result<Focused, ActionError> {
        Err(ActionError::HostUnsupported)
    }
}

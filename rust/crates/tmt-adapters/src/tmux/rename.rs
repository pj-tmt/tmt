//! One-release upgrade step. Ordinary host observations never read retired keys.
//! Storage ownership is fenced by the caller; tmux compares the captured values
//! inside its command queue before changing only this binding's options.

use super::{
    CommandRunner, OperationOptions, Tmux, TmuxError, TmuxFailure, evidence, metadata, socket_args,
};
use crate::process::runtime::observe_starts;
use std::time::Instant;
use tmt_core::{binding::BindingEntry, endpoint::ProcessIncarnation};

const OLD_AGENT: &str = "@tmux-team.agent";
const OLD_SERVER: &str = "@tmux-team.server-id";
const OLD_BADGE: &str = "@tmux-team.badge";
const OLD_BORDER: &str = "@tmux-team.border";
const OLD_PREFIX: &str = "#{?@tmux-team.badge, [#{@tmux-team.badge}],}";
const NEW_PREFIX: &str = "#{?@tmt.badge, [#{@tmt.badge}],}";

/// Opaque native proof, usable only for the exact captured record.
pub struct OptionRename {
    entry: BindingEntry,
    raw: String,
    server_process: ProcessIncarnation,
    badge: Option<String>,
    border: Option<String>,
}

impl<R: CommandRunner> Tmux<R> {
    /// `None` means this pane has no retired marker, not authority to adopt it.
    pub fn prepare_option_rename(
        &self,
        entry: &BindingEntry,
        deadline: Instant,
    ) -> Result<Option<OptionRename>, TmuxError> {
        let binding = entry
            .binding
            .as_ref()
            .ok_or_else(|| TmuxError::evidence("Missing binding for option rename"))?;
        let options = OperationOptions {
            deadline: Some(deadline),
            pane_ids: None,
        };
        let raw = self
            .rename_option(entry, OLD_AGENT, options)?
            .unwrap_or_default();
        if raw.is_empty() {
            return Ok(None);
        }
        if raw.len() > 8192
            || metadata::marker(&metadata::decode(&raw)).as_ref()
                != Some(&binding.marker(&entry.identity))
        {
            return Err(TmuxError::evidence(
                "Retired marker does not match the stored binding",
            ));
        }
        let mut args = socket_args(Some(&binding.server.socket_path));
        args.extend([
            "display-message".into(),
            "-p".into(),
            "-t".into(),
            binding.pane_id.clone(),
            evidence::endpoint_format().replace("@tmt.", "@tmux-team."),
        ]);
        let observed =
            evidence::parse_snapshot(&self.execute(args, options, TmuxFailure::Command)?, None)?;
        if observed.server != binding.server
            || observed.panes.len() != 1
            || observed.panes[0].id != binding.pane_id
            || observed.panes[0].pane_pid != binding.pane_pid
            || observed.panes[0].marker.as_ref() != Some(&binding.marker(&entry.identity))
        {
            return Err(TmuxError::evidence(
                "Retired option endpoint evidence changed",
            ));
        }
        let starts = observe_starts(
            &self.runner,
            &[binding.server.server_pid, binding.pane_pid],
            deadline,
        )
        .map_err(|error| TmuxError::command(TmuxFailure::Evidence, error))?;
        let server_process = starts
            .get(&binding.server.server_pid)
            .cloned()
            .ok_or_else(|| {
                TmuxError::evidence("Server incarnation unavailable during option rename")
            })?;
        if binding.pane_incarnation.as_deref().is_none_or(|token| {
            starts
                .get(&binding.pane_pid)
                .is_none_or(|process| process.start_identity() != token)
        }) {
            return Err(TmuxError::evidence(
                "Pane incarnation unavailable or changed during option rename",
            ));
        }
        let badge = self.rename_option(entry, OLD_BADGE, options)?;
        let border = self.rename_option(entry, OLD_BORDER, options)?;
        Ok(Some(OptionRename {
            entry: entry.clone(),
            raw,
            server_process,
            badge,
            border,
        }))
    }

    /// The caller must compare the full stored entry under its mutation lock.
    /// False/unknown never authorizes replacing metadata or retiring an identity.
    pub fn commit_option_rename(
        &self,
        proof: &OptionRename,
        deadline: Instant,
    ) -> Result<bool, TmuxError> {
        let binding = proof.entry.binding.as_ref().expect("prepared binding");
        let options = OperationOptions {
            deadline: Some(deadline),
            pane_ids: None,
        };
        let starts = observe_starts(
            &self.runner,
            &[binding.server.server_pid, binding.pane_pid],
            deadline,
        )
        .map_err(|error| TmuxError::command(TmuxFailure::Evidence, error))?;
        if starts.get(&binding.server.server_pid) != Some(&proof.server_process)
            || binding.pane_incarnation.as_deref().is_none_or(|token| {
                starts
                    .get(&binding.pane_pid)
                    .is_none_or(|process| process.start_identity() != token)
            })
        {
            return Ok(false);
        }
        let mut conditions = vec![
            equal("pane_pid", &binding.pane_pid.to_string()),
            equal("pid", &binding.server.server_pid.to_string()),
            equal("start_time", &binding.server.server_start_time),
            equal(OLD_SERVER, &binding.server.server_id),
            equal(OLD_AGENT, &proof.raw),
            empty_or_equal("@tmt.server-id", &binding.server.server_id),
            empty_or_equal("@tmt.agent", &proof.raw),
        ];
        let pane = &binding.pane_id;
        let mut commands = vec![
            format!(
                "set-option -s @tmt.server-id {}",
                word(&binding.server.server_id)
            ),
            format!("set-option -p -t {pane} @tmt.agent {}", word(&proof.raw)),
        ];
        if let Some(badge) = &proof.badge {
            conditions.extend([equal(OLD_BADGE, badge), empty_or_equal("@tmt.badge", badge)]);
            commands.extend([
                format!("set-option -p -t {pane} @tmt.badge {}", word(badge)),
                format!("set-option -pu -t {pane} {OLD_BADGE}"),
            ]);
        }
        if let Some(border) = &proof.border {
            // Inherited/global formats are never read-modify-written. A user's
            // local edit keeps both its value and the old ownership evidence.
            let local = self.rename_option(&proof.entry, "pane-border-format", options)?;
            if !border.starts_with(OLD_PREFIX) || local.as_ref() != Some(border) {
                // Refuse the whole pane, retaining the label used by the user's
                // format as well as its ownership evidence. The caller hints.
                return Ok(false);
            }
            let next = format!("{NEW_PREFIX}{}", &border[OLD_PREFIX.len()..]);
            conditions.extend([
                equal(OLD_BORDER, border),
                equal("pane-border-format", border),
                empty_or_equal("@tmt.border", &next),
            ]);
            commands.extend([
                format!("set-option -p -t {pane} pane-border-format {}", word(&next)),
                format!("set-option -p -t {pane} @tmt.border {}", word(&next)),
                format!("set-option -pu -t {pane} {OLD_BORDER}"),
            ]);
        }
        commands.push(format!("set-option -pu -t {pane} {OLD_AGENT}"));
        let condition = conditions
            .into_iter()
            .reduce(|left, right| format!("#{{&&:{left},{right}}}"))
            .expect("nonempty fence");
        let mut args = socket_args(Some(&binding.server.socket_path));
        args.extend([
            "if-shell".into(),
            "-F".into(),
            "-t".into(),
            pane.clone(),
            condition,
            commands.join(" ; "),
        ]);
        self.execute(args, options, TmuxFailure::MetadataWrite)?;
        // A false if-shell still exits zero. Verify the actual publication.
        Ok(self
            .rename_option(&proof.entry, "@tmt.agent", options)?
            .as_deref()
            == Some(&proof.raw)
            && self
                .rename_option(&proof.entry, OLD_AGENT, options)?
                .is_none())
    }

    fn rename_option(
        &self,
        entry: &BindingEntry,
        name: &str,
        options: OperationOptions<'_>,
    ) -> Result<Option<String>, TmuxError> {
        let binding = entry.binding.as_ref().expect("bound entry");
        let mut args = socket_args(Some(&binding.server.socket_path));
        args.extend([
            "show-options".into(),
            "-p".into(),
            "-qv".into(),
            "-t".into(),
            binding.pane_id.clone(),
            name.into(),
        ]);
        let output = self.execute(args, options, TmuxFailure::MetadataRead)?;
        if output.is_empty() {
            return Ok(None);
        }
        output
            .strip_suffix('\n')
            .filter(|value| value.len() <= 8192)
            .map(|value| Some(value.into()))
            .ok_or_else(|| TmuxError::evidence("Malformed option rename value"))
    }
}

fn literal(value: &str) -> String {
    value
        .replace('#', "##")
        .replace(',', "#,")
        .replace('}', "#}")
}
fn equal(key: &str, value: &str) -> String {
    format!("#{{==:#{{{key}}},{}}}", literal(value))
}
fn empty_or_equal(key: &str, value: &str) -> String {
    format!("#{{||:{},{}}}", equal(key, ""), equal(key, value))
}
// tmux's command parser accepts octal bytes in double quotes. Encoding every
// byte prevents expansion, statement separators and syntax from stored text.
fn word(value: &str) -> String {
    use std::fmt::Write;
    let mut result = String::from("\"");
    for byte in value.bytes() {
        write!(result, "\\{byte:03o}").expect("write string");
    }
    result.push('"');
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        process::{CommandError, CommandOutput, CommandRequest, runtime::ProcessObservation},
        scripted_runner::ScriptedRunner,
    };
    use std::time::Duration;
    use tmt_core::{
        binding::Binding,
        endpoint::ServerEvidence,
        host::HostKind,
        identity::{Identity, Lifetime},
    };

    struct Runner {
        commands: ScriptedRunner,
        stale_pid: Option<u64>,
    }
    impl CommandRunner for Runner {
        fn process_observation(&self, pid: u64, _: Instant) -> Option<ProcessObservation> {
            Some(ProcessObservation::Live(
                ProcessIncarnation::new(
                    pid,
                    if self.stale_pid == Some(pid) {
                        "changed"
                    } else {
                        "captured"
                    },
                )
                .unwrap(),
            ))
        }
        fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
            self.commands.execute(request)
        }
    }
    fn proof(border: Option<String>) -> OptionRename {
        let identity = Identity {
            id: "00000000-0000-4000-8000-000000000001".into(),
            name: "Alice".into(),
            canonical_name: "alice".into(),
            lifetime: Lifetime::Saved,
            created_at: "2026-10-09T00:00:00Z".into(),
            updated_at: "2026-10-09T00:00:00Z".into(),
        };
        let binding = Binding {
            id: "00000000-0000-4000-8000-000000000002".into(),
            identity_id: identity.id.clone(),
            server: ServerEvidence {
                host: HostKind::Tmux,
                server_id: "00000000-0000-4000-8000-000000000003".into(),
                socket_path: "/private/selected.sock".into(),
                server_pid: 321,
                server_start_time: "1700000000".into(),
            },
            pane_id: "%9".into(),
            pane_pid: 900,
            pane_incarnation: Some("captured".into()),
            session: Default::default(),
        };
        let mut document = serde_json::json!({"version": 1});
        metadata::replace(&mut document, &binding.marker(&identity));
        OptionRename {
            entry: BindingEntry {
                identity,
                binding: Some(binding),
            },
            raw: document.to_string(),
            server_process: ProcessIncarnation::new(321, "captured").unwrap(),
            badge: Some("label".into()),
            border,
        }
    }
    #[test]
    fn changed_server_or_pane_incarnation_has_no_tmux_effect() {
        for pid in [321, 900] {
            let tmux = Tmux::new(Runner {
                commands: ScriptedRunner::default(),
                stale_pid: Some(pid),
            });
            assert!(
                !tmux
                    .commit_option_rename(&proof(None), Instant::now() + Duration::from_secs(1))
                    .unwrap()
            );
            assert!(tmux.runner.commands.calls.borrow().is_empty());
        }
    }
    #[test]
    fn user_edited_or_unknown_border_keeps_the_entire_old_pane() {
        for border in [
            format!("{OLD_PREFIX}theme"),
            "unrecognized owner value".into(),
        ] {
            let commands = ScriptedRunner::new([Ok("user edited format\n")]);
            let tmux = Tmux::new(Runner {
                commands,
                stale_pid: None,
            });
            assert!(
                !tmux
                    .commit_option_rename(
                        &proof(Some(border)),
                        Instant::now() + Duration::from_secs(1)
                    )
                    .unwrap()
            );
            let calls = tmux.runner.commands.calls.borrow();
            assert_eq!(calls.len(), 1);
            assert!(calls[0].args.iter().any(|arg| arg == "show-options"));
            assert!(!calls[0].args.iter().any(|arg| arg == "if-shell"));
        }
    }
    #[test]
    fn command_queue_fence_compares_exact_old_values_and_readback_does_not_claim_false_cas() {
        let proof = proof(None);
        let tmux = Tmux::new(Runner {
            commands: ScriptedRunner::new([Ok(""), Ok("different new marker\n")]),
            stale_pid: None,
        });
        let deadline = Instant::now() + Duration::from_secs(1);
        assert!(!tmux.commit_option_rename(&proof, deadline).unwrap());
        let calls = tmux.runner.commands.calls.borrow();
        assert_eq!(calls.len(), 2);
        assert_eq!(
            &calls[0].args[..6],
            ["-S", "/private/selected.sock", "if-shell", "-F", "-t", "%9"]
        );
        let fence = &calls[0].args[6];
        for key in [
            OLD_AGENT,
            OLD_SERVER,
            OLD_BADGE,
            "pane_pid",
            "pid",
            "start_time",
            "@tmt.agent",
            "@tmt.server-id",
        ] {
            assert!(fence.contains(key), "missing {key}");
        }
        assert!(fence.contains(&literal(&proof.raw)));
        assert!(calls.iter().all(|call| call.deadline == deadline));
    }
}

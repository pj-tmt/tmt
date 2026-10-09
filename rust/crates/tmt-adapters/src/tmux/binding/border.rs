//! Pane-local presentation ownership; no shared tmux option is changed.

use super::super::{CommandRunner, OperationOptions, Tmux, TmuxError, TmuxFailure, socket_args};
use std::{io::Write, sync::Once};
use tmt_core::binding::Binding;

const FORMAT: &str = "pane-border-format";
const OWNER: &str = "@tmt.border";
const BADGE: &str = "#{?@tmt.badge, [#{@tmt.badge}],}";
static BORDER_HINT: Once = Once::new();

impl<R: CommandRunner> Tmux<R> {
    pub(super) fn update_badge_border(
        &self,
        binding: &Binding,
        enabled: bool,
        hint_requested: bool,
        options: OperationOptions<'_>,
    ) -> Result<(), TmuxError> {
        if !enabled {
            // Read the local owner only. An inherited option is not our claim.
            let Some(owner) = self.border_option(binding, "-p", OWNER, options)? else {
                return Ok(());
            };
            if !owner.starts_with(BADGE) {
                return Ok(());
            }
            // Compare on the server, not read-then-unset: a user edit between
            // the observation and cleanup must keep its pane-local override.
            let mut args = socket_args(Some(&binding.server.socket_path));
            args.extend([
                "if-shell".into(),
                "-F".into(),
                "-t".into(),
                binding.pane_id.clone(),
                "#{&&:#{@tmt.border},#{==:#{pane-border-format},#{@tmt.border}}}".into(),
                format!(
                    "set-option -p -u -t {} {FORMAT} ; set-option -p -u -t {} {OWNER}",
                    binding.pane_id, binding.pane_id
                ),
            ]);
            self.execute(args, options, TmuxFailure::Command)?;
            return Ok(());
        }

        // Even an empty local user format is an override. Existing owned
        // formats are dynamic, so refresh/rename need no further format write.
        if self
            .border_option(binding, "-p", FORMAT, options)?
            .is_none()
        {
            let inherited = self
                .border_option(binding, "-pA", FORMAT, options)?
                .ok_or_else(|| TmuxError::evidence("Missing effective pane border format"))?;
            if !inherited.contains("@tmt.badge") {
                let format = format!("{BADGE}{inherited}");
                // -o refuses a local user override created since the read.
                // Publish ownership only after a successful format write. If
                // ownership publication fails, cleanup conservatively refuses
                // the unowned format rather than risking a user's override.
                self.set_border_option(binding, FORMAT, &format, true, options)?;
                self.set_border_option(binding, OWNER, &format, false, options)?;
            }
        }
        if hint_requested
            && self
                .border_option(binding, "-wA", "pane-border-status", options)?
                .as_deref()
                == Some("off")
        {
            BORDER_HINT.call_once(|| {
                let _ = writeln!(std::io::stderr().lock(), "{}", border_hint(binding));
            });
        }
        Ok(())
    }

    fn border_option(
        &self,
        binding: &Binding,
        scope: &str,
        name: &str,
        options: OperationOptions<'_>,
    ) -> Result<Option<String>, TmuxError> {
        let mut args = socket_args(Some(&binding.server.socket_path));
        args.extend([
            "show-options".into(),
            scope.into(),
            "-qv".into(),
            "-t".into(),
            binding.pane_id.clone(),
            name.into(),
        ]);
        let text = self.execute(args, options, TmuxFailure::Command)?;
        // No local option prints no bytes; an explicitly empty value prints
        // one newline. Preserve the format verbatim, including its whitespace.
        if text.is_empty() {
            Ok(None)
        } else {
            text.strip_suffix('\n')
                .map(|value| Some(value.to_owned()))
                .ok_or_else(|| TmuxError::evidence("Malformed pane option output"))
        }
    }

    fn set_border_option(
        &self,
        binding: &Binding,
        name: &str,
        value: &str,
        only_if_unset: bool,
        options: OperationOptions<'_>,
    ) -> Result<(), TmuxError> {
        let mut args = socket_args(Some(&binding.server.socket_path));
        args.extend(["set-option".into(), "-p".into()]);
        if only_if_unset {
            args.push("-o".into());
        }
        args.extend([
            "-t".into(),
            binding.pane_id.clone(),
            name.into(),
            value.into(),
        ]);
        self.execute(args, options, TmuxFailure::Command)?;
        Ok(())
    }
}

fn border_hint(binding: &Binding) -> String {
    let socket = binding.server.socket_path.replace('\'', "'\\''");
    format!(
        "hint: pane borders are off; enable them with tmux -S '{socket}' set-option -w -t {} pane-border-status top",
        binding.pane_id
    )
}

#[cfg(test)]
mod tests;

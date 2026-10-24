//! Interactive terminal composition. The host admits a connection, adapters
//! relay bytes, and this invocation owns restoration before publishing a result.

use crate::{
    invocation::OutputMode,
    output::{Failure, after_cleanup, shell_word},
    target,
};
use crossterm::{
    cursor::Show,
    execute,
    style::{Attribute, ResetColor, SetAttribute},
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use std::io::{self, Write};
use tmt_adapters::{
    config::ConfigPaths,
    host::Host,
    pty::{
        self, AttachEnd, AttachSignals, Connection,
        protocol::{Eviction, Exit, MAX_DIMENSION, Size},
    },
    storage::Storage,
};
use tmt_core::binding::{BindingEntry, PaneIdentity};

pub(crate) struct Prepared {
    connection: Connection,
    name: String,
}

pub(crate) fn ensure_interactive(mode: OutputMode) -> Result<(), Failure> {
    if mode.json {
        return Err(Failure::new(
            "JSON_UNSUPPORTED",
            "Attachment is interactive and does not support --json. No pane input occurred.",
            1,
        ));
    }
    if tmt_cli_style::Interaction::detect(false).view() != tmt_cli_style::Mode::Interactive {
        return Err(Failure::new(
            "INTERACTION_REQUIRED",
            "Attachment needs interactive terminal input and output. Run tmt attach <name> in a terminal. No pane input occurred.",
            1,
        ));
    }
    Ok(())
}

pub(crate) fn prepare_observed(observed: PaneIdentity) -> Result<Prepared, Failure> {
    let host = Host::for_server(&observed.server);
    if !host.has_terminal_stream() {
        return Err(Failure::new(
            "HOST_UNSUPPORTED",
            "This host does not provide a built-in terminal stream. Use tmt focus <name> in its host. No pane input occurred.",
            1,
        ));
    }
    let identity = observed.identity.ok_or_else(|| {
        Failure::new(
            "NAME_NOT_FOUND",
            "Attach to a bound identity name or UUID. No pane input occurred.",
            3,
        )
    })?;
    let name = identity.name.clone();
    let connection = host
        .terminal_connection(&BindingEntry {
            identity,
            binding: observed.binding,
        })
        .map_err(|error| {
            Failure::new(
                "RECONCILIATION_FAILED",
                "Could not verify the member for attachment. No pane input occurred.",
                1,
            )
            .caused_by(error)
        })?;
    Ok(Prepared { connection, name })
}

fn prepare(target: &str) -> Result<Prepared, Failure> {
    let paths = ConfigPaths::discover().map_err(Failure::from)?;
    let mut storage = Storage::open(&paths.database).map_err(|error| {
        Failure::storage_access(
            error,
            &paths.global_dir,
            "No pane input occurred; retrying the identical command is safe.",
            "IDENTITY_ERROR",
            "Could not open identity storage.",
        )
    })?;
    let result = target::resolve_focus(&mut storage, target).and_then(prepare_observed);
    after_cleanup(result, || storage.close())
}

/// Clamping happens in the client; oversized Resize frames remain malformed.
fn window(cols: u16, rows: u16) -> Size {
    Size {
        cols: cols.clamp(1, MAX_DIMENSION),
        rows: rows.clamp(1, MAX_DIMENSION),
    }
}

struct Terminal<W: Write> {
    output: W,
    raw: bool,
    alternate: bool,
}

impl<W: Write> Terminal<W> {
    fn enter(output: W) -> io::Result<Self> {
        let mut terminal = Self {
            output,
            raw: false,
            alternate: false,
        };
        enable_raw_mode()?;
        terminal.raw = true;
        // Even a partial write must attempt to leave the alternate screen.
        terminal.alternate = true;
        if let Err(error) = execute!(terminal.output, EnterAlternateScreen) {
            return match terminal.restore() {
                Ok(()) => Err(error),
                Err(restore) => Err(io::Error::other(format!(
                    "terminal acquisition failed ({error}); restoration also failed ({restore})"
                ))),
            };
        }
        Ok(terminal)
    }

    fn restore(&mut self) -> io::Result<()> {
        let screen = if self.alternate {
            self.alternate = false;
            execute!(
                self.output,
                ResetColor,
                SetAttribute(Attribute::Reset),
                Show,
                LeaveAlternateScreen
            )
        } else {
            Ok(())
        };
        // A broken output stream must never prevent restoring the terminal's
        // exact saved input modes.
        let raw = if self.raw {
            self.raw = false;
            disable_raw_mode()
        } else {
            Ok(())
        };
        screen.and(raw)
    }
}

impl<W: Write> Drop for Terminal<W> {
    fn drop(&mut self) {
        // Explicit completion reports errors; unwinding still restores what it can.
        let _ = self.restore();
    }
}

pub fn execute(target: String, detach_others: bool, mode: OutputMode) -> io::Result<u8> {
    let prepared = match ensure_interactive(mode).and_then(|()| prepare(&target)) {
        Ok(prepared) => prepared,
        Err(error) => return error.publish(mode),
    };
    execute_prepared(prepared, detach_others, mode)
}

pub(crate) fn execute_prepared(
    mut prepared: Prepared,
    detach_others: bool,
    mode: OutputMode,
) -> io::Result<u8> {
    if let Err(error) = ensure_interactive(mode) {
        return error.publish(mode);
    }
    let mut signals = match AttachSignals::install() {
        Ok(signals) => signals,
        Err(error) => {
            return Failure::new(
                "ATTACH_FAILED",
                "Could not install attachment signal handling. No pane input occurred.",
                1,
            )
            .caused_by(error)
            .publish(mode);
        }
    };
    let initial = match crossterm::terminal::size() {
        Ok(size) => size,
        Err(error) => {
            return Failure::new(
                "ATTACH_FAILED",
                "Could not read the terminal size. No pane input occurred.",
                1,
            )
            .caused_by(error)
            .publish(mode);
        }
    };
    let effective = window(initial.0, initial.1);
    if effective.cols != initial.0 || effective.rows != initial.1 {
        let mut stderr = tmt_cli_style::stream::stderr();
        let terminal = stderr.terminal();
        tmt_cli_style::message::warning(
            &mut stderr,
            terminal,
            &format!(
                "Attachment uses {} columns and {} rows (terminal size clamped)",
                effective.cols, effective.rows
            ),
            None,
        )?;
    }
    let mut terminal = match Terminal::enter(tmt_cli_style::stream::stdout(false)) {
        Ok(terminal) => terminal,
        Err(error) => {
            return Failure::new(
                "ATTACH_FAILED",
                "Could not acquire the terminal. No pane input occurred.",
                1,
            )
            .caused_by(error)
            .publish(mode);
        }
    };
    let result = pty::attach(
        &mut prepared.connection,
        &io::stdin(),
        &mut terminal.output,
        &mut signals,
        detach_others,
        || crossterm::terminal::size().map(|(cols, rows)| window(cols, rows)),
    );
    if let Err(error) = terminal.restore() {
        return Failure::new("TERMINAL_RESTORE_FAILED", "Attachment ended, but terminal restoration failed. The member was not stopped; restore this terminal with: stty sane", 1).caused_by(error).publish(mode);
    }
    // Drop the subscriber before any human output or storage work can keep the
    // program waiting on an unread output queue.
    drop(prepared.connection);
    let end = match result {
        Ok(end) => end,
        Err(error) => return Failure::new("ATTACH_FAILED", format!("Attachment lost its terminal connection. This does not prove the agent ended; inspect it before retrying: tmt attach -- {}", shell_word(&prepared.name)), 1).caused_by(error).publish(mode),
    };
    let out = &mut terminal.output;
    let style = out.terminal();
    let (message, code) = match end {
        AttachEnd::Detached => (format!("Detached from {}; the member keeps running", prepared.name), 0),
        AttachEnd::Exited(exit) => {
            let code = match exit { Exit::Code(code) => code.clamp(0, 255) as u8, Exit::Signal(signal) => (128 + signal).clamp(0, 255) as u8 };
            (format!("{} ended ({exit:?}); resume with: tmt resume -- {}", prepared.name, shell_word(&prepared.name)), code)
        }
        AttachEnd::Evicted(Eviction::TakenOver) => ("Another viewer took sole use; this attachment ended and the member keeps running".into(), 0),
        AttachEnd::Evicted(Eviction::TooSlow) => ("This attachment stopped reading output and was disconnected; the member was not stopped".into(), 1),
        AttachEnd::Signal(signal) => (format!("Attachment interrupted by signal {signal}; the member was not stopped"), (128 + signal).clamp(0, 255) as u8),
    };
    if code == 0 {
        tmt_cli_style::message::success(out, style, &message)?;
    } else {
        tmt_cli_style::message::warning(out, style, &message, None)?;
    }
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn terminal_dimensions_are_clamped_before_encoding() {
        assert_eq!(window(1000, 1000), Size::new(1000, 1000).unwrap());
        assert_eq!(window(1001, u16::MAX), Size::new(1000, 1000).unwrap());
        assert_eq!(window(0, 0), Size::new(1, 1).unwrap());
    }
}

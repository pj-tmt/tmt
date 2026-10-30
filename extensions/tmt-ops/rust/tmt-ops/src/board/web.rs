//! `g  Web`: the top bar opens the web UI by running Remote's public
//! `tmt remote open --open`, which opens the user's own browser on the running local
//! Remote. `--open` is needed because the board's captured output is no terminal, and
//! `--json` would skip opening (until Remote #5713 lets `--open` win; then this reads
//! `--open --json`). Ops shows Remote's own result line, and never installs, serves or
//! pairs anything itself; the one-time read-only offer for the opened browser is
//! Remote's own behaviour of that command.

use crate::core::{Core, SquadError};

/// Runs `tmt remote open --open` and says what happened in one status line.
pub(super) fn open(core: &Core) -> Result<String, String> {
    outcome(core.text(&["remote", "open", "--open"]))
}

/// The status line for one `tmt remote open --open` result: Remote's own first line on
/// success, its error otherwise, naming the command that installs or starts Remote.
pub(super) fn outcome(result: Result<String, SquadError>) -> Result<String, String> {
    match result {
        Ok(stdout) => stdout
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| "tmt remote open printed nothing.".to_owned()),
        Err(error) => Err(
            if error.code == "EXTENSION_NOT_INSTALLED"
                || error.message.contains("official extension")
                    && error.message.contains("not installed")
            {
                "The web UI needs Remote: run tmt extension install remote.".into()
            } else if (error.code == "REMOTE_NOT_RUNNING"
                || error.message.contains("Remote is not running"))
                && !error.message.contains("tmt remote serve")
            {
                "Remote is not running: run tmt remote serve --background.".into()
            } else {
                // Remote names the command that starts it; keep its words when it does.
                error.message
            },
        ),
    }
}

#[cfg(test)]
mod tests;

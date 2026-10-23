//! tmux's default launch template and the pane marker it leaves. The template runs outside
//! core, so the marker is the only trace that TMT opened the pane: evidence of intent, never
//! authority to close it (`contracts/driver-protocol-v1.md`).

use super::{Tmux, TmuxError, TmuxFailure};
use crate::process::CommandRunner;
use std::time::Instant;
use tmt_driver_protocol::LaunchTemplate;

/// The pane option the shipped template sets on the pane it opens.
pub(super) const OPENED_OPTION: &str = "@tmt.opened";

/// Opens a new window in the session, or creates the session detached with the command as its
/// first window. Nothing is interpolated: `$1` is the session, `$2` the command line. It sets
/// the marker on the pane it opened and prints that pane's ID; a command that ends before the
/// marker is set leaves nothing to mark, and the script fails. An empty session or command is
/// refused first: tmux would read an empty session name as "any session".
const SCRIPT: &str = "[ -n \"$1\" ] && [ -n \"$2\" ] || { echo 'tmt-launch: a session and a command are required' >&2; exit 2; }; \
if tmux has-session -t \"=$1\" 2>/dev/null; then \
p=$(tmux new-window -d -P -F '#{pane_id}' -t \"=$1:\" -- \"$2\"); \
else p=$(tmux new-session -d -P -F '#{pane_id}' -s \"$1\" -- \"$2\"); fi \
&& tmux set-option -p -t \"$p\" @tmt.opened 1 && printf '%s\\n' \"$p\"";

pub fn launch_template() -> LaunchTemplate {
    let argv: Vec<String> = ["sh", "-c", SCRIPT, "tmt-launch", "{session}", "{command}"]
        .map(str::to_owned)
        .to_vec();
    LaunchTemplate::new(&argv).expect("the shipped tmux launch template is valid")
}

impl<R: CommandRunner> Tmux<R> {
    /// Whether the pane carries the marker the shipped template sets. A pane that was never
    /// opened by a template reads as no.
    pub fn pane_opened_by_tmt(
        &self,
        socket: &str,
        pane: &str,
        deadline: Instant,
    ) -> Result<bool, TmuxError> {
        if !super::valid_pane_id(pane) {
            return Err(TmuxError::evidence("pane id is invalid"));
        }
        let output = self.run(
            "tmux",
            vec![
                "-S".into(),
                socket.into(),
                "display-message".into(),
                "-p".into(),
                "-t".into(),
                pane.into(),
                format!("#{{{OPENED_OPTION}}}"),
            ],
            deadline,
            64,
            TmuxFailure::Evidence,
        )?;
        Ok(output.strip_suffix('\n').unwrap_or(&output) == "1")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn the_shipped_template_is_valid_and_keeps_the_script_free_of_the_placeholders() {
        let template = launch_template();
        let argv = template.argv();
        assert_eq!(&argv[..2], ["sh", "-c"]);
        assert!(!argv[2].contains("{session}") && !argv[2].contains("{command}"));
        assert!(argv[2].contains(OPENED_OPTION));
        assert_eq!(&argv[4..], ["{session}", "{command}"]);
    }

    /// The read is the one `stop --close` relies on, so it runs against a real server. The
    /// shipped template's side of the marker is checked by `host-templates.test.ts`.
    #[test]
    #[ignore = "requires tmux; creates and stops only a private explicit-socket server"]
    fn only_a_pane_carrying_the_marker_reads_as_opened_by_tmt() {
        use crate::{test_support::TestDirectory, tmux::server_id_tests::PrivateServer};
        let server = PrivateServer::new(TestDirectory::new(), "launch.sock");
        let command = |args: &[&str]| server.command(args).unwrap().trim().to_owned();
        command(&[
            "-f",
            "/dev/null",
            "new-session",
            "-d",
            "-s",
            "squad",
            "sleep 60",
        ]);
        let first = command(&["display-message", "-p", "-t", "=squad:", "#{pane_id}"]);
        let opened = command(&[
            "new-window",
            "-d",
            "-P",
            "-F",
            "#{pane_id}",
            "-t",
            "=squad:",
            "sleep 60",
        ]);
        let tmux = Tmux::default();
        let read = |pane: &str| {
            tmux.pane_opened_by_tmt(
                &server.socket,
                pane,
                Instant::now() + Duration::from_secs(5),
            )
        };
        assert!(!read(&opened).unwrap(), "an unmarked pane");
        command(&["set-option", "-p", "-t", &opened, OPENED_OPTION, "1"]);
        assert!(read(&opened).unwrap());
        assert!(!read(&first).unwrap(), "its neighbour stays unmarked");
        // A value other than the marker's is not the marker.
        command(&["set-option", "-p", "-t", &first, OPENED_OPTION, "0"]);
        assert!(!read(&first).unwrap());
        assert!(read("not-a-pane").is_err());
    }
}

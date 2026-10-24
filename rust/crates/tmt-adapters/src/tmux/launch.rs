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

    /// Closes exactly this pane with one `kill-pane`, and no retry. It reads the pane again in
    /// the same listing as every other pane and refuses unless the pane still runs `pane_pid`
    /// and still carries the mark. It also refuses when the pane is the server's only one:
    /// closing it would end the server and every session on it.
    pub fn close_opened_pane(
        &self,
        socket: &str,
        pane: &str,
        pane_pid: u64,
        deadline: Instant,
    ) -> Result<(), TmuxError> {
        if !super::valid_pane_id(pane) {
            return Err(TmuxError::evidence("pane id is invalid"));
        }
        let listing = self.run(
            "tmux",
            vec![
                "-S".into(),
                socket.into(),
                "list-panes".into(),
                "-a".into(),
                "-F".into(),
                format!("#{{pane_id}} #{{pane_pid}} #{{{OPENED_OPTION}}}"),
            ],
            deadline,
            1024 * 1024,
            TmuxFailure::Evidence,
        )?;
        let mut count = 0;
        let mut found = None;
        for row in listing.lines() {
            let mut fields = row.split(' ');
            let (Some(id), Some(pid)) = (fields.next(), fields.next()) else {
                continue;
            };
            count += 1;
            if id == pane {
                found = Some((pid.parse::<u64>().ok(), fields.next()));
            }
        }
        match found {
            None => Err(TmuxError::evidence("the pane no longer exists")),
            Some((pid, _)) if pid != Some(pane_pid) => Err(TmuxError::evidence(
                "the pane no longer runs the verified process",
            )),
            Some((_, mark)) if mark != Some("1") => Err(TmuxError::evidence(
                "the pane no longer carries the mark that tmt opened it",
            )),
            Some(_) if count < 2 => Err(TmuxError::evidence(
                "closing the server's only pane would end the tmux server",
            )),
            Some(_) => self
                .run(
                    "tmux",
                    vec![
                        "-S".into(),
                        socket.into(),
                        "kill-pane".into(),
                        "-t".into(),
                        pane.into(),
                    ],
                    deadline,
                    64,
                    TmuxFailure::Command,
                )
                .map(|_| ()),
        }
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

    fn close_calls(listing: &'static str) -> (Result<(), TmuxError>, Vec<Vec<String>>) {
        use crate::scripted_runner::ScriptedRunner;
        let tmux = Tmux::new(ScriptedRunner::new([Ok(listing), Ok("")]));
        let result = tmux.close_opened_pane(
            "/tmp/private.sock",
            "%7",
            4242,
            Instant::now() + Duration::from_secs(5),
        );
        let calls = tmux
            .runner
            .calls
            .borrow()
            .iter()
            .map(|call| call.args.clone())
            .collect();
        (result, calls)
    }

    #[test]
    fn a_marked_pane_still_running_the_verified_process_is_closed_with_one_kill_pane() {
        let (result, calls) = close_calls("%1 100 \n%7 4242 1\n");
        result.unwrap();
        assert_eq!(calls.len(), 2, "{calls:?}");
        assert!(calls[0].iter().any(|arg| arg == "list-panes"));
        assert_eq!(
            &calls[1][calls[1].len() - 5..],
            ["-S", "/tmp/private.sock", "kill-pane", "-t", "%7"]
        );
    }

    #[test]
    fn a_pane_that_changed_is_never_closed() {
        for (listing, reason) in [
            ("%1 100 \n", "no longer exists"),
            ("%1 100 \n%7 999 1\n", "no longer runs the verified process"),
            ("%1 100 \n%7 4242 \n", "no longer carries the mark"),
            ("%1 100 \n%7 4242 0\n", "no longer carries the mark"),
            ("%7 4242 1\n", "only pane"),
        ] {
            let (result, calls) = close_calls(listing);
            let error = result.unwrap_err().to_string();
            assert!(error.contains(reason), "{listing:?}: {error}");
            assert_eq!(
                calls.len(),
                1,
                "{listing:?}: only the listing ran: {calls:?}"
            );
        }
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

    /// The close `stop --close` runs, against a real server: only the marked pane the caller
    /// verified goes, and never the server's last one.
    #[test]
    #[ignore = "requires tmux; creates and stops only a private explicit-socket server"]
    fn only_the_marked_verified_pane_is_closed_and_never_the_last_one() {
        use crate::{test_support::TestDirectory, tmux::server_id_tests::PrivateServer};
        let server = PrivateServer::new(TestDirectory::new(), "close.sock");
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
        let pid_of = |pane: &str| {
            command(&["display-message", "-p", "-t", pane, "#{pane_pid}"])
                .parse::<u64>()
                .unwrap()
        };
        let tmux = Tmux::default();
        let close = |pane: &str, pid: u64| {
            tmux.close_opened_pane(
                &server.socket,
                pane,
                pid,
                Instant::now() + Duration::from_secs(5),
            )
        };
        let panes = || command(&["list-panes", "-a", "-F", "#{pane_id}"]);

        assert!(close(&opened, pid_of(&opened)).is_err(), "no mark");
        assert!(panes().contains(&opened));
        command(&["set-option", "-p", "-t", &opened, OPENED_OPTION, "1"]);
        assert!(
            close(&opened, pid_of(&opened) + 1).is_err(),
            "other process"
        );
        assert!(panes().contains(&opened));
        close(&opened, pid_of(&opened)).unwrap();
        assert!(!panes().contains(&opened), "the marked pane is gone");
        assert!(panes().contains(&first), "its neighbour stays");
        command(&["set-option", "-p", "-t", &first, OPENED_OPTION, "1"]);
        assert!(close(&first, pid_of(&first)).is_err(), "the only pane");
        assert!(panes().contains(&first), "the server is still up");
    }
}

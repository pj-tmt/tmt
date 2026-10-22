//! Row actions shared by the board and the plain commands: opening links,
//! copying, starting programs and focusing panes. No shell is ever involved.

use crate::{
    core::{Core, SquadError},
    runner::{self, RunError},
};
use serde_json::Value;
use std::{
    ffi::OsString,
    io::Write,
    os::unix::process::CommandExt,
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};

const TIMEOUT: Duration = Duration::from_secs(3);

fn unfinished(program: &str, error: RunError) -> String {
    match error {
        RunError::Spawn => format!("Could not start {program}."),
        RunError::Timeout => format!("{program} did not finish in time."),
        RunError::OutputLimit => format!("{program} printed more than expected."),
        RunError::Io => format!("Could not talk to {program}."),
        RunError::Cancelled => format!("{program} was cancelled."),
    }
}

/// Starts a program detached from the board: no shell, null stdio (so it can
/// never draw over the terminal), its own process group (so closing the
/// board's terminal does not end it), and a thread that reaps it.
pub fn spawn(argv: &[String]) -> Result<(), String> {
    let (program, args) = argv.split_first().ok_or("no program to run")?;
    crate::runner::assert_commands_allowed("spawn");
    let mut child = Command::new(program)
        .args(args)
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("Could not start {program}: {error}"))?;
    std::thread::spawn(move || child.wait());
    Ok(())
}

/// Only http(s) links with a host and no whitespace or control characters.
pub fn web_link(link: &str) -> Result<&str, String> {
    let rest = link
        .strip_prefix("https://")
        .or_else(|| link.strip_prefix("http://"))
        .ok_or_else(|| format!("Only http(s) links open; refused '{link}'."))?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if host.is_empty() || link.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(format!("'{link}' is not a valid web link."));
    }
    Ok(link)
}

/// What the board may hand to an opener: a web link, or a `mailto:` address with no
/// whitespace or control characters.
pub fn openable(link: &str) -> Result<&str, String> {
    match link.strip_prefix("mailto:") {
        Some(address) if !address.is_empty() => {
            if link.chars().any(|c| c.is_whitespace() || c.is_control()) {
                Err(format!("'{link}' is not a valid mail link."))
            } else {
                Ok(link)
            }
        }
        _ => web_link(link),
    }
}

/// The row's link for a bare `open`: `pr_link`, else `link`, else the first
/// `*_link` field.
pub fn default_link(row: &Value) -> Option<&str> {
    let fields = row["fields"].as_object()?;
    ["pr_link", "link"]
        .iter()
        .find_map(|key| fields.get(*key).and_then(Value::as_str))
        .or_else(|| {
            fields
                .iter()
                .find(|(key, _)| key.ends_with("_link"))
                .and_then(|(_, value)| value.as_str())
        })
}

/// Resolve exactly the chosen field/template. An invalid first choice never
/// substitutes another record field, and previews use the same resolver.
pub fn resolve_open(action: &crate::action::Action, row: &Value) -> Result<String, String> {
    resolve_open_parameters(
        action,
        crate::template::Parameters {
            row,
            ops: None,
            board_pane: None,
        },
    )
}

pub fn resolve_open_parameters(
    action: &crate::action::Action,
    parameters: crate::template::Parameters<'_>,
) -> Result<String, String> {
    let link = match action.args.first() {
        Some(template) => template.fill_parameters(parameters)?,
        None => default_link(parameters.row)
            .ok_or("This row has no link field.")?
            .to_owned(),
    };
    web_link(&link)?;
    Ok(link)
}

/// The opener's argv for `link`. A `{url}` in an argument is replaced by the link, so the
/// link stays one argument whatever it holds; an opener without one gets the link appended.
pub fn open_argv(link: &str, opener: Option<&[String]>) -> Vec<String> {
    let Some(opener) = opener else {
        let program = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        return vec![program.into(), link.to_owned()];
    };
    if opener.iter().any(|arg| arg.contains("{url}")) {
        opener
            .iter()
            .map(|arg| arg.replace("{url}", link))
            .collect()
    } else {
        let mut argv = opener.to_vec();
        argv.push(link.to_owned());
        argv
    }
}

/// What decides whether this machine can show a link to the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenHost {
    pub macos: bool,
    pub wsl: bool,
    pub ssh: bool,
    /// A graphical session is reachable (`DISPLAY` or `WAYLAND_DISPLAY`).
    pub display: bool,
}

impl OpenHost {
    pub fn current() -> Self {
        let set = |name: &str| std::env::var_os(name).is_some_and(|value| !value.is_empty());
        Self {
            macos: cfg!(target_os = "macos"),
            wsl: cfg!(target_os = "linux")
                && (set("WSL_DISTRO_NAME")
                    || std::fs::read_to_string("/proc/version")
                        .is_ok_and(|text| text.to_ascii_lowercase().contains("microsoft"))),
            ssh: set("SSH_CONNECTION") || set("SSH_TTY"),
            display: set("DISPLAY") || set("WAYLAND_DISPLAY"),
        }
    }
}

/// The opener to use when none is configured: `open` on macOS, `wslview` (else
/// `cmd.exe /c start`) under WSL, `xdg-open` with a graphical session. Over SSH there is
/// none, since the link would open on the remote machine, not in front of the user.
pub fn detect_opener(host: &OpenHost, on_path: &dyn Fn(&str) -> bool) -> Option<Vec<String>> {
    let argv = |words: &[&str]| Some(words.iter().map(|word| (*word).to_owned()).collect());
    if host.ssh {
        return None;
    }
    if host.macos {
        return argv(&["open", "{url}"]);
    }
    if host.wsl {
        if on_path("wslview") {
            return argv(&["wslview", "{url}"]);
        }
        return on_path("cmd.exe").then(|| {
            ["cmd.exe", "/c", "start", "", "{url}"]
                .iter()
                .map(|word| (*word).to_owned())
                .collect()
        });
    }
    (host.display && on_path("xdg-open")).then(|| vec!["xdg-open".into(), "{url}".into()])
}

/// Whether an executable of this name is on `PATH`.
pub fn on_path(program: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|dir| {
            std::fs::metadata(dir.join(program))
                .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        })
    })
}

/// `cmd.exe /c start` reads its argument as a command line, so a link holding any of its
/// operators is refused rather than quoted.
pub fn cmd_safe(link: &str) -> bool {
    !link.contains(['&', '|', '<', '>', '^', '%', '"', '(', ')'])
}

pub fn open(link: &str, opener: Option<&[String]>) -> Result<(), String> {
    let link = openable(link)?;
    let argv = open_argv(link, opener);
    if argv.first().is_some_and(|program| program == "cmd.exe") && !cmd_safe(link) {
        return Err("cmd.exe cannot open this link safely; copy it instead.".into());
    }
    spawn(&argv)
}

#[derive(Debug, PartialEq, Eq)]
pub enum Copied {
    Program,
    Clipboard,
    /// tmux kept the text but its set-clipboard is off.
    TmuxBuffer,
}

impl Copied {
    /// Where the text went, for JSON output.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Program => "program",
            Self::Clipboard => "clipboard",
            Self::TmuxBuffer => "tmux-buffer",
        }
    }

    pub fn describe(&self) -> &'static str {
        match self {
            Self::Program => "Copied with the configured clipboard program.",
            Self::Clipboard => "Copied to the clipboard.",
            Self::TmuxBuffer => "Copied to tmux buffer (tmux set-clipboard is off).",
        }
    }
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(BASE64[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// The OSC 52 sequence that asks the terminal to set its clipboard.
pub fn osc52(text: &str) -> String {
    format!("\u{1b}]52;c;{}\u{7}", base64(text.as_bytes()))
}

fn tmux(program: &Path, socket: &str, args: &[&str], input: &[u8]) -> Result<String, String> {
    let mut argv: Vec<OsString> = vec!["-S".into(), socket.into()];
    argv.extend(args.iter().map(OsString::from));
    let finished = runner::run(program, &argv, input, TIMEOUT, 64 * 1024)
        .map_err(|error| unfinished("tmux", error))?;
    if !finished.success {
        return Err("tmux refused the clipboard request.".into());
    }
    Ok(String::from_utf8_lossy(&finished.stdout).trim().to_owned())
}

/// Where a pane lives, as `session:window`, from one bounded `display-message`
/// on the invoker's server. Only a pane id (`%N`) is asked about, so a stored
/// holder can never pass an option. Any failure is `None`.
pub fn pane_place(program: &Path, socket: &str, pane: &str) -> Option<String> {
    let numbered = pane.strip_prefix('%')?;
    if numbered.is_empty() || !numbered.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let place = tmux(
        program,
        socket,
        &[
            "display-message",
            "-p",
            "-t",
            pane,
            "#{session_name}:#{window_name}",
        ],
        b"",
    )
    .ok()?;
    let place = place.lines().next()?.trim();
    (!place.is_empty() && place != ":").then(|| place.to_owned())
}

/// `tmux -V` as (major, minor); load-buffer -w needs 3.2.
fn tmux_version(text: &str) -> Option<(u32, u32)> {
    let number = text
        .trim()
        .strip_prefix("tmux ")?
        .trim_start_matches("next-");
    let mut parts = number.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor: String = parts
        .next()?
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    Some((major, minor.parse().ok()?))
}

/// tmux keeps the text as a buffer and, per its `set-clipboard`, passes it to
/// the attached terminal's clipboard.
fn copy_through_tmux(program: &Path, socket: &str, text: &str) -> Result<Copied, String> {
    let version = tmux(program, socket, &["-V"], b"")
        .ok()
        .and_then(|v| tmux_version(&v));
    if !version.is_some_and(|v| v >= (3, 2)) {
        return Err(
            "Copying inside tmux needs tmux 3.2 or later; set clipboard = [...] in ops.toml instead."
                .into(),
        );
    }
    tmux(
        program,
        socket,
        &["load-buffer", "-w", "-"],
        text.as_bytes(),
    )?;
    let setting = tmux(program, socket, &["show", "-sv", "set-clipboard"], b"").unwrap_or_default();
    Ok(if setting == "off" {
        Copied::TmuxBuffer
    } else {
        Copied::Clipboard
    })
}

/// A configured program wins; inside tmux the text goes through tmux's own
/// buffer and clipboard path; otherwise OSC 52 goes straight to the terminal.
pub fn copy(
    text: &str,
    program: Option<&[String]>,
    tmux_socket: Option<&str>,
) -> Result<Copied, String> {
    if let Some(argv) = program {
        let (name, args) = argv.split_first().ok_or("no clipboard program")?;
        let args: Vec<OsString> = args.iter().map(OsString::from).collect();
        let finished = runner::run(Path::new(name), &args, text.as_bytes(), TIMEOUT, 64 * 1024)
            .map_err(|error| unfinished(name, error))?;
        return if finished.success {
            Ok(Copied::Program)
        } else {
            Err(format!("{name} failed; nothing was copied."))
        };
    }
    if let Some(socket) = tmux_socket {
        return copy_through_tmux(Path::new("tmux"), socket, text);
    }
    let mut terminal = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/tty")
        .map_err(|error| format!("No terminal to copy through: {error}"))?;
    terminal
        .write_all(osc52(text).as_bytes())
        .and_then(|()| terminal.flush())
        .map_err(|error| format!("Could not write to the terminal: {error}"))?;
    Ok(Copied::Clipboard)
}

/// One bounded `tmux show -sv <option>` on the invoker's server. Any failure,
/// including a missing tmux, is `None`: the caller treats it as unsupported.
pub fn tmux_option(socket: &str, option: &str) -> Option<String> {
    tmux(Path::new("tmux"), socket, &["show", "-sv", option], b"")
        .ok()
        .filter(|value| !value.is_empty())
}

/// Whether the invoker's tmux hands OSC 8 on to the attached terminal: tmux 3.4 or
/// later whose client reports the `hyperlinks` terminal feature. Any failure is `false`.
pub fn tmux_passes_hyperlinks(socket: &str) -> bool {
    let program = Path::new("tmux");
    let new_enough = tmux(program, socket, &["-V"], b"")
        .ok()
        .and_then(|v| tmux_version(&v))
        .is_some_and(|v| v >= (3, 4));
    new_enough
        && tmux(
            program,
            socket,
            &["display-message", "-p", "#{client_termfeatures}"],
            b"",
        )
        .is_ok_and(|features| features.split(',').any(|feature| feature == "hyperlinks"))
}

/// Whether tmux sends Shift+Enter as `CSI 13;2 u` once the board asks for
/// modifyOtherKeys. The reader parses only that `csi-u` form; tmux's default
/// `xterm` form (`CSI 27;2;13 ~`) would be dropped, so nothing is requested then.
pub fn tmux_reports_shift_enter(extended_keys: &str, format: &str) -> bool {
    matches!(extended_keys, "on" | "always") && format == "csi-u"
}

/// The invoker's tmux server socket, from `TMUX` (`socket,pid,session`).
pub fn tmux_socket() -> Option<String> {
    let tmux = std::env::var("TMUX").ok()?;
    let socket = tmux.split(',').next()?;
    (!socket.is_empty()).then(|| socket.to_owned())
}

/// Where a focus went, where it came from, and the tmux client that moved.
pub struct Focus {
    pub pane: String,
    pub from: Option<String>,
    pub client: String,
}

/// Core owns the host exchange and all caller/server/neighbour validation.
/// Ops admits only the published success shapes; malformed success is not a swap.
pub fn swap(core: &Core, target: &str, direction: &str) -> Result<bool, SquadError> {
    let result = core.json(&["focus", target, "--into", direction])?;
    let neighbour = result["with"]["pane"]
        .as_str()
        .filter(|pane| !pane.is_empty());
    let swapped = result["swapped"]["pane"]
        .as_str()
        .filter(|pane| !pane.is_empty());
    if neighbour.is_some()
        && result["already"] == true
        && result.get("swapped").is_some_and(Value::is_null)
    {
        return Ok(false);
    }
    if neighbour.is_some()
        && swapped.is_some()
        && result.get("already").is_none_or(|value| value == false)
    {
        return Ok(true);
    }
    Err(SquadError::new(
        "SQUAD_CORE_UNAVAILABLE",
        "tmt focus --into returned no exchange result.",
    ))
}

pub fn focus(core: &Core, target: &str) -> Result<Focus, SquadError> {
    let focused = core.json(&["focus", target])?;
    Ok(Focus {
        pane: focused["focused"]["pane"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        from: focused["from"]["pane"].as_str().map(str::to_owned),
        client: focused["client"].as_str().unwrap_or_default().to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(macos: bool, wsl: bool, ssh: bool, display: bool) -> OpenHost {
        OpenHost {
            macos,
            wsl,
            ssh,
            display,
        }
    }
    fn words(argv: &[&str]) -> Option<Vec<String>> {
        Some(argv.iter().map(|word| (*word).to_owned()).collect())
    }

    #[test]
    fn the_opener_is_detected_per_platform_and_never_over_ssh() {
        let all = |_: &str| true;
        let none = |_: &str| false;
        assert_eq!(
            detect_opener(&host(true, false, false, false), &all),
            words(&["open", "{url}"])
        );
        assert_eq!(
            detect_opener(&host(false, false, false, true), &all),
            words(&["xdg-open", "{url}"])
        );
        // No graphical session or no xdg-open: nothing to show a link in.
        assert_eq!(detect_opener(&host(false, false, false, false), &all), None);
        assert_eq!(detect_opener(&host(false, false, false, true), &none), None);
        assert_eq!(
            detect_opener(&host(false, true, false, false), &all),
            words(&["wslview", "{url}"])
        );
        assert_eq!(
            detect_opener(&host(false, true, false, false), &|p| p == "cmd.exe"),
            words(&["cmd.exe", "/c", "start", "", "{url}"])
        );
        assert_eq!(detect_opener(&host(false, true, false, false), &none), None);
        for local in [
            host(true, false, true, true),
            host(false, false, true, true),
            host(false, true, true, true),
        ] {
            assert_eq!(detect_opener(&local, &all), None, "{local:?}");
        }
    }

    #[test]
    fn a_link_with_spaces_quotes_and_semicolons_stays_one_argument() {
        let link = "https://example.com/a b?x='1';y=\"2\"&z=$(rm -rf /)";
        let template = vec!["firefox".to_owned(), "--new-tab={url}".to_owned()];
        let argv = open_argv(link, Some(&template));
        assert_eq!(argv, ["firefox".to_owned(), format!("--new-tab={link}")]);
        let plain = vec!["open".to_owned(), "-a".to_owned(), "Safari".to_owned()];
        assert_eq!(
            open_argv(link, Some(&plain)),
            ["open", "-a", "Safari", link].map(String::from)
        );
    }

    #[test]
    fn only_web_and_mail_links_open() {
        for link in [
            "https://example.com/x",
            "http://example.com",
            "mailto:a@example.com",
            "mailto:a@example.com?subject=hi",
        ] {
            assert_eq!(openable(link), Ok(link));
        }
        for link in [
            "javascript:alert(1)",
            "tmt:jump/worker",
            "file:///etc/passwd",
            "ftp://example.com",
            "mailto:",
            "mailto:a b@example.com",
            "https://",
            "https://exa mple.com",
            "/relative",
        ] {
            assert!(openable(link).is_err(), "{link}");
        }
    }

    #[test]
    fn cmd_exe_refuses_a_link_it_would_read_as_commands() {
        assert!(cmd_safe("https://example.com/a?b=1;c"));
        assert!(!cmd_safe("https://example.com/?a=1&b=2"));
        assert!(!cmd_safe("https://example.com/%41"));
    }

    #[test]
    fn swap_uses_only_the_core_focus_port_and_preserves_refusals() {
        let root =
            std::env::temp_dir().join(format!("ops-swap-port-{}", crate::id::new_v4().unwrap()));
        std::fs::create_dir(&root).unwrap();
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                std::fs::remove_dir_all(&self.0).unwrap();
            }
        }
        let _cleanup = Cleanup(root.clone());
        let executable = root.join("core");
        crate::test_support::write_ready_executable(
            &executable,
            r#"#!/bin/sh
root=${0%/*}
printf '%s\n' "$@" > "$root/argv"
cat "$root/result"
if [ -e "$root/fail" ]; then exit 1; fi
"#,
        );
        let core = Core::at(executable);
        for direction in ["left", "right", "up", "down"] {
            std::fs::write(
                root.join("result"),
                r#"{"swapped":{"pane":"%7"},"with":{"pane":"%8"}}"#,
            )
            .unwrap();
            assert!(swap(&core, "identity-uuid", direction).unwrap());
            let argv = std::fs::read_to_string(root.join("argv")).unwrap();
            assert_eq!(
                argv.lines().collect::<Vec<_>>(),
                ["focus", "identity-uuid", "--into", direction, "--json"]
            );
        }
        std::fs::write(
            root.join("result"),
            r#"{"swapped":null,"with":{"pane":"%7"},"already":true}"#,
        )
        .unwrap();
        assert!(!swap(&core, "identity-uuid", "left").unwrap());
        for malformed in [
            "{}",
            r#"{"swapped":{"pane":"%7"}}"#,
            r#"{"swapped":null,"with":{"pane":"%7"}}"#,
            r#"{"swapped":{"pane":"%7"},"with":{"pane":"%8"},"already":true}"#,
            r#"{"swapped":{"pane":"%7"},"with":{"pane":"%8"},"already":"false"}"#,
        ] {
            std::fs::write(root.join("result"), malformed).unwrap();
            assert_eq!(
                swap(&core, "identity-uuid", "left").unwrap_err().code,
                "SQUAD_CORE_UNAVAILABLE"
            );
        }
        std::fs::write(
            root.join("result"),
            r#"{"error":{"code":"NAME_NOT_FOUND","message":"Target is gone."}}"#,
        )
        .unwrap();
        std::fs::write(root.join("fail"), "").unwrap();
        let error = swap(&core, "identity-uuid", "left").unwrap_err();
        assert_eq!(error.code, "NAME_NOT_FOUND");
        assert_eq!(error.message, "Target is gone.");
    }

    use serde_json::json;

    #[test]
    fn shift_enter_is_requested_only_for_extended_keys_in_the_csi_u_form() {
        for (extended, format, wanted) in [
            ("on", "csi-u", true),
            ("always", "csi-u", true),
            ("on", "xterm", false),
            ("off", "csi-u", false),
            ("", "", false),
            ("on", "", false),
        ] {
            assert_eq!(
                tmux_reports_shift_enter(extended, format),
                wanted,
                "{extended:?} {format:?}"
            );
        }
    }

    #[test]
    fn only_http_links_with_a_host_open() {
        for link in [
            "https://example.com/pull/412",
            "http://localhost:8080/x?y=1#z",
        ] {
            assert_eq!(web_link(link), Ok(link));
        }
        for link in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "ssh://host",
            "https://",
            "https:///path",
            "https://exa mple.com",
            "https://example.com/\u{1b}[2J",
            "HTTPS://example.com",
            "example.com",
        ] {
            assert!(web_link(link).is_err(), "{link}");
        }
        let row = json!({"fields": {"issue_link": "https://i/1", "pr_link": "https://p/2"}});
        assert_eq!(default_link(&row), Some("https://p/2"));
        assert_eq!(
            default_link(&json!({"fields": {"doc_link": "https://d"}})),
            Some("https://d")
        );
        assert_eq!(default_link(&json!({"fields": {}})), None);
    }

    #[test]
    fn osc52_encodes_utf8_as_base64_and_versions_gate_tmux() {
        assert_eq!(osc52("hi"), "\u{1b}]52;c;aGk=\u{7}");
        assert_eq!(
            osc52("安装 x"),
            format!("\u{1b}]52;c;{}\u{7}", "5a6J6KOFIHg=")
        );
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"abc"), "YWJj");
        assert_eq!(tmux_version("tmux 3.7c"), Some((3, 7)));
        assert_eq!(tmux_version("tmux 3.2"), Some((3, 2)));
        assert_eq!(tmux_version("tmux next-3.8"), Some((3, 8)));
        assert!(tmux_version("tmux 3.1c").is_some_and(|v| v < (3, 2)));
        assert_eq!(tmux_version("screen 4"), None);
    }

    #[test]
    fn a_configured_clipboard_program_gets_the_text_on_stdin_without_a_shell() {
        let out = std::env::temp_dir().join(format!("tmt-ops-clip-{}", std::process::id()));
        let program = vec![
            "/bin/sh".to_owned(),
            "-c".to_owned(),
            format!("cat > '{}'", out.display()),
        ];
        let text = "auth-fix: $(rm -rf ~) `id` ; rotate";
        assert_eq!(
            copy(text, Some(&program), Some("/nonexistent.sock")),
            Ok(Copied::Program)
        );
        assert_eq!(std::fs::read_to_string(&out).unwrap(), text);
        let _ = std::fs::remove_file(out);
        let failing = vec!["/bin/sh".to_owned(), "-c".to_owned(), "exit 3".to_owned()];
        assert!(copy("x", Some(&failing), None).is_err());
    }

    #[test]
    fn inside_tmux_the_text_goes_to_a_buffer_on_the_invoker_socket_from_tmux_3_2() {
        let dir = std::env::temp_dir().join(format!("tmt-ops-tmux-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("tmux");
        let calls = dir.join("calls");
        // Records "-S <socket> <command>" per call and the loaded buffer.
        let script = format!(
            "#!/bin/sh\necho \"$*\" >> '{calls}'\ncase \"$3\" in\n\
             -V) cat '{dir}/version' ;;\nload-buffer) cat > '{dir}/buffer' ;;\nshow) cat '{dir}/setting' ;;\nesac\n",
            calls = calls.display(),
            dir = dir.display()
        );
        crate::test_support::write_ready_executable(&fake, &script);
        let set = |name: &str, value: &str| std::fs::write(dir.join(name), value).unwrap();

        assert!(!calls.exists(), "readiness does not record a tmux call");
        assert!(!dir.join("buffer").exists(), "readiness does not copy text");

        set("version", "tmux 3.1c\n");
        let refused = copy_through_tmux(&fake, "/sock/a", "x").unwrap_err();
        assert!(refused.contains("tmux 3.2 or later"), "{refused}");
        assert!(!dir.join("buffer").exists(), "an old tmux gets nothing");

        set("version", "tmux 3.4\n");
        set("setting", "external\n");
        assert_eq!(
            copy_through_tmux(&fake, "/sock/a", "a; $(b)"),
            Ok(Copied::Clipboard)
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("buffer")).unwrap(),
            "a; $(b)"
        );
        set("setting", "off\n");
        assert_eq!(
            copy_through_tmux(&fake, "/sock/a", "c"),
            Ok(Copied::TmuxBuffer)
        );
        assert_eq!(
            std::fs::read_to_string(&calls).unwrap(),
            "-S /sock/a -V\n-S /sock/a -V\n-S /sock/a load-buffer -w -\n-S /sock/a show -sv set-clipboard\n\
             -S /sock/a -V\n-S /sock/a load-buffer -w -\n-S /sock/a show -sv set-clipboard\n"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}

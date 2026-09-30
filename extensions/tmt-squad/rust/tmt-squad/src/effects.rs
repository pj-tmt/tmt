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
    }
}

/// Starts a program detached from the board: no shell, null stdio (so it can
/// never draw over the terminal), its own process group (so closing the
/// board's terminal does not end it), and a thread that reaps it.
pub fn spawn(argv: &[String]) -> Result<(), String> {
    let (program, args) = argv.split_first().ok_or("no program to run")?;
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

pub fn open(link: &str, opener: Option<&[String]>) -> Result<(), String> {
    let link = web_link(link)?;
    let mut argv: Vec<String> = match opener {
        Some(opener) => opener.to_vec(),
        None if cfg!(target_os = "macos") => vec!["open".into()],
        None => vec!["xdg-open".into()],
    };
    argv.push(link.to_owned());
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
            "Copying inside tmux needs tmux 3.2 or later; set clipboard = [...] in squad.toml instead."
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
    use serde_json::json;

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
        let out = std::env::temp_dir().join(format!("tmt-squad-clip-{}", std::process::id()));
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
        let dir = std::env::temp_dir().join(format!("tmt-squad-tmux-{}", std::process::id()));
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
        crate::test_support::write_executable(&fake, &script);
        let set = |name: &str, value: &str| std::fs::write(dir.join(name), value).unwrap();

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

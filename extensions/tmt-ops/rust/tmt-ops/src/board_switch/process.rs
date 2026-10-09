//! Selected-process evidence for replacing former boards; names are never authority.

use super::{fail, text};
use crate::core::SquadError;
#[cfg(target_os = "linux")]
use std::fs;
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Process {
    pub pid: u32,
    pub parent: u32,
    pub group: u32,
    pub foreground: u32,
    pub tty: String,
    pub start: String,
}

pub(super) fn table() -> Result<Vec<Process>, SquadError> {
    let uid = nix::unistd::geteuid().as_raw();
    let output = text(
        Path::new("/bin/ps"),
        &["-axo", "pid=,ppid=,uid=,pgid=,tpgid=,tty=,stat=,lstart="],
    )?;
    Ok(output
        .lines()
        .filter_map(|line| {
            let fields: Vec<_> = line.split_whitespace().collect();
            if fields.len() != 12
                || fields[2].parse::<u32>().ok()? != uid
                || fields[6].contains('Z')
            {
                return None;
            }
            Some(Process {
                pid: fields[0].parse().ok()?,
                parent: fields[1].parse().ok()?,
                group: fields[3].parse().ok()?,
                foreground: fields[4].parse().ok()?,
                tty: fields[5].into(),
                start: fields[7..].join(" "),
            })
        })
        .collect())
}

pub(super) fn in_pane(process: &Process, root: u32, tty: &str, table: &[Process]) -> bool {
    if process.group == 0
        || process.group != process.foreground
        || process.tty != tty.strip_prefix("/dev/").unwrap_or(tty)
    {
        return false;
    }
    let mut pid = process.pid;
    for _ in 0..32 {
        if pid == root {
            return true;
        }
        let Some(parent) = table.iter().find(|p| p.pid == pid).map(|p| p.parent) else {
            return false;
        };
        if parent == pid || parent == 0 {
            return false;
        }
        pid = parent;
    }
    false
}

pub(super) fn former(executable: &Path, prefix: &Path) -> bool {
    executable.starts_with(prefix.join("lib/tmt-squad"))
        && executable
            .file_name()
            .is_some_and(|name| name == "tmt-squad")
        && executable
            .components()
            .all(|part| matches!(part, Component::RootDir | Component::Normal(_)))
}

pub(super) struct Launch {
    pub executable: Option<PathBuf>,
    pub cwd: PathBuf,
    pub arguments: Vec<String>,
}

#[cfg(target_os = "linux")]
pub(super) fn kernel(pid: u32) -> Result<(PathBuf, PathBuf), SquadError> {
    use std::os::unix::fs::MetadataExt;
    let root = PathBuf::from(format!("/proc/{pid}"));
    if fs::metadata(&root).map_err(fail)?.uid() != nix::unistd::geteuid().as_raw() {
        return Err(fail("Process owner changed."));
    }
    let raw = fs::read_link(root.join("exe")).map_err(fail)?;
    let raw = raw
        .to_str()
        .ok_or_else(|| fail("Non-text executable path."))?;
    Ok((
        PathBuf::from(raw.strip_suffix(" (deleted)").unwrap_or(raw)),
        fs::read_link(root.join("cwd")).map_err(fail)?,
    ))
}

#[cfg(target_os = "macos")]
pub(super) fn kernel(pid: u32) -> Result<(PathBuf, PathBuf), SquadError> {
    // txt is the kernel executable vnode path, including after unlink.
    // Neither a process name nor ps argv[0] establishes executable authority.
    let files = text(
        Path::new("/usr/sbin/lsof"),
        &["-a", "-p", &pid.to_string(), "-d", "txt,cwd", "-Fn"],
    )?;
    let mut descriptor = "";
    let mut executable = None;
    let mut cwd = None;
    for line in files.lines() {
        if let Some(value) = line.strip_prefix('f') {
            descriptor = value;
        }
        if let Some(value) = line.strip_prefix('n') {
            if descriptor == "cwd" {
                cwd = Some(PathBuf::from(value));
            }
            if descriptor == "txt" && executable.is_none() {
                executable = Some(PathBuf::from(
                    value.strip_suffix(" (deleted)").unwrap_or(value),
                ));
            }
        }
    }
    Ok((
        executable.ok_or_else(|| fail("Kernel reported no executable path."))?,
        cwd.ok_or_else(|| fail("Kernel reported no process cwd."))?,
    ))
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(super) fn kernel(_: u32) -> Result<(PathBuf, PathBuf), SquadError> {
    Err(fail("Board switching is unavailable on this platform."))
}

pub(super) fn launch(pid: u32, prefix: &Path) -> Result<Launch, SquadError> {
    read_launch(pid, prefix, false)
}

/// Only the switch's explicitly consented, independently verified lease holder
/// may use former argv without the old installation's executable namespace.
pub(super) fn lease_launch(pid: u32, prefix: &Path) -> Result<Launch, SquadError> {
    read_launch(pid, prefix, true)
}

fn read_launch(pid: u32, prefix: &Path, clock_holder: bool) -> Result<Launch, SquadError> {
    let (executable, cwd) = match kernel(pid) {
        Ok((executable, cwd)) => (Some(executable), cwd),
        Err(_) if clock_holder => (None, process_cwd(pid)?),
        Err(error) => return Err(error),
    };
    #[cfg(target_os = "linux")]
    let arguments = {
        use std::io::Read;
        let mut bytes = Vec::new();
        fs::File::open(format!("/proc/{pid}/cmdline"))
            .map_err(fail)?
            .take(128 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(fail)?;
        if bytes.len() > 128 * 1024 || bytes.pop() != Some(0) {
            return Err(fail("Unusable or excessive process arguments."));
        }
        let words = bytes
            .split(|b| *b == 0)
            .map(|part| String::from_utf8(part.to_vec()).map_err(fail))
            .collect::<Result<Vec<_>, _>>()?;
        let _ = prefix;
        let _ = clock_holder;
        words
    };
    #[cfg(target_os = "macos")]
    let arguments = {
        let raw = text(
            Path::new("/bin/ps"),
            &["-ww", "-p", &pid.to_string(), "-o", "args="],
        )?;
        if clock_holder {
            lease_mac_arguments(raw.trim_end_matches('\n'), executable.as_deref(), prefix)?
        } else {
            mac_arguments(
                raw.trim_end_matches('\n'),
                executable.as_deref().expect("installed executable"),
                prefix,
            )?
        }
    };
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let arguments = {
        let _ = prefix;
        return Err(fail("Unsupported process arguments."));
    };
    Ok(Launch {
        executable,
        cwd,
        arguments,
    })
}

fn process_cwd(pid: u32) -> Result<PathBuf, SquadError> {
    #[cfg(target_os = "linux")]
    {
        fs::read_link(format!("/proc/{pid}/cwd")).map_err(fail)
    }
    #[cfg(target_os = "macos")]
    {
        let files = text(
            Path::new("/usr/sbin/lsof"),
            &["-a", "-p", &pid.to_string(), "-d", "cwd", "-Fn"],
        )?;
        files
            .lines()
            .find_map(|line| line.strip_prefix('n'))
            .map(PathBuf::from)
            .ok_or_else(|| fail("Kernel reported no process cwd."))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = pid;
        Err(fail("No process cwd evidence on this platform."))
    }
}

#[cfg(any(target_os = "macos", test))]
fn mac_arguments(raw: &str, executable: &Path, prefix: &Path) -> Result<Vec<String>, SquadError> {
    let words = mac_words(raw, Some(executable), prefix, &["tmt-squad", "tmt-sq"])?;
    board_arguments(&words).ok_or_else(|| fail("Unrecognized former board arguments."))?;
    Ok(words)
}

#[cfg(any(target_os = "macos", test))]
fn lease_mac_arguments(
    raw: &str,
    executable: Option<&Path>,
    prefix: &Path,
) -> Result<Vec<String>, SquadError> {
    let words = mac_words(raw, executable, prefix, &["tmt-squad", "tmt-sq", "sq"])?;
    lease_board_arguments(&words).ok_or_else(|| fail("Unrecognized former lease-holder argv."))?;
    Ok(words)
}

#[cfg(any(target_os = "macos", test))]
fn mac_words(
    raw: &str,
    executable: Option<&Path>,
    prefix: &Path,
    names: &[&str],
) -> Result<Vec<String>, SquadError> {
    let mut programs: Vec<String> = executable
        .into_iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect();
    for &name in names {
        programs.push(prefix.join("bin").join(name).to_string_lossy().into_owned());
        programs.push(name.into());
    }
    if names.contains(&"sq") {
        // Lease-holder authority comes from the consented PID/pane incarnation,
        // not argv[0]'s path. Admit a named former program even after unlink.
        for command in [" ui", " board"] {
            if let Some((program, _)) = raw.split_once(command)
                && Path::new(program)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| names.contains(&name))
            {
                programs.push(program.into());
            }
        }
    }
    let (program, rest) = programs
        .iter()
        .find_map(|program| {
            raw.strip_prefix(program)
                .and_then(|tail| tail.strip_prefix(' '))
                .map(|tail| (program, tail))
        })
        .ok_or_else(|| fail("Unrecognized former board argv[0]."))?;
    // Values have the config's identifier grammar. No flattened text with
    // whitespace inside a value is accepted as evidence for retirement.
    if rest.contains(['\n', '\r', '\t', '\0']) || rest.split(' ').any(str::is_empty) {
        return Err(fail("Ambiguous former board arguments."));
    }
    let mut words = vec![program.clone()];
    words.extend(rest.split(' ').map(str::to_owned));
    Ok(words)
}

pub(super) fn lease_board_arguments(arguments: &[String]) -> Option<Vec<String>> {
    if !matches!(
        Path::new(arguments.first()?).file_name()?.to_str()?,
        "tmt-squad" | "tmt-sq" | "sq"
    ) || !matches!(arguments.get(1)?.as_str(), "board" | "ui")
    {
        return None;
    }
    let mut words = arguments.to_vec();
    words[1] = "board".into();
    board_arguments(&words)
}

pub(super) fn started_before_lease(pid: u32, since_ms: i64) -> Result<bool, SquadError> {
    // The original lease acquisition time never advances during renewal. A
    // later process start proves PID reuse; the captured start is also rechecked
    // through table() immediately before TERM. ps has second precision.
    let raw = text(
        Path::new("/usr/bin/env"),
        &[
            "TZ=UTC0",
            "LC_ALL=C",
            "/bin/ps",
            "-p",
            &pid.to_string(),
            "-o",
            "lstart=",
        ],
    )?;
    lease_start_precedes(raw.trim(), since_ms)
}

fn lease_start_precedes(raw: &str, since_ms: i64) -> Result<bool, SquadError> {
    let start = jiff::civil::DateTime::strptime("%a %b %e %T %Y", raw)
        .map_err(fail)?
        .to_zoned(jiff::tz::TimeZone::UTC)
        .map_err(fail)?
        .timestamp()
        .as_millisecond();
    Ok(start <= since_ms)
}

pub(super) fn shell(pid: u32) -> Result<bool, SquadError> {
    #[cfg(target_os = "linux")]
    let executable = fs::read_link(format!("/proc/{pid}/exe")).map_err(fail)?;
    #[cfg(target_os = "macos")]
    let executable = {
        let output = text(
            Path::new("/usr/sbin/lsof"),
            &["-a", "-p", &pid.to_string(), "-d", "txt", "-Fn"],
        )?;
        PathBuf::from(
            output
                .lines()
                .find_map(|line| line.strip_prefix('n'))
                .ok_or_else(|| fail("No parent shell executable evidence."))?,
        )
    };
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    return Err(fail(
        "No parent shell executable evidence on this platform.",
    ));
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let shells = std::fs::read_to_string("/etc/shells").map_err(fail)?;
        Ok(shells
            .lines()
            .filter(|line| !line.starts_with('#'))
            .any(|line| std::fs::canonicalize(line).is_ok_and(|shell| shell == executable)))
    }
}

pub(super) fn board_arguments(arguments: &[String]) -> Option<Vec<String>> {
    if arguments.get(1)? != "board" {
        return None;
    }
    let mut result = vec!["ui".into()];
    let mut seen = Vec::new();
    let mut index = 2;
    while index < arguments.len() {
        let flag = arguments[index].as_str();
        let (name, inline) = flag
            .split_once('=')
            .map_or((flag, None), |(a, b)| (a, Some(b)));
        if seen.contains(&name) {
            return None;
        }
        seen.push(name);
        match name {
            "--popup" if inline.is_none() => result.push(name.into()),
            "--tabs" | "--squad" => {
                let value = match inline {
                    Some(value) => value,
                    None => {
                        index += 1;
                        arguments.get(index)?
                    }
                };
                if value.is_empty()
                    || !value.bytes().all(|b| {
                        b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b',')
                    })
                {
                    return None;
                }
                result.extend([name.into(), value.into()]);
            }
            _ => return None,
        }
        index += 1;
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lease_holder_argv_admits_ui_without_relaxing_installed_board_provenance() {
        let prefix = Path::new("/owned");
        let outside = Path::new("/removed original/tmt-squad");
        assert!(!former(outside, prefix));
        for raw in [
            "tmt-squad ui --tabs product",
            "sq ui --squad product",
            "/removed original/tmt-squad ui --popup",
        ] {
            let words = lease_mac_arguments(raw, None, prefix).unwrap();
            assert!(lease_board_arguments(&words).is_some());
            assert!(mac_arguments(raw, outside, prefix).is_err());
        }
        for raw in [
            "user ui",
            "tmt-squad cron run",
            "tmt-squad ui --json",
            "tmt-squad ui --tabs product cron run",
        ] {
            assert!(lease_mac_arguments(raw, None, prefix).is_err(), "{raw}");
        }
    }

    #[test]
    fn a_process_born_after_the_original_lease_cannot_use_its_pid() {
        let since = "2026-10-09T01:00:00Z"
            .parse::<jiff::Timestamp>()
            .unwrap()
            .as_millisecond();
        assert!(lease_start_precedes("Fri Oct  9 00:59:59 2026", since).unwrap());
        assert!(lease_start_precedes("Fri Oct  9 01:00:00 2026", since).unwrap());
        assert!(!lease_start_precedes("Fri Oct  9 01:00:01 2026", since).unwrap());
        assert!(lease_start_precedes("unknown", since).is_err());
    }

    #[test]
    fn namespace_and_board_grammar_require_both_evidence_sources() {
        let prefix = Path::new("/owned prefix");
        let executable = prefix.join("lib/tmt-squad/releases/id/tmt-squad");
        assert!(former(&executable, prefix));
        for path in [
            "/other/lib/tmt-squad/releases/id/tmt-squad",
            "/owned prefix/lib/tmt-squad-other/tmt-squad",
            "/owned prefix/lib/tmt-squad/../user/tmt-squad",
        ] {
            assert!(!former(Path::new(path), prefix));
        }
        for program in [
            executable.to_str().unwrap(),
            "/owned prefix/bin/tmt-sq",
            "tmt-squad",
        ] {
            let words = mac_arguments(
                &format!("{program} board --tabs product,leads --squad product --popup"),
                &executable,
                prefix,
            )
            .unwrap();
            assert_eq!(
                board_arguments(&words).unwrap(),
                [
                    "ui",
                    "--tabs",
                    "product,leads",
                    "--squad",
                    "product",
                    "--popup"
                ]
            );
        }
    }
    #[test]
    fn flattened_mac_text_must_match_only_the_bounded_board_grammar() {
        let prefix = Path::new("/owned");
        let executable = prefix.join("lib/tmt-squad/releases/id/tmt-squad");
        for command in [
            "tmt-squad cron run",
            "tmt-squad ui",
            "tmt-squad board --json",
            "tmt-squad board --tabs",
            "tmt-squad board --tabs product, leads",
            "tmt-squad board --tabs product --tabs leads",
            "user board --tabs product",
            "tmt-squad board --tabs product cron run",
            "tmt-squad  board",
            "tmt-squad board --squad=UPPER",
            "tmt-squad board --popup=yes",
        ] {
            assert!(
                mac_arguments(command, &executable, prefix).is_err(),
                "{command}"
            );
        }
        assert_eq!(
            board_arguments(
                &mac_arguments("tmt-sq board --tabs=product,leads", &executable, prefix).unwrap()
            )
            .unwrap(),
            ["ui", "--tabs", "product,leads"]
        );
        assert!(
            board_arguments(&[
                "tmt-squad".into(),
                "board".into(),
                "--tabs".into(),
                "".into()
            ])
            .is_none()
        );
    }
}

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
    pub executable: PathBuf,
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
    let (executable, cwd) = kernel(pid)?;
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
        words
    };
    #[cfg(target_os = "macos")]
    let arguments = {
        let raw = text(
            Path::new("/bin/ps"),
            &["-ww", "-p", &pid.to_string(), "-o", "args="],
        )?;
        mac_arguments(raw.trim_end_matches('\n'), &executable, prefix)?
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

#[cfg(any(target_os = "macos", test))]
fn mac_arguments(raw: &str, executable: &Path, prefix: &Path) -> Result<Vec<String>, SquadError> {
    let mut programs = vec![executable.to_string_lossy().into_owned()];
    for name in ["tmt-squad", "tmt-sq"] {
        programs.push(prefix.join("bin").join(name).to_string_lossy().into_owned());
        programs.push(name.into());
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
    board_arguments(&words).ok_or_else(|| fail("Unrecognized former board arguments."))?;
    Ok(words)
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

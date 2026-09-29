//! `tmt squad hotkeys`: tmux prefix keys that open the board. The bindings
//! live in a squad-owned file; with consent, one owned `source-file` line is
//! added to the user's tmux configuration. Nothing else there is touched.

use crate::{
    config::{Config, TmuxKeys},
    consent::{Consent, ask},
    core::{Core, SquadError},
    effects, runner,
};
use serde_json::{Value, json};
use std::{
    ffi::OsString,
    fs,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// Notes that mark squad's own bindings in `list-keys`.
const NOTE: &str = "tmt squad";
const MARK: &str = "# tmt squad hotkeys";

fn failed(code: &str, message: impl Into<String>) -> SquadError {
    SquadError::new(code, message)
}

/// The stable command to record: the first `tmt` on PATH that resolves to
/// the running executable (for example `~/.local/bin/tmt`), so an upgrade
/// that moves the release does not strand the bindings.
pub fn launcher(executable: &Path, search: Option<OsString>) -> PathBuf {
    let Ok(target) = fs::canonicalize(executable) else {
        return executable.to_path_buf();
    };
    search
        .iter()
        .flat_map(std::env::split_paths)
        .map(|directory| directory.join("tmt"))
        .find(|candidate| fs::canonicalize(candidate).is_ok_and(|resolved| resolved == target))
        .unwrap_or_else(|| executable.to_path_buf())
}

/// A path as one single-quoted shell and tmux word. Paths that would need
/// escaping (quotes, `#`, `$`, `\`, control characters) are refused.
fn quoted(path: &Path) -> Result<String, SquadError> {
    let text = path
        .to_str()
        .filter(|text| {
            path.is_absolute()
                && !text
                    .chars()
                    .any(|c| c.is_control() || "'\"#$\\".contains(c))
        })
        .ok_or_else(|| {
            failed(
                "SQUAD_ACTION_REFUSED",
                format!(
                    "{} cannot be written into tmux configuration safely; use a plain absolute path.",
                    path.display()
                ),
            )
        })?;
    Ok(format!("'{text}'"))
}

/// The squad-owned tmux file: every line is generated.
pub fn bindings(keys: &TmuxKeys, launcher: &Path) -> Result<String, SquadError> {
    let tmt = quoted(launcher)?;
    let mut text = format!(
        "{MARK}: generated; `tmt squad hotkeys install` replaces this file.\n\
         bind-key -N \"{NOTE} popup\" {popup} display-popup -E -w 90% -h 85% \"exec {tmt} squad board --popup\"\n\
         bind-key -N \"{NOTE} pane\" {pane} split-window -h \"exec {tmt} squad board\"\n",
        popup = keys.popup,
        pane = keys.pane,
    );
    if let Some(back) = &keys.back {
        text.push_str(&format!(
            "bind-key -N \"{NOTE} back\" {back} run-shell \"{tmt} squad back\"\n"
        ));
    }
    Ok(text)
}

/// The one line squad owns in the user's tmux configuration.
pub fn owned_line(file: &Path) -> Result<String, SquadError> {
    Ok(format!("source-file -q {} {MARK}", quoted(file)?))
}

/// The files tmux 3.x loads, in its order (observed with 3.7: all that exist).
pub fn candidates(home: &Path, xdg: Option<&Path>) -> Vec<PathBuf> {
    let mut paths = vec![home.join(".tmux.conf")];
    if let Some(xdg) = xdg.filter(|xdg| xdg.is_absolute()) {
        paths.push(xdg.join("tmux/tmux.conf"));
    }
    let config = home.join(".config/tmux/tmux.conf");
    if !paths.contains(&config) {
        paths.push(config);
    }
    paths
}

fn contains_line(text: &str, line: &str) -> bool {
    text.lines().any(|existing| existing == line)
}

/// Appends the line once, keeping every existing byte.
fn with_line(text: &str, line: &str) -> String {
    if contains_line(text, line) {
        return text.to_owned();
    }
    let mut out = text.to_owned();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(line);
    out.push('\n');
    out
}

/// Removes exactly the owned line, keeping every other byte.
fn without_line(text: &str, line: &str) -> String {
    text.split_inclusive('\n')
        .filter(|existing| existing.trim_end_matches(['\n', '\r']) != line)
        .collect()
}

/// Prefix-table keys a configuration text binds (`bind`/`bind-key`).
fn file_keys(text: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    for line in text.lines() {
        let mut words = line.split_whitespace();
        if !matches!(words.next(), Some("bind" | "bind-key")) {
            continue;
        }
        let mut table = "prefix".to_owned();
        while let Some(word) = words.next() {
            match word {
                "-T" => table = words.next().unwrap_or_default().to_owned(),
                "-N" => {
                    // A quoted note may span words.
                    let note = words.next().unwrap_or_default();
                    if note.starts_with('"') && !(note.len() > 1 && note.ends_with('"')) {
                        for rest in words.by_ref() {
                            if rest.ends_with('"') {
                                break;
                            }
                        }
                    }
                }
                "-n" => table = "root".into(),
                flag if flag.starts_with('-') && flag.len() > 1 => {}
                key => {
                    if table == "prefix" {
                        found.push((key.trim_matches('"').to_owned(), line.trim().to_owned()));
                    }
                    break;
                }
            }
        }
    }
    found
}

/// Parses `list-keys -N -P "" -T prefix` (key, note) and `list-keys -T
/// prefix` (key, command) into `(key, note, command)`. `list-keys -F` is
/// newer than tmux 3.2, so only these forms are used (3.3a and 3.7 checked;
/// 3.7 indents the note listing by one space).
fn parse_keys(notes: &str, commands: &str) -> Vec<(String, String, String)> {
    let notes: std::collections::BTreeMap<&str, &str> = notes
        .lines()
        .filter_map(|line| {
            let (key, note) = line.trim_start().split_once(char::is_whitespace)?;
            Some((key, note.trim()))
        })
        .collect();
    commands
        .lines()
        .filter_map(|line| {
            let (_, bound) = line.split_once("-T prefix")?;
            let (key, command) = bound.trim_start().split_once(char::is_whitespace)?;
            Some((
                key.to_owned(),
                notes.get(key).copied().unwrap_or_default().to_owned(),
                command.trim().to_owned(),
            ))
        })
        .collect()
}

/// Prefix bindings on the running server: `(key, note, command)`.
fn server_keys(socket: &str) -> Result<Vec<(String, String, String)>, SquadError> {
    let notes = tmux(socket, &["list-keys", "-N", "-P", "", "-T", "prefix"])?;
    let commands = tmux(socket, &["list-keys", "-T", "prefix"])?;
    Ok(parse_keys(&notes, &commands))
}

fn tmux(socket: &str, args: &[&str]) -> Result<String, SquadError> {
    let mut argv: Vec<OsString> = vec!["-S".into(), socket.into()];
    argv.extend(args.iter().map(OsString::from));
    let finished = runner::run(
        Path::new("tmux"),
        &argv,
        b"",
        Duration::from_secs(5),
        1024 * 1024,
    )
    .map_err(|_| failed("SQUAD_ACTION_FAILED", "tmux did not respond."))?;
    if !finished.success {
        return Err(failed(
            "SQUAD_ACTION_FAILED",
            format!("tmux {} failed.", args.join(" ")),
        ));
    }
    Ok(String::from_utf8_lossy(&finished.stdout).into_owned())
}

fn chosen(keys: &TmuxKeys) -> Vec<&str> {
    let mut all = vec![keys.popup.as_str(), keys.pane.as_str()];
    all.extend(keys.back.as_deref());
    all
}

/// Existing bindings for the chosen keys that are not squad's.
fn collisions(
    keys: &TmuxKeys,
    files: &[(PathBuf, String)],
    server: &[(String, String, String)],
) -> Vec<String> {
    let mut found = Vec::new();
    for key in chosen(keys) {
        for (path, text) in files {
            for (bound, line) in file_keys(text) {
                if bound == key {
                    found.push(format!("prefix {key} in {}: {line}", path.display()));
                }
            }
        }
        for (bound, note, command) in server {
            if bound == key && !note.starts_with(NOTE) {
                found.push(format!("prefix {key} on the running server: {command}"));
            }
        }
    }
    found
}

fn read(path: &Path) -> Result<Option<Vec<u8>>, SquadError> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(failed(
            "SQUAD_ACTION_FAILED",
            format!("Could not read {}: {error}", path.display()),
        )),
    }
}

/// Symbolic links followed before a target is refused as a loop.
const LINK_HOPS: usize = 8;

/// The real file behind `path`, following links (dotfile managers link
/// `~/.tmux.conf` into a repository). A missing plain path is itself; a link
/// whose target is missing, or a chain of more than [`LINK_HOPS`], is refused.
fn real_path(path: &Path) -> Result<PathBuf, SquadError> {
    let mut current = path.to_path_buf();
    for _ in 0..=LINK_HOPS {
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                let target = fs::read_link(&current).map_err(|error| {
                    failed(
                        "SQUAD_ACTION_FAILED",
                        format!("Could not read the link {}: {error}", current.display()),
                    )
                })?;
                // A relative target is relative to the link's real directory.
                current = match current.parent() {
                    Some(parent) if target.is_relative() => fs::canonicalize(parent)
                        .unwrap_or_else(|_| parent.to_path_buf())
                        .join(target),
                    _ => target,
                };
            }
            Ok(_) => return Ok(current),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && current == path => {
                return Ok(current);
            }
            Err(_) => {
                return Err(failed(
                    "SQUAD_ACTION_REFUSED",
                    format!(
                        "{} is a link to {}, which does not exist; nothing was written. Fix the link, or add the line from --print yourself.",
                        path.display(),
                        current.display()
                    ),
                ));
            }
        }
    }
    Err(failed(
        "SQUAD_ACTION_REFUSED",
        format!(
            "{} follows more than {LINK_HOPS} links; nothing was written. Add the line from --print yourself.",
            path.display()
        ),
    ))
}

/// Replaces the real file behind `path` with `text` only if it still holds
/// `expected` (the bytes the plan showed). A link stays a link: the backup,
/// the temporary file and the rename all happen beside the real file. An
/// existing file is first copied byte for byte to a backup; the replacement
/// is atomic and keeps the file's mode.
fn publish(
    path: &Path,
    expected: Option<&[u8]>,
    text: &str,
) -> Result<Option<PathBuf>, SquadError> {
    let path = &real_path(path)?;
    let io = |error: std::io::Error| {
        failed(
            "SQUAD_ACTION_FAILED",
            format!(
                "Could not write {}: {error}. Nothing else was changed; add the line from --print yourself.",
                path.display()
            ),
        )
    };
    if read(path)?.as_deref() != expected {
        return Err(failed(
            "SQUAD_CONFIG_CHANGED",
            format!(
                "{} changed after the plan was shown; nothing was written. Run the command again.",
                path.display()
            ),
        ));
    }
    let directory = path
        .parent()
        .ok_or_else(|| io(std::io::Error::other("no directory")))?;
    fs::create_dir_all(directory).map_err(io)?;
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let backup = match expected {
        Some(original) => {
            let millis = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_millis());
            let backup = directory.join(format!("{name}.tmt-squad-backup-{millis}"));
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&backup)
                .and_then(|mut file| file.write_all(original))
                .map_err(io)?;
            Some(backup)
        }
        None => None,
    };
    let mode = fs::metadata(path).map_or(0o644, |metadata| metadata.permissions().mode());
    let temporary = directory.join(format!(".{name}.tmt-squad-{}", std::process::id()));
    let written = fs::write(&temporary, text)
        .and_then(|()| fs::set_permissions(&temporary, fs::Permissions::from_mode(mode)))
        .and_then(|()| fs::rename(&temporary, path));
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written.map_err(io)?;
    Ok(backup)
}

/// Everything a command needs to plan: the files, the keys and the texts.
struct Plan {
    keys: TmuxKeys,
    squad_file: PathBuf,
    bindings: String,
    line: String,
    candidates: Vec<(PathBuf, Option<Vec<u8>>)>,
    target: PathBuf,
    /// The real file behind `target`, or why it cannot be written.
    resolved: Result<PathBuf, SquadError>,
    socket: Option<String>,
}

fn plan(core: &Core, config: &Config, explicit: Option<&Path>) -> Result<Plan, SquadError> {
    let keys = config.tmux_keys()?;
    let squad_file = config
        .path()
        .parent()
        .ok_or_else(|| failed("SQUAD_CONFIG_INVALID", "squad.toml has no directory."))?
        .join("squad.tmux.conf");
    let tmt = launcher(core.executable(), std::env::var_os("PATH"));
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|home| home.is_absolute())
        .ok_or_else(|| failed("SQUAD_ACTION_REFUSED", "HOME must be an absolute path."))?;
    let xdg = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    let mut paths = candidates(&home, xdg.as_deref());
    if let Some(explicit) = explicit {
        if !explicit.is_absolute() {
            return Err(failed(
                "SQUAD_ACTION_REFUSED",
                "--config must be an absolute path.",
            ));
        }
        paths.retain(|path| path != explicit);
        paths.insert(0, explicit.to_path_buf());
    }
    let candidates = paths
        .into_iter()
        .map(|path| Ok((path.clone(), read(&path)?)))
        .collect::<Result<Vec<_>, SquadError>>()?;
    let target = match explicit {
        Some(explicit) => explicit.to_path_buf(),
        None => candidates
            .iter()
            .find(|(_, bytes)| bytes.is_some())
            .map_or_else(|| home.join(".tmux.conf"), |(path, _)| path.clone()),
    };
    Ok(Plan {
        bindings: bindings(&keys, &tmt)?,
        line: owned_line(&squad_file)?,
        keys,
        squad_file,
        candidates,
        resolved: real_path(&target),
        target,
        socket: effects::tmux_socket(),
    })
}

fn text(bytes: &Option<Vec<u8>>) -> String {
    bytes
        .as_deref()
        .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
        .unwrap_or_default()
}

fn keys_json(keys: &TmuxKeys) -> Value {
    json!({"popup": keys.popup, "pane": keys.pane, "back": keys.back})
}

/// `install [--print] [--yes] [--config <path>]`.
pub fn install(
    core: &Core,
    config: &Config,
    explicit: Option<&Path>,
    print: bool,
    consent: Consent,
) -> Result<Value, SquadError> {
    let plan = plan(core, config, explicit)?;
    let original = plan
        .candidates
        .iter()
        .find(|(path, _)| *path == plan.target)
        .and_then(|(_, bytes)| bytes.clone());
    let current = text(&original);
    let files: Vec<(PathBuf, String)> = plan
        .candidates
        .iter()
        .filter_map(|(path, bytes)| Some((path.clone(), text(&Some(bytes.clone()?)))))
        .collect();
    let server = match &plan.socket {
        Some(socket) => server_keys(socket)?,
        None => Vec::new(),
    };
    let collisions = collisions(&plan.keys, &files, &server);
    let installed = contains_line(&current, &plan.line)
        && read(&plan.squad_file)?.as_deref() == Some(plan.bindings.as_bytes());
    let document = json!({
        "target": plan.target,
        "resolved": plan.resolved.as_ref().ok(),
        "creates": original.is_none(),
        "line": plan.line,
        "squadFile": plan.squad_file,
        "bindings": plan.bindings,
        "keys": keys_json(&plan.keys),
        "loadsRunningServer": plan.socket.is_some(),
        "collisions": collisions,
    });
    if print {
        return Ok(document);
    }
    if !collisions.is_empty() {
        return Err(taken(&collisions));
    }
    if let Err(error) = &plan.resolved {
        return Err(error.clone());
    }
    if installed {
        let mut document = document;
        document["installed"] = json!(true);
        document["changed"] = json!(false);
        return Ok(document);
    }
    let summary = format!(
        "tmt squad hotkeys will:\n  write {} (squad's bindings: prefix {} popup, prefix {} pane{})\n  {} {} with the line:\n    {}{}",
        plan.squad_file.display(),
        plan.keys.popup,
        plan.keys.pane,
        plan.keys
            .back
            .as_ref()
            .map_or(String::new(), |key| format!(", prefix {key} back")),
        if original.is_some() {
            "add to"
        } else {
            "create"
        },
        match &plan.resolved {
            Ok(real) if *real != plan.target => {
                format!(
                    "{} (a link; the real file {} is edited)",
                    plan.target.display(),
                    real.display()
                )
            }
            _ => plan.target.display().to_string(),
        },
        plan.line,
        if plan.socket.is_some() {
            "\n  load the bindings into the running tmux server"
        } else {
            ""
        },
    );
    ask(consent, &summary, "Install these tmux hotkeys?")?;
    // The user's file first: if it cannot be written, nothing changed. Its
    // line is quiet (`-q`), so squad's own file may follow.
    let backup = publish(
        &plan.target,
        original.as_deref(),
        &with_line(&current, &plan.line),
    )?;
    let squad_before = read(&plan.squad_file)?;
    publish(&plan.squad_file, squad_before.as_deref(), &plan.bindings)?;
    if let Some(socket) = &plan.socket {
        let squad_file = plan.squad_file.to_string_lossy().into_owned();
        tmux(socket, &["source-file", &squad_file])?;
    }
    let mut document = document;
    document["installed"] = json!(true);
    document["changed"] = json!(true);
    document["backup"] = json!(backup);
    Ok(document)
}

/// `remove [--yes]`: the owned line from every file that has it, and the
/// running server's keys that are still squad's own.
pub fn remove(core: &Core, config: &Config, consent: Consent) -> Result<Value, SquadError> {
    let plan = plan(core, config, None)?;
    let owners: Vec<(PathBuf, Vec<u8>)> = plan
        .candidates
        .iter()
        .filter_map(|(path, bytes)| {
            let bytes = bytes.clone()?;
            contains_line(&text(&Some(bytes.clone())), &plan.line).then(|| (path.clone(), bytes))
        })
        .collect();
    let server = match &plan.socket {
        Some(socket) => server_keys(socket)?,
        None => Vec::new(),
    };
    let ours: Vec<String> = server
        .iter()
        .filter(|(_, note, _)| note.starts_with(NOTE))
        .map(|(key, _, _)| key.clone())
        .collect();
    if owners.is_empty() && ours.is_empty() {
        return Ok(json!({"removed": [], "unbound": [], "changed": false}));
    }
    let summary = format!(
        "tmt squad hotkeys will remove the line\n    {}\n  from {}{}",
        plan.line,
        owners
            .iter()
            .map(|(path, _)| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", "),
        if ours.is_empty() {
            String::new()
        } else {
            format!(
                "\n  and unbind prefix {} on the running server",
                ours.join(", ")
            )
        }
    );
    ask(consent, &summary, "Remove these tmux hotkeys?")?;
    let mut backups = Vec::new();
    for (path, bytes) in &owners {
        let kept = without_line(&text(&Some(bytes.clone())), &plan.line);
        backups.push(publish(path, Some(bytes), &kept)?);
    }
    if let Some(socket) = &plan.socket {
        for key in &ours {
            tmux(socket, &["unbind-key", "-T", "prefix", key])?;
        }
    }
    Ok(json!({
        "removed": owners.iter().map(|(path, _)| path).collect::<Vec<_>>(),
        "backups": backups,
        "unbound": ours,
        "changed": true,
    }))
}

/// The executable a generated file records, if any.
fn recorded(bindings: &str) -> Option<PathBuf> {
    let start = bindings.find("exec '")? + "exec '".len();
    let end = bindings[start..].find('\'')? + start;
    Some(PathBuf::from(&bindings[start..end]))
}

/// `show`: read-only status.
pub fn report(core: &Core, config: &Config) -> Result<Value, SquadError> {
    let plan = plan(core, config, None)?;
    let installed_in: Vec<&PathBuf> = plan
        .candidates
        .iter()
        .filter(|(_, bytes)| contains_line(&text(bytes), &plan.line))
        .map(|(path, _)| path)
        .collect();
    let written = read(&plan.squad_file)?.map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
    let executable = written.as_deref().and_then(recorded);
    let server = match &plan.socket {
        Some(socket) => server_keys(socket)?,
        None => Vec::new(),
    };
    Ok(json!({
        "installed": !installed_in.is_empty(),
        "sourcedFrom": installed_in,
        "squadFile": plan.squad_file,
        "keys": keys_json(&plan.keys),
        "current": written.as_deref() == Some(plan.bindings.as_str()),
        "executable": executable,
        "executableExists": executable.as_ref().map(|path| path.exists()),
        "serverKeys": server
            .iter()
            .filter(|(_, note, _)| note.starts_with(NOTE))
            .map(|(key, _, _)| key)
            .collect::<Vec<_>>(),
    }))
}

/// Keys another binding already uses, and where to choose others.
fn taken(collisions: &[String]) -> SquadError {
    SquadError::hinted(
        "SQUAD_HOTKEY_TAKEN",
        &format!("Nothing was changed: {}.", collisions.join("; ")),
        " ",
        "Choose other keys under [tmux] in squad.toml.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn taken_keys_keep_their_json_message_and_split_the_next_step() {
        let error = taken(&[
            "prefix S is bound to x".into(),
            "prefix B is bound to y".into(),
        ]);
        assert_eq!(
            error.to_json().to_string(),
            r#"{"error":{"code":"SQUAD_HOTKEY_TAKEN","message":"Nothing was changed: prefix S is bound to x; prefix B is bound to y. Choose other keys under [tmux] in squad.toml."}}"#
        );
        assert_eq!(
            error.human().1,
            Some("Choose other keys under [tmux] in squad.toml.")
        );
    }

    fn keys() -> TmuxKeys {
        TmuxKeys {
            popup: "S".into(),
            pane: "B".into(),
            back: None,
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("tmt-squad-hotkeys-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn the_stable_launcher_is_recorded_not_the_release_it_resolves_to() {
        let root = scratch("launcher");
        let release = root.join("releases/abc123/bin");
        fs::create_dir_all(&release).unwrap();
        fs::write(release.join("tmt"), "").unwrap();
        let bin = root.join("bin");
        fs::create_dir_all(&bin).unwrap();
        std::os::unix::fs::symlink(release.join("tmt"), bin.join("tmt")).unwrap();
        let other = root.join("other");
        fs::create_dir_all(&other).unwrap();
        fs::write(other.join("tmt"), "").unwrap();
        let path = std::env::join_paths([&other, &bin]).unwrap();
        assert_eq!(launcher(&release.join("tmt"), Some(path)), bin.join("tmt"));
        assert_eq!(
            launcher(&release.join("tmt"), None),
            release.join("tmt"),
            "without a matching PATH entry the executable itself is used"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn generated_bindings_carry_notes_and_one_quoted_launcher() {
        let text = bindings(&keys(), Path::new("/Users/me/.local/bin/tmt")).unwrap();
        assert_eq!(
            text.lines().skip(1).collect::<Vec<_>>(),
            [
                "bind-key -N \"tmt squad popup\" S display-popup -E -w 90% -h 85% \"exec '/Users/me/.local/bin/tmt' squad board --popup\"",
                "bind-key -N \"tmt squad pane\" B split-window -h \"exec '/Users/me/.local/bin/tmt' squad board\"",
            ]
        );
        let with_back = bindings(
            &TmuxKeys {
                back: Some("b".into()),
                ..keys()
            },
            Path::new("/x/tmt"),
        )
        .unwrap();
        assert!(
            with_back
                .ends_with("bind-key -N \"tmt squad back\" b run-shell \"'/x/tmt' squad back\"\n")
        );
        assert_eq!(
            recorded(&text),
            Some(PathBuf::from("/Users/me/.local/bin/tmt"))
        );
        for bad in [
            "relative/tmt",
            "/a'b/tmt",
            "/a#b/tmt",
            "/a$b/tmt",
            "/a\"b/tmt",
            "/a\nb",
        ] {
            assert!(bindings(&keys(), Path::new(bad)).is_err(), "{bad}");
        }
        assert_eq!(
            owned_line(Path::new("/c/tmux-team/squad.tmux.conf")).unwrap(),
            "source-file -q '/c/tmux-team/squad.tmux.conf' # tmt squad hotkeys"
        );
    }

    #[test]
    fn the_owned_line_is_added_once_and_removed_alone() {
        let line = "source-file -q '/c/squad.tmux.conf' # tmt squad hotkeys";
        let original = "set -g mouse on\n# keep me\nbind r source ~/.tmux.conf";
        let added = with_line(original, line);
        assert_eq!(added, format!("{original}\n{line}\n"));
        assert_eq!(with_line(&added, line), added, "idempotent");
        assert_eq!(without_line(&added, line), format!("{original}\n"));
        assert_eq!(with_line("", line), format!("{line}\n"));
        let similar = format!("{line} extra\n# {line}\n");
        assert_eq!(without_line(&similar, line), similar, "only the exact line");
    }

    #[test]
    fn tmux_loads_home_then_xdg_then_dot_config() {
        let home = Path::new("/h");
        assert_eq!(
            candidates(home, Some(Path::new("/x"))),
            [
                PathBuf::from("/h/.tmux.conf"),
                PathBuf::from("/x/tmux/tmux.conf"),
                PathBuf::from("/h/.config/tmux/tmux.conf")
            ]
        );
        assert_eq!(candidates(home, Some(Path::new("/h/.config"))).len(), 2);
        assert_eq!(candidates(home, Some(Path::new("relative"))).len(), 2);
    }

    #[test]
    fn collisions_come_from_files_and_foreign_server_bindings_only() {
        let files = vec![(
            PathBuf::from("/h/.tmux.conf"),
            "bind S choose-session\nbind -n B send-keys x\nbind-key -T copy-mode S x\nbind-key -r -N \"my note\" B resize-pane -L\n".to_owned(),
        )];
        let server = vec![
            (
                "S".to_owned(),
                "tmt squad popup".to_owned(),
                "display-popup …".to_owned(),
            ),
            ("B".to_owned(), String::new(), "split-window".to_owned()),
            ("p".to_owned(), String::new(), "previous-window".to_owned()),
        ];
        let found = collisions(&keys(), &files, &server);
        assert_eq!(
            found,
            [
                "prefix S in /h/.tmux.conf: bind S choose-session",
                "prefix B in /h/.tmux.conf: bind-key -r -N \"my note\" B resize-pane -L",
                "prefix B on the running server: split-window",
            ]
        );
    }

    #[test]
    fn a_linked_configuration_stays_a_link_and_dangling_or_looping_links_refuse() {
        use std::os::unix::fs::symlink;
        let root = scratch("links");
        let dotfiles = root.join("dotfiles");
        fs::create_dir_all(&dotfiles).unwrap();
        let real = dotfiles.join("tmux.conf");
        fs::write(&real, "set -g mouse on\n").unwrap();
        let home = root.join("home");
        fs::create_dir_all(&home).unwrap();
        let link = home.join(".tmux.conf");
        // A relative link through a second link, as dotfile managers make.
        symlink("dotfiles/tmux.conf", root.join("hop")).unwrap();
        symlink("../hop", &link).unwrap();
        let canonical = |path: &Path| fs::canonicalize(path).unwrap();
        assert_eq!(canonical(&real_path(&link).unwrap()), canonical(&real));
        assert_eq!(real_path(&real).unwrap(), real);
        assert_eq!(
            real_path(&home.join("absent")).unwrap(),
            home.join("absent")
        );

        let backup = publish(&link, Some(b"set -g mouse on\n"), "set -g mouse on\nline\n")
            .unwrap()
            .unwrap();
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink(),
            "still a link"
        );
        assert_eq!(
            fs::read_to_string(&real).unwrap(),
            "set -g mouse on\nline\n"
        );
        assert_eq!(
            canonical(backup.parent().unwrap()),
            canonical(&dotfiles),
            "backup beside the real file"
        );
        assert_eq!(
            fs::read_dir(&home).unwrap().count(),
            1,
            "nothing written beside the link"
        );

        let dangling = home.join("dangling.conf");
        symlink(root.join("missing.conf"), &dangling).unwrap();
        let refused = publish(&dangling, None, "line\n").unwrap_err();
        assert_eq!(refused.code, "SQUAD_ACTION_REFUSED");
        assert!(refused.message.contains("--print"), "{}", refused.message);
        assert!(
            fs::symlink_metadata(&dangling)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(!root.join("missing.conf").exists(), "nothing was created");

        symlink(root.join("loop-b"), root.join("loop-a")).unwrap();
        symlink(root.join("loop-a"), root.join("loop-b")).unwrap();
        assert_eq!(
            real_path(&root.join("loop-a")).unwrap_err().code,
            "SQUAD_ACTION_REFUSED"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn an_unwritable_directory_refuses_with_guidance_and_changes_nothing() {
        let root = scratch("readonly");
        let path = root.join(".tmux.conf");
        fs::write(&path, "keep\n").unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o555)).unwrap();
        let error = publish(&path, Some(b"keep\n"), "keep\nline\n").unwrap_err();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(error.code, "SQUAD_ACTION_FAILED");
        assert!(error.message.contains("--print"), "{}", error.message);
        assert_eq!(fs::read_to_string(&path).unwrap(), "keep\n");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn server_listings_from_tmux_3_3_and_3_7_parse_alike() {
        let commands = "bind-key -r -T prefix B       resize-pane -L\n\
            bind-key    -T prefix S       display-popup -E -h \"85%\" -w \"90%\" \"exec '/x y/tmt' squad board --popup\"\n\
            bind-key    -T prefix p       previous-window\n\
            bind-key    -T root   S       send-keys x\n";
        for notes in [
            "S       tmt squad popup\np       Select the previous window\n",
            " S       tmt squad popup\n p       Select the previous window\n",
        ] {
            assert_eq!(
                parse_keys(notes, commands),
                [
                    ("B".into(), String::new(), "resize-pane -L".into()),
                    (
                        "S".into(),
                        "tmt squad popup".into(),
                        "display-popup -E -h \"85%\" -w \"90%\" \"exec '/x y/tmt' squad board --popup\"".into()
                    ),
                    ("p".into(), "Select the previous window".into(), "previous-window".into()),
                ]
            );
        }
    }

    #[test]
    fn publication_rechecks_backs_up_byte_for_byte_and_keeps_the_mode() {
        let root = scratch("publish");
        let path = root.join(".tmux.conf");
        let original = b"set -g mouse on\r\n\xff raw\n".to_vec();
        fs::write(&path, &original).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let backup = publish(&path, Some(&original), "new\n").unwrap().unwrap();
        assert_eq!(fs::read(&backup).unwrap(), original);
        assert_eq!(fs::read_to_string(&path).unwrap(), "new\n");
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );

        let changed = publish(&path, Some(&original), "other\n").unwrap_err();
        assert_eq!(changed.code, "SQUAD_CONFIG_CHANGED");
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "new\n",
            "nothing written"
        );

        let created = root.join("fresh/.tmux.conf");
        assert_eq!(
            publish(&created, None, "line\n").unwrap(),
            None,
            "no backup of nothing"
        );
        assert_eq!(fs::read_to_string(&created).unwrap(), "line\n");
        let leftovers = fs::read_dir(&root)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                name.contains(".tmt-squad-") && !name.contains("backup")
            })
            .count();
        assert_eq!(leftovers, 0, "no temporary files remain");
        let _ = fs::remove_dir_all(root);
    }
}

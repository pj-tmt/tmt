//! One Ops-owned former-board switch, shared by replacement and command startup.

mod process;
mod record;

use crate::{
    consent::{self, Consent},
    core::{Core, SquadError},
    effects, migration, runner,
};
use clap::{Arg, ArgAction, ArgMatches, Command};
use nix::{
    sys::signal::{Signal, kill},
    unistd::Pid,
};
use serde_json::{Value, json};
use std::{
    ffi::OsString,
    fs,
    io::Write,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};
use tmt_cli_style::{CommandSpec, Example, Interaction, Mode, OutputModes};

const SPEC: &CommandSpec = &CommandSpec {
    name: "migration",
    summary: "Complete the Squad to Ops board migration",
    examples: &[Example {
        command: "tmt ops migration switch --yes",
        note: "Switch former boards on this tmux server",
    }],
    outputs: OutputModes::Human,
    details: "",
};
const SWITCH: &CommandSpec = &CommandSpec {
    name: "switch",
    summary: "Switch verified former boards to Ops in their existing panes",
    examples: SPEC.examples,
    outputs: OutputModes::Human,
    details: "Only former board processes under the verified install prefix are stopped. Other panes and processes stay running.",
};

pub(crate) fn grammar() -> Command {
    tmt_cli_style::command(SPEC)
        .subcommand_required(true)
        .subcommand(
            tmt_cli_style::command(SWITCH)
                .arg(
                    Arg::new("yes")
                        .long("yes")
                        .action(ArgAction::SetTrue)
                        .help("Consent without a prompt"),
                )
                .arg(
                    Arg::new("prefix")
                        .long("prefix")
                        .value_name("PATH")
                        .help("The current native installation prefix"),
                )
                .arg(
                    Arg::new("socket")
                        .long("socket")
                        .value_name("PATH")
                        .help("The tmux server socket to switch"),
                ),
        )
}

fn fail(error: impl std::fmt::Display) -> SquadError {
    SquadError::new("OPS_BOARD_SWITCH_FAILED", error.to_string())
}

fn text(program: &Path, arguments: &[&str]) -> Result<String, SquadError> {
    let arguments: Vec<OsString> = arguments.iter().map(OsString::from).collect();
    let output = runner::run(program, &arguments, b"", Duration::from_secs(3), 256 * 1024)
        .map_err(|_| fail("Could not read bounded process/tmux evidence."))?;
    if !output.success {
        return Err(fail("Process/tmux evidence is unavailable."));
    }
    String::from_utf8(output.stdout).map_err(fail)
}

fn tmux(socket: &str, arguments: &[&str]) -> Result<String, SquadError> {
    let mut args = vec!["-S", socket];
    args.extend_from_slice(arguments);
    text(Path::new("tmux"), &args)
}

#[derive(Clone)]
struct Pane {
    id: String,
    pid: u32,
    tty: String,
    dead: bool,
}

fn panes(socket: &str) -> Result<Vec<Pane>, SquadError> {
    let output = tmux(
        socket,
        &[
            "list-panes",
            "-a",
            "-F",
            "#{pane_id}\t#{pane_pid}\t#{pane_tty}\t#{pane_dead}",
        ],
    )?;
    let rows: Vec<_> = output
        .lines()
        .filter_map(|line| {
            let fields: Vec<_> = line.split('\t').collect();
            if fields.len() != 4
                || !fields[0]
                    .strip_prefix('%')?
                    .bytes()
                    .all(|b| b.is_ascii_digit())
            {
                return None;
            }
            Some(Pane {
                id: fields[0].into(),
                pid: fields[1].parse().ok()?,
                tty: fields[2].into(),
                dead: fields[3] == "1",
            })
        })
        .collect();
    if rows.len() > 256 {
        return Err(fail("More than 256 panes exceed the switch bound."));
    }
    Ok(rows)
}

fn installed_prefix() -> Option<PathBuf> {
    let executable = fs::canonicalize(std::env::current_exe().ok()?).ok()?;
    let prefix = executable.ancestors().nth(5)?;
    executable
        .starts_with(prefix.join("lib/tmt-ops/releases"))
        .then(|| prefix.to_owned())
}

fn verify_prefix(core: &Core, selected: Option<&str>) -> Result<PathBuf, SquadError> {
    let prefix = installed_prefix()
        .ok_or_else(|| fail("Run the current managed Ops installation to switch boards."))?;
    if let Some(selected) = selected
        && fs::canonicalize(selected).map_err(fail)? != prefix
    {
        return Err(fail("The selected prefix is not this Ops installation."));
    }
    let active = fs::canonicalize(prefix.join("lib/tmt-ops/current/tmt-ops")).map_err(fail)?;
    if active != fs::canonicalize(std::env::current_exe().map_err(fail)?).map_err(fail)? {
        return Err(fail("This is not the current Ops executable."));
    }
    let shown = core.json(&[
        "extension",
        "ls",
        "--prefix",
        prefix
            .to_str()
            .ok_or_else(|| fail("Non-text install prefix."))?,
    ])?;
    if !shown["extensions"].as_array().is_some_and(|rows| {
        rows.iter().any(|row| {
            row["name"] == "ops" && row["installed"] == true && row.get("status").is_none()
        })
    }) {
        return Err(fail("Core could not verify this Ops installation."));
    }
    Ok(prefix)
}

fn command(prefix: &Path, socket: Option<&str>) -> String {
    let mut result = format!(
        "tmt ops migration switch --yes --prefix {}",
        quote(&prefix.to_string_lossy())
    );
    if let Some(socket) = socket {
        result.push_str(&format!(" --socket {}", quote(socket)));
    }
    result
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

struct Discovery {
    boards: Vec<Value>,
    deferred: bool,
}

fn discover(prefix: &Path, socket: &str) -> Result<Discovery, SquadError> {
    let panes = panes(socket)?;
    let table = process::table()?;
    let mut boards = Vec::new();
    let mut deferred = false;
    for pane in panes.iter().filter(|pane| !pane.dead) {
        for observed in table
            .iter()
            .filter(|p| process::in_pane(p, pane.pid, &pane.tty, &table))
        {
            let Ok((executable, _)) = process::kernel(observed.pid) else {
                continue;
            };
            if !process::former(&executable, prefix) {
                continue;
            }
            let Ok(launch) = process::launch(observed.pid, prefix) else {
                deferred = true;
                continue;
            };
            let Some(args) = process::board_arguments(&launch.arguments) else {
                deferred = true;
                continue;
            };
            let Some(root) = table.iter().find(|p| p.pid == pane.pid) else {
                continue;
            };
            if !launch.cwd.is_absolute() || !launch.cwd.is_dir() {
                return Err(fail(
                    "A former board's cwd is unavailable; it was left running.",
                ));
            }
            boards.push(json!({"pid": observed.pid, "start": observed.start, "executable": launch.executable,
                "pane":pane.id,"rootPid":root.pid,"rootStart":root.start,"tty":pane.tty,
                "cwd":launch.cwd,"args":args,"state":"old",
                "remain":tmux(socket,&["show-options","-p","-v","-t",&pane.id,"remain-on-exit"])?.trim()}));
        }
    }
    if boards.len() > 32 {
        return Err(fail("More than 32 former boards exceed the switch bound."));
    }
    Ok(Discovery { boards, deferred })
}

fn still_old(board: &Value, prefix: &Path, socket: &str) -> Result<bool, SquadError> {
    let table = process::table()?;
    let pid = number(board, "pid")?;
    let Some(observed) = table.iter().find(|p| p.pid == pid) else {
        return Ok(false);
    };
    if observed.start != string(board, "start")? {
        return Err(fail("A former board PID was reused; no signal was sent."));
    }
    let pane = panes(socket)?
        .into_iter()
        .find(|pane| pane.id == string(board, "pane").unwrap_or_default())
        .ok_or_else(|| fail("The former board pane disappeared."))?;
    if !process::in_pane(observed, pane.pid, &pane.tty, &table) || pane.tty != string(board, "tty")?
    {
        return Err(fail(
            "The former board is no longer in its original foreground pane.",
        ));
    }
    if pane.pid != number(board, "rootPid")?
        || !table
            .iter()
            .any(|p| p.pid == pane.pid && p.start == string(board, "rootStart").unwrap_or_default())
    {
        return Err(fail(
            "The original pane process changed; no signal was sent.",
        ));
    }
    let launch = process::launch(pid, prefix)?;
    if !process::former(&launch.executable, prefix)
        || launch.executable.to_str() != Some(string(board, "executable")?)
        || process::board_arguments(&launch.arguments).as_ref() != Some(&strings(board, "args")?)
    {
        return Err(fail(
            "The former executable or board arguments changed; no signal was sent.",
        ));
    }
    let latest = process::table()?;
    if !latest.iter().any(|p| {
        p.pid == pid
            && p.start == observed.start
            && process::in_pane(p, pane.pid, &pane.tty, &latest)
    }) {
        return Err(fail(
            "Process identity or foreground ownership changed; no signal was sent.",
        ));
    }
    Ok(true)
}

fn stop(
    board: &mut Value,
    prefix: &Path,
    socket: &str,
    deadline: Instant,
) -> Result<(), SquadError> {
    if !still_old(board, prefix, socket)? {
        board["state"] = json!("stopped");
        return Ok(());
    }
    // Retain a dedicated pane through process exit. This is a pane-scoped
    // option, not a kill/respawn of its shell or process group.
    let pane = string(board, "pane")?.to_owned();
    tmux(
        socket,
        &["set-option", "-p", "-t", &pane, "remain-on-exit", "on"],
    )?;
    if still_old(board, prefix, socket)? {
        kill(
            Pid::from_raw(i32::try_from(number(board, "pid")?).map_err(fail)?),
            Signal::SIGTERM,
        )
        .map_err(fail)?;
    }
    // Once TERM was sent, an exiting process may lose its tty/foreground
    // before it is reaped. Wait only for that exact incarnation, never signal again.
    while process::table()?.iter().any(|p| {
        p.pid == number(board, "pid").unwrap_or_default()
            && p.start == string(board, "start").unwrap_or_default()
    }) {
        if Instant::now() >= deadline {
            return Err(fail("The former board did not stop before the deadline."));
        }
        thread::sleep(Duration::from_millis(50));
    }
    board["state"] = json!("stopped");
    Ok(())
}

fn live_ops(board: &Value, socket: &str, active: &Path) -> Result<bool, SquadError> {
    let Some(pane) = panes(socket)?
        .into_iter()
        .find(|pane| pane.id == string(board, "pane").unwrap_or_default())
    else {
        return Ok(false);
    };
    let table = process::table()?;
    for p in table
        .iter()
        .filter(|p| process::in_pane(p, pane.pid, &pane.tty, &table))
    {
        let Ok((executable, _)) = process::kernel(p.pid) else {
            continue;
        };
        if executable == active
            && record::read_ready(Path::new(string(board, "ready")?))? == p.pid.to_string()
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn launch_line(board: &Value, core: &Core) -> Result<String, SquadError> {
    let env = vec![
        format!("TMT_EXECUTABLE={}", core.executable().display()),
        format!("TMT_OPS_SWITCH_READY={}", string(board, "ready")?),
    ];
    let mut argv = vec!["/usr/bin/env".to_owned()];
    argv.extend(env);
    argv.extend([
        core.executable().to_string_lossy().into_owned(),
        "ops".into(),
    ]);
    argv.extend(strings(board, "args")?);
    let line = format!(
        "(cd {} && exec {})",
        quote(string(board, "cwd")?),
        argv.iter()
            .map(|word| quote(word))
            .collect::<Vec<_>>()
            .join(" ")
    );
    if !Path::new(string(board, "cwd")?).is_dir() || line.len() > 1023 {
        return Err(fail(
            "The saved cwd or launch exceeds the safe shell input bound.",
        ));
    }
    Ok(line)
}

fn restart(
    board: &mut Value,
    core: &Core,
    socket: &str,
    active: &Path,
    deadline: Instant,
    mut save: impl FnMut(&Value) -> Result<(), SquadError>,
) -> Result<(), SquadError> {
    if live_ops(board, socket, active)? {
        board["state"] = json!("started");
        return Ok(());
    }
    if board["state"] == "started" {
        return Ok(());
    }
    if board["state"] == "launching" {
        return Err(fail(
            "A prior launch is not confirmed; the pending record was retained instead of submitting it twice.",
        ));
    }
    let pane = panes(socket)?
        .into_iter()
        .find(|pane| pane.id == string(board, "pane").unwrap_or_default())
        .ok_or_else(|| fail("A stopped board's pane disappeared; its launch was retained."))?;
    if pane.tty != string(board, "tty")? || (pane.dead && pane.pid != number(board, "rootPid")?) {
        return Err(fail(
            "The original stopped pane changed; no launch was submitted.",
        ));
    }
    let line = launch_line(board, core)?;
    if pane.dead {
        board["state"] = json!("launching");
        save(board)?;
        tmux(
            socket,
            &[
                "respawn-pane",
                "-t",
                &pane.id,
                "-c",
                string(board, "cwd")?,
                &line,
            ],
        )?;
    } else {
        let table = process::table()?;
        let root = table
            .iter()
            .find(|p| p.pid == number(board, "rootPid").unwrap_or_default())
            .ok_or_else(|| fail("The original parent shell is gone."))?;
        if root.start != string(board, "rootStart")?
            || pane.pid != root.pid
            || !process::in_pane(root, pane.pid, &pane.tty, &table)
            || !process::shell(root.pid)?
        {
            return Err(fail(
                "The original shell is not back in the foreground; no input was submitted.",
            ));
        }
        board["state"] = json!("launching");
        save(board)?;
        tmux(socket, &["send-keys", "-t", &pane.id, "-l", "--", &line])?;
        tmux(socket, &["send-keys", "-t", &pane.id, "Enter"])?;
    }
    while !live_ops(board, socket, active)? {
        if Instant::now() >= deadline {
            return Err(fail(
                "The relaunched Ops board has not confirmed a loaded screen.",
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
    board["state"] = json!("started");
    Ok(())
}

fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str, SquadError> {
    value[key]
        .as_str()
        .ok_or_else(|| fail("Invalid pending switch record."))
}
fn number(value: &Value, key: &str) -> Result<u32, SquadError> {
    u32::try_from(
        value[key]
            .as_u64()
            .ok_or_else(|| fail("Invalid pending process identity."))?,
    )
    .map_err(fail)
}
fn strings(value: &Value, key: &str) -> Result<Vec<String>, SquadError> {
    value[key]
        .as_array()
        .ok_or_else(|| fail("Invalid saved arguments."))?
        .iter()
        .map(|v| {
            v.as_str()
                .map(str::to_owned)
                .ok_or_else(|| fail("Invalid saved argument."))
        })
        .collect()
}

pub(crate) fn run(matches: &ArgMatches, interaction: Interaction) -> Result<Value, SquadError> {
    let (_, flags) = matches
        .subcommand()
        .ok_or_else(|| fail("Expected migration switch."))?;
    let core = Core::discover()?;
    let prefix = verify_prefix(&core, flags.get_one::<String>("prefix").map(String::as_str))?;
    let socket = flags
        .get_one::<String>("socket")
        .cloned()
        .or_else(effects::tmux_socket);
    consent::ask(
        Consent::new(flags.get_flag("yes"), interaction.prompt()),
        "Switch verified former boards to Ops in their existing panes?",
        "Proceed?",
    )?;
    Ok(attempt(&core, &prefix, socket.as_deref()))
}

fn attempt(core: &Core, prefix: &Path, socket: Option<&str>) -> Value {
    match switch(core, prefix, socket) {
        Ok(report) => report,
        Err(error) => json!({"complete":false,"switched":0,
            "command":command(prefix, socket),"reason":error.message}),
    }
}

fn switch(core: &Core, prefix: &Path, socket: Option<&str>) -> Result<Value, SquadError> {
    let hint = command(prefix, socket);
    let Some(socket) = socket else {
        return Ok(json!({"complete":false,"switched":0,"command":hint}));
    };
    let root = migration::data_root(core)?;
    let mut record = record::Record::open(&root, prefix, socket)?;
    let fresh = discover(prefix, socket)?;
    for board in fresh.boards {
        if !record
            .boards
            .iter()
            .any(|old| old["pid"] == board["pid"] && old["start"] == board["start"])
        {
            record.add(board)?;
        }
    }
    record.save()?;
    let deadline = Instant::now() + Duration::from_secs(60);
    let active = fs::canonicalize(std::env::current_exe().map_err(fail)?).map_err(fail)?;
    let outcome = (|| {
        for board in &record.boards {
            if board["state"] == "old" || board["state"] == "stopped" {
                launch_line(board, core)?;
            }
        }
        for index in 0..record.boards.len() {
            if Instant::now() >= deadline {
                return Err(fail("The board-switch deadline elapsed."));
            }
            if record.boards[index]["state"] == "old" {
                stop(&mut record.boards[index], prefix, socket, deadline)?;
                record.save()?;
            }
        }
        let initial = migration::paths(core, None)?;
        if initial.legacy && migration::retry(core)?.legacy {
            return Err(fail(
                "Migration is still deferred; scheduled sends remain paused in new Ops boards.",
            ));
        }
        for index in 0..record.boards.len() {
            let mut board = record.boards[index].clone();
            let result = restart(&mut board, core, socket, &active, deadline, |staged| {
                record.boards[index] = staged.clone();
                record.save()
            });
            record.boards[index] = board;
            result?;
            record.save()?;
            let board = &record.boards[index];
            if let Some(remain) = board["remain"].as_str() {
                let pane = string(board, "pane")?;
                if remain.is_empty() {
                    tmux(
                        socket,
                        &["set-option", "-p", "-u", "-t", pane, "remain-on-exit"],
                    )?;
                } else {
                    tmux(
                        socket,
                        &["set-option", "-p", "-t", pane, "remain-on-exit", remain],
                    )?;
                }
            }
        }
        if fresh.deferred {
            return Err(fail(
                "A former process has unrecognized board arguments; it was left running.",
            ));
        }
        Ok::<_, SquadError>(())
    })();
    // Save partial progress even after a failed effect. No rollback or second
    // signal is inferred from an unconfirmed launch.
    record.save()?;
    let switched = record
        .boards
        .iter()
        .filter(|board| board["state"] == "started")
        .count();
    match outcome {
        Ok(()) => {
            record.finish()?;
            Ok(json!({"complete":true,"switched":switched}))
        }
        Err(error) => {
            Ok(json!({"complete":false,"switched":switched,"command":hint,"reason":error.message}))
        }
    }
}

/// One pre-dispatch offer. Protocol/help/completion paths never reach here.
pub(crate) fn offer(interaction: Interaction) -> Result<(), SquadError> {
    if std::env::var_os("TMT_OPS_SWITCH_READY").is_some() {
        return Ok(());
    }
    let Some(prefix) = installed_prefix() else {
        return Ok(());
    };
    let core = Core::discover()?;
    let shown = core.json(&["config", "show"])?;
    let old = shown["paths"]["global"]
        .as_str()
        .and_then(|path| Path::new(path).parent())
        .is_some_and(|path| path.join("squad.toml").exists());
    let socket = effects::tmux_socket();
    let root = migration::data_root(&core)?;
    if !old && !record::exists(&root) {
        let Some(socket) = &socket else {
            return Ok(());
        };
        let found = discover(&prefix, socket)?;
        if found.boards.is_empty() && !found.deferred {
            return Ok(());
        }
    }
    let prefix = verify_prefix(&core, None)?;
    let hint = command(&prefix, socket.as_deref());
    if interaction.prompt() != Mode::Interactive {
        eprintln!("{hint}");
        return Ok(());
    }
    if consent::ask(
        Consent::Ask,
        "Former Squad boards can be switched to Ops in their existing panes.",
        "Switch now?",
    )
    .is_err()
    {
        return Ok(());
    }
    let result = attempt(&core, &prefix, socket.as_deref());
    if result["complete"] == true || result["switched"].as_u64().unwrap_or(0) > 0 {
        eprintln!("switched {} boards to Ops", result["switched"]);
    }
    if result["complete"] == false
        && let Some(reason) = result["reason"].as_str()
    {
        eprintln!("{reason}");
    }
    if let Some(command) = result["command"].as_str() {
        eprintln!("{command}");
    }
    Ok(())
}

/// A successful first loaded draw acknowledges the private launch marker.
pub(crate) fn ready(core: &Core) -> Result<Option<fs::File>, SquadError> {
    let Some(path) = std::env::var_os("TMT_OPS_SWITCH_READY").map(PathBuf::from) else {
        return Ok(None);
    };
    record::ready_file(&path, &migration::data_root(core)?).map(Some)
}
pub(crate) fn acknowledge(file: &mut fs::File) -> std::io::Result<()> {
    file.set_len(0)?;
    file.write_all(std::process::id().to_string().as_bytes())?;
    file.sync_all()
}

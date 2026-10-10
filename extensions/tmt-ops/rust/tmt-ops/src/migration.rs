//! Invocation-owned Ops cutover. Core still discovers both public roots; this
//! owner alone chooses a layout and handles legacy files, never backup globs.
use crate::core::{Core, SquadError};
use nix::{
    fcntl::{Flock, FlockArg, OFlag},
    unistd::getuid,
};
use serde_json::{Value, json};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Mutex,
};
use tmt_cli_style::message;

const COMPLETE: &str = ".ops-paths-v1";
const CUTOVER: &str = ".ops-paths-cutover-v1";
const PENDING: &str = ".ops-paths-pending.json";
const LOCK: &str = ".ops-paths.lock";
const LIMIT: u64 = 64 * 1024 * 1024;

#[derive(Clone, Debug)]
pub(crate) struct Paths {
    pub config: PathBuf,
    pub legacy: bool,
    pub notice: Option<String>,
    pub board_notice: Option<String>,
}
impl Paths {
    fn at(directory: &Path, legacy: bool) -> Self {
        Self {
            config: directory.join(if legacy { "squad.toml" } else { "ops.toml" }),
            legacy,
            notice: None,
            board_notice: None,
        }
    }
    pub fn subtree(&self) -> &'static str {
        if self.legacy { "squad" } else { "ops" }
    }
}
fn failed(error: impl std::fmt::Display) -> SquadError {
    SquadError::new(
        "SQUAD_PATH_MIGRATION_FAILED",
        format!("Ops path migration: {error}"),
    )
}
fn notice(text: &str) {
    let mut stderr = tmt_cli_style::stream::stderr();
    let terminal = stderr.terminal();
    let _ = message::warning(&mut stderr, terminal, text, None);
}

/// The invocation has one shared layout decision. Only the UI clock retries a
/// deferred decision; completed layouts and failures remain cached.
#[derive(Default)]
pub(crate) struct Decision(Mutex<Option<Result<Paths, SquadError>>>);

impl Decision {
    /// Read the selected layout without attempting a cutover or creating paths.
    pub(crate) fn is_current(&self) -> bool {
        self.0
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|result| result.as_ref().is_ok_and(|paths| !paths.legacy))
    }

    pub fn board_notice(&self) -> Option<String> {
        self.0
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|result| match result {
                Ok(paths) => paths.board_notice.clone().or_else(|| paths.notice.clone()),
                Err(error) => Some(error.message.clone()),
            })
    }
}

pub(crate) fn paths(core: &Core, shown: Option<&Value>) -> Result<Paths, SquadError> {
    let mut decision = core.paths.0.lock().unwrap();
    decision
        .get_or_insert_with(|| {
            let owned;
            let shown = match shown {
                Some(shown) => shown,
                None => {
                    owned = core.config_show()?;
                    &owned
                }
            };
            let directory = shown["paths"]["global"]
                .as_str()
                .and_then(|path| Path::new(path).parent())
                .filter(|path| path.is_absolute())
                .ok_or_else(|| failed("Core reported no absolute global config directory."))?;
            let (paths, warning) = match ready(directory).map_err(failed)? {
                Some(ready) => ready,
                None => prepare(
                    directory,
                    &data_root(core)?,
                    jiff::Timestamp::now().as_millisecond(),
                )
                .map_err(failed)?,
            };
            if let Some(warning) = warning {
                notice(&warning);
            }
            Ok(paths)
        })
        .clone()
}

/// Never hold the decision mutex while taking the cutover lock: a legacy
/// service may hold a shared file guard and then resolve its root via paths().
/// A contended cutover lock is another deferred attempt, not a UI wait.
pub(crate) fn retry(core: &Core) -> Result<Paths, SquadError> {
    let previous = paths(core, None)?;
    if !previous.legacy {
        return Ok(previous);
    }
    let root = data_root(core)?;
    let next = match prepare_mode(
        previous
            .config
            .parent()
            .expect("selected config has a parent"),
        &root,
        jiff::Timestamp::now().as_millisecond(),
        FlockArg::LockExclusiveNonblock,
    ) {
        Ok((paths, _)) => Ok(paths),
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(previous),
        Err(error) => Err(failed(error)),
    };
    let mut decision = core.paths.0.lock().unwrap();
    // Another retry may already have promoted. Never revert a completed layout.
    if decision
        .as_ref()
        .is_some_and(|current| current.as_ref().is_ok_and(|paths| paths.legacy))
    {
        *decision = Some(next);
    }
    decision.as_ref().expect("initialized decision").clone()
}
pub(crate) fn data_root(core: &Core) -> Result<PathBuf, SquadError> {
    let value = core.api("storage.root", json!({}))?;
    value["dataRoot"]
        .as_str()
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| failed("storage.root returned no absolute dataRoot."))
}
pub(crate) fn state_root(core: &Core) -> Result<PathBuf, SquadError> {
    let paths = paths(core, None)?;
    let root = data_root(core)?.join(paths.subtree());
    if paths.legacy && !exists(&root).map_err(failed)? {
        return Err(failed(
            "Legacy state moved to Ops; restart this invocation before writing.",
        ));
    }
    Ok(root)
}
fn identity(metadata: &fs::Metadata) -> Value {
    json!({"device":metadata.dev(),"inode":metadata.ino(),"length":metadata.len(),"mtime":metadata.mtime(),"mtimeNs":metadata.mtime_nsec()})
}
fn ready(config: &Path) -> io::Result<Option<(Paths, Option<String>)>> {
    if !marked(config, COMPLETE)? {
        return Ok(None);
    }
    let mut paths = Paths::at(config, false);
    let old = config.join("squad.toml");
    match fs::symlink_metadata(&old) {
        Ok(metadata) => {
            let marker: Value = serde_json::from_slice(&bytes(&config.join(COMPLETE))?)
                .map_err(io::Error::other)?;
            if marker["ignoredConfig"] != identity(&metadata) {
                paths.notice = Some(format!(
                    "Legacy config {} has reappeared. Its settings are not used; move wanted settings into {}, then delete the legacy file. Restart boards started before the upgrade.",
                    old.display(),
                    paths.config.display()
                ));
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let warning = paths.notice.clone();
    Ok(Some((paths, warning)))
}

/// Pending new-version config writes share the cutover lock; a retained
/// invocation cannot recreate a migrated old file, including an absent baseline.
pub(crate) fn config_write_guard(path: &Path) -> Result<Option<Flock<File>>, SquadError> {
    if path.file_name().is_none_or(|name| name != "squad.toml") {
        return Ok(None);
    }
    let parent = path
        .parent()
        .ok_or_else(|| failed("config has no parent"))?;
    let guard = lock_mode(&parent.join(LOCK), true, FlockArg::LockShared).map_err(failed)?;
    if completed(parent).map_err(failed)? {
        return Err(SquadError::new(
            "SQUAD_CONFIG_CHANGED",
            "Legacy config moved to Ops; restart the board before editing.",
        ));
    }
    Ok(guard)
}
pub(crate) type LegacyGuard = Option<Flock<File>>;
pub(crate) fn state_guard(core: &Core) -> Result<LegacyGuard, SquadError> {
    config_write_guard(&paths(core, None)?.config)
}
fn completed(directory: &Path) -> io::Result<bool> {
    Ok(marked(directory, COMPLETE)? || marked(directory, CUTOVER)?)
}
fn marked(directory: &Path, name: &str) -> io::Result<bool> {
    match fs::symlink_metadata(directory.join(name)) {
        Ok(metadata) => {
            regular(&metadata)?;
            Ok(true)
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}
fn exists(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}
fn regular(metadata: &fs::Metadata) -> io::Result<()> {
    if metadata.is_file() && metadata.uid() == getuid().as_raw() {
        Ok(())
    } else {
        Err(io::Error::other(
            "expected an owned regular file (no symlinks)",
        ))
    }
}
pub(crate) fn directory(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() && metadata.uid() == getuid().as_raw() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "{} is not an owned directory",
            path.display()
        )))
    }
}
pub(crate) fn open(path: &Path) -> io::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags((OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK).bits())
        .open(path)?;
    regular(&file.metadata()?)?;
    Ok(file)
}
fn bytes(path: &Path) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    open(path)?.take(LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > LIMIT {
        return Err(io::Error::other("migration file exceeds 64 MiB"));
    }
    Ok(bytes)
}
fn write_new_file(path: &Path, bytes: &[u8], mode: u32) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}
fn mark_cutover(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let stage = path.with_extension("tmp");
    if exists(&stage)? {
        regular(&fs::symlink_metadata(&stage)?)?;
        fs::remove_file(&stage)?;
    }
    write_new_file(&stage, bytes, 0o600)?;
    fs::hard_link(&stage, path)?;
    sync(path.parent().unwrap())?;
    fs::remove_file(stage)
}
fn sync(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}
#[cfg(test)]
fn lock(path: &Path, create: bool) -> io::Result<Option<Flock<File>>> {
    lock_mode(path, create, FlockArg::LockExclusive)
}
fn lock_mode(path: &Path, create: bool, mode: FlockArg) -> io::Result<Option<Flock<File>>> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(create)
        .truncate(false)
        .mode(0o600)
        .custom_flags((OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK).bits())
        .open(path);
    let file = match file {
        Ok(file) => file,
        Err(e) if !create && e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    regular(&file.metadata()?)?;
    Flock::lock(file, mode)
        .map(Some)
        .map_err(|(_, e)| io::Error::from_raw_os_error(e as i32))
}

/// Keep legacy lock inodes held until publication and archival are complete.
/// No old clock can renew/acquire while its lease is examined or copied.
fn legacy_locks(root: &Path, locks: &mut Vec<Flock<File>>) -> io::Result<()> {
    directory(root)?;
    let mut entries = fs::read_dir(root)?.collect::<io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let kind = entry.file_type()?;
        if kind.is_dir() {
            legacy_locks(&entry.path(), locks)?;
        } else if kind.is_file() {
            if entry.file_name().to_string_lossy().ends_with(".lock")
                && let Some(guard) =
                    lock_mode(&entry.path(), false, FlockArg::LockExclusiveNonblock)?
            {
                locks.push(guard);
            }
        } else {
            return Err(io::Error::other(
                "legacy state contains a symlink or special file",
            ));
        }
    }
    Ok(())
}
/// Both-present state is ignored, except for excluding a still-live old clock.
/// Do not enumerate unrelated legacy files or wait on their writers.
fn clock_directory(root: &Path) -> io::Result<bool> {
    for path in [root.to_owned(), root.join("cron")] {
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_file() => return Ok(false),
            Ok(_) => directory(&path)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error),
        }
    }
    Ok(true)
}
fn legacy_clock_lock(root: &Path, locks: &mut Vec<Flock<File>>) -> io::Result<()> {
    let cron = root.join("cron");
    if !clock_directory(root)? {
        return Ok(());
    }
    if let Some(guard) = lock_mode(
        &cron.join("clock.lock"),
        false,
        FlockArg::LockExclusiveNonblock,
    )? {
        locks.push(guard);
    }
    Ok(())
}
fn live_clock_holder(root: &Path, now: i64) -> io::Result<Option<tmt_ops::cron::Holder>> {
    if !clock_directory(root)? {
        return Ok(None);
    }
    let path = root.join("cron/clock.json");
    if !exists(&path)? {
        return Ok(None);
    }
    let value: Value = serde_json::from_slice(&bytes(&path)?).map_err(io::Error::other)?;
    let since = value["sinceMs"]
        .as_i64()
        .ok_or_else(|| io::Error::other("invalid legacy clock lease"))?;
    let expiry = value["expiresMs"]
        .as_i64()
        .filter(|expiry| *expiry > since)
        .ok_or_else(|| io::Error::other("invalid legacy clock lease"))?;
    let pid = value["pid"]
        .as_u64()
        .filter(|pid| *pid > 0 && *pid <= u32::MAX as u64)
        .ok_or_else(|| io::Error::other("invalid legacy clock holder"))?;
    if value["version"] != 1 {
        return Err(io::Error::other("unsupported legacy clock lease"));
    }
    let pane = value["pane"]
        .as_str()
        .filter(|pane| !pane.chars().any(char::is_control));
    Ok(
        (since <= now && now < expiry).then(|| tmt_ops::cron::Holder {
            pid: pid as u32,
            pane: pane.map(str::to_owned),
            since_ms: since,
            expires_ms: expiry,
        }),
    )
}

/// Observe the legacy holder through the same admission used by path migration.
/// A lease identifies a candidate; the switch owner still verifies its process.
pub(crate) fn legacy_clock_holder(root: &Path) -> io::Result<Option<tmt_ops::cron::Holder>> {
    live_clock_holder(&root.join("squad"), jiff::Timestamp::now().as_millisecond())
}

fn live_clock(root: &Path, now: i64) -> io::Result<Option<(String, String)>> {
    let Some(holder) = live_clock_holder(root, now)? else {
        return Ok(None);
    };
    let pid = holder.pid;
    let pane = holder.pane.as_deref();
    // After the one switch offer, the board shows only this line: it names the fix.
    // Quitting releases the lease; the board's clock worker then completes the cutover.
    let board = match pane {
        Some(pane) => format!("the old board in pane {pane}"),
        None => format!("old board PID {pid}"),
    };
    Ok(Some((
        format!(
            "Ops migration deferred: old clock PID {pid}{}; legacy config/state stay active until the board switch completes.",
            pane.map(|pane| format!(" in pane {pane}"))
                .unwrap_or_default()
        ),
        format!("Ops migration pending: quit {board} (q); Ops then retries."),
    )))
}
fn copy(source: &Path, target: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(source)?;
    if metadata.is_dir() {
        directory(source)?;
        fs::DirBuilder::new().mode(0o700).create(target)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy(&entry.path(), &target.join(entry.file_name()))?;
        }
        sync(target)
    } else {
        regular(&metadata)?;
        write_new_file(
            target,
            &bytes(source)?,
            metadata.permissions().mode() & 0o777,
        )
    }
}
fn equal(left: &Path, right: &Path) -> io::Result<()> {
    let left_kind = fs::symlink_metadata(left)?;
    let right_kind = fs::symlink_metadata(right)?;
    if left_kind.is_dir() && right_kind.is_dir() {
        directory(left)?;
        directory(right)?;
        let names = |path| -> io::Result<Vec<_>> {
            let mut names = fs::read_dir(path)?
                .map(|entry| entry.map(|entry| entry.file_name()))
                .collect::<io::Result<Vec<_>>>()?;
            names.sort();
            Ok(names)
        };
        let entries = names(left)?;
        if entries != names(right)? {
            return Err(io::Error::other("copied state tree differs"));
        }
        for name in entries {
            equal(&left.join(&name), &right.join(name))?;
        }
        Ok(())
    } else {
        regular(&left_kind)?;
        regular(&right_kind)?;
        if bytes(left)? == bytes(right)? {
            Ok(())
        } else {
            Err(io::Error::other("copied file bytes differ"))
        }
    }
}
fn validate_config(path: &Path) -> io::Result<()> {
    let bytes = bytes(path)?;
    let text = std::str::from_utf8(&bytes).map_err(io::Error::other)?;
    text.parse::<toml_edit::DocumentMut>()
        .map_err(io::Error::other)?;
    Ok(())
}

struct Move {
    source: PathBuf,
    target: PathBuf,
    stage: PathBuf,
    backup: PathBuf,
    config: bool,
}
impl Move {
    fn new(parent: &Path, config: bool, stamp: &str) -> Self {
        let (old, new) = if config {
            ("squad.toml", "ops.toml")
        } else {
            ("squad", "ops")
        };
        Self {
            source: parent.join(old),
            target: parent.join(new),
            stage: parent.join(format!(".ops-migrate-{new}-{stamp}")),
            backup: parent.join(format!("{old}.migrated-{stamp}")),
            config,
        }
    }
    fn stage(&self) -> io::Result<()> {
        if exists(&self.target)? || exists(&self.backup)? {
            return Ok(());
        }
        if exists(&self.stage)? && equal(&self.source, &self.stage).is_err() {
            // The journal owns this unpublished staging path. An interrupted
            // copy is disposable; neither source nor user backups are removed.
            let metadata = fs::symlink_metadata(&self.stage)?;
            if metadata.is_dir() {
                directory(&self.stage)?;
                fs::remove_dir_all(&self.stage)?;
            } else {
                regular(&metadata)?;
                fs::remove_file(&self.stage)?;
            }
        }
        if !exists(&self.stage)? {
            copy(&self.source, &self.stage)?;
        }
        equal(&self.source, &self.stage)?;
        if self.config {
            validate_config(&self.stage)?;
        }
        sync(self.source.parent().unwrap())
    }
    fn publish(&self) -> io::Result<()> {
        if !exists(&self.target)? {
            // The migration lock excludes all new-version publishers. File
            // publication is create-only; directory rename follows the same guard.
            if self.config {
                fs::hard_link(&self.stage, &self.target)?;
                fs::remove_file(&self.stage)?;
            } else {
                fs::rename(&self.stage, &self.target)?;
            }
            sync(self.target.parent().unwrap())?;
        }
        Ok(())
    }
    fn archive(&self) -> io::Result<()> {
        // Cutover is already durable. Archive originals without reading them:
        // Ops may have legitimate later writes, so equality no longer applies.
        if exists(&self.backup)? {
            // This move already archived its source; a recreated legacy name
            // belongs to the user and must remain untouched.
            return Ok(());
        }
        if exists(&self.source)? {
            fs::rename(&self.source, &self.backup)?;
            sync(self.source.parent().unwrap())?;
        } else if !exists(&self.backup)? {
            return Err(io::Error::other(
                "migration original and backup are missing",
            ));
        }
        Ok(())
    }
}

fn prepare(config: &Path, data: &Path, now: i64) -> io::Result<(Paths, Option<String>)> {
    prepare_mode(config, data, now, FlockArg::LockExclusive)
}

fn prepare_mode(
    config: &Path,
    data: &Path,
    now: i64,
    mode: FlockArg,
) -> io::Result<(Paths, Option<String>)> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(config)?;
    directory(config)?;
    let _lock = lock_mode(&config.join(LOCK), true, mode)?;
    if let Some(ready) = ready(config)? {
        return Ok(ready);
    }
    let cutover = marked(config, CUTOVER)?;
    let old_state = data.join("squad");
    let pending = config.join(PENDING);
    let journal = if cutover {
        config.join(CUTOVER)
    } else {
        pending.clone()
    };
    let (stamp, move_config, move_state, warning) = if exists(&journal)? {
        let value: Value = serde_json::from_slice(&bytes(&journal)?).map_err(io::Error::other)?;
        let stamp = value["stamp"]
            .as_str()
            .filter(|stamp| {
                !stamp.is_empty()
                    && stamp
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || byte == b'-')
            })
            .ok_or_else(|| io::Error::other("invalid migration journal"))?;
        (
            stamp.to_owned(),
            value["config"]
                .as_bool()
                .ok_or_else(|| io::Error::other("invalid migration journal"))?,
            value["state"]
                .as_bool()
                .ok_or_else(|| io::Error::other("invalid migration journal"))?,
            value["warning"].as_str().map(str::to_owned),
        )
    } else {
        if cutover {
            return Err(io::Error::other("cutover cleanup journal is missing"));
        }
        let old_config = config.join("squad.toml");
        let mut ignored = Vec::new();
        let select = |old: &Path, new: &Path, ignored: &mut Vec<String>| -> io::Result<bool> {
            if !exists(old)? {
                return Ok(false);
            }
            if exists(new)? {
                ignored.push(old.display().to_string());
                Ok(false)
            } else {
                Ok(true)
            }
        };
        let move_config = select(&old_config, &config.join("ops.toml"), &mut ignored)?;
        let move_state = select(&old_state, &data.join("ops"), &mut ignored)?;
        let stamp = format!("{now}-{}", std::process::id());
        let warning = (!ignored.is_empty()).then(|| {
            format!(
                "Ops paths take precedence; legacy paths left untouched: {}.",
                ignored.join(", ")
            )
        });
        (stamp, move_config, move_state, warning)
    };
    let mut guards = Vec::new();
    if !cutover && exists(&old_state)? {
        if let Err(error) = if move_state {
            legacy_locks(&old_state, &mut guards)
        } else {
            legacy_clock_lock(&old_state, &mut guards)
        } {
            if error.kind() != io::ErrorKind::WouldBlock {
                return Err(error);
            }
            let (warning, board_notice) = live_clock(&old_state, now)?.unwrap_or_else(|| (
                "Ops migration deferred: legacy state has an active writer. Retry after it finishes; this invocation keeps using legacy config and state.".into(),
                "Ops migration pending: quit old Squad boards (q); Ops then retries.".into(),
            ));
            let mut paths = Paths::at(config, true);
            paths.notice = Some(warning.clone());
            paths.board_notice = Some(board_notice);
            return Ok((paths, Some(warning)));
        }
        if let Some((warning, board_notice)) = live_clock(&old_state, now)? {
            let mut paths = Paths::at(config, true);
            paths.notice = Some(warning.clone());
            paths.board_notice = Some(board_notice);
            return Ok((paths, Some(warning)));
        }
    }
    if !cutover && !exists(&pending)? {
        write_new_file(
            &pending,
            json!({"stamp":stamp,"config":move_config,"state":move_state,"warning":warning})
                .to_string()
                .as_bytes(),
            0o600,
        )?;
        sync(config)?;
    }
    let mut moves = Vec::new();
    if move_config {
        moves.push(Move::new(config, true, &stamp));
    }
    if move_state {
        moves.push(Move::new(data, false, &stamp));
    }
    if !cutover {
        for item in &moves {
            item.stage()?;
        }
        for item in &moves {
            item.publish()?;
        }
        // No source is archived until every target has been published and
        // verified, and the decision to use Ops has reached durable storage.
        for item in &moves {
            equal(&item.source, &item.target)?;
        }
        let ignored = if move_config {
            None
        } else {
            fs::symlink_metadata(config.join("squad.toml"))
                .ok()
                .map(|metadata| identity(&metadata))
        };
        mark_cutover(
            &config.join(CUTOVER),
            json!({"version":1,"ignoredConfig":ignored,"stamp":stamp,"config":move_config,"state":move_state,"warning":warning})
                .to_string()
                .as_bytes(),
        )?;
    }
    let mut paths = Paths::at(config, false);
    paths.notice = warning;
    let cleanup = (|| {
        for item in &moves {
            item.archive()?;
        }
        mark_cutover(&config.join(COMPLETE), &bytes(&config.join(CUTOVER))?)?;
        match fs::remove_file(pending) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        sync(config)
    })();
    if let Err(error) = cleanup {
        paths.notice = Some(format!(
            "Ops cutover is complete; retained originals could not all be archived: {error}. Ops settings/state are active. Restart old boards; the next invocation retries backup cleanup."
        ));
    }
    if let Some((_, Some(reappeared))) = ready(config)? {
        paths.notice = Some(reappeared);
    }
    let warning = paths.notice.clone();
    Ok((paths, warning))
}

#[cfg(test)]
mod tests;

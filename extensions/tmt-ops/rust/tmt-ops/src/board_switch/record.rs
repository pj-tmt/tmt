//! Bounded private recovery state, separate from the path migration journal.

use super::{fail, string};
use crate::core::SquadError;
use nix::fcntl::{Flock, FlockArg, OFlag};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

const NAME: &str = ".ops-board-switch-v1.json";
const READY: &str = ".ops-board-switch-ready-";
const LIMIT: u64 = 1024 * 1024;

pub(super) fn exists(root: &Path) -> bool {
    root.join(NAME).exists()
}

fn private(file: &File) -> Result<(), SquadError> {
    let metadata = file.metadata().map_err(fail)?;
    if !metadata.is_file()
        || metadata.uid() != nix::unistd::geteuid().as_raw()
        || metadata.mode() & 0o077 != 0
        || metadata.len() > LIMIT
    {
        return Err(fail("Switch recovery state is not a bounded private file."));
    }
    Ok(())
}

fn open(path: &Path, write: bool, create: bool) -> Result<File, SquadError> {
    let file = OpenOptions::new()
        .read(true)
        .write(write)
        .create(create)
        .truncate(false)
        .mode(0o600)
        .custom_flags((OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK).bits())
        .open(path)
        .map_err(fail)?;
    private(&file)?;
    Ok(file)
}

pub(super) struct Record {
    _guard: Flock<File>,
    root: PathBuf,
    prefix: PathBuf,
    socket: String,
    pub boards: Vec<Value>,
}

impl Record {
    pub fn open(root: &Path, prefix: &Path, socket: &str) -> Result<Self, SquadError> {
        let guard = switch_lock(root)?;
        let path = root.join(NAME);
        let boards = match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(fail(error)),
            Ok(_) => {
                let file = open(&path, false, false)?;
                let mut bytes = Vec::new();
                file.take(LIMIT + 1).read_to_end(&mut bytes).map_err(fail)?;
                if bytes.len() as u64 > LIMIT {
                    return Err(fail("Pending record exceeds its bound."));
                }
                let value: Value = serde_json::from_slice(&bytes).map_err(fail)?;
                if value["version"] != 1
                    || value["prefix"] != prefix.to_string_lossy().as_ref()
                    || (value["socket"] != socket
                        && !(value["socket"] == ""
                            && value["boards"].as_array().is_some_and(Vec::is_empty)))
                {
                    return Err(fail(
                        "Pending switch belongs to a different installation or tmux socket.",
                    ));
                }
                let boards = value["boards"]
                    .as_array()
                    .ok_or_else(|| fail("Invalid pending switch inventory."))?
                    .clone();
                if boards.len() > 32 {
                    return Err(fail("Pending switch inventory exceeds 32 boards."));
                }
                for board in &boards {
                    let ready = Path::new(string(board, "ready")?);
                    check_ready_path(ready, root)?;
                    if !matches!(
                        board["state"].as_str(),
                        Some("old" | "stopped" | "launching" | "started")
                    ) {
                        return Err(fail("Invalid pending board state."));
                    }
                }
                boards
            }
        };
        Ok(Self {
            _guard: guard,
            root: root.into(),
            prefix: prefix.into(),
            socket: socket.into(),
            boards,
        })
    }
    pub fn add(&mut self, mut board: Value) -> Result<(), SquadError> {
        if self.boards.len() >= 32 {
            return Err(fail("Pending switch inventory exceeds 32 boards."));
        }
        let digest = Sha256::digest(
            format!(
                "{}:{}:{}:{}",
                self.socket, board["pane"], board["pid"], board["start"]
            )
            .as_bytes(),
        );
        let digest: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
        let ready = self.root.join(format!("{READY}{digest}"));
        open(&ready, true, true)?;
        board["ready"] = json!(ready);
        self.boards.push(board);
        Ok(())
    }
    pub fn save(&self) -> Result<(), SquadError> {
        publish_switch_record(
            &self.root,
            NAME,
            &json!({"version":1,"prefix":self.prefix,"socket":self.socket,"boards":self.boards}),
        )
    }
    pub fn finish(self) -> Result<(), SquadError> {
        for board in &self.boards {
            let ready = Path::new(string(board, "ready")?);
            check_ready_path(ready, &self.root)?;
            if let Ok(file) = open(ready, false, false) {
                private(&file)?;
                fs::remove_file(ready).map_err(fail)?;
            }
        }
        fs::remove_file(self.root.join(NAME)).map_err(fail)?;
        File::open(&self.root)
            .and_then(|file| file.sync_all())
            .map_err(fail)
    }
}

fn switch_lock(root: &Path) -> Result<Flock<File>, SquadError> {
    let file = open(&root.join(".ops-board-switch.lock"), true, true)?;
    Flock::lock(file, FlockArg::LockExclusiveNonblock)
        .map_err(|_| fail("Another board switch is running."))
}

fn publish_switch_record(root: &Path, name: &str, value: &Value) -> Result<(), SquadError> {
    let bytes = serde_json::to_vec(value).map_err(fail)?;
    if bytes.len() as u64 > LIMIT {
        return Err(fail("Pending switch record exceeds 1 MiB."));
    }
    let stage = root.join(format!(".ops-board-switch-{}.tmp", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&stage)
        .map_err(fail)?;
    let result = (|| {
        file.write_all(&bytes).map_err(fail)?;
        file.sync_all().map_err(fail)?;
        fs::rename(&stage, root.join(name)).map_err(fail)?;
        File::open(root)
            .and_then(|file| file.sync_all())
            .map_err(fail)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&stage);
    }
    result
}

fn check_ready_path(path: &Path, root: &Path) -> Result<(), SquadError> {
    if path.parent() != Some(root)
        || !path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                name.strip_prefix(READY).is_some_and(|suffix| {
                    suffix.len() == 64 && suffix.bytes().all(|b| b.is_ascii_hexdigit())
                })
            })
    {
        return Err(fail("Invalid board readiness path."));
    }
    Ok(())
}
pub(super) fn ready_file(path: &Path, root: &Path) -> Result<File, SquadError> {
    check_ready_path(path, root)?;
    open(path, true, false)
}
pub(super) fn read_ready(path: &Path) -> Result<String, SquadError> {
    let file = open(path, false, false)?;
    let mut value = String::new();
    file.take(17).read_to_string(&mut value).map_err(fail)?;
    if value.len() > 16 {
        return Err(fail("Invalid readiness marker."));
    }
    Ok(value)
}

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

const OFFER: &str = ".ops-board-switch-offer-v1.json";

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

/// Offer disposition never supplies process authority or replaces pending launches.
fn offered_scopes(root: &Path) -> Result<Vec<Value>, SquadError> {
    let path = root.join(OFFER);
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(fail(error)),
        Ok(_) => (),
    }
    let file = open(&path, false, false)?;
    let mut bytes = Vec::new();
    file.take(LIMIT + 1).read_to_end(&mut bytes).map_err(fail)?;
    if bytes.len() as u64 > LIMIT {
        return Err(fail("Switch-offer record exceeds its bound."));
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(fail)?;
    let scopes = value["scopes"]
        .as_array()
        .filter(|scopes| value["version"] == 1 && scopes.len() <= 32)
        .ok_or_else(|| fail("Invalid switch-offer scopes."))?;
    for scope in scopes {
        if scope["prefix"].as_str().is_none()
            || scope["socket"].as_str().is_none()
            || scope["interactive"].as_bool().is_none()
            || !scope["boards"]
                .as_array()
                .is_some_and(|boards| boards.len() <= 33)
        {
            return Err(fail("Invalid switch-offer inventory."));
        }
    }
    Ok(scopes.clone())
}

pub(super) fn remember_offer(
    root: &Path,
    prefix: &Path,
    socket: &str,
    boards: &[Value],
    interactive: bool,
) -> Result<bool, SquadError> {
    let _guard = switch_lock(root)?;
    let mut scopes = offered_scopes(root)?;
    let same_scope = |scope: &Value| {
        scope["prefix"] == prefix.to_string_lossy().as_ref() && scope["socket"] == socket
    };
    if scopes.iter().any(|scope| {
        same_scope(scope)
            && (scope["interactive"] == interactive || scope["interactive"] == true)
            && boards
                .iter()
                .all(|board| scope["boards"].as_array().unwrap().contains(board))
    }) {
        return Ok(false);
    }
    let previous = scopes
        .iter_mut()
        .find(|scope| same_scope(scope) && scope["interactive"] == interactive);
    let next = json!({"prefix":prefix,"socket":socket,"boards":boards,"interactive":interactive});
    if let Some(previous) = previous {
        *previous = next;
    } else {
        if scopes.len() >= 32 {
            return Err(fail(
                "Switch-offer scopes exceed 32 installations or sockets.",
            ));
        }
        scopes.push(next);
    }
    if boards.len() > 33 {
        return Err(fail("Switch-offer inventory exceeds its bound."));
    }
    publish_switch_record(root, OFFER, &json!({"version":1,"scopes":scopes}))?;
    Ok(true)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn an_offer_is_remembered_for_incarnations_without_touching_launch_progress() {
        let root = std::env::temp_dir().join(format!("ops-switch-offer-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        Record::open(&root, Path::new("/owned"), "/socket")
            .unwrap()
            .save()
            .unwrap();
        let pending = fs::read(root.join(NAME)).unwrap();
        let one = json!({"pid":42,"start":"first","pane":"%1"});
        let next = json!({"pid":42,"start":"replacement","pane":"%1"});
        assert!(
            remember_offer(
                &root,
                Path::new("/owned"),
                "/socket",
                std::slice::from_ref(&one),
                false
            )
            .unwrap()
        );
        assert!(
            remember_offer(
                &root,
                Path::new("/owned"),
                "/socket",
                std::slice::from_ref(&one),
                true
            )
            .unwrap()
        );
        assert!(
            !remember_offer(
                &root,
                Path::new("/owned"),
                "/socket",
                std::slice::from_ref(&one),
                true
            )
            .unwrap()
        );
        assert!(
            remember_offer(
                &root,
                Path::new("/owned"),
                "/socket",
                &[one.clone(), next.clone()],
                true
            )
            .unwrap()
        );
        assert!(
            remember_offer(
                &root,
                Path::new("/owned"),
                "/other",
                std::slice::from_ref(&one),
                true
            )
            .unwrap()
        );
        assert!(!remember_offer(&root, Path::new("/owned"), "/socket", &[one], true).unwrap());
        assert!(
            !remember_offer(
                &root,
                Path::new("/owned"),
                "/socket",
                std::slice::from_ref(&next),
                false
            )
            .unwrap()
        );
        assert_eq!(
            fs::metadata(root.join(OFFER)).unwrap().mode() & 0o777,
            0o600
        );
        assert_eq!(fs::read(root.join(NAME)).unwrap(), pending);
        fs::remove_file(root.join(OFFER)).unwrap();
        let victim = root.join("victim");
        let victim_bytes = serde_json::to_vec(&json!({"version":1,"scopes":[]})).unwrap();
        fs::write(&victim, &victim_bytes).unwrap();
        fs::set_permissions(&victim, fs::Permissions::from_mode(0o600)).unwrap();
        std::os::unix::fs::symlink(&victim, root.join(OFFER)).unwrap();
        assert!(remember_offer(&root, Path::new("/owned"), "/socket", &[next], true).is_err());
        assert_eq!(fs::read(victim).unwrap(), victim_bytes);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn private_record_recovers_progress_and_rejects_links_and_other_sockets() {
        let root = std::env::temp_dir().join(format!("ops-switch-record-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        Record::open(&root, Path::new("/owned"), "")
            .unwrap()
            .save()
            .unwrap();
        let mut record = Record::open(&root, Path::new("/owned"), "/socket").unwrap();
        record
            .add(json!({"pid":42,"start":"start","pane":"%1","state":"stopped"}))
            .unwrap();
        record.save().unwrap();
        drop(record);
        assert!(Record::open(&root, Path::new("/owned"), "/other").is_err());
        let record = Record::open(&root, Path::new("/owned"), "/socket").unwrap();
        assert_eq!(record.boards[0]["state"], "stopped");
        record.finish().unwrap();
        std::os::unix::fs::symlink(root.join("unrelated"), root.join(NAME)).unwrap();
        assert!(Record::open(&root, Path::new("/owned"), "/socket").is_err());
        fs::remove_dir_all(root).unwrap();
    }
}

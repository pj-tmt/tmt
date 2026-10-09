//! The per-client back stack: where each jump came from, kept per tmux server
//! and client in the user's cache directory. It is UI state, not a store:
//! losing or corrupting it only means `back` has nothing to return to.

use crate::{
    core::{Core, SquadError},
    effects::{self, Focus},
};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};

const LIMIT: usize = 32;

/// `$XDG_CACHE_HOME/tmt-ops/back`, else `~/.cache/tmt-ops/back`.
pub fn directory() -> Option<PathBuf> {
    crate::cache::directory("back")
}

/// One client's stack on one server.
pub struct Stack {
    socket: String,
    client: String,
    path: PathBuf,
}

/// FNV-1a: a stable file name for any socket path and client name.
fn fnv64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

impl Stack {
    pub fn new(directory: &Path, socket: &str, client: &str) -> Self {
        let key = fnv64(format!("{socket}\0{client}").as_bytes());
        Self {
            socket: socket.to_owned(),
            client: client.to_owned(),
            path: directory.join(format!("{key:016x}.json")),
        }
    }

    /// Unreadable, corrupt or foreign content reads as empty.
    fn read(&self) -> Vec<String> {
        let Ok(text) = fs::read_to_string(&self.path) else {
            return Vec::new();
        };
        let Ok(document) = serde_json::from_str::<Value>(&text) else {
            return Vec::new();
        };
        if document["socket"] != self.socket.as_str() || document["client"] != self.client.as_str()
        {
            return Vec::new();
        }
        document["from"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|pane| pane.as_str().map(str::to_owned))
            .collect()
    }

    /// Replaces the file atomically, in a directory only the user can read.
    fn write(&self, from: &[String]) -> Result<(), String> {
        let document = json!({"socket": self.socket, "client": self.client, "from": from});
        crate::cache::replace(&self.path, document.to_string().as_bytes())
            .map_err(|error| format!("Could not save the back stack: {error}"))
    }

    /// Records where a jump came from; the oldest entries fall off.
    pub fn push(&self, pane: &str) -> Result<(), String> {
        let mut from = self.read();
        from.push(pane.to_owned());
        let excess = from.len().saturating_sub(LIMIT);
        self.write(&from[excess..])
    }

    /// Takes the newest entry, if any.
    pub fn pop(&self) -> Result<Option<String>, String> {
        let mut from = self.read();
        let Some(pane) = from.pop() else {
            return Ok(None);
        };
        self.write(&from)?;
        Ok(Some(pane))
    }
}

fn stack_for(client: &str) -> Result<Stack, String> {
    let socket = effects::tmux_socket().ok_or("Not inside tmux.")?;
    let directory =
        directory().ok_or("No cache directory: set XDG_CACHE_HOME or HOME to an absolute path.")?;
    Ok(Stack::new(&directory, &socket, client))
}

/// Shows a member and records where the client came from. The jump stands
/// even when recording fails; the returned warning says `back` cannot return.
pub fn jump(core: &Core, member: &str) -> Result<(Focus, Option<String>), SquadError> {
    let focus = effects::focus(core, member)?;
    let warning = match (&focus.from, focus.client.is_empty()) {
        (Some(from), false) => stack_for(&focus.client)
            .and_then(|stack| stack.push(from))
            .err(),
        _ => Some("tmt did not report the client or the pane it left.".into()),
    };
    Ok((focus, warning))
}

/// Returns the invoker's client to where its last jump came from, or `None`
/// when nothing is recorded for it. The client is resolved by core, exactly
/// as a focus would; a stale entry is dropped and its failure reported.
pub fn back(core: &Core) -> Result<Option<Focus>, SquadError> {
    let view = core.json(&["focus", "--client"])?;
    let client = view["client"].as_str().unwrap_or_default();
    let failed = |message: String| SquadError::new("SQUAD_ACTION_FAILED", message);
    let stack = stack_for(client).map_err(failed)?;
    let Some(pane) = stack.pop().map_err(failed)? else {
        return Ok(None);
    };
    effects::focus(core, &pane).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn scratch(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("tmt-ops-back-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        path
    }

    #[test]
    fn each_client_on_each_server_has_its_own_bounded_stack() {
        let directory = scratch("stacks").join("back");
        let mine = Stack::new(&directory, "/tmp/tmux-501/default", "/dev/ttys004");
        let other_client = Stack::new(&directory, "/tmp/tmux-501/default", "/dev/ttys005");
        let other_server = Stack::new(&directory, "/tmp/tmux-501/work", "/dev/ttys004");
        assert_eq!(mine.pop(), Ok(None), "nothing to go back to");
        mine.push("%1").unwrap();
        mine.push("%2").unwrap();
        other_client.push("%9").unwrap();
        assert_eq!(other_server.pop(), Ok(None));
        assert_eq!(mine.pop(), Ok(Some("%2".into())));
        assert_eq!(mine.pop(), Ok(Some("%1".into())));
        assert_eq!(mine.pop(), Ok(None));
        assert_eq!(other_client.pop(), Ok(Some("%9".into())));
        let mode = fs::metadata(&directory).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);

        for pane in 0..40 {
            mine.push(&format!("%{pane}")).unwrap();
        }
        assert_eq!(mine.read().len(), LIMIT);
        assert_eq!(mine.read()[0], "%8", "the oldest entries fall off");
        let _ = fs::remove_dir_all(directory.parent().unwrap());
    }

    #[test]
    fn corrupt_or_foreign_files_read_as_empty_and_are_replaced() {
        let directory = scratch("corrupt");
        let stack = Stack::new(&directory, "/s", "c");
        fs::create_dir_all(&directory).unwrap();
        fs::write(&stack.path, "{not json").unwrap();
        assert_eq!(stack.pop(), Ok(None));
        fs::write(
            &stack.path,
            r#"{"socket":"/other","client":"c","from":["%1"]}"#,
        )
        .unwrap();
        assert_eq!(stack.pop(), Ok(None), "a colliding file is never trusted");
        stack.push("%3").unwrap();
        assert_eq!(stack.pop(), Ok(Some("%3".into())));
        let leftovers: Vec<_> = fs::read_dir(&directory)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
        let _ = fs::remove_dir_all(directory);
    }
}

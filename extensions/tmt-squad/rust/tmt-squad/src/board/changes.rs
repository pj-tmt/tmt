//! What lets the board reload before its interval: core's change cursor
//! (`tmt api changes.cursor`) and squad.toml's file stamp. Neither replaces
//! the interval: the cursor does not cover pane presence or notebooks, and a
//! core without it only has the interval.

use crate::core::Core;
use serde_json::{Value, json};
use std::{path::PathBuf, time::SystemTime};

/// The two change signals as last read. A signal that could not be read is
/// None and never counts as a change on its own.
#[derive(Debug, Clone, PartialEq)]
pub struct Stamp {
    cursor: Option<Value>,
    /// squad.toml's modification time and length; None when it is missing.
    config: Option<(SystemTime, u64)>,
    /// The field provider cache directory's modification time: it moves
    /// when a provider run saves new values.
    fields: Option<SystemTime>,
}

impl Stamp {
    /// A stamp with only a cursor, for tests of what reads it.
    #[cfg(test)]
    pub fn cursor(cursor: u64) -> Self {
        Self {
            cursor: Some(json!(cursor)),
            config: None,
            fields: None,
        }
    }

    /// Whether `now` shows a change since `self`. squad.toml appearing,
    /// disappearing or being rewritten counts; the cursor counts only when
    /// both reads succeeded, so a failed read never triggers a reload.
    pub fn moved(&self, now: &Stamp) -> bool {
        self.config != now.config
            || self.fields != now.fields
            || matches!((&self.cursor, &now.cursor), (Some(then), Some(now)) if then != now)
    }
}

pub struct Changes {
    core: Core,
    config: Option<PathBuf>,
    fields: Option<PathBuf>,
    /// False once core reports it has no change cursor; never asked again.
    cursor: bool,
}

impl Changes {
    /// `config` is squad.toml's path, when it could be found; `fields` the
    /// field provider cache directory.
    pub fn new(core: Core, config: Option<PathBuf>, fields: Option<PathBuf>) -> Self {
        Self {
            core,
            config,
            fields,
            cursor: true,
        }
    }

    pub fn stamp(&mut self) -> Stamp {
        Stamp {
            cursor: self.cursor(),
            config: self.config.as_ref().and_then(|path| {
                let metadata = std::fs::metadata(path).ok()?;
                Some((metadata.modified().ok()?, metadata.len()))
            }),
            fields: self
                .fields
                .as_ref()
                .and_then(|path| std::fs::metadata(path).ok()?.modified().ok()),
        }
    }

    fn cursor(&mut self) -> Option<Value> {
        if !self.cursor {
            return None;
        }
        match self.core.api("changes.cursor", json!({})) {
            Ok(document) => document.get("cursor").cloned(),
            // An older core: it has no such operation.
            Err(error) if error.code == "API_INPUT_INVALID" => {
                self.cursor = false;
                None
            }
            Err(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in tmt whose `api` prints the cursor file's contents as the
    /// cursor, or a core error when the file holds `error <code>`; each call
    /// is logged.
    fn fake(name: &str) -> (Changes, PathBuf, PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("squad-changes-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (cursor, log, config) = (
            dir.join("cursor"),
            dir.join("calls"),
            dir.join("squad.toml"),
        );
        let fake = dir.join("tmt");
        crate::test_support::write_executable(
            &fake,
            &format!(
                "#!/bin/sh\ncat >> '{log}'; echo >> '{log}'\n\
                 set -- $(cat '{cursor}')\n\
                 if [ \"$1\" = error ]; then\n\
                 echo \"{{\\\"error\\\":{{\\\"code\\\":\\\"$2\\\",\\\"message\\\":\\\"no\\\"}}}}\"; exit 2\n\
                 fi\n\
                 echo \"{{\\\"cursor\\\":$1}}\"\n",
                log = log.display(),
                cursor = cursor.display(),
            ),
        );
        std::fs::write(&cursor, "7").unwrap();
        let changes = Changes::new(
            Core::at(fake),
            Some(config.clone()),
            Some(dir.join("fields")),
        );
        (changes, cursor, config, dir)
    }

    fn calls(dir: &std::path::Path) -> usize {
        std::fs::read_to_string(dir.join("calls"))
            .unwrap_or_default()
            .lines()
            .count()
    }

    #[test]
    fn a_new_cursor_or_a_rewritten_squad_toml_is_a_change_and_a_repeat_is_not() {
        let (mut changes, cursor, config, dir) = fake("moved");
        let first = changes.stamp();
        assert_eq!(first.cursor, Some(json!(7)));
        assert!(first.config.is_none(), "no squad.toml yet");
        assert!(!first.moved(&changes.stamp()));
        let request = std::fs::read_to_string(dir.join("calls")).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(request.lines().next().unwrap()).unwrap(),
            json!({"version": 1, "operation": "changes.cursor", "input": {}}),
            "one public API request, strict empty input"
        );

        std::fs::write(&cursor, "8").unwrap();
        let second = changes.stamp();
        assert!(first.moved(&second), "the cursor advanced");

        std::fs::write(&config, "[board]\n").unwrap();
        let third = changes.stamp();
        assert!(second.moved(&third), "squad.toml appeared");
        std::fs::write(&config, "[board]\nrefresh = \"2s\"\n").unwrap();
        assert!(third.moved(&changes.stamp()), "squad.toml was rewritten");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_failed_read_is_no_change_and_an_older_core_is_never_asked_again() {
        let (mut changes, cursor, _config, dir) = fake("fallback");
        let before = changes.stamp();
        std::fs::write(&cursor, "error SQUAD_CORE_UNAVAILABLE").unwrap();
        let failed = changes.stamp();
        assert_eq!(failed.cursor, None);
        assert!(!before.moved(&failed) && !failed.moved(&before));
        assert!(changes.cursor, "a transient failure keeps asking");

        std::fs::write(&cursor, "error API_INPUT_INVALID").unwrap();
        assert_eq!(changes.stamp().cursor, None);
        let asked = calls(&dir);
        std::fs::write(&cursor, "9").unwrap();
        assert_eq!(changes.stamp().cursor, None, "the fallback is permanent");
        assert_eq!(calls(&dir), asked, "and costs no more core calls");
        let _ = std::fs::remove_dir_all(dir);
    }
}

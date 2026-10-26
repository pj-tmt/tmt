//! What lets the board reload before its interval: core's change cursor
//! (`tmt api changes.cursor`) and ops.toml's file stamp. Neither replaces
//! the interval: the cursor does not cover pane presence or notebooks, and a
//! core without it only has the interval.

use crate::core::Core;
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    time::{Duration, Instant, SystemTime},
};

/// The two change signals as last read. A signal that could not be read is
/// None and never counts as a change on its own.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Stamp {
    cursor: Option<Value>,
    /// Core's usage cursor: the sampler's writes, which do not wake the board (#5019).
    usage: Option<Value>,
    /// ops.toml's modification time and length; None when it is missing.
    config: Option<(SystemTime, u64)>,
    layout: Option<PathBuf>,
    /// The field provider cache directory's modification time: it moves
    /// when a provider run saves new values.
    fields: Option<SystemTime>,
}

impl Stamp {
    /// Only successful durable observations admit reuse. Usage has its own
    /// sampling owner; local config/provider stamps invalidate projections too.
    pub(super) fn reusable(&self, now: &Stamp) -> bool {
        self.has_cursor() && now.has_cursor() && !self.moved(now)
    }

    pub(super) fn has_cursor(&self) -> bool {
        self.cursor.as_ref().and_then(Value::as_u64).is_some()
    }
    /// A cutover changes the watched file even when its bytes/stamp match.
    pub(super) fn layout_moved(&self, now: &Stamp) -> bool {
        self.layout != now.layout
    }

    /// History is stored in core tables; unlike pane/notebook freshness it may
    /// reuse a successful durable cursor within the same closed time bucket. It
    /// changes with either cursor: usage rows are its evidence, the roster and
    /// bindings its causes. Both only grow, so their sum moves exactly when one does.
    pub(super) fn history_cursor(&self) -> Option<u64> {
        let cursor = self.cursor.as_ref().and_then(Value::as_u64)?;
        // A core without the usage cursor counts those writes in `cursor`.
        let usage = self.usage.as_ref().and_then(Value::as_u64).unwrap_or(0);
        Some(cursor.saturating_add(usage))
    }

    #[cfg(test)]
    pub(super) fn with_layout(mut self, path: &str) -> Self {
        self.layout = Some(PathBuf::from(path));
        self
    }

    /// A stamp with only a cursor, for tests of what reads it.
    #[cfg(test)]
    pub fn cursor(cursor: u64) -> Self {
        Self {
            cursor: Some(json!(cursor)),
            usage: None,
            config: None,
            layout: None,
            fields: None,
        }
    }

    /// Whether `now` shows a change since `self`. ops.toml appearing,
    /// disappearing or being rewritten counts; the cursor counts only when
    /// both reads succeeded, so a failed read never triggers a reload. The usage
    /// cursor never counts: usage views refresh on their own clock.
    pub fn moved(&self, now: &Stamp) -> bool {
        self.layout_moved(now)
            || self.config != now.config
            || self.fields != now.fields
            || matches!((&self.cursor, &now.cursor), (Some(then), Some(now)) if then != now)
    }
}

pub struct Changes {
    config: Option<PathBuf>,
    fields: Option<PathBuf>,
    /// False once core reports it has no change cursor; never asked again.
    cursor: bool,
    latest: Option<(Instant, Option<Value>, Option<Value>)>,
}

impl Changes {
    /// `config` is ops.toml's path, when it could be found; `fields` the
    /// field provider cache directory.
    pub fn new(config: Option<PathBuf>, fields: Option<PathBuf>) -> Self {
        Self {
            config,
            fields,
            cursor: true,
            latest: None,
        }
    }

    pub(super) fn retarget(&mut self, config: Option<PathBuf>) {
        if self.config != config {
            self.latest = None;
        }
        self.config = config;
    }

    /// Refresh-off views observe promotion without polling Core or the filesystem.
    pub(super) fn layout_stamp(&self) -> Stamp {
        Stamp {
            layout: self.config.clone(),
            ..Stamp::default()
        }
    }

    pub fn stamp(&mut self, core: &Core) -> Stamp {
        self.stamp_at(core, Instant::now(), Duration::ZERO)
    }

    /// Poll and pre-load checks share one cursor observation per second; file
    /// stamps are always read. Explicit generation changes use `stamp` instead.
    pub(super) fn stamp_cached(&mut self, core: &Core) -> Stamp {
        self.stamp_at(core, Instant::now(), Duration::from_secs(1))
    }

    fn stamp_at(&mut self, core: &Core, now: Instant, every: Duration) -> Stamp {
        let (cursor, usage) = match &self.latest {
            Some((seen, cursor, usage)) if now.saturating_duration_since(*seen) < every => {
                (cursor.clone(), usage.clone())
            }
            _ => {
                let (cursor, usage) = self.cursors(core);
                self.latest = Some((now, cursor.clone(), usage.clone()));
                (cursor, usage)
            }
        };
        Stamp {
            layout: self.config.clone(),
            cursor,
            usage,
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

    /// The change cursor and the usage cursor, from one request. A core before the
    /// usage cursor returns only the first.
    fn cursors(&mut self, core: &Core) -> (Option<Value>, Option<Value>) {
        if !self.cursor {
            return (None, None);
        }
        match core.api("changes.cursor", json!({})) {
            Ok(document) => (
                document.get("cursor").cloned(),
                document.get("usage").cloned(),
            ),
            // An older core: it has no such operation.
            Err(error) if error.code == "API_INPUT_INVALID" => {
                self.cursor = false;
                (None, None)
            }
            Err(_) => (None, None),
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
        let (cursor, log, config) = (dir.join("cursor"), dir.join("calls"), dir.join("ops.toml"));
        let fake = dir.join("tmt");
        crate::test_support::write_ready_executable(
            &fake,
            &format!(
                "#!/bin/sh\ncat >> '{log}'; echo >> '{log}'\n\
                 set -- $(cat '{cursor}')\n\
                 if [ \"$1\" = error ]; then\n\
                 echo \"{{\\\"error\\\":{{\\\"code\\\":\\\"$2\\\",\\\"message\\\":\\\"no\\\"}}}}\"; exit 2\n\
                 fi\n\
                 if [ -n \"$2\" ]; then echo \"{{\\\"cursor\\\":$1,\\\"usage\\\":$2}}\"; else echo \"{{\\\"cursor\\\":$1}}\"; fi\n",
                log = log.display(),
                cursor = cursor.display(),
            ),
        );
        std::fs::write(&cursor, "7").unwrap();
        let changes = Changes::new(Some(config.clone()), Some(dir.join("fields")));
        (changes, cursor, config, dir)
    }

    fn calls(dir: &std::path::Path) -> usize {
        std::fs::read_to_string(dir.join("calls"))
            .unwrap_or_default()
            .lines()
            .count()
    }

    #[test]
    fn cached_cursor_checks_keep_local_stamps_fresh_and_never_hide_the_next_poll() {
        let (mut changes, cursor, config, dir) = fake("cached");
        let core = Core::at(dir.join("tmt"));
        let start = Instant::now();
        let every = Duration::from_secs(1);
        let first = changes.stamp_at(&core, start, every);
        std::fs::write(&cursor, "8").unwrap();
        std::fs::write(&config, "[board]\n").unwrap();
        let immediate = changes.stamp_at(&core, start + Duration::from_millis(500), every);
        assert_eq!(first.cursor, immediate.cursor);
        assert!(
            first.moved(&immediate),
            "local file changes are independent of Core polling"
        );
        assert_eq!(calls(&dir), 1, "pre-load check reuses this poll");
        let next = changes.stamp_at(&core, start + every, every);
        assert_eq!(
            next.cursor,
            Some(json!(8)),
            "change during acquisition appears next poll"
        );
        assert_eq!(calls(&dir), 2);
        assert!(!first.reusable(&next));
        assert!(next.reusable(&next));
        assert!(!Stamp::default().reusable(&Stamp::default()));
        assert!(
            next.reusable(&Stamp {
                usage: Some(json!(999)),
                ..next.clone()
            }),
            "usage has its own clock"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_new_cursor_or_a_rewritten_squad_toml_is_a_change_and_a_repeat_is_not() {
        let (mut changes, cursor, config, dir) = fake("moved");
        let core = Core::at(dir.join("tmt"));
        let first = changes.stamp(&core);
        assert_eq!(first.cursor, Some(json!(7)));
        assert!(first.config.is_none(), "no ops.toml yet");
        assert!(!first.moved(&changes.stamp(&core)));
        let request = std::fs::read_to_string(dir.join("calls")).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(request.lines().next().unwrap()).unwrap(),
            json!({"version": 1, "operation": "changes.cursor", "input": {}}),
            "one public API request, strict empty input"
        );

        std::fs::write(&cursor, "8").unwrap();
        let second = changes.stamp(&core);
        assert!(first.moved(&second), "the cursor advanced");

        std::fs::write(&config, "[board]\n").unwrap();
        let third = changes.stamp(&core);
        assert!(second.moved(&third), "ops.toml appeared");
        std::fs::write(&config, "[board]\nrefresh = \"2s\"\n").unwrap();
        assert!(third.moved(&changes.stamp(&core)), "ops.toml was rewritten");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_usage_cursor_move_wakes_nothing_but_still_moves_the_history_cursor() {
        let (mut changes, cursor, _config, dir) = fake("usage");
        let core = Core::at(dir.join("tmt"));
        std::fs::write(&cursor, "7 1").unwrap();
        let first = changes.stamp(&core);
        std::fs::write(&cursor, "7 2").unwrap();
        let sampled = changes.stamp(&core);
        assert!(!first.moved(&sampled), "a sampler write is not a reload");
        assert_ne!(
            first.history_cursor(),
            sampled.history_cursor(),
            "but the usage history it left is new"
        );

        std::fs::write(&cursor, "8 2").unwrap();
        let roster = changes.stamp(&core);
        assert!(
            sampled.moved(&roster),
            "a roster, request or focus change is"
        );
        assert_ne!(sampled.history_cursor(), roster.history_cursor());

        // A core before the usage cursor counts those writes in `cursor` alone.
        std::fs::write(&cursor, "9").unwrap();
        assert_eq!(changes.stamp(&core).history_cursor(), Some(9));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_failed_read_is_no_change_and_an_older_core_is_never_asked_again() {
        let (mut changes, cursor, _config, dir) = fake("fallback");
        let core = Core::at(dir.join("tmt"));
        let before = changes.stamp(&core);
        std::fs::write(&cursor, "error SQUAD_CORE_UNAVAILABLE").unwrap();
        let failed = changes.stamp(&core);
        assert_eq!(failed.cursor, None);
        assert!(!before.moved(&failed) && !failed.moved(&before));
        assert!(changes.cursor, "a transient failure keeps asking");

        std::fs::write(&cursor, "error API_INPUT_INVALID").unwrap();
        assert_eq!(changes.stamp(&core).cursor, None);
        let asked = calls(&dir);
        std::fs::write(&cursor, "9").unwrap();
        assert_eq!(
            changes.stamp(&core).cursor,
            None,
            "the fallback is permanent"
        );
        assert_eq!(calls(&dir), asked, "and costs no more core calls");
        let _ = std::fs::remove_dir_all(dir);
    }
    #[test]
    fn retargeting_missing_config_is_a_layout_change_even_with_identical_metadata() {
        let (mut changes, _, _, dir) = fake("retarget");
        let core = Core::at(dir.join("tmt"));
        changes.retarget(Some(dir.join("squad.toml")));
        let before = changes.stamp(&core);
        changes.retarget(Some(dir.join("ops.toml")));
        let next = changes.stamp(&core);
        assert_eq!(before.config, next.config);
        assert!(before.layout_moved(&next));
        assert!(before.moved(&next));
        assert!(!next.layout_moved(&changes.stamp(&core)));
        std::fs::remove_dir_all(dir).unwrap();
    }
}

//! Activity: the one board projection of core's public `session.activity`.
//! Every surface reads this typed value; none re-parses the row.
use serde_json::Value;

/// Core's admitted phase. `waiting` is not a public core phase yet, so it
/// parses as unknown like any other unrecognized token.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    Working,
    Idle,
    Ended,
    #[default]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ObservedActivity {
    pub phase: Phase,
    /// When core admitted the current phase; only a running phase has one.
    pub since_ms: Option<u64>,
    /// When core admitted the last transition. It is not a heartbeat.
    pub last_ms: Option<u64>,
}

/// The largest integer a JSON producer can carry exactly.
const MAX_SAFE_MS: u64 = (1 << 53) - 1;

impl ObservedActivity {
    /// Fails closed per row: a missing or malformed document is unknown, and a
    /// timestamp that is not a safe integer is absent.
    pub fn of(row: &Value) -> Self {
        let document = &row["session"]["activity"];
        let ms = |key: &str| document[key].as_u64().filter(|ms| *ms <= MAX_SAFE_MS);
        let phase = match document["state"].as_str() {
            Some("working") => Phase::Working,
            Some("idle") => Phase::Idle,
            Some("ended") => Phase::Ended,
            _ => Phase::Unknown,
        };
        Self {
            phase,
            since_ms: ms("sinceMs").filter(|_| matches!(phase, Phase::Working | Phase::Idle)),
            last_ms: ms("lastActivityMs"),
        }
    }
}

/// A member row's STATE cell inputs: core activity first, then the request
/// attention and the squad task state, which stay human facts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Member {
    pub activity: ObservedActivity,
    /// A request waits on the user: the only waiting evidence today.
    pub waiting: bool,
    /// Core's idle alert marks this member over its threshold: the squad's alert
    /// is on, the idle phase is older than the threshold, and nothing excepts it.
    pub idle_over: bool,
    pub task: Option<String>,
}

/// The words of one STATE cell. Every surface paints these; none composes its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Label {
    pub state: &'static str,
    /// How long the phase has run (`12m`), or when core last saw an event (`4m ago`).
    pub age: Option<String>,
    /// The core time `age` counts from, so a frame clock can follow it.
    pub at: Option<u64>,
    /// The idle alert is over for this member while it shows as idle: the cell
    /// marks it `◷` in the attention colour and the word `idle` still follows.
    pub idle_over: bool,
    /// The sanitized task state, even when it repeats the activity word: only
    /// the painter knows which primary words it shows, so it removes a repeat.
    pub task: Option<String>,
}

impl Member {
    /// `stale` means the board's read is too old or failed; it never becomes idle.
    pub fn label(&self, stale: bool, now_ms: u64) -> Label {
        let activity = self.activity;
        let (state, since) = if self.waiting {
            ("waiting", None)
        } else if stale {
            ("stale", activity.last_ms)
        } else {
            match activity.phase {
                Phase::Working => ("working", activity.since_ms),
                Phase::Idle => ("idle", activity.since_ms),
                Phase::Ended => ("ended", activity.last_ms),
                // Unknown shows no age: its cell is the muted `?` alone.
                Phase::Unknown => ("unknown", None),
            }
        };
        let running = matches!(state, "working" | "idle");
        // A future time (clock rollback) shows no age rather than a made-up one.
        let at = since.filter(|at| *at <= now_ms);
        let age = at.map(|at| {
            let age = crate::requests::age(now_ms, at);
            if running { age } else { format!("{age} ago") }
        });
        let task = self
            .task
            .as_deref()
            .map(crate::board::notes::sanitize)
            .filter(|task| !task.is_empty());
        Label {
            state,
            age,
            at,
            idle_over: self.idle_over && state == "idle",
            task,
        }
    }
}

impl Label {
    /// The shown state word: unknown activity has none, only the `?` that the
    /// glyph also shows, so it never reads as a fault beside the task state.
    pub fn word(&self) -> &'static str {
        if self.state == "unknown" {
            "?"
        } else {
            self.state
        }
    }

    /// Fits `width` cells as the state words and the muted task text. The task
    /// is dropped whole first, then the age; the state word stays. A task that
    /// repeats the state word is left out.
    pub fn fit(&self, width: usize) -> (String, Option<String>) {
        use unicode_width::UnicodeWidthStr;
        let word = self.word();
        let head = match &self.age {
            Some(age) => format!("{word} {age}"),
            None => word.to_owned(),
        };
        if let Some(task) = self.task.as_ref().filter(|task| *task != self.state) {
            let tail = if self.state == "unknown" {
                format!(" {task}")
            } else {
                format!(" · {task}")
            };
            let room = width.saturating_sub(head.width());
            if tail.width() <= room {
                return (head, Some(tail));
            }
            // A suffix that does not fit whole is dropped, never cut inside a word.
        }
        if head.width() <= width {
            return (head, None);
        }
        if word.width() <= width {
            return (word.to_owned(), None);
        }
        (crate::board::view::fit(word, width), None)
    }
}

/// How old the board's core read may be before its activity is stale:
/// `max(refresh interval, 5 s) + 10 s`. Refresh off uses the 5 s floor.
pub fn stale_after(refresh: Option<std::time::Duration>) -> std::time::Duration {
    const FLOOR: std::time::Duration = std::time::Duration::from_secs(5);
    const GRACE: std::time::Duration = std::time::Duration::from_secs(10);
    refresh.unwrap_or_default().max(FLOOR) + GRACE
}

/// Agent counts by phase over an identity set the caller has already
/// deduplicated; the tally itself takes no ID. Ended is neither working nor idle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tally {
    pub working: usize,
    pub idle: usize,
    pub unknown: usize,
}

impl Tally {
    /// A stale read counts as unknown, never as working or idle.
    pub fn add(&mut self, activity: ObservedActivity, stale: bool) {
        if stale {
            self.unknown += 1;
            return;
        }
        match activity.phase {
            Phase::Working => self.working += 1,
            Phase::Idle => self.idle += 1,
            Phase::Unknown => self.unknown += 1,
            Phase::Ended => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn of(activity: Value) -> ObservedActivity {
        ObservedActivity::of(&json!({"id": "a", "session": {"activity": activity}}))
    }

    #[test]
    fn admitted_phases_keep_their_times() {
        for (state, phase) in [("working", Phase::Working), ("idle", Phase::Idle)] {
            assert_eq!(
                of(json!({"state": state, "sinceMs": 10, "lastActivityMs": 12, "providers": {}})),
                ObservedActivity {
                    phase,
                    since_ms: Some(10),
                    last_ms: Some(12)
                }
            );
        }
        // Core gives an ended or unknown phase no start time; one is ignored.
        for (state, phase) in [("ended", Phase::Ended), ("unknown", Phase::Unknown)] {
            assert_eq!(
                of(json!({"state": state, "sinceMs": 10, "lastActivityMs": 12})),
                ObservedActivity {
                    phase,
                    since_ms: None,
                    last_ms: Some(12)
                }
            );
        }
    }

    #[test]
    fn malformed_documents_fail_closed_per_row() {
        let unknown = ObservedActivity::default();
        assert_eq!(ObservedActivity::of(&json!({"id": "a"})), unknown);
        assert_eq!(
            ObservedActivity::of(&json!({"session": "working"})),
            unknown
        );
        assert_eq!(of(json!(null)), unknown);
        assert_eq!(of(json!({"state": 1})), unknown);
        // A provider waiting phase is slice B's; until core publishes it, it is unknown.
        assert_eq!(of(json!({"state": "waiting", "sinceMs": 5})), unknown);
        assert_eq!(of(json!({"state": "Working"})), unknown);
        for bad in [json!(-1), json!(1.5), json!("12"), json!(MAX_SAFE_MS + 1)] {
            assert_eq!(
                of(json!({"state": "idle", "sinceMs": bad, "lastActivityMs": bad})),
                ObservedActivity {
                    phase: Phase::Idle,
                    since_ms: None,
                    last_ms: None
                }
            );
        }
        assert_eq!(
            of(json!({"state": "idle", "sinceMs": MAX_SAFE_MS, "lastActivityMs": 0})).since_ms,
            Some(MAX_SAFE_MS)
        );
    }

    const NOW: u64 = 60 * 60 * 1000;

    fn member(phase: Phase, since: Option<u64>, last: Option<u64>, task: Option<&str>) -> Member {
        Member {
            activity: ObservedActivity {
                phase,
                since_ms: since,
                last_ms: last,
            },
            waiting: false,
            idle_over: false,
            task: task.map(str::to_owned),
        }
    }

    fn words(label: &Label, width: usize) -> String {
        let (head, task) = label.fit(width);
        head + task.as_deref().unwrap_or("")
    }

    #[test]
    fn labels_put_core_activity_first_and_the_task_second() {
        let working = member(
            Phase::Working,
            Some(NOW - 12 * 60_000),
            Some(1),
            Some("review"),
        );
        assert_eq!(
            words(&working.label(false, NOW), 80),
            "working 12m · review"
        );
        let idle = member(Phase::Idle, Some(NOW - 5_000), None, Some("idle"));
        assert_eq!(
            words(&idle.label(false, NOW), 80),
            "idle 5s",
            "an equal task is omitted"
        );
        let ended = member(Phase::Ended, None, Some(NOW - 4 * 60_000), None);
        assert_eq!(words(&ended.label(false, NOW), 80), "ended 4m ago");
        // Unknown has no word and no age: the `?` and then the task state.
        let unknown = member(Phase::Unknown, None, Some(NOW - 60_000), Some("testing"));
        assert_eq!(words(&unknown.label(false, NOW), 80), "? testing");
        assert_eq!(unknown.label(false, NOW).state, "unknown");
        let bare = member(Phase::Unknown, None, None, None);
        assert_eq!(words(&bare.label(false, NOW), 80), "?");
        // A request that waits on the user wins; a stale read never becomes idle.
        let waiting = Member {
            waiting: true,
            ..working.clone()
        };
        assert_eq!(words(&waiting.label(true, NOW), 80), "waiting · review");
        assert_eq!(
            words(&working.label(true, NOW), 80),
            "stale 59m ago · review",
            "stale shows when core last saw an event, not the phase age"
        );
        assert_eq!(words(&idle.label(true, NOW), 80), "stale · idle");
    }

    #[test]
    fn ages_are_never_invented() {
        // A future time (clock rollback) shows no age; nothing subtracts past zero.
        let future = member(Phase::Working, Some(NOW + 1), Some(NOW + 1), None);
        assert_eq!(future.label(false, NOW).age, None);
        assert_eq!(future.label(true, NOW).age, None);
        // A start after the last event is checked on its own against now.
        let since_after_last = member(Phase::Idle, Some(NOW - 60_000), Some(NOW - 120_000), None);
        assert_eq!(
            since_after_last.label(false, NOW).age.as_deref(),
            Some("1m")
        );
        assert_eq!(
            since_after_last.label(true, NOW).age.as_deref(),
            Some("2m ago")
        );
        let missing = member(Phase::Working, None, None, None);
        assert_eq!(missing.label(false, NOW).age, None);
        assert_eq!(missing.label(true, NOW).age, None);
    }

    #[test]
    fn fitting_drops_the_whole_task_first_then_the_age() {
        let label = member(
            Phase::Working,
            Some(NOW - 12 * 60_000),
            None,
            Some("review"),
        )
        .label(false, NOW);
        assert_eq!(
            label.fit(20),
            ("working 12m".into(), Some(" · review".into()))
        );
        // Never a word cut in the middle: a suffix that does not fit is dropped whole.
        assert_eq!(label.fit(19), ("working 12m".into(), None));
        assert_eq!(label.fit(16), ("working 12m".into(), None));
        assert_eq!(label.fit(15), ("working 12m".into(), None));
        assert_eq!(label.fit(11), ("working 12m".into(), None));
        assert_eq!(label.fit(10), ("working".into(), None));
        assert_eq!(label.fit(7), ("working".into(), None));
        assert_eq!(label.fit(4).0.chars().count(), 4);
        let hostile = Member {
            task: Some("\u{1b}[31mred".into()),
            ..member(Phase::Idle, None, None, None)
        };
        assert!(!words(&hostile.label(false, NOW), 80).contains('\u{1b}'));
    }

    #[test]
    fn tallies_skip_ended_agents_and_count_stale_reads_as_unknown() {
        let mut tally = Tally::default();
        for phase in [Phase::Working, Phase::Idle, Phase::Ended, Phase::Unknown] {
            tally.add(
                ObservedActivity {
                    phase,
                    ..Default::default()
                },
                false,
            );
        }
        assert_eq!(
            tally,
            Tally {
                working: 1,
                idle: 1,
                unknown: 1
            }
        );
        // A stale read counts every agent as unknown, never working or idle.
        let mut stale = Tally::default();
        for phase in [Phase::Working, Phase::Idle, Phase::Ended] {
            stale.add(
                ObservedActivity {
                    phase,
                    ..Default::default()
                },
                true,
            );
        }
        assert_eq!(
            stale,
            Tally {
                working: 0,
                idle: 0,
                unknown: 3
            }
        );
    }
}

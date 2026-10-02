//! One squad's status, read once for `ls` and the board alike. The raw reads
//! happen under the staleness observer's lock, so an older snapshot never
//! overwrites a newer observation; the document is then built from them.
//! `staleness` owns what is observed and published; this module only decides
//! which reads run and in what order.

use crate::{
    config::{Layout, Reminders, Section, States},
    core::{Core, SquadError},
    me::Me,
    provider,
    requests::{self, Sent, Window},
    rows::Rows,
    squad::{Member, Squad},
    staleness, status,
};
use serde_json::{Value, json};
use std::path::Path;

/// Which optional reads the caller needs beyond the observation's own.
#[derive(Debug, Clone, Copy)]
pub struct Reads {
    /// Every identity's metadata, for `meta.*` column sources.
    pub metadata: bool,
    /// The lead's notes even when nothing is observed: the board's notes pane.
    pub notes: bool,
}

/// Ordinary status reads or the verified lead's reminder, without live presence.
#[derive(Debug, Clone, Copy)]
pub enum Mode<'a> {
    Read(Reads),
    /// The roster must independently establish this identity as the only lead.
    Reminder {
        lead: &'a str,
    },
}

/// The raw reads of one squad, and their observed ages.
pub struct Observation {
    pub members: Vec<Member>,
    /// Field providers' cached values. Nothing runs here; a caller may refresh
    /// them before building the document.
    pub cached: provider::Cache,
    /// The lead's `notes.read`, when it was read.
    pub notes: Option<Result<Value, SquadError>>,
    room: Option<Window>,
    snapshot: staleness::Snapshot,
    /// Generations durably claimed before handing context to the hook.
    pub reminder: Option<staleness::Reminder>,
}

/// Reads the squad under the observer's lock and records what it saw. The
/// lead's notes and the room history are read for the observation only when
/// it can be published; a busy or disabled observer reports unknown ages.
pub fn observe(
    core: &Core,
    config: &Path,
    squad: &Squad,
    reminders: Reminders,
    providers: &[provider::Provider],
    mode: Mode<'_>,
) -> Result<Observation, SquadError> {
    observe_with(core, squad, providers, mode, || {
        staleness::Observer::begin(config, squad, reminders)
    })
}

fn observe_with(
    core: &Core,
    squad: &Squad,
    providers: &[provider::Provider],
    mode: Mode<'_>,
    begin: impl FnOnce() -> staleness::Observer,
) -> Result<Observation, SquadError> {
    let observer = begin();
    let members = match mode {
        Mode::Read(reads) => squad.members(core, reads.metadata)?,
        Mode::Reminder { .. } => squad.roster(core)?,
    };
    let cached = provider::Cache::load(&squad.name);
    let wants_notes = matches!(mode, Mode::Read(Reads { notes: true, .. }));
    let notes = (observer.active() || wants_notes)
        .then(|| members.iter().find(|member| member.is_lead()))
        .flatten()
        .map(|lead| core.api("notes.read", json!({"identityId": lead.id})));
    let room = if observer.active() {
        requests::room_window(core, squad).ok()
    } else {
        None
    };
    let input = staleness::Input {
        members: &members,
        providers,
        fields: &cached,
        notes: notes.as_ref().and_then(|notes| notes.as_ref().ok()),
        room: room.as_ref(),
        now: status::now_ms(),
    };
    let (snapshot, reminder) = match mode {
        Mode::Read(_) => (
            observer.record(
                input.members,
                input.providers,
                input.fields,
                input.notes,
                input.room,
                input.now,
            ),
            None,
        ),
        Mode::Reminder { lead } => observer.record_for_reminder(input, lead),
    };
    Ok(Observation {
        members,
        cached,
        notes,
        room,
        snapshot,
        reminder,
    })
}

/// How a squad's document is arranged, from its configuration.
pub struct Shape<'a> {
    pub layout: Layout,
    pub states: &'a States,
    pub sections: &'a [Section],
    pub rows: &'a Rows,
}

/// A squad's document, with what the board keeps beside it.
pub struct Projected {
    pub document: Value,
    /// The user's requests in the room, for the replies pane.
    pub sent: Option<Sent>,
    pub notes: Option<Result<Value, SquadError>>,
}

impl Observation {
    /// The status document: cached field values, columns and sections, the
    /// observed ages on every row, and the user's requests over them. The
    /// room history read for the observation is reused for the requests.
    pub fn document(
        self,
        core: &Core,
        squad: &Squad,
        me: Option<&Me>,
        providers: &[provider::Provider],
        shape: Shape<'_>,
    ) -> Result<Projected, SquadError> {
        let Self {
            mut members,
            cached,
            notes,
            room,
            snapshot,
            reminder: _,
        } = self;
        provider::apply(providers, &mut members, &cached);
        let mut document = status::document(
            squad,
            shape.layout,
            shape.states,
            shape.sections,
            shape.rows,
            members,
        );
        snapshot.apply(&mut document);
        let sent = requests::overlay(core, squad, me, &mut document, room)?;
        Ok(Projected {
            document,
            sent,
            notes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use std::{
        fs,
        path::PathBuf,
        time::{Duration, Instant},
    };

    /// A fake `tmt` that logs every core read and answers the ones a squad's
    /// status needs.
    struct Fixture {
        root: PathBuf,
        core: Core,
        squad: Squad,
        config: Config,
    }

    impl Fixture {
        fn new(name: &str, roster: Value) -> Self {
            let root =
                std::env::temp_dir().join(format!("squad-observe-{name}-{}", std::process::id()));
            fs::create_dir(&root).unwrap();
            fs::write(root.join("roster"), json!({"members": roster}).to_string()).unwrap();
            // Read fixtures stay disabled: enabled tests inject a private cache via observe_with.
            fs::write(root.join("squad.toml"), "").unwrap();
            let executable = root.join("tmt");
            crate::test_support::write_ready_executable(
                &executable,
                r#"#!/bin/sh
root=${0%/*}
case "$1" in
  api)
    input=$(cat)
    case "$input" in
      *'"rooms.roster"'*)
        printf '%s\n' roster >> "$root/calls"
        if [ -p "$root/release" ]; then read -r release < "$root/release"; fi
        cat "$root/roster" ;;
      *'"notes.read"'*) printf '%s\n' notes >> "$root/calls"; printf '%s\n' '{"identityId":"SOL","content":"ship it"}' ;;
      *'"requests.list"'*) printf '%s\n' room >> "$root/calls"; printf '%s\n' '{"items":[]}' ;;
      *) exit 2 ;;
    esac ;;
  ls) printf '%s\n' ls >> "$root/calls"; printf '%s\n' '{"identities":[]}' ;;
  *) exit 2 ;;
esac
"#,
            );
            Self {
                core: Core::at(executable),
                squad: Squad {
                    name: "p".into(),
                    room_id: "33333333-3333-4333-8333-333333333333".into(),
                },
                config: Config::read(root.join("squad.toml")).unwrap(),
                root,
            }
        }

        fn calls(&self) -> Vec<String> {
            fs::read_to_string(self.root.join("calls"))
                .unwrap_or_default()
                .lines()
                .map(str::to_owned)
                .collect()
        }

        /// One squad's document as `ls` (no notes) or the board (notes) reads it.
        fn read(&self, notes: bool) -> Projected {
            let config = &self.config;
            let layout = config.layout("p").unwrap();
            let states = config.states("p", layout).unwrap();
            let sections = config.sections("p").unwrap();
            let rows = config.rows("p").unwrap();
            observe(
                &self.core,
                config.path(),
                &self.squad,
                config.reminders("p").unwrap(),
                &[],
                Mode::Read(Reads {
                    metadata: false,
                    notes,
                }),
            )
            .unwrap()
            .document(
                &self.core,
                &self.squad,
                None,
                &[],
                Shape {
                    layout,
                    states: &states,
                    sections: &sections,
                    rows: &rows,
                },
            )
            .unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn member(id: &str, metadata: Value) -> Value {
        json!({"id": id, "name": id.to_lowercase(), "lifetime": "saved", "metadata": metadata})
    }

    #[test]
    fn ls_and_the_board_read_one_document_and_the_notes_once() {
        let fixture = Fixture::new(
            "one",
            json!([
                member("SOL", json!({"squad.p.lead.marker": "true"})),
                member("RIN", json!({"squad.p.state": "working"})),
            ]),
        );
        // Explicit crew preserves the original disabled-observation contract.
        fs::write(fixture.config.path(), "[squad.p]\nlayout = \"crew\"\n").unwrap();
        let mut fixture = fixture;
        fixture.config = Config::read(fixture.config.path().to_owned()).unwrap();
        // `ls`: with reminders off nothing is observed, so neither the notes
        // nor the room history is read for it.
        let listed = fixture.read(false);
        assert_eq!(fixture.calls(), ["roster", "ls"]);
        assert!(listed.notes.is_none() && listed.sent.is_none());
        assert_eq!(listed.document["squad"]["lead"]["id"], "SOL");
        assert_eq!(
            listed.document["squad"]["notesStaleness"]["state"],
            "disabled"
        );
        assert_eq!(
            listed.document["sections"][0]["rows"][0]["staleness"]["state"],
            "disabled"
        );

        // The board shows the notes: the one notes.read the observation would
        // make serves the pane too, and the document is the same.
        let shown = fixture.read(true);
        assert_eq!(
            fixture.calls(),
            ["roster", "ls", "roster", "ls", "notes"],
            "the lead's notes are read once, after the roster"
        );
        assert_eq!(shown.document, listed.document);
        assert_eq!(
            shown.notes.unwrap().unwrap()["content"],
            "ship it",
            "the pane gets the read itself"
        );
    }

    #[test]
    fn without_a_lead_no_notes_are_read() {
        let fixture = Fixture::new("leaderless", json!([member("RIN", json!({}))]));
        let shown = fixture.read(true);
        assert!(shown.notes.is_none());
        assert!(!fixture.calls().contains(&"notes".to_owned()));
        assert!(shown.document["squad"]["lead"].is_null());
    }
    impl Fixture {
        fn enabled(&self) -> Reminders {
            Reminders {
                enabled: true,
                stale_after: Duration::from_secs(60),
            }
        }

        fn observer(&self, settings: Reminders) -> staleness::Observer {
            staleness::Observer::in_directory(
                self.config.path(),
                &self.squad,
                settings,
                self.root.join("cache"),
            )
        }

        fn remind(&self, settings: Reminders, lead: &str) -> Observation {
            observe_with(
                &self.core,
                &self.squad,
                &[],
                Mode::Reminder { lead },
                || self.observer(settings),
            )
            .unwrap()
        }

        fn prime_notes(&self) {
            let members = self.squad.roster(&self.core).unwrap();
            self.observer(self.enabled()).record(
                &members,
                &[],
                &provider::Cache::at(None),
                Some(&json!({"identityId": "SOL", "content": "ship it"})),
                None,
                status::now_ms() - 60_001,
            );
            fs::remove_file(self.root.join("calls")).unwrap();
        }
    }

    #[test]
    fn reminder_reads_only_the_roster_when_disabled_and_creates_no_cache() {
        let fixture = Fixture::new(
            "reminder-off",
            json!([member("SOL", json!({"squad.p.lead.marker": "true"}))]),
        );
        let result = fixture.remind(Reminders::default(), "SOL");
        assert!(result.reminder.is_none());
        assert_eq!(fixture.calls(), ["roster"]);
        assert!(!fixture.root.join("cache").exists());
    }

    #[test]
    fn reminder_claims_notes_before_handoff_and_never_reads_ls() {
        let fixture = Fixture::new(
            "reminder-claim",
            json!([member("SOL", json!({"squad.p.lead.marker": "true"}))]),
        );
        fixture.prime_notes();
        let result = fixture.remind(fixture.enabled(), "SOL");
        assert_eq!(
            result.reminder,
            Some(staleness::Reminder {
                members: vec![],
                notes: true
            })
        );
        assert_eq!(fixture.calls(), ["roster", "notes", "room"]);
        let path = fs::read_dir(fixture.root.join("cache"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| path.extension().is_some_and(|ext| ext == "json"))
            .unwrap();
        let cache: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(
            cache["notes"]["claimed"], true,
            "claim is already durable when returned"
        );
        drop(result);
        assert!(
            fixture.remind(fixture.enabled(), "SOL").reminder.is_none(),
            "a lost handoff is not retried"
        );
        assert!(!fixture.calls().iter().any(|call| call == "ls"));
    }

    #[test]
    fn reminder_holds_the_observer_lock_before_and_through_the_roster_read() {
        let fixture = Fixture::new(
            "reminder-lock",
            json!([member("SOL", json!({"squad.p.lead.marker": "true"}))]),
        );
        fixture.prime_notes();
        let fifo = fixture.root.join("release");
        nix::unistd::mkfifo(
            &fifo,
            nix::sys::stat::Mode::S_IRUSR | nix::sys::stat::Mode::S_IWUSR,
        )
        .unwrap();
        // Keep a reader open so release cannot hang if the worker fails first.
        let _release = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&fifo)
            .unwrap();
        std::thread::scope(|scope| {
            let reading = scope.spawn(|| fixture.remind(fixture.enabled(), "SOL"));
            let deadline = Instant::now() + Duration::from_secs(5);
            while fixture.calls().is_empty() {
                assert!(Instant::now() < deadline, "roster read did not begin");
                std::thread::sleep(Duration::from_millis(5));
            }
            let competing = fixture.observer(fixture.enabled());
            let locked = !competing.active();
            drop(competing);
            fs::write(&fifo, "continue\n").unwrap();
            let result = reading.join().unwrap();
            assert!(
                locked,
                "the roster runs while this room's observation lock is held"
            );
            assert!(result.reminder.is_some());
        });
        fs::remove_file(fifo).unwrap();
        assert!(
            fixture.observer(fixture.enabled()).active(),
            "completion releases the lock"
        );
    }

    #[test]
    fn reminder_contention_and_changed_lead_never_claim() {
        let fixture = Fixture::new(
            "reminder-unavailable",
            json!([member("SOL", json!({"squad.p.lead.marker": "true"}))]),
        );
        fixture.prime_notes();
        let held = fixture.observer(fixture.enabled());
        assert!(fixture.remind(fixture.enabled(), "SOL").reminder.is_none());
        assert_eq!(
            fixture.calls(),
            ["roster"],
            "contention skips optional reads"
        );
        drop(held);
        assert!(fixture.remind(fixture.enabled(), "RIN").reminder.is_none());
        assert!(
            fixture.remind(fixture.enabled(), "SOL").reminder.is_some(),
            "a rejected lead did not consume the generation"
        );
    }
}

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
    reads: Reads,
) -> Result<Observation, SquadError> {
    let observer = staleness::Observer::begin(config, squad, reminders);
    let members = squad.members(core, reads.metadata)?;
    let cached = provider::Cache::load(&squad.name);
    let notes = (observer.active() || reads.notes)
        .then(|| members.iter().find(|member| member.is_lead()))
        .flatten()
        .map(|lead| core.api("notes.read", json!({"identityId": lead.id})));
    let room = if observer.active() {
        requests::room_window(core, squad).ok()
    } else {
        None
    };
    let snapshot = observer.record(
        &members,
        providers,
        &cached,
        notes.as_ref().and_then(|notes| notes.as_ref().ok()),
        room.as_ref(),
        status::now_ms(),
    );
    Ok(Observation {
        members,
        cached,
        notes,
        room,
        snapshot,
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
    use std::{fs, path::PathBuf};

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
            fs::write(root.join("squad.toml"), "").unwrap();
            let executable = root.join("tmt");
            crate::test_support::write_executable(
                &executable,
                r#"#!/bin/sh
root=${0%/*}
case "$1" in
  api)
    input=$(cat)
    case "$input" in
      *'"rooms.roster"'*) printf '%s\n' roster >> "$root/calls"; cat "$root/roster" ;;
      *'"notes.read"'*) printf '%s\n' notes >> "$root/calls"; printf '%s\n' '{"content":"ship it"}' ;;
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
                    room_id: "room".into(),
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
                Reads {
                    metadata: false,
                    notes,
                },
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
}

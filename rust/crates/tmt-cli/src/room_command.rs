//! CLI projection of the shared room owner. No Office process or pane routing.

mod dispatch;

use crate::{
    identity_context,
    invocation::{OutputMode, RoomOperation},
    output::{Failure, after_cleanup},
};
use std::io::{self, Write};
use tmt_adapters::{
    config::ConfigPaths,
    room::RoomWire,
    storage::{RoomStoreError, Storage},
};
use tmt_core::room::{self, MeetingRoom, ResolveError, RoomRepository};

pub(super) fn failure(error: RoomStoreError) -> Failure {
    let (message, exit) = match &error {
        RoomStoreError::Invalid => ("Invalid room name, identifier or membership.", 1),
        RoomStoreError::NotFound => ("Room was not found.", 3),
        RoomStoreError::RevisionConflict => ("Room changed; refresh before retrying.", 1),
        RoomStoreError::IdentityInactive => ("A selected identity is no longer active.", 3),
        RoomStoreError::Retired => (
            "Room is retired; history is retained but new work is disabled.",
            1,
        ),
        RoomStoreError::Storage(_) => ("Could not access room storage.", 1),
    };
    Failure::new(error.code(), message, exit).caused_by(error)
}

pub(super) fn resolve(storage: &mut Storage, selector: &str) -> Result<MeetingRoom, Failure> {
    room::resolve_room(storage, selector).map_err(|error| resolve_failure(error, selector))
}

pub(super) fn resolve_history(
    storage: &mut Storage,
    selector: &str,
) -> Result<MeetingRoom, Failure> {
    room::resolve_historical_room(storage, selector)
        .map_err(|error| resolve_failure(error, selector))
}

fn resolve_failure(error: ResolveError<RoomStoreError>, selector: &str) -> Failure {
    match error {
        ResolveError::Repository(error) => failure(error),
        ResolveError::NotFound => Failure::new(
            "ROOM_NOT_FOUND",
            format!("Room '{selector}' was not found."),
            3,
        ),
        ResolveError::Ambiguous(rooms) => Failure::new(
            "ROOM_AMBIGUOUS",
            format!(
                "Room name '{selector}' is ambiguous. Use one of these UUIDs: {}.",
                rooms
                    .iter()
                    .map(|room| room.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            1,
        ),
    }
}

enum Report {
    One(MeetingRoom),
    List(Vec<MeetingRoom>),
}

fn run(operation: RoomOperation) -> Result<Report, Failure> {
    let selector = match &operation {
        RoomOperation::Membership { identity, .. } => {
            Some(identity_context::required(identity.as_deref())?)
        }
        _ => None,
    };
    let paths = ConfigPaths::discover().map_err(|error| {
        Failure::new("CONFIG_ERROR", "Could not resolve configuration paths.", 1).caused_by(error)
    })?;
    let mut storage = Storage::open(&paths.database).map_err(|error| {
        Failure::storage_access(
            error,
            &paths.global_dir,
            "No room was changed.",
            "STORAGE_UNAVAILABLE",
            "Could not access room storage.",
        )
    })?;
    let pending = (|| match operation {
        RoomOperation::Dispatch { .. } => unreachable!("dispatch has its own composition"),
        RoomOperation::Create(name) => room::create(&mut storage, name)
            .map(Report::One)
            .map_err(failure),
        RoomOperation::List => storage
            .list_meeting_rooms()
            .map(Report::List)
            .map_err(failure),
        RoomOperation::Show(room) => resolve_history(&mut storage, &room).map(Report::One),
        RoomOperation::Retire(room) => {
            let room = resolve_history(&mut storage, &room)?;
            storage
                .retire_meeting_room(&room.id, room.revision)
                .map(Report::One)
                .map_err(failure)
        }
        RoomOperation::Membership { room, change, .. } => {
            let room = resolve(&mut storage, &room)?;
            let identity = identity_context::resolve(
                &mut storage,
                selector.expect("membership has an identity selector"),
            )?;
            storage
                .change_meeting_membership(&room.id, &identity.id, change)
                .map(Report::One)
                .map_err(failure)
        }
    })();
    after_cleanup(pending, || storage.close())
}

fn state(room: &MeetingRoom) -> &'static str {
    if room.retired { "retired" } else { "active" }
}

/// `ACTIVE n` and `RETIRED n` sections sorted by name; the id is shortened
/// because commands take the room's name.
fn write_list(
    output: &mut impl Write,
    terminal: tmt_cli_style::Terminal,
    rooms: &[MeetingRoom],
) -> io::Result<()> {
    use tmt_cli_style::{
        Token,
        list::{self, Section},
        table::{Cell, Column, Table},
        value,
    };
    let mut sorted: Vec<&MeetingRoom> = rooms.iter().collect();
    sorted.sort_by(|a, b| a.name.cmp(&b.name));
    let sections: Vec<Section> = ["active", "retired"]
        .into_iter()
        .filter_map(|title| {
            let matching: Vec<&&MeetingRoom> =
                sorted.iter().filter(|room| state(room) == title).collect();
            let mut rows = Table::new(&[Column::Name, Column::Fixed, Column::Fixed]);
            for room in &matching {
                let members = match room.member_ids.len() {
                    1 => "1 member".to_owned(),
                    count => format!("{count} members"),
                };
                rows.row([
                    Cell::from(&room.name),
                    Cell::from(members),
                    Cell::styled(value::short_id(&room.id), Token::Dim),
                ]);
            }
            (!matching.is_empty()).then_some(Section {
                title,
                count: Some(matching.len()),
                rows,
                note: None,
                hint: None,
            })
        })
        .collect();
    list::write(output, terminal, &sections)
}

pub fn execute(operation: RoomOperation, mode: OutputMode) -> io::Result<u8> {
    if matches!(operation, RoomOperation::Dispatch { .. }) {
        return dispatch::execute(operation, mode);
    }
    let report = match run(operation) {
        Ok(report) => report,
        Err(error) => return error.publish(mode),
    };
    let mut out = tmt_cli_style::stream::stdout(mode.json);
    if mode.json {
        let value = match &report {
            Report::One(room) => serde_json::json!({"room": RoomWire::from(room)}),
            Report::List(rooms) => {
                serde_json::json!({"rooms": rooms.iter().map(RoomWire::from).collect::<Vec<_>>()})
            }
        };
        writeln!(out, "{value}")?;
    } else {
        let terminal = out.terminal();
        match &report {
            Report::One(room) => tmt_cli_style::detail::write(
                &mut out,
                terminal,
                &room.name,
                &[
                    ("state", state(room).into()),
                    ("revision", room.revision.to_string()),
                    ("id", room.id.clone()),
                    (
                        "members",
                        if room.member_ids.is_empty() {
                            "-".into()
                        } else {
                            room.member_ids.join(", ")
                        },
                    ),
                ],
            )?,
            Report::List(rooms) if rooms.is_empty() => {
                writeln!(out, "No rooms.")?;
                tmt_cli_style::message::hint(&mut out, terminal, "tmt room create <name>")?;
            }
            Report::List(rooms) => write_list(&mut out, terminal, rooms)?,
        }
    }
    Ok(0)
}

// Source-checked command samples for the printed-command guard.
#[cfg(test)]
pub(crate) const PRINTED_HINTS: &[crate::cli_style_tests::HintSpec] =
    &[crate::cli_style_tests::HintSpec::core(
        "tmt room create <name>",
        &[""],
        &[],
    )];

#[cfg(test)]
pub(crate) use dispatch::PRINTED_HINTS as DISPATCH_HINTS;

#[cfg(test)]
mod tests {
    use super::*;

    fn room(name: &str, id: &str, retired: bool, members: usize) -> MeetingRoom {
        MeetingRoom {
            id: id.into(),
            name: name.into(),
            revision: 1,
            retired,
            member_ids: (0..members)
                .map(|index| format!("member-{index}"))
                .collect(),
        }
    }

    #[test]
    fn rooms_list_in_state_sections_by_name_with_short_ids() {
        let mut output = Vec::new();
        write_list(
            &mut output,
            tmt_cli_style::Terminal::PLAIN,
            &[
                room("zeta", "11111111-aaaa", false, 2),
                room("old", "22222222-bbbb", true, 0),
                room("alpha", "33333333-cccc", false, 1),
            ],
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "ACTIVE 2\n  alpha  1 member   33333333\n  zeta   2 members  11111111\n\nRETIRED 1\n  old    0 members  22222222\n"
        );
    }
}

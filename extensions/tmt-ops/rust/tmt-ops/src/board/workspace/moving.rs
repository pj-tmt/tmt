//! `m` lifts the selected agent's membership out of its squad; ↑↓ carry it between the squads
//! the list shows, Common pool last, and Enter applies the move through the existing membership
//! path (Common pool is leaving the squad). Esc puts it back. The list is projected, never
//! mutated, and nothing changes until the operation's result arrives.
use super::{OpsTarget, OpsTreeRow};
use crate::board::app::{App, Effect, Request};
use crate::membership::workspace::Operation;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    text::{Line, Span},
};
use tmt_cli_style::Role;
use tmt_tui::components::strip;

/// One lifted membership. `None` rooms are Common pool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::board) struct Moving {
    pub member: String,
    pub name: String,
    pub source: Option<String>,
    pub target: Option<String>,
    /// Enter sent the operation; the result line ends the move.
    pub applying: bool,
}

/// Where the list paints a lifted membership: the source row becomes a placeholder, and the
/// lifted line follows the target group's last row unless the target is the source.
pub(in crate::board) struct Placement {
    pub source: usize,
    pub after: Option<usize>,
    /// The lifted line's tree depth: one level inside its target group.
    pub depth: u8,
}

impl Placement {
    /// The list line a row paints on: rows after the lifted line move down one.
    pub(in crate::board) fn line(&self, index: usize) -> usize {
        index + usize::from(self.after.is_some_and(|after| index > after))
    }
}

/// A name as painted on one tree line: whole up to 40 cells, else fitted.
fn short(name: &str) -> String {
    if unicode_width::UnicodeWidthStr::width(name) > 40 {
        crate::board::view::fit(name, 40)
    } else {
        name.to_owned()
    }
}

impl Moving {
    pub(in crate::board) fn target_name(&self, rows: &[OpsTreeRow]) -> String {
        match &self.target {
            None => "Common pool".into(),
            Some(room) => rows
                .iter()
                .find(|row| row.target == OpsTarget::Squad(room.clone()))
                .map_or_else(|| room.clone(), |row| row.title.clone()),
        }
    }
    fn member_target(&self) -> OpsTarget {
        OpsTarget::Member {
            room_id: self.source.clone(),
            identity_id: self.member.clone(),
        }
    }
}

/// The squads the list shows, in list order, then Common pool when the list shows it.
fn targets(rows: &[OpsTreeRow]) -> Vec<Option<String>> {
    let mut targets: Vec<_> = rows
        .iter()
        .filter_map(|row| match &row.target {
            OpsTarget::Squad(room) => Some(Some(room.clone())),
            _ => None,
        })
        .collect();
    if rows.iter().any(|row| row.target == OpsTarget::Pool) {
        targets.push(None);
    }
    targets
}

/// The last row of a group: the group row and every deeper row after it.
fn group_end(rows: &[OpsTreeRow], group: &OpsTarget) -> Option<usize> {
    let start = rows.iter().position(|row| &row.target == group)?;
    let depth = rows[start].depth;
    Some(
        rows[start + 1..]
            .iter()
            .take_while(|row| row.depth > depth)
            .count()
            + start,
    )
}

/// The move's placement in `rows`, or none while nothing is lifted.
pub(in crate::board) fn placement(app: &App, rows: &[OpsTreeRow]) -> Option<Placement> {
    let moving = app.moving.as_ref().filter(|moving| !moving.applying)?;
    let source = rows
        .iter()
        .position(|row| row.target == moving.member_target())?;
    let group = moving
        .target
        .clone()
        .map_or(OpsTarget::Pool, OpsTarget::Squad);
    let after = (moving.target != moving.source)
        .then(|| group_end(rows, &group))
        .flatten();
    let depth = match after {
        Some(_) => rows
            .iter()
            .find(|row| row.target == group)
            .map_or(1, |row| row.depth + 1),
        None => rows[source].depth,
    };
    Some(Placement {
        source,
        after,
        depth,
    })
}

/// `⇅ worker → product`, the lifted membership, painted with the selection style.
pub(in crate::board) fn paint_lifted(
    buffer: &mut Buffer,
    area: Rect,
    app: &App,
    rows: &[OpsTreeRow],
    depth: u8,
) {
    let Some(moving) = app.moving.as_ref() else {
        return;
    };
    let look = app.look();
    let text = format!(
        " {}⇅ {}  → {}",
        "  ".repeat(usize::from(depth)),
        short(&moving.name),
        moving.target_name(rows)
    );
    let style = look
        .role(Role::Accent)
        .add_modifier(ratatui::style::Modifier::BOLD);
    let fill = usize::from(area.width)
        .saturating_sub(unicode_width::UnicodeWidthStr::width(text.as_str()));
    strip::paint_left(
        buffer,
        area,
        Line::from(vec![Span::styled(text, style), Span::raw(" ".repeat(fill))]),
    );
}

/// `┆ worker · moving`, the lifted membership's old place.
pub(in crate::board) fn paint_placeholder(buffer: &mut Buffer, area: Rect, app: &App, depth: u8) {
    let Some(moving) = app.moving.as_ref() else {
        return;
    };
    let text = format!(
        " {}┆ {} · moving",
        "  ".repeat(usize::from(depth)),
        short(&moving.name)
    );
    strip::paint_left(buffer, area, Line::styled(text, app.look().role(Role::Dim)));
}

impl App {
    /// `m` on an agent row lifts that membership.
    pub(in crate::board) fn begin_move(&mut self) -> Effect {
        if self.flat_agents() {
            return self.say("Show squads (f) to move an agent between them.");
        }
        let Some(OpsTarget::Member {
            room_id,
            identity_id,
        }) = self.workspace.selected.clone()
        else {
            return self.say("Select an agent to move.");
        };
        let rows = self.workspace_rows();
        let Some(name) = rows
            .iter()
            .find(|row| {
                row.target
                    == OpsTarget::Member {
                        room_id: room_id.clone(),
                        identity_id: identity_id.clone(),
                    }
            })
            .map(|row| row.title.clone())
        else {
            return self.say("Select a visible agent to move.");
        };
        if targets(&rows).len() < 2 {
            return self.say("There is no other squad to move it to.");
        }
        self.moving = Some(Moving {
            member: identity_id,
            name,
            source: room_id.clone(),
            target: room_id,
            applying: false,
        });
        self.invalidate_frame_geometry();
        Effect::None
    }

    /// While a membership is lifted, ↑↓ choose, Enter applies and Esc cancels; every other key
    /// is ignored so no other action runs on a half-chosen move.
    pub(in crate::board) fn moving_key(&mut self, key: KeyEvent) -> Option<Effect> {
        let moving = self.moving.as_ref().filter(|moving| !moving.applying)?;
        let rows = self.workspace_rows();
        let targets = targets(&rows);
        let at = targets
            .iter()
            .position(|target| *target == moving.target)
            .unwrap_or(0);
        let step = |by: isize| {
            let len = targets.len() as isize;
            targets[(at as isize + by).rem_euclid(len) as usize].clone()
        };
        Some(match key.code {
            KeyCode::Up => {
                let target = step(-1);
                self.moving.as_mut().expect("lifted").target = target;
                self.invalidate_frame_geometry();
                Effect::None
            }
            KeyCode::Down => {
                let target = step(1);
                self.moving.as_mut().expect("lifted").target = target;
                self.invalidate_frame_geometry();
                Effect::None
            }
            KeyCode::Esc => {
                self.moving = None;
                self.invalidate_frame_geometry();
                Effect::None
            }
            KeyCode::Enter => self.apply_move(&rows),
            _ => Effect::None,
        })
    }

    fn apply_move(&mut self, rows: &[OpsTreeRow]) -> Effect {
        let Some(moving) = self.moving.clone() else {
            return Effect::None;
        };
        let name = moving.target_name(rows);
        if moving.target == moving.source {
            self.moving = None;
            self.invalidate_frame_geometry();
            return self.say(format!("{} stays in {name}.", moving.name));
        }
        let room = |id: &str| super::manage::room(self, id);
        let operation = match (&moving.source, &moving.target) {
            (source, Some(destination)) => {
                let Some(destination) = room(destination) else {
                    return self.say(format!("{name} is no longer observed; refresh first."));
                };
                let source = match source {
                    Some(id) => match room(id) {
                        Some(source) => Some(source),
                        None => {
                            return self
                                .say("The source squad is no longer observed; refresh first.");
                        }
                    },
                    None => None,
                };
                Operation::Assign {
                    move_member: source.is_some(),
                    source,
                    destination,
                    member: moving.member.clone(),
                }
            }
            (Some(source), None) => {
                let Some(source) = room(source) else {
                    return self.say("The source squad is no longer observed; refresh first.");
                };
                Operation::Remove {
                    room: source,
                    member: moving.member.clone(),
                }
            }
            (None, None) => unreachable!("a pool member's pool target is its source"),
        };
        self.moving.as_mut().expect("lifted").applying = true;
        self.invalidate_frame_geometry();
        Effect::Act(Request::WorkspaceOperation(
            super::manage::Change::Membership(operation),
        ))
    }

    /// The move's one status line, from the same outcome the forms report; nothing is applied
    /// before it arrives, so a refused move leaves the list as it was.
    pub(in crate::board) fn finished_move(
        &self,
        moving: Moving,
        outcome: &Result<crate::membership::Outcome, crate::core::SquadError>,
    ) -> String {
        let name = match &moving.target {
            None => "Common pool".to_owned(),
            Some(room) => {
                super::manage::room(self, room).map_or_else(|| room.clone(), |room| room.name)
            }
        };
        match outcome {
            Ok(outcome) if outcome.complete => format!("Moved {} to {name}.", moving.name),
            Ok(outcome) => format!(
                "Partially applied or unknown: {}. Nothing is retried automatically.",
                outcome.document["failed"]["error"]["message"]
                    .as_str()
                    .unwrap_or("some steps were not confirmed; inspect the refreshed membership")
            ),
            Err(error) => format!(
                "{}: {}",
                if crate::board::app::workspace::not_applied(&error.code) {
                    "Not applied"
                } else {
                    "Unknown"
                },
                error.message
            ),
        }
    }
}

#[cfg(test)]
mod tests;

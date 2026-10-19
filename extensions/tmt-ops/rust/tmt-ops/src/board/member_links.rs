//! Resolved member references over the held identity inventory. Display keeps
//! its own geometry; raw conversation source remains untouched.
use super::inventory::Inventory;
use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Resolved {
    pub range: Range<usize>,
    pub identity: String,
    pub name: String,
}

pub(super) fn resolve(text: &str, inventory: &Inventory) -> Vec<Resolved> {
    if inventory.error.is_some() || !text.contains("#tmt:") {
        return Vec::new();
    }
    let mut found = Vec::new();
    let mut end = 0;
    for (start, _) in text.match_indices("#tmt:") {
        if start < end {
            continue;
        }
        let tail = &text[start + 5..];
        let mut matches: Vec<_> = inventory
            .agents
            .iter()
            .flat_map(|(id, agent)| {
                [id.as_str(), agent.canonical.as_str()]
                    .into_iter()
                    .filter_map(move |key| {
                        if key.is_empty() || !tail.starts_with(key) {
                            return None;
                        }
                        let next = tail[key.len()..].chars().next();
                        if next.is_some_and(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                            return None;
                        }
                        Some((key.len(), id, agent))
                    })
            })
            .collect();
        matches.sort_by_key(|(len, _, _)| std::cmp::Reverse(*len));
        let Some((len, id, agent)) = matches.first() else {
            continue;
        };
        if matches
            .iter()
            .any(|(other_len, other_id, _)| other_len == len && other_id != id)
        {
            continue;
        }
        end = start + 5 + len;
        found.push(Resolved {
            range: start..end,
            identity: (*id).clone(),
            name: super::notes::sanitize(&agent.name),
        });
    }
    found
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Link {
    pub column: usize,
    pub width: usize,
    pub identity: String,
    pub occurrence: usize,
}

use super::{
    app::{App, Choice, Effect, Menu, MenuEntry, RowTarget},
    workspace::OpsTarget,
};
use ratatui::{
    Frame,
    crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind},
    layout::Rect,
    style::Modifier,
};
use std::cell::RefCell;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Hit {
    area: Rect,
    identity: String,
    reader: bool,
    occurrence: usize,
}
#[derive(Default)]
pub(super) struct State {
    hits: Vec<Hit>,
    focused: Option<Hit>,
    pressed: Option<(Hit, ratatui::layout::Position, bool)>,
    screen: Rect,
}
impl State {
    pub fn begin_frame(&mut self, screen: Rect) {
        self.hits.clear();
        self.screen = screen;
    }
    pub fn invalidate(&mut self) {
        self.hits.clear();
        self.focused = None;
        self.pressed = None;
    }
}

/// Geometry is clipped to cells actually painted in the current viewport.
pub(super) struct Paint {
    pub reader: bool,
    pub role: tmt_cli_style::Role,
}

pub(super) fn paint_hits(
    frame: &mut Frame,
    app: &App,
    area: Rect,
    offset: usize,
    shown: usize,
    lines: &[Vec<Link>],
    presentation: Paint,
) {
    let mut state = app.member_links.borrow_mut();
    for (index, links) in lines
        .iter()
        .enumerate()
        .skip(offset)
        .take(shown.min(usize::from(area.height)))
    {
        for link in links {
            if link.column >= usize::from(area.width) {
                continue;
            }
            let width = link.width.min(usize::from(area.width) - link.column) as u16;
            if width == 0 {
                continue;
            }
            let hit = Hit {
                area: Rect::new(
                    area.x + link.column as u16,
                    area.y + (index - offset) as u16,
                    width,
                    1,
                ),
                identity: link.identity.clone(),
                reader: presentation.reader,
                occurrence: link.occurrence,
            };
            let focused = state.focused.as_ref().is_some_and(|focused| {
                focused.reader == hit.reader
                    && focused.identity == hit.identity
                    && focused.occurrence == hit.occurrence
            });
            for x in hit.area.x..hit.area.right() {
                frame.buffer_mut()[(x, hit.area.y)].set_style(
                    app.look().role(presentation.role).add_modifier(
                        Modifier::UNDERLINED
                            | if focused {
                                Modifier::BOLD
                            } else {
                                Modifier::empty()
                            },
                    ),
                );
            }
            state.hits.push(hit);
        }
    }
}

pub(super) fn menu_area(screen: Rect, anchor: Rect, desired_height: u16) -> Option<Rect> {
    if anchor.intersection(screen) != anchor {
        return None;
    }
    let below = screen.bottom().saturating_sub(anchor.bottom());
    let above = anchor.y.saturating_sub(screen.y);
    let (available, below) = if below >= desired_height || below >= above {
        (below, true)
    } else {
        (above, false)
    };
    let height = desired_height.min(available);
    if height < 5 {
        return None;
    }
    let width = screen.width.min(48);
    Some(Rect::new(
        anchor.x.min(screen.right().saturating_sub(width)),
        if below {
            anchor.bottom()
        } else {
            anchor.y - height
        },
        width,
        height,
    ))
}

impl App {
    fn member_link_target(&self, identity: &str) -> Option<OpsTarget> {
        if self.references.error.is_some() || !self.references.agents.contains_key(identity) {
            return None;
        }
        let snapshot = self.workspace_snapshot()?;
        if !snapshot.roster.complete() {
            return None;
        }
        // Prefer the current membership, then a visible membership, then the
        // acquired identity's first membership. Never invent Common pool.
        let same = |target: &&OpsTarget| matches!(target, OpsTarget::Member { identity_id, .. } if identity_id == identity);
        self.workspace
            .selected
            .as_ref()
            .filter(|target| same(target))
            .filter(|target| snapshot.roster.contains(target))
            .cloned()
            .or_else(|| {
                self.workspace_rows()
                    .iter()
                    .map(|row| &row.target)
                    .find(|target| same(target) && snapshot.roster.contains(target))
                    .cloned()
            })
            .or_else(|| {
                snapshot
                    .rows
                    .keys()
                    .find(|target| same(target) && snapshot.roster.contains(target))
                    .cloned()
            })
    }
    fn member_link_jump(&self, target: &OpsTarget) -> Result<crate::action::Action, String> {
        use crate::config::OpsKeyScope;
        let row = self
            .target_row(&RowTarget::Workspace(target.clone()))
            .ok_or("Member unavailable")?;
        if row["presence"] != "active"
            || !row["pane"].is_object()
            || row["pane"]["location"]["host"].as_str().is_none()
        {
            return Err("target is offline or has no live binding".into());
        }
        let keys = self.workspace_context_bindings(
            Some(target),
            OpsKeyScope::Member,
            row["pane"]["location"]["host"].as_str(),
        );
        let action = keys.get("j").ok_or("No j action configured")?;
        crate::config::admit_workspace_action(action, "member mention j")
            .map_err(|error| error.message)?;
        if !matches!(
            action.verb,
            crate::action::Verb::Jump | crate::action::Verb::Run
        ) {
            return Err("j is not a host action".into());
        }
        if action.verb == crate::action::Verb::Run {
            action.argv(row)?;
        } else {
            super::overview::actions::host(row, self.invoker_tmux).map_err(str::to_owned)?;
        }
        Ok(action.clone())
    }
    fn open_member_link(&mut self, hit: Hit) -> Effect {
        let Some(target) = self.member_link_target(&hit.identity) else {
            return self.say("This member is unavailable; refresh the board.");
        };
        let jump = self.member_link_jump(&target);
        let title = self.references.agents[&hit.identity].name.clone();
        let entries = vec![
            MenuEntry {
                key: "j".into(),
                label: jump.as_ref().map_or_else(
                    |reason| format!("Jump to host · {reason}"),
                    |_| "Jump to host".into(),
                ),
                choice: jump.map_or_else(Choice::Unavailable, |_| Choice::MemberLink {
                    target: target.clone(),
                    jump: true,
                }),
            },
            MenuEntry {
                key: "⏎".into(),
                label: "Show in Agents".into(),
                choice: Choice::MemberLink {
                    target: target.clone(),
                    jump: false,
                },
            },
        ];
        if menu_area(
            self.member_links.borrow().screen,
            hit.area,
            entries.len() as u16 + 4,
        )
        .is_none()
        {
            return self.say("Not enough room beside this mention; resize to open its actions.");
        }
        self.menu = Some(Menu {
            row_send: None,
            link: None,
            prefill: String::new(),
            title,
            entries,
            selected: 1,
            anchor: Some(hit.area),
            surface: RefCell::new(None),
        });
        Effect::None
    }
    pub(super) fn member_link_mouse(&mut self, event: MouseEvent) -> Option<Effect> {
        let point = (event.column, event.row).into();
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let hit = self
                    .member_links
                    .borrow()
                    .hits
                    .iter()
                    .find(|hit| hit.area.contains(point))
                    .cloned();
                self.member_links.borrow_mut().pressed = None;
                let hit = hit?;
                if hit.reader && self.rendered_selection_at(point) {
                    // A conversation link remains clickable, but the first drag
                    // must select its rendered name rather than open a menu.
                    self.member_links.borrow_mut().pressed = Some((hit, point, false));
                    None
                } else {
                    Some(self.open_member_link(hit))
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if let Some((_, start, dragged)) = &mut self.member_links.borrow_mut().pressed {
                    *dragged |= point != *start;
                }
                None
            }
            MouseEventKind::Up(MouseButton::Left) => {
                let (hit, _, dragged) = self.member_links.borrow_mut().pressed.take()?;
                let current = self
                    .member_links
                    .borrow()
                    .hits
                    .iter()
                    .any(|current| current == &hit);
                if !dragged && current && hit.area.contains(point) {
                    self.conversation_source_escape();
                    Some(self.open_member_link(hit))
                } else {
                    None
                }
            }
            _ => None,
        }
    }
    fn reader_link_default_key(&self, event: &str) -> bool {
        self.input.is_none()
            && self.focus == 1
            && !self
                .workspace_binding_sources()
                .and_then(|sources| sources.get(event))
                .is_some_and(|source| source != "workspace default")
    }
    pub(in crate::board) fn member_link_owns_enter(&self) -> bool {
        if !self.reader_link_default_key("enter") {
            return false;
        }
        let state = self.member_links.borrow();
        state
            .focused
            .as_ref()
            .is_some_and(|focused| state.hits.iter().any(|hit| hit.reader && hit == focused))
    }
    pub(super) fn member_link_key(&mut self, key: KeyEvent) -> Option<Effect> {
        if !key.modifiers.is_empty()
            || super::app::event_name(key).is_none_or(|event| !self.reader_link_default_key(&event))
        {
            return None;
        }
        let mut state = self.member_links.borrow_mut();
        let mut seen = std::collections::BTreeSet::new();
        let hits: Vec<_> = state
            .hits
            .iter()
            .filter(|hit| hit.reader && seen.insert((hit.identity.clone(), hit.occurrence)))
            .cloned()
            .collect();
        if hits.is_empty() {
            state.focused = None;
            return None;
        }
        let index = state
            .focused
            .as_ref()
            .and_then(|focused| hits.iter().position(|hit| hit == focused));
        match key.code {
            KeyCode::Left | KeyCode::Right => {
                let i = index.map_or(0, |i| {
                    if key.code == KeyCode::Left {
                        (i + hits.len() - 1) % hits.len()
                    } else {
                        (i + 1) % hits.len()
                    }
                });
                state.focused = Some(hits[i].clone());
                Some(Effect::None)
            }
            KeyCode::Enter if index.is_some() => {
                let hit = hits[index.unwrap()].clone();
                drop(state);
                Some(self.open_member_link(hit))
            }
            KeyCode::Esc if index.is_some() => {
                state.focused = None;
                Some(Effect::None)
            }
            _ => None,
        }
    }
    pub(super) fn member_link_action(&mut self, target: &OpsTarget, jump: bool) -> Effect {
        let OpsTarget::Member { identity_id, .. } = target else {
            return self.say("This link has no member target.");
        };
        if self.references.error.is_some()
            || !self.references.agents.contains_key(identity_id)
            || !self.workspace_snapshot().is_some_and(|snapshot| {
                snapshot.roster.complete() && snapshot.roster.contains(target)
            })
        {
            return self.say(
                "This member or membership changed or became unavailable; refresh the board.",
            );
        }
        if jump {
            let action = match self.member_link_jump(target) {
                Ok(action) => action,
                Err(reason) => return self.say(reason),
            };
            return self.perform_member_host(&action, target);
        }
        // An explicit navigation is session-only. It never changes shared hide
        // preferences or loses a retained conversation draft.
        self.remember_draft();
        self.input = None;
        self.searching = false;
        self.search.clear();
        self.workspace.query.clear();
        self.workspace.filter = super::workspace::AgentsFilter::All;
        self.workspace.visibility.scope.clear();
        self.workspace.visibility.reveal_hidden = true;
        self.shell.tab = super::shell::OpsTab::Agents;
        self.focus = 0;
        self.choose(Choice::WorkspaceTarget(target.clone()))
    }
}

/// Plain preview projection: only resolved member markup changes its display.
pub(super) fn paint_preview(
    frame: &mut Frame,
    app: &App,
    area: Rect,
    text: &str,
    role: tmt_cli_style::Role,
) -> bool {
    if resolve(text, &app.references).is_empty() {
        return false;
    }
    let mapped = super::markdown::render_plain_with_members(
        text,
        usize::from(area.width).max(1),
        app.look(),
        &app.references,
    );
    for (index, line) in mapped
        .lines
        .iter()
        .take(usize::from(area.height))
        .enumerate()
    {
        tmt_tui::components::strip::paint_left(
            frame.buffer_mut(),
            Rect::new(area.x, area.y + index as u16, area.width, 1),
            line.clone(),
        );
    }
    paint_hits(
        frame,
        app,
        area,
        0,
        usize::from(area.height),
        &mapped.mentions,
        Paint {
            reader: false,
            role,
        },
    );
    true
}

impl App {
    pub(super) fn member_link_hint(&self) -> Option<&'static str> {
        let state = self.member_links.borrow();
        (self.input.is_none() && self.focus == 1 && state.hits.iter().any(|hit| hit.reader))
            .then_some("←→ mentions · Enter actions · Tab panes · Esc back")
    }
}

#[cfg(test)]
pub(super) fn capture_menu(app: &mut App) {
    let hit = app
        .member_links
        .borrow()
        .hits
        .iter()
        .find(|hit| hit.reader)
        .cloned();
    if let Some(hit) = hit {
        app.open_member_link(hit);
    }
}

#[cfg(test)]
mod tests;

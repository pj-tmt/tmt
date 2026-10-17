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

fn word_char(ch: char) -> bool {
    ch.is_alphanumeric() || matches!(ch, '-' | '_')
}

pub(super) fn resolve(text: &str, inventory: &Inventory) -> Vec<Resolved> {
    if inventory.error.is_some() {
        return Vec::new();
    }
    let mut found = Vec::new();
    let mut end = 0;
    for (start, ch) in text.char_indices() {
        if start < end {
            continue;
        }
        let legacy = text[start..].starts_with("#tmt:");
        let prefix = if legacy { 5 } else { usize::from(ch == '@') };
        if !legacy
            && text[..start]
                .chars()
                .next_back()
                .is_some_and(|ch| word_char(ch) || ch == '@')
        {
            continue;
        }
        let tail = &text[start + prefix..];
        let mut matches = Vec::new();
        for (id, agent) in &inventory.agents {
            let keys = [
                agent.canonical.as_str(),
                if legacy { id.as_str() } else { "" },
            ];
            for key in keys {
                if key.is_empty()
                    || (prefix == 0
                        && !key
                            .chars()
                            .any(|ch| ch.is_numeric() || matches!(ch, '-' | '_')))
                    || !tail.starts_with(key)
                    || tail[key.len()..].chars().next().is_some_and(word_char)
                {
                    continue;
                }
                matches.push((key.len(), id, agent));
            }
        }
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
        end = start + prefix + len;
        found.push(Resolved {
            range: start..end,
            identity: (*id).clone(),
            name: if legacy {
                super::notes::sanitize(&agent.name)
            } else {
                text[start..end].to_owned()
            },
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
    surface: Rect,
    identity: String,
    reader: bool,
    occurrence: usize,
}
struct Cached {
    text: String,
    width: usize,
    look: crate::look::Look,
    mapped: Option<std::rc::Rc<super::markdown::Mapped>>,
    resolved: Option<std::rc::Rc<[Resolved]>>,
    used: bool,
}

#[derive(Default)]
pub(super) struct State {
    hits: Vec<Hit>,
    focused: Option<Hit>,
    pressed: Option<(Hit, ratatui::layout::Position, bool)>,
    screen: Rect,
    hovered: Option<Hit>,
    inventory: Option<std::sync::Arc<()>>,
    cache: Vec<Cached>,
    #[cfg(test)]
    builds: usize,
}
impl State {
    pub fn begin_frame(&mut self, screen: Rect) {
        self.hits.clear();
        self.screen = screen;
        self.cache.retain(|entry| entry.used);
        for entry in &mut self.cache {
            entry.used = false;
        }
    }
    pub fn invalidate(&mut self) {
        self.hits.clear();
        self.focused = None;
        self.pressed = None;
        self.hovered = None;
    }
    fn cached(
        &mut self,
        text: &str,
        width: usize,
        look: crate::look::Look,
        inventory: &Inventory,
    ) -> &mut Cached {
        if self
            .inventory
            .as_ref()
            .is_none_or(|held| !std::sync::Arc::ptr_eq(held, &inventory.generation))
        {
            self.cache.clear();
            self.inventory = Some(std::sync::Arc::clone(&inventory.generation));
        }
        let index = self
            .cache
            .iter()
            .position(|entry| entry.text == text && entry.width == width && entry.look == look)
            .unwrap_or_else(|| {
                self.cache.push(Cached {
                    text: text.to_owned(),
                    width,
                    look,
                    mapped: None,
                    resolved: None,
                    used: true,
                });
                #[cfg(test)]
                {
                    self.builds += 1;
                }
                self.cache.len() - 1
            });
        let entry = &mut self.cache[index];
        entry.used = true;
        entry
    }
    fn preview(
        &mut self,
        text: &str,
        width: usize,
        look: crate::look::Look,
        inventory: &Inventory,
    ) -> std::rc::Rc<super::markdown::Mapped> {
        let entry = self.cached(text, width, look, inventory);
        std::rc::Rc::clone(entry.mapped.get_or_insert_with(|| {
            std::rc::Rc::new(super::markdown::render_plain_with_members(
                text, width, look, inventory,
            ))
        }))
    }
    fn plain(
        &mut self,
        text: &str,
        look: crate::look::Look,
        inventory: &Inventory,
    ) -> std::rc::Rc<[Resolved]> {
        let entry = self.cached(text, usize::MAX, look, inventory);
        std::rc::Rc::clone(
            entry
                .resolved
                .get_or_insert_with(|| resolve(text, inventory).into()),
        )
    }
    pub(super) fn move_pointer(&mut self, point: ratatui::layout::Position, active: bool) -> bool {
        let next = active
            .then(|| {
                self.hits
                    .iter()
                    .rev()
                    .find(|hit| hit.area.contains(point))
                    .cloned()
            })
            .flatten();
        let changed = self.hovered != next;
        self.hovered = next;
        changed
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
                surface: area,
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
                    && focused.surface == hit.surface
                    && focused.identity == hit.identity
                    && focused.occurrence == hit.occurrence
            });
            for x in hit.area.x..hit.area.right() {
                let hover = state
                    .hovered
                    .as_ref()
                    .is_some_and(|hover| hover.area == hit.area && hover.identity == hit.identity);
                let style = app
                    .look()
                    .role(presentation.role)
                    .remove_modifier(Modifier::BOLD)
                    .add_modifier(Modifier::UNDERLINED);
                frame.buffer_mut()[(x, hit.area.y)]
                    .set_style(ratatui::style::Style::new().remove_modifier(Modifier::BOLD));
                frame.buffer_mut()[(x, hit.area.y)].set_style(if focused || hover {
                    style
                        .patch(app.look().selection())
                        .remove_modifier(Modifier::BOLD)
                } else {
                    style
                });
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
    fn member_link_binding(
        &self,
        target: &OpsTarget,
        verb: crate::action::Verb,
    ) -> Option<crate::action::Action> {
        use crate::{action::Verb, config::OpsKeyScope};
        let row_target = RowTarget::Workspace(target.clone());
        let row = self.target_row(&row_target)?;
        let keys = self.workspace_context_bindings(
            Some(target),
            OpsKeyScope::Member,
            row["pane"]["location"]["host"].as_str(),
        );
        let preferred = match verb {
            Verb::Talk => "enter",
            Verb::Copy => "y",
            Verb::Annotate => "a",
            _ => return None,
        };
        let action = keys
            .get(preferred)
            .filter(|action| action.verb == verb)
            .or_else(|| keys.values().find(|action| action.verb == verb))?;
        crate::config::admit_workspace_action(action, "member link").ok()?;
        match verb {
            Verb::Talk => {
                self.send_target(row_target, false)?;
            }
            Verb::Annotate => {
                if action.args.first().and_then(|arg| arg.literal()) == Some("member") {
                    return None;
                }
                self.send_target(row_target, false)?.note?;
            }
            Verb::Copy => {
                action.args.first()?.fill(row).ok()?;
            }
            _ => return None,
        }
        Some(action.clone())
    }
    fn member_link_valid(&self, target: &OpsTarget) -> bool {
        matches!(target, OpsTarget::Member {identity_id, ..} if self.references.error.is_none()
            && self.references.agents.contains_key(identity_id)
            && self.workspace_snapshot().is_some_and(|snapshot| snapshot.roster.complete() && snapshot.roster.contains(target)))
    }
    pub(super) fn member_link_command(
        &mut self,
        target: &OpsTarget,
        action: &crate::action::Action,
    ) -> Effect {
        if !self.member_link_valid(target)
            || self.member_link_binding(target, action.verb).as_ref() != Some(action)
        {
            return self.say("This member action changed or became unavailable; reopen its menu.");
        }
        if matches!(
            action.verb,
            crate::action::Verb::Talk | crate::action::Verb::Annotate
        ) {
            self.member_link_action(target, false);
        }
        self.perform_context(action, Some(target))
    }
    fn open_member_link(&mut self, hit: Hit) -> Effect {
        let Some(target) = self.member_link_target(&hit.identity) else {
            return self.say("This member is unavailable; refresh the board.");
        };
        let jump = self.member_link_jump(&target);
        let title = self.references.agents[&hit.identity].name.clone();
        let mut entries = Vec::new();
        let command = |key: &str, label: &str, action| MenuEntry {
            key: key.into(),
            label: label.into(),
            choice: Choice::MemberLinkAction {
                target: target.clone(),
                action,
            },
        };
        if let Some(action) = self.member_link_binding(&target, crate::action::Verb::Talk) {
            entries.push(command("t", "Message", action));
        }
        if jump.is_ok() {
            entries.push(MenuEntry {
                key: "j".into(),
                label: "Jump to host".into(),
                choice: Choice::MemberLink {
                    target: target.clone(),
                    jump: true,
                },
            });
        }
        for (key, label, verb) in [
            ("y", "Copy", crate::action::Verb::Copy),
            ("a", "Note to lead", crate::action::Verb::Annotate),
        ] {
            if let Some(action) = self.member_link_binding(&target, verb) {
                entries.push(command(key, label, action));
            }
        }
        entries.push(MenuEntry {
            key: String::new(),
            label: "Show in Agents".into(),
            choice: Choice::MemberLink {
                target: target.clone(),
                jump: false,
            },
        });
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
            selected: 0,
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
    fn row_name_hits(&self) -> Vec<Hit> {
        let index = if self.workspace_snapshot().is_some() {
            self.workspace_rows()
                .iter()
                .position(|row| Some(&row.target) == self.workspace.selected.as_ref())
        } else {
            Some(self.selected)
        };
        let Some(index) = index else {
            return Vec::new();
        };
        let rows: Vec<_> = self
            .hits
            .borrow()
            .iter()
            .filter_map(|hit| {
                (hit.target == super::app::HitTarget::Row(index))
                    .then_some(Rect::new(hit.x, hit.y, hit.width, 1))
            })
            .collect();
        self.member_links
            .borrow()
            .hits
            .iter()
            .filter(|hit| {
                !hit.reader
                    && rows
                        .iter()
                        .any(|row| row.intersection(hit.area) == hit.area)
            })
            .cloned()
            .collect()
    }
    pub(in crate::board) fn row_names_hint(&self) -> bool {
        self.input.is_none()
            && self.focus == 0
            && !self.bindings().contains_key("@")
            && !self.row_name_hits().is_empty()
    }
    fn reader_link_default_key(&self, event: &str) -> bool {
        self.input.is_none()
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
        state.focused.as_ref().is_some_and(|focused| {
            state
                .hits
                .iter()
                .any(|hit| hit.reader == (self.focus == 1) && hit == focused)
        })
    }
    pub(super) fn member_link_key(&mut self, key: KeyEvent) -> Option<Effect> {
        if self.focus == 0
            && matches!(
                key.code,
                KeyCode::Up
                    | KeyCode::Down
                    | KeyCode::PageUp
                    | KeyCode::PageDown
                    | KeyCode::Home
                    | KeyCode::End
            )
        {
            self.member_links.borrow_mut().focused = None;
            return None;
        }
        if key.code == KeyCode::Char('@')
            && self.focus == 0
            && self.input.is_none()
            && (key.modifiers.is_empty()
                || key.modifiers == ratatui::crossterm::event::KeyModifiers::SHIFT)
            && !self.bindings().contains_key("@")
        {
            let hits = self.row_name_hits();
            let first = hits.first()?.clone();
            if hits.len() == 1 {
                self.member_links.borrow_mut().focused = None;
                return Some(self.open_member_link(first));
            }
            self.member_links.borrow_mut().focused = Some(first);
            return Some(Effect::None);
        }
        if !key.modifiers.is_empty()
            || super::app::event_name(key).is_none_or(|event| !self.reader_link_default_key(&event))
        {
            return None;
        }
        let mut seen = std::collections::BTreeSet::new();
        let candidates = if self.focus == 0 {
            self.row_name_hits()
        } else {
            self.member_links
                .borrow()
                .hits
                .iter()
                .filter(|hit| hit.reader)
                .cloned()
                .collect()
        };
        let hits: Vec<_> = candidates
            .into_iter()
            .filter(|hit| {
                seen.insert((
                    hit.identity.clone(),
                    hit.occurrence,
                    hit.surface.x,
                    hit.surface.y,
                    hit.surface.width,
                    hit.surface.height,
                ))
            })
            .collect();
        let mut state = self.member_links.borrow_mut();
        if self.focus == 0
            && state
                .focused
                .as_ref()
                .is_none_or(|focused| !hits.contains(focused))
        {
            return None;
        }
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
        if !self.member_link_valid(target) {
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
    let mapped = app.member_links.borrow_mut().preview(
        text,
        usize::from(area.width).max(1),
        app.look(),
        &app.references,
    );
    if mapped.mentions.iter().all(Vec::is_empty) {
        return false;
    }
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

/// Decorate a fitted plain row while resolving only its complete original text.
/// Truncation cannot turn an unknown/longer name into another member's link.
pub(super) fn paint_plain_hits(
    frame: &mut Frame,
    app: &App,
    area: Rect,
    source: &str,
    displayed: &str,
    reader: bool,
) {
    let resolved = app
        .member_links
        .borrow_mut()
        .plain(source, app.look(), &app.references);
    let mut links = Vec::new();
    let mut at = 0;
    for (occurrence, link) in resolved.iter().enumerate() {
        let raw = &source[link.range.clone()];
        if let Some(offset) = displayed[at..].find(raw) {
            let start = at + offset;
            let column = unicode_width::UnicodeWidthStr::width(&displayed[..start]);
            let width = unicode_width::UnicodeWidthStr::width(raw);
            links.push(Link {
                column,
                width,
                identity: link.identity.clone(),
                occurrence,
            });
            at = start + raw.len();
        }
    }
    paint_hits(
        frame,
        app,
        area,
        0,
        1,
        &[links],
        Paint {
            reader,
            role: tmt_cli_style::Role::Text,
        },
    );
}

impl App {
    pub(super) fn member_link_hint(&self) -> Option<&'static str> {
        let state = self.member_links.borrow();
        if self.input.is_some() {
            return None;
        }
        if self.focus == 0 {
            return state
                .focused
                .as_ref()
                .filter(|hit| !hit.reader && self.row_name_hits().contains(hit))
                .map(|_| "←→ name  ⏎ menu  Esc back");
        }
        state
            .hits
            .iter()
            .any(|hit| hit.reader)
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

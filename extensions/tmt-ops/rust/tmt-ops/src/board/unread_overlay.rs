//! The Unread overlay: the unread answers of a scope, grouped by agent, shown in full and
//! acknowledged as they are painted. It owns no acknowledgment path: the blocks it paints in
//! its viewport go through the same `unread.writer` the conversation feeds.
use super::{
    app::{App, Effect, Overlay as OverlayId},
    exchange_view,
    recent_responses::{self, Observation},
    unread::{Answer, BodyKind, Page, walk::Body},
    workspace::OpsTarget,
};
use ratatui::{
    Frame,
    crossterm::event::{Event, KeyCode, KeyEventKind},
    layout::Rect,
    text::Line,
};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet},
    ops::Range,
};
use tmt_cli_style::Role;
use tmt_tui::components::{Modal, Placement, ScrollState, strip};

/// Which members' unread an open overlay lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum UnreadScope {
    All,
    Member(String),
    /// A squad or the Common pool: the members of the group.
    Group(Vec<String>),
}

/// A painted unread count that opens the overlay when pressed: the target it belongs to, or
/// everyone for the header strip's count.
#[derive(Debug)]
pub(in crate::board) struct Hit {
    pub area: Rect,
    pub target: Option<OpsTarget>,
}

/// What a body read gave for one answer.
enum Text<'a> {
    /// The member's answers have not been read yet, or the read is still running.
    Waiting,
    /// The read failed.
    Failed,
    /// The answer is not in what was read.
    Missing,
    /// Core keeps no text for it.
    Unavailable,
    Found {
        text: &'a str,
        truncated: bool,
    },
}

/// One answer's block in the built overlay.
struct Block {
    item: usize,
    rows: Range<usize>,
    /// The rows of the answer text; empty while there is no text to read.
    body: Range<usize>,
}

struct Built {
    width: usize,
    look: crate::look::Look,
    version: u64,
    lines: Vec<Line<'static>>,
    blocks: Vec<Block>,
}

pub(in crate::board) struct Overlay {
    /// Which opening this is; a read started for an earlier one is dropped.
    session: u64,
    /// The members of the scope in the list's order.
    members: Vec<String>,
    /// Every answer shown since the overlay opened, kept after it is acknowledged so a read
    /// block does not vanish under the reader; later answers are added, never reordered.
    items: BTreeMap<(String, String), Answer>,
    pages: BTreeMap<String, Observation>,
    /// Members whose body read was demanded; one read is in flight at a time.
    asked: BTreeSet<String>,
    /// Members whose read failed; true once the reader has scrolled away from their answers,
    /// which is what allows one retry when they scroll back.
    failed: BTreeMap<String, bool>,
    /// The members with an answer inside the viewport of the last paint.
    visible: RefCell<BTreeSet<String>>,
    scroll: RefCell<ScrollState>,
    /// Bumped whenever what is built could change besides the width and the look.
    version: Cell<u64>,
    built: RefCell<Option<Built>>,
    /// The answers whose text is inside the viewport of the last paint.
    painted: RefCell<Vec<Body>>,
    top: Cell<Option<usize>>,
    /// Core holds back older unread answers than the page the board keeps.
    more: bool,
}

impl Overlay {
    pub fn new(session: u64, members: Vec<String>) -> Self {
        Self {
            session,
            members,
            items: BTreeMap::new(),
            pages: BTreeMap::new(),
            asked: BTreeSet::new(),
            failed: BTreeMap::new(),
            visible: RefCell::default(),
            scroll: RefCell::default(),
            version: Cell::new(0),
            built: RefCell::default(),
            painted: RefCell::default(),
            top: Cell::new(None),
            more: false,
        }
    }

    fn bump(&self) {
        self.version.set(self.version.get() + 1);
    }

    /// Adds the scope's unread answers that the overlay has not shown yet.
    pub fn sync(&mut self, page: &Page) {
        let before = self.items.len();
        for member in &self.members {
            for answer in page.answers(member).filter(|answer| !answer.acknowledged) {
                self.items
                    .entry((member.clone(), answer.request.clone()))
                    .or_insert_with(|| answer.clone());
            }
        }
        let more = page.next_before.is_some();
        if self.items.len() != before || self.more != more {
            self.more = more;
            self.bump();
        }
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The members with something to show, in the list's order.
    fn shown_members(&self) -> impl Iterator<Item = &String> {
        self.members
            .iter()
            .filter(|member| self.items.keys().any(|(owner, _)| owner == *member))
    }

    fn ordered(&self) -> Vec<&Answer> {
        let mut answers = Vec::with_capacity(self.items.len());
        for member in &self.members {
            let mut group: Vec<_> = self
                .items
                .iter()
                .filter(|((owner, _), _)| owner == member)
                .map(|(_, answer)| answer)
                .collect();
            group.sort_by(|a, b| {
                (a.answered_at_ms, &a.request).cmp(&(b.answered_at_ms, &b.request))
            });
            answers.extend(group);
        }
        answers
    }

    /// The next member whose answers must be read: the first group at or below the
    /// viewport's top, then the others, each once.
    pub fn next_read(&self) -> Option<&str> {
        let ordered = self.ordered();
        let start = self
            .top
            .get()
            .and_then(|top| ordered.get(top))
            .map(|answer| answer.member.as_str());
        let mut members: Vec<&str> = self.shown_members().map(String::as_str).collect();
        if let Some(start) = start
            && let Some(at) = members.iter().position(|member| *member == start)
        {
            members.rotate_left(at);
        }
        members
            .into_iter()
            .find(|member| !self.asked.contains(*member))
    }

    pub fn arrived(&mut self, member: &str, observation: Observation) {
        self.asked.insert(member.to_owned());
        if matches!(observation, Observation::Unavailable) {
            self.failed.insert(member.to_owned(), false);
        }
        self.pages.insert(member.to_owned(), observation);
        self.bump();
    }

    /// A failed read is asked again once, when the reader scrolls back to its answers.
    pub fn retry_scrolled_back(&mut self) {
        let visible = self.visible.borrow().clone();
        for (member, away) in self.failed.clone() {
            if !visible.contains(&member) {
                self.failed.insert(member, true);
            } else if away {
                self.failed.remove(&member);
                self.asked.remove(&member);
                self.pages.remove(&member);
                self.bump();
            }
        }
    }

    /// The bodies to hand to the writer: the answers whose text was painted in the viewport.
    pub fn painted(&self) -> Vec<Body> {
        self.painted.borrow().clone()
    }

    pub fn member_at_top(&self) -> Option<String> {
        let ordered = self.ordered();
        self.top
            .get()
            .and_then(|top| ordered.get(top))
            .map(|answer| answer.member.clone())
    }

    fn title(&self, names: &dyn Fn(&str) -> String) -> String {
        let answers = self.items.len();
        let members = self.shown_members().count();
        match (self.members.len(), members) {
            (_, 0) => "Unread".to_owned(),
            (1, _) => format!(
                "Unread · {} · {answers} answer{}",
                names(&self.members[0]),
                if answers == 1 { "" } else { "s" }
            ),
            _ => format!(
                "Unread · {answers} answer{} from {members} agent{}",
                if answers == 1 { "" } else { "s" },
                if members == 1 { "" } else { "s" }
            ),
        }
    }

    fn text<'a>(&'a self, actor: &str, answer: &Answer) -> Text<'a> {
        match self.pages.get(&answer.member) {
            None | Some(Observation::Pending) => Text::Waiting,
            Some(Observation::Unavailable) => Text::Failed,
            Some(Observation::Observed(page)) => text_of(page, actor, answer),
        }
    }

    fn build(&self, app: &App, actor: &str, width: usize, look: crate::look::Look) -> Built {
        let ordered = self.ordered();
        let mut lines = Vec::new();
        let mut blocks = Vec::new();
        let mut current: Option<&str> = None;
        let now = crate::status::now_ms();
        for (index, answer) in ordered.iter().enumerate() {
            let name = app.member_label(&answer.member);
            if current != Some(answer.member.as_str()) {
                current = Some(answer.member.as_str());
                if !lines.is_empty() {
                    lines.push(Line::default());
                }
                let count = ordered
                    .iter()
                    .filter(|other| other.member == answer.member)
                    .count();
                lines.push(rule(&name, count, width, look));
            }
            let start = lines.len();
            lines.push(exchange_view::header(
                exchange_view::Sender::Agent(&name),
                "you",
                &format!(
                    "in turn · {}",
                    super::conversation::exchange::age_stamp(&crate::requests::age(
                        now,
                        answer.answered_at_ms
                    ))
                ),
                false,
                width,
                look,
            ));
            let body_start = lines.len();
            match self.text(actor, answer) {
                Text::Found { text, truncated } => {
                    let text = super::notes::sanitize(text);
                    let mapped = super::markdown::render_conversation_with_members(
                        &text,
                        width.saturating_sub(3).max(1),
                        look,
                        Some(&app.references),
                    );
                    lines.extend(
                        mapped
                            .lines
                            .into_iter()
                            .map(|line| exchange_view::body(line, look)),
                    );
                    if truncated {
                        lines.push(caption(
                            "◌ answer truncated · full text in the agent's pane",
                            look,
                        ));
                    }
                }
                Text::Waiting => lines.push(caption("Loading answers…", look)),
                Text::Failed => lines.push(caption(
                    "✗ Answers unavailable · scroll away and back to retry",
                    look,
                )),
                Text::Missing => lines.push(caption("○ answer not loaded · open it in chat", look)),
                Text::Unavailable => lines.push(caption("○ reply text unavailable", look)),
            }
            let end = lines.len();
            let has_text = matches!(self.text(actor, answer), Text::Found { .. });
            blocks.push(Block {
                item: index,
                rows: start..end,
                body: if has_text { body_start..end } else { 0..0 },
            });
        }
        if self.more && !lines.is_empty() {
            lines.push(Line::default());
            lines.push(caption(
                "○ Older unread answers are not loaded · they appear as these are read",
                look,
            ));
        }
        Built {
            width,
            look,
            version: self.version.get(),
            lines,
            blocks,
        }
    }

    pub fn render(&self, frame: &mut Frame, app: &App, body: Rect) {
        let look = app.look();
        let Some(actor) = app.view.as_ref().and_then(|view| view.me_id.clone()) else {
            return;
        };
        let names = |member: &str| app.member_label(member);
        let mut modal = Modal {
            title: self.title(&names),
            placement: if body.height < 8 {
                Placement::Body
            } else {
                Placement::Center
            },
        };
        let areas = modal.areas(body, [110, 32], true, true);
        if areas.content.height == 0 {
            modal.title = "Enlarge terminal · Esc closes unread".into();
        }
        modal.paint(areas, frame.buffer_mut(), &look.theme, look.depth);
        app.peek_areas
            .set_dialog(OverlayId::Unread, Some(areas.outer));
        let width = usize::from(areas.content.width);
        let stale = self.built.borrow().as_ref().is_none_or(|built| {
            built.width != width || built.look != look || built.version != self.version.get()
        });
        if stale {
            *self.built.borrow_mut() = Some(self.build(app, &actor, width, look));
        }
        let built = self.built.borrow();
        let built = built.as_ref().expect("built just above");
        let mut scroll = self.scroll.borrow_mut();
        scroll.update(areas.content, built.lines.len(), None);
        let offset = scroll.offset();
        let height = usize::from(areas.content.height);
        let visible = offset..offset + height;
        for (i, line) in built.lines.iter().skip(offset).take(height).enumerate() {
            strip::paint_left(
                frame.buffer_mut(),
                Rect::new(
                    areas.content.x,
                    areas.content.y + i as u16,
                    areas.content.width,
                    1,
                ),
                line.clone(),
            );
        }
        let ordered = self.ordered();
        let mut painted = Vec::new();
        let mut top = None;
        let mut visible_members = BTreeSet::new();
        for block in &built.blocks {
            if block.rows.start < visible.end
                && block.rows.end > visible.start
                && let Some(answer) = ordered.get(block.item)
            {
                visible_members.insert(answer.member.clone());
            }
            if top.is_none() && block.rows.end > offset {
                top = Some(block.item);
            }
            let seen = block.body.start < visible.end && block.body.end > visible.start;
            if seen && let Some(answer) = ordered.get(block.item) {
                painted.push(Body {
                    actor: actor.clone(),
                    member: answer.member.clone(),
                    request: answer.request.clone(),
                    kind: answer.kind.unwrap_or(BodyKind::Final),
                    at_ms: Some(answer.answered_at_ms),
                    revision: Some(answer.revision),
                });
            }
        }
        self.top.set(top);
        *self.visible.borrow_mut() = visible_members;
        *self.painted.borrow_mut() = painted;
        if self.items.is_empty() {
            strip::paint_left(
                frame.buffer_mut(),
                Rect::new(areas.content.x, areas.content.y, areas.content.width, 1),
                Line::styled("Nothing unread here.", look.role(Role::Dim)),
            );
        }
        strip::paint_left(
            frame.buffer_mut(),
            areas.footer,
            Line::styled(
                "↑↓ PgUp PgDn scroll · ⏎ open in chat · Esc close · shown answers are marked read",
                look.role(Role::Dim),
            ),
        );
    }

    pub fn invalidate(&self) {
        self.bump();
        let n = self.scroll.borrow().content();
        self.scroll.borrow_mut().update(Rect::default(), n, None);
    }
}

fn rule(name: &str, count: usize, width: usize, look: crate::look::Look) -> Line<'static> {
    let label = format!("── {name} · {count} ");
    let fill = width.saturating_sub(unicode_width::UnicodeWidthStr::width(label.as_str()));
    Line::styled(format!("{label}{}", "─".repeat(fill)), look.role(Role::Dim))
}

fn caption(text: &str, look: crate::look::Look) -> Line<'static> {
    exchange_view::feedback(Line::styled(text.to_owned(), look.role(Role::Dim)), look)
}

/// The text of one unread answer in a member's loaded answers: the reply to the request for a
/// `Final`, the turn that acknowledges the request for a `Turn`.
fn text_of<'a>(page: &'a recent_responses::Page, actor: &str, answer: &Answer) -> Text<'a> {
    use recent_responses::timeline::Kind;
    let Some(timeline) = &page.timeline else {
        return Text::Missing;
    };
    for entry in &timeline.entries {
        match (&entry.kind, answer.kind) {
            (Kind::Reply { request, body }, Some(BodyKind::Final))
                if *request == answer.request =>
            {
                return match body {
                    Some(body) => Text::Found {
                        text: &body.text,
                        truncated: body.truncated,
                    },
                    None => Text::Unavailable,
                };
            }
            (
                Kind::Final {
                    response_id,
                    available,
                },
                Some(BodyKind::Turn),
            ) if entry
                .acknowledgements
                .iter()
                .any(|key| key.identity == actor && key.request == answer.request) =>
            {
                return match timeline.responses.get(response_id).filter(|_| *available) {
                    Some(item) => Text::Found {
                        text: &item.text,
                        truncated: item.truncated,
                    },
                    None => Text::Unavailable,
                };
            }
            _ => {}
        }
    }
    Text::Missing
}

impl App {
    /// A member's name as the list shows it.
    pub(in crate::board) fn member_label(&self, member: &str) -> String {
        self.workspace_snapshot()
            .and_then(|snapshot| snapshot.roster.members.get(member))
            .map_or_else(|| "agent".to_owned(), |member| member.name.clone())
    }

    /// The members of `scope` in the order the Agents list shows them.
    pub(in crate::board) fn unread_scope_members(&self, scope: &UnreadScope) -> Vec<String> {
        let listed: Vec<String> = self
            .workspace_rows()
            .iter()
            .filter_map(|row| match &row.target {
                OpsTarget::Member { identity_id, .. } => Some(identity_id.clone()),
                _ => None,
            })
            .collect();
        let mut members: Vec<String> = match scope {
            UnreadScope::All => self
                .workspace_snapshot()
                .map(|snapshot| snapshot.roster.members.keys().cloned().collect())
                .unwrap_or_default(),
            UnreadScope::Member(member) => vec![member.clone()],
            UnreadScope::Group(group) => group.clone(),
        };
        members.sort_by_key(|member| {
            listed
                .iter()
                .position(|listed| listed == member)
                .unwrap_or(usize::MAX)
        });
        members.dedup();
        members
    }

    /// Records a painted unread count of `target` (or of everyone) as a target for a press.
    pub(in crate::board) fn push_unread_hit(&self, area: Rect, target: Option<&OpsTarget>) {
        if area.is_empty() {
            return;
        }
        self.unread_hits.borrow_mut().push(Hit {
            area,
            target: target.cloned(),
        });
    }

    /// A row's STATE count, when it has one.
    pub(in crate::board) fn push_unread_row_hit(&self, area: Rect, target: &OpsTarget) {
        if self.unread_target(target).total > 0 {
            self.push_unread_hit(area, Some(target));
        }
    }

    /// A left press on a painted unread count opens the overlay for what it counts.
    pub(in crate::board) fn unread_count_mouse(
        &mut self,
        event: ratatui::crossterm::event::MouseEvent,
    ) -> Option<Effect> {
        use ratatui::crossterm::event::{MouseButton, MouseEventKind};
        if event.kind != MouseEventKind::Down(MouseButton::Left) {
            return None;
        }
        let at = (event.column, event.row).into();
        let target = self
            .unread_hits
            .borrow()
            .iter()
            .find(|hit| hit.area.contains(at))
            .map(|hit| hit.target.clone())?;
        let scope = self.unread_scope_of(target.as_ref());
        Some(self.open_unread(&scope))
    }

    /// What a count belongs to: an agent, a squad or the Common pool, or everyone.
    fn unread_scope_of(&self, target: Option<&OpsTarget>) -> UnreadScope {
        match (target, self.workspace_snapshot()) {
            (Some(OpsTarget::Member { identity_id, .. }), _) => {
                UnreadScope::Member(identity_id.clone())
            }
            (Some(OpsTarget::Squad(room)), Some(snapshot)) => snapshot
                .roster
                .squads
                .iter()
                .find(|squad| &squad.room_id == room)
                .map_or(UnreadScope::All, |squad| {
                    UnreadScope::Group(squad.members.to_vec())
                }),
            (Some(OpsTarget::Pool), Some(snapshot)) => UnreadScope::Group(
                snapshot
                    .roster
                    .pool()
                    .map(|member| member.id.clone())
                    .collect(),
            ),
            _ => UnreadScope::All,
        }
    }

    /// The `u` key: the selected agent, the selected squad or the Common pool when one is
    /// selected in the list, otherwise everyone.
    pub(in crate::board) fn workspace_unread(&mut self) -> Effect {
        let selected = self
            .workspace_selection_visible()
            .then(|| self.workspace.selected.clone())
            .flatten();
        let scope = self.unread_scope_of(selected.as_ref());
        self.open_unread(&scope)
    }

    /// Opens the overlay for `scope`; "Nothing unread here." when there is nothing to show.
    pub(in crate::board) fn open_unread(&mut self, scope: &UnreadScope) -> Effect {
        let members = self.unread_scope_members(scope);
        self.unread_session += 1;
        let mut overlay = Overlay::new(self.unread_session, members);
        let Some(actor) = self.view.as_ref().and_then(|view| view.me_id.clone()) else {
            return self.say("Nothing unread here.");
        };
        if let Some(page) = self.home_leads.unread(&actor) {
            overlay.sync(page);
        }
        if overlay.is_empty() {
            return self.say("Nothing unread here.");
        }
        self.unread_overlay = Some(overlay);
        Effect::None
    }

    pub(in crate::board) fn unread_overlay_event(&mut self, event: &Event) -> Option<Effect> {
        let overlay = self.unread_overlay.as_ref()?;
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => match key.code {
                KeyCode::Esc | KeyCode::Char('q') => {
                    self.unread_overlay = None;
                    self.unread.displayed.clear();
                    return Some(Effect::None);
                }
                KeyCode::Enter => {
                    let member = overlay.member_at_top();
                    self.unread_overlay = None;
                    if let Some(member) = member {
                        return Some(self.open_member_conversation(&member));
                    }
                    return Some(Effect::None);
                }
                _ => {}
            },
            _ => {}
        }
        overlay.scroll.borrow_mut().input(event);
        Some(Effect::None)
    }

    /// Selects `member` and shows their conversation, as Enter on their row does.
    pub(in crate::board) fn open_member_conversation(&mut self, member: &str) -> Effect {
        let room_id = self.workspace_snapshot().and_then(|snapshot| {
            snapshot
                .roster
                .squads
                .iter()
                .find(|squad| squad.members.iter().any(|id| id == member))
                .map(|squad| squad.room_id.clone())
        });
        let target = OpsTarget::Member {
            room_id,
            identity_id: member.to_owned(),
        };
        self.workspace.selected = Some(target.clone());
        self.show_conversation_of(&target);
        Effect::None
    }

    /// After a draw: the answers whose text the overlay painted in its viewport are read, as
    /// the conversation's painted answers are. A block not scrolled into view stays unread.
    pub(in crate::board) fn unread_overlay_drawn(&mut self) {
        self.unread_overlay_sync();
        if let Some(overlay) = &mut self.unread_overlay {
            overlay.retry_scrolled_back();
        }
        let Some(overlay) = &self.unread_overlay else {
            return;
        };
        let Some(actor) = self.view.as_ref().and_then(|view| view.me_id.clone()) else {
            return;
        };
        let page = self.home_leads.unread(&actor);
        let next: Vec<Body> = overlay
            .painted()
            .into_iter()
            .filter_map(|mut body| {
                let answer = page?.answer(&body.member, &body.request)?;
                if answer.acknowledged {
                    return None;
                }
                body.revision = Some(answer.revision);
                Some(body)
            })
            .collect();
        for seen in &next {
            if !self.unread.displayed.contains(seen) {
                self.unread.writer.body(seen);
            }
        }
        self.unread.displayed = next;
    }

    /// The overlay's own demand on the selected lane: the next member's answers to read.
    pub(in crate::board) fn unread_overlay_read(
        &self,
    ) -> Option<(u64, recent_responses::AnswerRead)> {
        let overlay = self.unread_overlay.as_ref()?;
        let actor = self.view.as_ref()?.me_id.clone()?;
        let member = overlay.next_read()?;
        Some((
            overlay.session,
            recent_responses::AnswerRead {
                key: (Some(actor), member.to_owned()),
                before: None,
                held: None,
                rebuild: false,
            },
        ))
    }

    pub(in crate::board) fn unread_overlay_arrived(
        &mut self,
        session: u64,
        read: &recent_responses::AnswerRead,
        observation: Observation,
    ) {
        if let Some(overlay) = self
            .unread_overlay
            .as_mut()
            .filter(|o| o.session == session)
        {
            overlay.arrived(&read.key.1, observation);
        }
    }

    /// New unread answers join an open overlay below what it shows.
    pub(in crate::board) fn unread_overlay_sync(&mut self) {
        let Some(actor) = self.view.as_ref().and_then(|view| view.me_id.clone()) else {
            return;
        };
        if let (Some(overlay), Some(page)) =
            (&mut self.unread_overlay, self.home_leads.unread(&actor))
        {
            overlay.sync(page);
        }
    }
}

#[cfg(test)]
pub(in crate::board) mod tests;

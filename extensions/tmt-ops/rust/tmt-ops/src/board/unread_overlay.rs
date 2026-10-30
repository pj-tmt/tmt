//! The Unread overlay owns stable scoped rows and one selected detail. List previews never
//! acknowledge; selected body rows and terminal unavailable captions use the conversation's
//! exact-revision writer. Loading and failed reads grant no evidence.
use super::{
    app::{App, Effect},
    recent_responses::{self, Observation},
    unread::{Answer, BodyKind, Page, walk::Body},
    workspace::OpsTarget,
};
use ratatui::{layout::Rect, text::Line};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet},
    ops::Range,
};
use tmt_tui::components::{ListFrame, ListRow, ListState, ScrollState};

mod input;
mod surface;

/// Which members' unread an open overlay lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum UnreadScope {
    All,
    Member(String),
    /// A squad or the Common pool: the members of the group.
    Group(Vec<String>),
}

/// One bounded continuation on the existing selected lane, fenced by overlay session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::board) struct PageRead {
    pub actor: String,
    pub before: serde_json::Value,
}

impl PageRead {
    pub fn read(&self, core: &crate::core::Core) -> Result<Page, String> {
        let document = core
            .api(
                "requests.list",
                serde_json::json!({
                    "originatorId":self.actor,"view":"answers","limit":super::unread::ANSWERS_LIMIT,
                    "before":self.before
                }),
            )
            .map_err(|error| error.to_string())?;
        let page = Page::read(&self.actor, &document)
            .ok_or("Unread continuation returned malformed answer metadata")?;
        if let Some(next) = &page.next_before {
            let key = |cursor: &serde_json::Value| {
                (
                    cursor["answeredAtMs"].as_u64().unwrap_or(0),
                    cursor["requestId"].as_str().unwrap_or("").to_owned(),
                )
            };
            if key(next) >= key(&self.before) {
                return Err("Unread continuation did not advance".into());
            }
        }
        Ok(page)
    }
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

struct Built {
    width: usize,
    look: crate::look::Look,
    version: u64,
    /// The minute the ages in the headers were computed for.
    minute: u64,
    lines: Vec<Line<'static>>,
    body: Range<usize>,
}

pub(in crate::board) struct Overlay {
    /// Which opening this is; a read started for an earlier one is dropped.
    session: u64,
    /// Frozen originator for this opening; changing viewer cannot inherit its evidence.
    actor: Option<String>,
    /// The members of the scope in the list's order.
    members: Vec<String>,
    /// Every answer shown since the overlay opened, kept after it is acknowledged so a read
    /// block does not vanish under the reader; later answers are added, never reordered.
    items: BTreeMap<(String, String), Answer>,
    /// The keys of `items` in the order they are shown: sorted when the overlay opens, later
    /// arrivals appended, so what is on screen never moves.
    order: Vec<(String, String)>,
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
    list: RefCell<ListState>,
    list_frame: RefCell<Option<ListFrame>>,
    hover: Cell<Option<usize>>,
    narrow: Cell<bool>,
    detail_open: bool,
    detail_focus: bool,
    confirmation: Option<input::Confirmation>,
    confirm_hits: Cell<Option<(Rect, Rect)>>,
    /// Core holds back older unread answers than the page the board keeps.
    more: bool,
    /// Only the open overlay advances older metadata; HOME refreshes still read the head.
    before: Option<serde_json::Value>,
    next_before: Option<serde_json::Value>,
    page_failed: bool,
}

impl Overlay {
    pub fn new(session: u64, members: Vec<String>) -> Self {
        Self {
            session,
            actor: None,
            members,
            items: BTreeMap::new(),
            order: Vec::new(),
            pages: BTreeMap::new(),
            asked: BTreeSet::new(),
            failed: BTreeMap::new(),
            visible: RefCell::default(),
            scroll: RefCell::default(),
            version: Cell::new(0),
            built: RefCell::default(),
            painted: RefCell::default(),
            list: RefCell::default(),
            list_frame: RefCell::default(),
            hover: Cell::new(None),
            narrow: Cell::new(false),
            detail_open: false,
            detail_focus: false,
            confirmation: None,
            confirm_hits: Cell::new(None),
            more: false,
            before: None,
            next_before: None,
            page_failed: false,
        }
    }

    fn bump(&self) {
        self.version.set(self.version.get() + 1);
    }

    /// Adds the scope's unread answers that the overlay has not shown yet.
    pub fn sync(&mut self, page: &Page) {
        if self
            .actor
            .as_deref()
            .is_some_and(|actor| actor != page.actor)
        {
            return;
        }
        self.actor = Some(page.actor.clone());
        self.merge(page, self.before.is_none());
    }

    fn merge(&mut self, page: &Page, continuation: bool) {
        let opening = self.order.is_empty();
        let mut arrived = Vec::new();
        let mut revised = false;
        for member in &self.members {
            for answer in page.answers(member) {
                let key = (member.clone(), answer.request.clone());
                if let Some(held) = self.items.get_mut(&key) {
                    if answer.revision > held.revision
                        || (answer.revision == held.revision
                            && held.kind.is_none()
                            && answer.kind.is_some())
                    {
                        *held = answer.clone();
                        revised = true;
                        self.pages.remove(member);
                        self.asked.remove(member);
                        self.failed.remove(member);
                    } else if answer.revision == held.revision && answer.acknowledged {
                        held.acknowledged = true;
                    }
                } else if !answer.acknowledged {
                    self.items.insert(key.clone(), answer.clone());
                    arrived.push(key);
                }
            }
        }
        // `members` is in list order, so the opening keeps the list's grouping.
        arrived.sort_by_key(|key| {
            let answer = &self.items[key];
            (
                if opening {
                    self.members.iter().position(|member| *member == key.0)
                } else {
                    None
                },
                answer.answered_at_ms,
                key.1.clone(),
            )
        });
        let changed = !arrived.is_empty();
        // A member whose text was read is read again for its late answer; what it showed
        // stays until the new read lands, so nothing on screen moves.
        for (member, _) in &arrived {
            if !self.failed.contains_key(member) {
                self.asked.remove(member);
            }
        }
        self.order.extend(arrived);
        let rows = (0..self.order.len())
            .map(|index| ListRow {
                id: format!("answer-{index}"),
                disabled: false,
            })
            .collect();
        self.list
            .borrow_mut()
            .reconcile(rows)
            .expect("stable unread rows");
        let more = if continuation {
            self.next_before = page.next_before.clone();
            self.next_before.is_some()
        } else {
            self.more
        };
        if changed || revised || self.more != more {
            self.more = more;
            self.bump();
        }
    }

    /// The active overlay already owns these retained entries. Include older unread
    /// metadata in displayed counts without counting the HOME head page twice.
    pub fn additional_count(&self, page: &Page, member: &str) -> super::unread::Count {
        if self.actor.as_deref() != Some(&page.actor) {
            return super::unread::Count::default();
        }
        let total = self
            .items
            .values()
            .filter(|answer| {
                answer.member == member
                    && !answer.acknowledged
                    && page.answer(member, &answer.request).is_none()
            })
            .count();
        super::unread::Count {
            total,
            more: total > 0 && self.more,
        }
    }

    /// Metadata reads never grant acknowledgment. Each read yields to the lane, and a
    /// failed read stays stopped until this overlay is reopened.
    fn next_page(&self, actor: &str) -> Option<PageRead> {
        if self.actor.as_deref() != Some(actor)
            || self.page_failed
            || self.items.values().any(|answer| !answer.acknowledged)
        {
            return None;
        }
        Some(PageRead {
            actor: actor.into(),
            before: self.next_before.clone()?,
        })
    }

    fn page_arrived(
        &mut self,
        read: &PageRead,
        result: Result<Page, String>,
    ) -> Result<(), String> {
        let page = match result {
            Ok(page) => page,
            Err(error) => {
                self.page_failed = true;
                return Err(error);
            }
        };
        self.before = Some(read.before.clone());
        self.merge(&page, true);
        Ok(())
    }

    pub fn acknowledged(&mut self, completed: &super::unread::publish::Completed) {
        self.bump();
        for (sent, outcome) in &completed.items {
            let key = (sent.member.clone(), sent.request.clone());
            if let Some(answer) = self.items.get_mut(&key) {
                let keep = answer.settle(sent, outcome);
                if !keep {
                    // Keep the already painted block but never offer it again.
                    answer.acknowledged = true;
                }
            }
        }
    }

    /// The members with something to show, in the list's order.
    fn shown_members(&self) -> impl Iterator<Item = &String> {
        self.members
            .iter()
            .filter(|member| self.items.keys().any(|(owner, _)| owner == *member))
    }

    fn ordered(&self) -> Vec<&Answer> {
        self.order
            .iter()
            .filter_map(|key| self.items.get(key))
            .collect()
    }

    /// Prioritize the selected detail, including deliberate older-body paging, then
    /// acquire the other agents' head pages for list previews, each once.
    pub fn next_read(&self) -> Option<&str> {
        let start = self.selected_answer().map(|answer| answer.member.as_str());
        if self.detail_visible()
            && let Some(answer) = self.selected_answer()
            && let Some(Observation::Observed(page)) = self.pages.get(&answer.member)
            && matches!(
                self.text(self.actor.as_deref().unwrap_or_default(), answer),
                Text::Missing
            )
            && page.timeline.is_some()
            && page.cursor().is_some()
        {
            return Some(&answer.member);
        }
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

    fn selected_index(&self) -> Option<usize> {
        self.list
            .borrow()
            .selected()?
            .strip_prefix("answer-")?
            .parse()
            .ok()
    }

    fn selected_answer(&self) -> Option<&Answer> {
        self.order
            .get(self.selected_index()?)
            .and_then(|key| self.items.get(key))
    }

    fn detail_visible(&self) -> bool {
        !self.narrow.get() || self.detail_open
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

    pub fn invalidate(&self) {
        self.bump();
        self.list_frame.borrow_mut().take();
        self.hover.set(None);
        self.confirm_hits.set(None);
        self.painted.borrow_mut().clear();
        let n = self.scroll.borrow().content();
        self.scroll.borrow_mut().update(Rect::default(), n, None);
    }
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
        if self.unread_counted.get() && self.unread_target(target).total > 0 {
            self.push_unread_hit(area, Some(target));
        }
    }

    /// Highlight the painted unread count under the pointer, in the composer chip's hover
    /// style, while the board itself takes input.
    pub(in crate::board) fn paint_unread_hover(&self, buffer: &mut ratatui::buffer::Buffer) {
        let Some(area) = self.unread_hover.get() else {
            return;
        };
        if self.modal_open()
            || self.input.is_some()
            || !self.unread_hits.borrow().iter().any(|hit| hit.area == area)
        {
            return;
        }
        let look = self.look();
        let style = tmt_cli_style::theme::screen::chip(
            &look.theme,
            tmt_cli_style::theme::screen::ChipKind::Secondary,
            tmt_cli_style::theme::screen::ChipState::Hot,
            look.depth,
        );
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                buffer[(x, y)].set_style(style);
            }
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

    /// Opens the scope, including its explicit empty state.
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
        overlay.actor = Some(actor);
        self.unread_overlay = Some(overlay);
        Effect::None
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

    /// After a successful draw: visible bodies and terminal unavailable captions admit
    /// their exact revisions. Hidden blocks, loading and failed reads stay unread.
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
        let next: Vec<Body> = overlay
            .painted()
            .into_iter()
            .filter(|body| {
                body.actor == actor
                    && overlay
                        .items
                        .get(&(body.member.clone(), body.request.clone()))
                        .is_some_and(|answer| {
                            !answer.acknowledged
                                && Some(answer.revision) == body.revision
                                && answer.kind == Some(body.kind)
                        })
            })
            .collect();
        for seen in &next {
            if !self.unread.displayed.contains(seen) {
                self.unread.writer.body(seen);
            }
        }
        self.unread.displayed = next;
    }

    pub(in crate::board) fn unread_overlay_page(&self) -> Option<(u64, PageRead)> {
        let overlay = self.unread_overlay.as_ref()?;
        let actor = self.view.as_ref()?.me_id.as_deref()?;
        overlay.next_page(actor).map(|read| (overlay.session, read))
    }

    pub(in crate::board) fn unread_overlay_page_arrived(
        &mut self,
        session: u64,
        read: &PageRead,
        result: Result<Page, String>,
    ) {
        if self.view.as_ref().and_then(|view| view.me_id.as_deref()) != Some(&read.actor) {
            return;
        }
        let Some(overlay) = self
            .unread_overlay
            .as_mut()
            .filter(|overlay| overlay.session == session)
        else {
            return;
        };
        if overlay.next_page(&read.actor).as_ref() != Some(read) {
            return;
        }
        if let Err(error) = overlay.page_arrived(read, result) {
            self.say(format!(
                "Older unread answers unavailable: {error} · reopen Unread to retry"
            ));
        }
    }

    /// The overlay's own demand on the selected lane: the next member's answers to read.
    pub(in crate::board) fn unread_overlay_read(
        &self,
    ) -> Option<(u64, recent_responses::AnswerRead)> {
        let overlay = self.unread_overlay.as_ref()?;
        let actor = self.view.as_ref()?.me_id.clone()?;
        if overlay.actor.as_deref() != Some(&actor) {
            return None;
        }
        let member = overlay.next_read()?;
        let older = overlay.detail_visible()
            && overlay.selected_answer().is_some_and(|answer| {
                answer.member == member && matches!(overlay.text(&actor, answer), Text::Missing)
            });
        let held = if older {
            match overlay.pages.get(member) {
                Some(Observation::Observed(page))
                    if page.timeline.is_some() && page.cursor().is_some() =>
                {
                    Some(page.clone())
                }
                _ => None,
            }
        } else {
            None
        };
        Some((
            overlay.session,
            recent_responses::AnswerRead {
                key: (Some(actor), member.to_owned()),
                before: held.as_ref().and_then(|page| page.cursor()),
                held,
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

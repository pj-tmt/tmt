//! One read-only entity detail projection and renderer for every board row.
//! Acquisition remains on the refresh worker; this module only consumes snapshots.
use super::{
    app::{App, Effect, RowTarget},
    home_leads::{Kind, MessageKey},
};
use crate::look::Look;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    text::{Line, Span},
};
use serde_json::{Value, json};
use tmt_cli_style::Role;
use unicode_width::UnicodeWidthStr;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Target {
    Row(RowTarget),
    Job(String),
}

#[derive(Default)]
pub(super) struct State {
    pub expanded: Vec<Target>,
    /// Bodies belong to active rows and exact request identities, never row positions.
    bodies: Vec<(MessageKey, Result<String, String>)>,
    pub reader: Option<Reader>,
}

impl State {
    pub fn contains(&self, target: &Target) -> bool {
        self.expanded.contains(target)
    }
    pub fn toggle(&mut self, target: Target) {
        if let Some(index) = self.expanded.iter().position(|entry| entry == &target) {
            self.expanded.remove(index);
        } else {
            self.expanded.push(target);
        }
    }
    pub fn reconcile(&mut self, alive: &[Target], keys: &[MessageKey]) {
        self.expanded.retain(|target| alive.contains(target));
        if self
            .reader
            .as_ref()
            .is_some_and(|reader| reader.frozen.is_none() && !alive.contains(&reader.target))
        {
            self.reader = None;
        }
        self.bodies.retain(|(key, _)| keys.contains(key));
    }
    fn body_failed(&self, key: &MessageKey) -> bool {
        self.bodies
            .iter()
            .any(|(current, body)| current == key && body.is_err())
    }
    fn body(&self, key: &MessageKey) -> Option<&str> {
        self.bodies
            .iter()
            .find(|(current, _)| current == key)
            .map(|(_, body)| match body {
                Ok(text) | Err(text) => text.as_str(),
            })
    }
}

#[derive(Clone)]
pub(super) struct Field {
    pub label: String,
    pub text: String,
    pub role: Role,
}

#[derive(Clone)]
pub(super) struct Reply {
    pub from: String,
    pub age: String,
    pub body: String,
    pub key: Option<MessageKey>,
    pub acquired: bool,
    /// Only successful exact bodies may produce source-range attachments.
    pub source: bool,
}

#[derive(Clone, Default)]
pub(super) struct Detail {
    pub fields: Vec<Field>,
    pub reply: Option<Reply>,
    pub show_reply: bool,
}

/// A collapsed field suppresses detail only if its complete text survives the
/// same cell budget and overflow policy that paint uses.
pub(super) fn uncut(text: &str, width: usize, flow: tmt_tui::style::TextFlow) -> bool {
    use tmt_tui::style::TextFlow;
    let text = tmt_cli_style::table::escape(text);
    let width = width.min(usize::from(u16::MAX));
    if width == 0 {
        return false;
    }
    match flow {
        TextFlow::Wrap | TextFlow::Clamp(_) => {
            let content = |text: &str| {
                text.chars()
                    .filter(|ch| !ch.is_whitespace())
                    .collect::<String>()
            };
            content(&tmt_tui::text::lines(&text, width as u16, flow).join("")) == content(&text)
        }
        _ => text.width() <= width,
    }
}

impl Detail {
    fn add(&mut self, label: &str, text: Option<&str>, role: Role) {
        if let Some(text) = text.filter(|text| !text.is_empty()) {
            self.fields.push(Field {
                label: label.to_owned(),
                text: super::notes::sanitize(text),
                role,
            });
        }
    }
    pub fn additional(mut self, visible: &[&str]) -> Self {
        self.fields
            .retain(|field| !visible.contains(&field.label.as_str()));
        self
    }
    /// Cache keys contain exactly the presentation inputs; request identities stay
    /// in application state, outside the renderer's data.
    pub fn value(&self) -> Value {
        json!({"fields": self.fields.iter().map(|field| json!({"label":field.label,"text":field.text,"role":field.role.name()})).collect::<Vec<_>>(),
            "reply": self.reply.as_ref().map(|reply| json!({"from":reply.from,"age":reply.age,"body":reply.body})),
            "show_reply": self.show_reply})
    }
}

fn age(at: Option<u64>, now: u64) -> String {
    at.filter(|at| *at > 0 && *at <= now)
        .map(|at| crate::requests::age(now, at))
        .unwrap_or_else(|| "–".into())
}

impl App {
    pub(super) fn row_detail(&self, target: &RowTarget, visible: &[&str]) -> Option<Detail> {
        if matches!(target, RowTarget::Home(target) if target.section == super::home::USAGE) {
            return self.overview_detail(self.body_width());
        }
        let row = match target {
            RowTarget::Workspace(_) => self.target_row(target)?,
            RowTarget::Home(target) if target.section == super::home::LEADS => {
                &self
                    .home_leads
                    .leads
                    .iter()
                    .find(|lead| {
                        lead.squad == target.squad && Some(lead.id()) == target.member.as_deref()
                    })?
                    .row
            }
            RowTarget::Home(target) => {
                &self
                    .view
                    .as_ref()?
                    .home
                    .as_ref()?
                    .sections
                    .iter()
                    .find(|section| section.key == target.section)?
                    .rows
                    .iter()
                    .find(|row| {
                        row.squad == target.squad
                            && row.member["id"].as_str() == target.member.as_deref()
                    })?
                    .member
            }
            RowTarget::Member {
                tab,
                section,
                squad,
                id,
            } if self.shown_tab() == Some(tab) => self.view.as_ref()?.document["sections"]
                [*section]["rows"]
                .as_array()?
                .iter()
                .find(|row| row["id"] == *id && row["squad"].as_str().unwrap_or(tab) == squad)?,
            RowTarget::Lead { .. } => self.target_row(target)?,
            _ => return None,
        };
        let home_lead =
            matches!(target, RowTarget::Home(target) if target.section == super::home::LEADS);
        let home_member = matches!(target, RowTarget::Home(_)) && !home_lead;
        let mut detail = Detail {
            show_reply: true,
            ..Detail::default()
        };
        if home_lead
            && let RowTarget::Home(target) = target
            && let Some(lead) = self.home_leads.leads.iter().find(|lead| {
                lead.squad == target.squad && Some(lead.id()) == target.member.as_deref()
            })
        {
            for (label, text) in super::home::hidden_lead_metadata(
                &self.labels,
                lead,
                self.body_width(),
                crate::status::now_ms(),
            ) {
                detail.add(label, Some(&text), Role::Muted);
            }
        }
        if !visible.contains(&"task") {
            detail.add("task", row["fields"]["task"].as_str(), Role::Text);
        }
        if !home_member && !visible.contains(&"pending") {
            let pending = super::view::waiting::text(row).map(|text| format!("◆ {text}"));
            detail.add("pending", pending.as_deref(), Role::Waiting);
        }
        if !home_lead && !home_member {
            if !visible.contains(&"note") {
                detail.add(
                    "note",
                    row["note"]
                        .as_str()
                        .or_else(|| row["fields"]["note"].as_str()),
                    Role::Text,
                );
            }
            let links = row["fields"]
                .as_object()
                .into_iter()
                .flatten()
                .filter(|(key, _)| {
                    (key.as_str() == "link" || key.ends_with("_link"))
                        && !visible.contains(&key.as_str())
                })
                .filter_map(|(_, value)| value.as_str())
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
                .join("  ");
            if !visible.contains(&"links") {
                detail.add("links", Some(&links), Role::Link);
            }
            if !visible.contains(&"model") {
                detail.add("model", row["fields"]["model"].as_str(), Role::Text);
            }
        }
        let now = crate::status::now_ms();
        if !visible.contains(&"chips") {
            for (label, text) in crate::board::row_chips::detail(&self.labels, row, now) {
                detail.add(&label, Some(&text), Role::Muted);
            }
        }
        let (squad, id) = match target {
            RowTarget::Workspace(super::workspace::OpsTarget::Member { identity_id, .. }) => (
                row["squad"].as_str().unwrap_or("Common pool"),
                identity_id.as_str(),
            ),
            RowTarget::Workspace(_) => return None,
            RowTarget::Home(target) => (target.squad.as_str(), target.member.as_deref()?),
            RowTarget::Member { squad, id, .. } | RowTarget::Lead { squad, id, .. } => {
                (squad.as_str(), id.as_str())
            }
        };
        let sender = self.view.as_ref().and_then(|view| view.me_id.clone());
        if let Some(reply) = self
            .home_leads
            .replies
            .iter()
            .chain(self.view.iter().flat_map(|view| &view.replies))
            .filter(|reply| reply["recipientId"] == id)
            .max_by_key(|reply| (reply["submittedAtMs"].as_u64(), reply["requestId"].as_str()))
        {
            let key = sender
                .zip(reply["requestId"].as_str())
                .map(|(sender, request)| MessageKey {
                    sender,
                    lead: id.into(),
                    squad: squad.to_owned(),
                    request: request.into(),
                    kind: Kind::Reply,
                });
            let body = reply["response"]
                .as_str()
                .or_else(|| key.as_ref().and_then(|key| self.row_details.body(key)));
            detail.reply = Some(Reply {
                from: row["name"].as_str().unwrap_or("–").into(),
                age: age(reply["submittedAtMs"].as_u64(), now),
                body: body.unwrap_or("(reading reply…)").into(),
                source: body.is_some()
                    && (reply["response"].is_string()
                        || key
                            .as_ref()
                            .is_none_or(|key| !self.row_details.body_failed(key))),
                key,
                acquired: body.is_some(),
            });
        } else if home_lead {
            let lead = self
                .home_leads
                .leads
                .iter()
                .find(|lead| lead.squad == *squad && lead.id() == id)?;
            if let Some(exchange) = lead
                .exchange
                .as_ref()
                .filter(|exchange| exchange.kind == Kind::Reply)
            {
                let key = sender
                    .zip(exchange.request.clone())
                    .map(|(sender, request)| MessageKey {
                        sender,
                        lead: id.into(),
                        squad: squad.to_owned(),
                        request,
                        kind: Kind::Reply,
                    });
                let body = key.as_ref().and_then(|key| self.row_details.body(key));
                detail.reply = Some(Reply {
                    from: lead.name().into(),
                    age: age(exchange.since_ms, now),
                    body: body.unwrap_or("(reading reply…)").into(),
                    source: body.is_some()
                        && key
                            .as_ref()
                            .is_none_or(|key| !self.row_details.body_failed(key)),
                    key,
                    acquired: body.is_some(),
                });
            }
        }
        Some(detail.additional(visible))
    }

    fn overview_detail(&self, width: u16) -> Option<Detail> {
        let view = self.view.as_ref()?;
        let home = view.home.as_ref()?;
        let mut detail = Detail::default();
        if let Some(usage) = self.home_header_usage(std::time::Instant::now()) {
            for (label, text) in super::home::hidden_usage(&usage, width) {
                detail.add(label, Some(&text), Role::Muted);
            }
        }
        if view.home_rate.values().any(|rate| rate.settings.enabled)
            && let Some(standings) = &self.limits
        {
            for (label, text) in
                super::home::hidden_limits(standings, crate::status::now_ms(), width)
            {
                detail.add(label, Some(&text), Role::Muted);
            }
        }
        if let Some(names) = super::home::hidden_needs(home, width) {
            detail.add("needs you", Some(&names), Role::Waiting);
        }
        detail.add(
            "overview",
            Some(&format!(
                "{} squads · {} members · ◐ {} review",
                home.squads.len(),
                home.summary.members,
                home.summary.review
            )),
            Role::Muted,
        );
        if width < super::home::SM.cells {
            detail.add(
                "states",
                Some(&format!(
                    "● {} working · ◌ {} idle · ✗ {} blocked",
                    home.summary.working, home.summary.idle, home.summary.blocked
                )),
                Role::Muted,
            );
        } else if width < super::home::MD.cells {
            detail.add(
                "blocked",
                Some(&format!("✗ {} blocked", home.summary.blocked)),
                Role::Muted,
            );
        }
        Some(detail)
    }

    pub(super) fn selected_detail_target(&self) -> Option<Target> {
        if let Some(list) = &self.cron_list {
            return list.selected().map(Target::Job);
        }
        if self.jobs_focus {
            return self.jobs_selected().map(Target::Job);
        }
        self.row_target(self.selected).map(Target::Row)
    }
    pub(super) fn detail(&self, target: &Target) -> Option<Detail> {
        match target {
            Target::Row(target) => self.row_detail(target, &[]),
            Target::Job(id) => self
                .cron
                .cron
                .as_ref()?
                .jobs
                .iter()
                .find(|job| super::cronboard::detail_id(job) == *id)
                .map(|job| super::cronboard::job_detail(job, self.cron.now_ms())),
        }
    }
    pub(super) fn toggle_row_detail(&mut self) -> Effect {
        if let Some(target) = self
            .selected_detail_target()
            .filter(|target| self.detail(target).is_some())
        {
            if matches!(target, Target::Row(_)) {
                self.focus = self
                    .effective_board()
                    .and_then(|board| {
                        board
                            .panes
                            .iter()
                            .position(|pane| *pane == crate::config::Pane::Rows)
                    })
                    .unwrap_or(0);
            }
            self.row_details.toggle(target);
            self.follow = true;
            self.reconcile_row_details();
        }
        Effect::None
    }
    fn active_detail_targets(&self) -> Vec<Target> {
        let mut targets = self.row_details.expanded.clone();
        if self.scrolls.visible(crate::config::Pane::Replies)
            && let Some(target) = self.conversation_target()
        {
            targets.push(Target::Row(target));
        }
        if let Some(reader) = &self.row_details.reader {
            targets.push(reader.target.clone());
        }
        targets
    }
    pub(super) fn reconcile_row_details(&mut self) {
        if self.row_details.reader.as_ref().is_some_and(|reader| {
            reader.frozen.is_none()
                && self
                    .detail(&reader.target)
                    .and_then(|detail| detail.reply)
                    .is_none()
        }) {
            self.row_details.reader = None;
        }
        let alive = self
            .active_detail_targets()
            .into_iter()
            .filter(|target| self.detail(target).is_some())
            .collect::<Vec<_>>();
        let keys = alive
            .iter()
            .filter_map(|target| self.detail(target)?.reply?.key)
            .collect::<Vec<_>>();
        self.row_details.reconcile(&alive, &keys);
    }
    pub(super) fn detail_read(&self) -> Option<MessageKey> {
        if self.loading() {
            return None;
        }
        self.active_detail_targets()
            .iter()
            .filter_map(|target| self.detail(target)?.reply)
            .find(|reply| !reply.acquired)
            .and_then(|reply| reply.key)
    }
    pub(super) fn apply_message(&mut self, key: &MessageKey, body: Result<String, String>) {
        if self.loading()
            || !self.active_detail_targets().iter().any(|target| {
                self.detail(target)
                    .and_then(|detail| detail.reply)
                    .is_some_and(|reply| reply.key.as_ref() == Some(key))
            })
        {
            return;
        }
        self.row_details
            .bodies
            .retain(|(current, _)| current != key);
        self.row_details.bodies.push((
            key.clone(),
            body.map_err(|error| {
                format!("(reply unavailable: {})", super::notes::sanitize(&error))
            }),
        ));
    }
    pub(super) fn view_row_reply(&mut self) -> Effect {
        if let Some(target) = self.selected_detail_target().filter(|target| {
            self.detail(target)
                .is_some_and(|detail| detail.reply.is_some())
        }) {
            self.row_details.reader = Some(Reader::new(
                target,
                self.conversation_reply().filter(|reply| reply.acquired),
            ));
        }
        Effect::None
    }
    pub(super) fn reply_hint(&self) -> bool {
        self.selected_detail_target().is_some_and(|target| {
            self.row_details.contains(&target)
                && self
                    .detail(&target)
                    .is_some_and(|detail| detail.reply.is_some())
        })
    }
}

#[derive(Default)]
struct Block {
    lines: Vec<Line<'static>>,
    more: Option<usize>,
}

/// Render every row kind at its own text-column indent. No selection background
/// enters this block: only the gutter acknowledges the selected parent.
pub(super) fn render(
    data: &Value,
    width: u16,
    indent: u16,
    look: Look,
    selected: bool,
) -> Vec<Line<'static>> {
    block(data, width, indent, look, selected).lines
}

fn block(data: &Value, width: u16, indent: u16, look: Look, selected: bool) -> Block {
    if data.is_null() {
        return Block::default();
    }
    let fields = data["fields"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let labels = if width < super::home::COMPACT {
        5
    } else {
        fields
            .iter()
            .filter_map(|field| field["label"].as_str())
            .map(UnicodeWidthStr::width)
            .max()
            .unwrap_or(5)
            .clamp(5, 10)
    };
    let indent = usize::from(indent.min(width.saturating_sub(1)));
    let value_width = usize::from(width)
        .saturating_sub(indent + labels + 2)
        .max(1);
    let muted = look.role(Role::Muted);
    let mut output = Vec::new();
    let mut more = false;
    let mut line = |label: &str, value: Line<'static>| {
        let mut spans = vec![
            Span::raw(" ".repeat(indent.saturating_sub(1))),
            Span::styled(
                "│",
                look.role(if selected { Role::Accent } else { Role::Dim }),
            ),
            Span::styled(
                if fields.is_empty() && data["show_reply"] != true {
                    String::new()
                } else {
                    format!("{:<labels$}  ", super::view::fit(label, labels).trim_end())
                },
                muted,
            ),
        ];
        spans.extend(value.spans.into_iter().map(|span| {
            let style = value.style.patch(span.style);
            span.style(style)
        }));
        output.push(Line::from(spans));
    };
    for field in fields {
        let label = field["label"].as_str().unwrap_or_default();
        let role = field["role"]
            .as_str()
            .and_then(crate::look::role)
            .unwrap_or(Role::Text);
        let text = super::notes::sanitize(field["text"].as_str().unwrap_or_default());
        let wrapped =
            tmt_tui::text::lines(&text, value_width as u16, tmt_tui::style::TextFlow::Wrap);
        for (index, text) in wrapped.iter().take(3).enumerate() {
            let text = if index == 2 && wrapped.len() > 3 {
                format!(
                    "{}…",
                    super::view::fit(text, value_width.saturating_sub(1)).trim_end()
                )
            } else {
                text.clone()
            };
            line(
                if index == 0 { label } else { "" },
                Line::styled(text, look.role(role)),
            );
        }
    }
    if data["show_reply"] == true {
        if data["reply"].is_null() {
            line(
                "reply",
                Line::styled(
                    super::view::fit("no reply yet", value_width)
                        .trim_end()
                        .to_owned(),
                    muted,
                ),
            );
        } else {
            let reply = &data["reply"];
            line(
                "reply",
                Line::styled(
                    super::view::fit(
                        &format!(
                            "from {} · {}",
                            super::notes::sanitize(reply["from"].as_str().unwrap_or("–")),
                            reply["age"].as_str().unwrap_or("–")
                        ),
                        value_width,
                    )
                    .trim_end()
                    .to_owned(),
                    muted,
                ),
            );
            let lines = super::markdown::render(
                &super::notes::sanitize(reply["body"].as_str().unwrap_or_default()),
                value_width,
                look,
            );
            for text in lines.iter().take(6) {
                line("", text.clone().style(look.role(Role::Text)));
            }
            if lines.len() > 6 {
                line(
                    "",
                    Line::styled(
                        super::view::fit(
                            &format!("… {} more lines · v view", lines.len() - 6),
                            value_width,
                        )
                        .trim_end()
                        .to_owned(),
                        muted,
                    ),
                );
                more = true;
            }
        }
    } else if fields.is_empty() {
        line("", Line::styled("no details yet", muted));
    }
    for line in &mut output {
        line.spans.push(Span::raw(
            " ".repeat(usize::from(width).saturating_sub(line.width())),
        ));
    }
    Block {
        more: more.then(|| output.len() - 1),
        lines: output,
    }
}

/// Paint list-reserved lines using the same renderer, leaving scrolling and hit
/// geometry entirely with the List component.
pub(super) fn paint_list(
    buffer: &mut Buffer,
    map: &tmt_tui::components::ListFrame,
    rows: &[Value],
    look: Look,
    selected: Option<&str>,
    indent: u16,
) {
    for row in &map.geometry {
        let Some(data) = rows.iter().find(|data| data["id"] == row.id) else {
            continue;
        };
        let lines = render(
            &data["expanded_detail"],
            map.viewport.width,
            indent,
            look,
            selected == Some(row.id.as_str()),
        );
        for (index, line) in lines.into_iter().enumerate() {
            let at = row.lines.start + 1 + index;
            if at < map.offset || at >= map.offset + usize::from(map.viewport.height) {
                continue;
            }
            tmt_tui::components::strip::paint_left(
                buffer,
                Rect::new(
                    map.viewport.x,
                    map.viewport.y + (at - map.offset) as u16,
                    map.viewport.width,
                    1,
                ),
                line,
            );
        }
    }
}

/// Record only the generated overflow line, using this frame's row/scroll geometry.
#[allow(clippy::too_many_arguments)]
pub(super) fn more_hit(
    app: &App,
    row: usize,
    data: &Value,
    width: u16,
    indent: u16,
    start: usize,
    area: Rect,
    offset: usize,
    viewport: usize,
) {
    if data["reply"].is_null() {
        return;
    }
    let Some(more) = block(data, width, indent, app.look(), row == app.selected).more else {
        return;
    };
    let at = start + more;
    if at >= offset
        && at < offset + viewport
        && let Some(target) = app.row_target(row)
    {
        app.detail_more_hits.borrow_mut().push((
            Rect::new(area.x, area.y + (at - offset) as u16, area.width, 1),
            Target::Row(target),
        ));
    }
}

pub(super) struct Reader {
    /// A paused conversation opens exactly the response it currently displays.
    pub frozen: Option<Reply>,
    pub selection: Option<super::response::Selection>,
    pub source_area: std::cell::Cell<Option<Rect>>,
    pub target: Target,
    pub scroll: std::cell::RefCell<tmt_tui::components::ScrollState>,
    rendered: std::cell::RefCell<Option<ReaderRender>>,
}

struct ReaderRender {
    source: String,
    width: u16,
    look: Look,
    lines: Vec<Line<'static>>,
}

impl Reader {
    pub fn new(target: Target, frozen: Option<Reply>) -> Self {
        Self {
            target,
            frozen,
            selection: None,
            source_area: Default::default(),
            scroll: Default::default(),
            rendered: Default::default(),
        }
    }

    fn prepare(&self, reply: &Reply, width: u16, look: Look) {
        let mut rendered = self.rendered.borrow_mut();
        if rendered.as_ref().is_some_and(|cached| {
            cached.source == reply.body && cached.width == width && cached.look == look
        }) {
            return;
        }
        *rendered = Some(ReaderRender {
            source: reply.body.clone(),
            width,
            look,
            lines: super::markdown::render(
                &super::notes::sanitize(&reply.body),
                usize::from(width),
                look,
            ),
        });
    }
}

impl App {
    pub(super) fn reader_reply(&self) -> Option<Reply> {
        let reader = self.row_details.reader.as_ref()?;
        reader
            .frozen
            .clone()
            .or_else(|| self.detail(&reader.target)?.reply)
    }
}

pub(super) fn render_reader(frame: &mut ratatui::Frame, app: &App, body: Rect) {
    use tmt_tui::components::{Modal, Placement, strip};
    let Some(reader) = &app.row_details.reader else {
        return;
    };
    let Some(reply) = app.reader_reply() else {
        return;
    };
    let look = app.look();
    let modal = Modal {
        title: format!("reply from {}", reply.from),
        placement: Placement::Body,
    };
    let areas = modal.areas(body, [body.width, body.height], true, false);
    modal.paint(areas, frame.buffer_mut(), &look.theme, look.depth);
    reader.source_area.set(Some(areas.content));
    let mut scroll = reader.scroll.borrow_mut();
    if let Some(selection) = &reader.selection {
        let layout = selection.source.layout(areas.content.width);
        scroll.update(areas.content, layout.lines.len(), None);
        if let Some(caret) = layout.paint(
            frame.buffer_mut(),
            areas.content,
            scroll.offset(),
            selection.source.selection(),
            look.role(Role::Text),
            look.role(Role::Text)
                .add_modifier(ratatui::style::Modifier::REVERSED),
        ) {
            frame.set_cursor_position(caret);
        }
    } else {
        reader.prepare(&reply, areas.content.width, look);
        let rendered = reader.rendered.borrow();
        let lines = &rendered.as_ref().unwrap().lines;
        scroll.update(areas.content, lines.len(), None);
        for (index, line) in lines
            .iter()
            .skip(scroll.offset())
            .take(usize::from(areas.content.height))
            .enumerate()
        {
            strip::paint_left(
                frame.buffer_mut(),
                Rect::new(
                    areas.content.x,
                    areas.content.y + index as u16,
                    areas.content.width,
                    1,
                ),
                line.clone().style(look.role(Role::Text)),
            );
        }
    }
    strip::paint_left(
        frame.buffer_mut(),
        areas.position,
        Line::styled(
            reader.selection.as_ref().map_or_else(
                || scroll.position(),
                |selection| format!("source {} · {}", selection.key.request, scroll.position()),
            ),
            look.role(Role::Muted),
        ),
    );
    strip::paint_left(
        frame.buffer_mut(),
        areas.footer,
        super::view::footer::hint_line(
            if reader.selection.is_some() {
                "Shift-arrows / drag select · y copy · a annotate · Esc close"
            } else {
                "↑↓ scroll · PgUp/PgDn page · s select source · Esc close"
            },
            look,
        ),
    );
}

#[cfg(test)]
mod tests;

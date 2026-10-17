//! One selected-agent conversation for every shell placement. Identity owns
//! reading state; Scrolls owns the viewport and acquisition stays on the worker.
use std::collections::BTreeMap;

use super::{
    app::{App, RowTarget},
    markdown::{self, Mapped},
    notes::{NotesCursor, sanitize},
    row_detail::{Reply, Target},
    scroll::Step,
};
use crate::{config::Pane, look::Look};
use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
};
use tmt_cli_style::Role;
use tmt_tui::components::strip;
use unicode_width::UnicodeWidthStr;

pub(in crate::board) mod exchange;
pub(super) mod layout;
pub(super) mod outgoing;
pub(in crate::board) use outgoing::FeedbackClock;
mod source;
mod transcript;

#[derive(Clone)]
pub(super) struct HeaderHit {
    pub area: Rect,
    pub target: RowTarget,
}

pub(super) type ConversationKey = (Option<String>, String);

#[derive(Default)]
pub(super) struct State {
    sessions: BTreeMap<ConversationKey, Session>,
    pub(in crate::board) active: Option<ConversationKey>,
    sequence: u64,
    pending_send: Option<outgoing::Ticket>,
    pub header_hits: Vec<HeaderHit>,
    reply_hits: Vec<source::ReplyHit>,
    source_drag: Option<source::ReplyHit>,
    response_area: Option<Rect>,
    pub area: Rect,
    pub pane: Option<Rect>,
}

struct Rendered {
    width: usize,
    gutter: usize,
    look: Look,
    identities: Option<std::sync::Arc<()>>,
    mapped: Mapped,
    anchor_text: String,
    response_sources: Option<Vec<Option<usize>>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Change {
    Current,
    NewOutput,
    Unavailable,
}

#[derive(Clone, Copy)]
enum AuthoredLine {
    Outgoing { answer: bool },
    ReplyHeading,
    Status { sequence: u64 },
}
impl AuthoredLine {
    fn role(self) -> Role {
        match self {
            Self::ReplyHeading => Role::Accent,
            _ => Role::Muted,
        }
    }
}

struct Session {
    reply: Option<Reply>,
    text: String,
    rendered: Option<Rendered>,
    anchor: NotesCursor,
    follow: bool,
    change: Change,
    outgoing: std::collections::VecDeque<outgoing::Entry>,
    before_send: Option<Reply>,
    response_spans: Vec<(
        Option<u64>,
        super::home_leads::MessageKey,
        std::ops::Range<usize>,
    )>,
    exact_source: Option<source::Source>,
    authored_lines: BTreeMap<usize, AuthoredLine>,
    compact_separators: std::collections::BTreeSet<usize>,
    pane_output: super::recent_responses::Observation,
    pane_output_fresh: bool,
    transcript_context: transcript::Context,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            reply: None,
            text: "(no response in loaded history)".into(),
            rendered: None,
            anchor: Default::default(),
            follow: true,
            change: Change::Current,
            outgoing: Default::default(),
            before_send: None,
            response_spans: Default::default(),
            exact_source: None,
            authored_lines: Default::default(),
            compact_separators: Default::default(),
            pane_output: Default::default(),
            pane_output_fresh: false,
            transcript_context: Default::default(),
        }
    }
}

impl Session {
    /// A replaced request must not steal the text somebody is reading. It is
    /// admitted when follow resumes; appending to the same source keeps its anchor.
    fn update(&mut self, incoming: Option<Reply>) {
        if incoming.as_ref().is_some_and(|reply| {
            reply
                .key
                .as_ref()
                .is_some_and(|key| key.kind == super::home_leads::Kind::Question)
        }) && self
            .reply
            .as_ref()
            .zip(incoming.as_ref())
            .is_none_or(|(old, new)| !same_source(old, new))
        {
            if self.outgoing.iter().any(outgoing::Entry::pending)
                || !self.follow && self.reply.is_some()
            {
                self.change = Change::NewOutput;
                return;
            }
            // A new incoming request starts a new bounded feedback group. Old
            // explicit sends do not replace the selected agent's new question.
            self.outgoing.clear();
            self.before_send = None;
            self.response_spans.clear();
            self.authored_lines.clear();
            self.compact_separators.clear();
        }
        if !self.outgoing.is_empty() {
            self.correlate(incoming.as_ref());
            return;
        }
        // Acquisition may restart after a hidden pane or fresh snapshot. A
        // loading placeholder for the same request must not erase known source.
        if self
            .reply
            .as_ref()
            .zip(incoming.as_ref())
            .is_some_and(|(old, new)| old.source && !new.acquired && same_source(old, new))
        {
            return;
        }
        let changed = match (&self.reply, &incoming) {
            (Some(old), Some(new)) => !same_source(old, new) || old.body != new.body,
            (None, None) => false,
            _ => true,
        };
        if !changed {
            if self.change == Change::Unavailable && incoming.is_some() {
                self.change = Change::Current;
            }
            self.reply = incoming;
            return;
        }
        if !self.follow && self.reply.is_some() {
            self.change = if incoming.is_some() {
                Change::NewOutput
            } else {
                Change::Unavailable
            };
            if !self
                .reply
                .as_ref()
                .zip(incoming.as_ref())
                .is_some_and(|(old, new)| same_source(old, new))
            {
                return;
            }
        }
        self.text = sanitize(
            incoming
                .as_ref()
                .map_or("(no response in loaded history)", |reply| &reply.body),
        );
        self.reply = incoming;
        self.rendered = None;
    }

    /// The one acquired-context update used by legacy and grouped placements.
    fn observe_context(
        &mut self,
        app: &App,
        target: &RowTarget,
        row: &serde_json::Value,
    ) -> String {
        self.update(
            app.detail(&Target::Row(target.clone()))
                .and_then(|detail| detail.reply),
        );
        let projected = match target {
            RowTarget::Workspace(target) => app
                .workspace_snapshot()
                .and_then(|snapshot| super::workspace::member_target_row(&snapshot.roster, target)),
            _ => None,
        };
        let label = projected.as_ref().and_then(|row| app.activity_label(row));
        let state = label.as_ref().map_or_else(
            || row["state"].as_str().unwrap_or("unknown"),
            |label| label.state,
        );
        let words = label.as_ref().map_or_else(
            || sanitize(state),
            |label| {
                let (head, task) = label.fit(usize::MAX);
                head + task.as_deref().unwrap_or("")
            },
        );
        let stale = app.loading() || app.error.is_some() || app.workspace_snapshot().is_some_and(|snapshot| {
                use super::workspace::{OpsObservation, OpsTarget};
                snapshot.roster.observation != OpsObservation::Complete || !snapshot.roster.membership_complete
                    || matches!(target, RowTarget::Workspace(OpsTarget::Member { room_id: Some(room), .. })
                        if snapshot.roster.squads.iter().find(|squad| &squad.room_id == room)
                            .is_none_or(|squad| squad.observation != OpsObservation::Complete))
            }) || app.view.as_ref().is_none_or(|view| view.document["partial"] == true);
        let name = row["name"].as_str().unwrap_or("unknown agent");
        let observation = projected
            .as_ref()
            .and_then(|row| row.activity.as_ref())
            .filter(|_| !stale && !app.activity_stale(std::time::Instant::now()))
            .map(|member| member.activity);
        self.observe_progress(
            name,
            observation,
            crate::status::now_ms(),
            outgoing::motion_allowed(app, target),
        );
        self.correlate_loaded(
            app,
            &sanitize(row["name"].as_str().unwrap_or("unknown agent")),
        );
        self.transcript_context(app, row);
        words
    }

    fn prepare(
        &mut self,
        width: usize,
        look: Look,
        inventory: &super::inventory::Inventory,
    ) -> bool {
        self.prepare_with_gutter(width, 0, look, inventory)
    }

    fn prepare_grouped(
        &mut self,
        width: usize,
        look: Look,
        inventory: &super::inventory::Inventory,
    ) -> bool {
        self.prepare_with_gutter(width, 3, look, inventory)
    }

    fn prepare_with_gutter(
        &mut self,
        width: usize,
        gutter: usize,
        look: Look,
        inventory: &super::inventory::Inventory,
    ) -> bool {
        let identities = self
            .text
            .contains("#tmt:")
            .then(|| std::sync::Arc::clone(&inventory.generation));
        let changed = self.rendered.as_ref().is_none_or(|rendered| {
            rendered.width != width
                || rendered.gutter != gutter
                || rendered.look != look
                || match (&rendered.identities, &identities) {
                    (None, None) => false,
                    (Some(old), Some(new)) => !std::sync::Arc::ptr_eq(old, new),
                    _ => true,
                }
        });
        if changed {
            let mut mapped = markdown::render_with_members(
                &self.text,
                width.saturating_sub(gutter).max(1),
                look,
                Some(inventory),
            );
            let mut lines = Vec::with_capacity(mapped.lines.len());
            let mut sources = Vec::with_capacity(mapped.sources.len());
            let mut mentions = Vec::with_capacity(mapped.mentions.len());
            for ((mut line, source), mut links) in mapped
                .lines
                .into_iter()
                .zip(mapped.sources)
                .zip(mapped.mentions)
            {
                if line.width() == 0 && self.compact_separators.contains(&source) {
                    continue;
                }
                if let Some(kind) = self.authored_lines.get(&source) {
                    let style = look.role(kind.role());
                    line.style = style;
                    for span in &mut line.spans {
                        span.style = style;
                    }
                    links.clear();
                }
                lines.push(line);
                sources.push(source);
                mentions.push(links);
            }
            mapped = Mapped {
                lines,
                sources,
                mentions,
            };
            let transcript = self.merge_pane_output(mapped, width, look);
            self.anchor.reconcile(&transcript.anchor_text);
            self.rendered = Some(Rendered {
                width,
                gutter,
                look,
                identities,
                mapped: transcript.mapped,
                anchor_text: transcript.anchor_text,
                response_sources: transcript.response_sources,
            });
        }
        changed
    }
}

/// Membership is a read/dispatch context, not the identity of a response.
fn same_source(old: &Reply, new: &Reply) -> bool {
    match (&old.key, &new.key) {
        (Some(old), Some(new)) => old.same_source(new),
        (None, None) => true,
        _ => false,
    }
}

impl App {
    pub(super) fn conversation_reply(&self) -> Option<Reply> {
        if !self.scrolls.visible(Pane::Replies) {
            return None;
        }
        let target = self.conversation_target()?;
        let id = self.target_row(&target)?["id"].as_str()?;
        let state = self.conversation.borrow();
        let identity = (self.view.as_ref()?.me_id.clone(), id.to_owned());
        state.sessions.get(&identity)?.reply.clone()
    }

    pub(super) fn conversation_input_area(&self) -> Option<Rect> {
        let pane = self
            .conversation
            .borrow()
            .pane
            .filter(|pane| pane.height > 0)?;
        if self.composer_projection().is_some() {
            return Some(pane);
        }
        let input = self.input.as_ref()?;
        let target = self.conversation_target()?;
        // Placement follows the selected membership being discussed. A note
        // can be delivered to its lead, and Status has no message draft key.
        // Neither changes the conversation that owns this input band.
        (input.target() == Some(&target)).then_some(pane)
    }

    pub(super) fn conversation_target(&self) -> Option<RowTarget> {
        let target = self.row_target(self.selected)?;
        self.target_row(&target)?["id"].as_str()?;
        Some(target)
    }

    /// All response scrolling passes through this method. Other panes keep
    /// their existing owner, and a wheel never changes the keyboard recipient.
    pub(super) fn scroll_pane(&self, pane: Pane, step: Step) {
        self.scrolls.scroll(pane, step);
        if pane != Pane::Replies {
            return;
        }
        let mut state = self.conversation.borrow_mut();
        let Some(identity) = state.active.clone() else {
            return;
        };
        state.reply_hits.clear();
        state.source_drag = None;
        let response_area = state
            .response_area
            .filter(|_| self.scrolls.visible(Pane::Replies));
        let Some(session) = state.sessions.get_mut(&identity) else {
            return;
        };
        if let Some(source) = &mut session.exact_source {
            source.scroll(self.scrolls.offset(pane), step, self.scrolls.visible(pane));
            return;
        }
        if matches!(step, Step::Lines(0) | Step::Pages(0)) {
            return;
        }
        let forward = matches!(step, Step::Lines(lines) | Step::Pages(lines) if lines > 0);
        let at_bottom =
            response_area
                .zip(session.rendered.as_ref())
                .is_some_and(|(area, rendered)| {
                    let content = rendered.mapped.lines.len();
                    self.scrolls
                        .offset(pane)
                        .saturating_add(super::scroll::Scrolls::viewport(area, content))
                        >= content
                });
        session.follow = matches!(step, Step::Bottom) || forward && at_bottom;
        if session.follow {
            session.change = Change::Current;
        } else if let Some(rendered) = &session.rendered {
            session.anchor.select_visual(
                &rendered.anchor_text,
                &rendered.mapped.sources,
                self.scrolls.offset(pane),
            );
        }
    }
}

pub(in crate::board) struct LinkPreview {
    pub(in crate::board) value: String,
    key: String,
    wording: String,
}
pub(in crate::board) fn link_preview(
    app: &App,
    row: &serde_json::Value,
    bindings: &crate::action::Bindings,
) -> Option<LinkPreview> {
    let (key, action) = bindings.get("o").map(|action| ("o", action)).or_else(|| {
        bindings
            .iter()
            .find(|(_, action)| action.verb == crate::action::Verb::Open)
            .map(|(key, action)| (key.as_str(), action))
    })?;
    if action.verb == crate::action::Verb::Open
        && action.args.is_empty()
        && crate::effects::default_link(row).is_none()
    {
        return None;
    }
    let value = if action.verb == crate::action::Verb::Open {
        crate::effects::resolve_open(action, row)
    } else {
        super::overview::actions::link_preview(
            action,
            row,
            app.view.as_ref().and_then(|view| view.opener.as_deref()),
        )
    }
    .unwrap_or_else(|error| format!("Unavailable: {error}"));
    let wording = if action.verb == crate::action::Verb::Open && action.args.is_empty() {
        "open $link"
    } else {
        &action.text
    };
    Some(LinkPreview {
        value: sanitize(&value),
        key: key.into(),
        wording: sanitize(wording),
    })
}

/// Agents and compact explicit conversations retain this metadata header.
/// Session, response and editor owners are shared with the shared exchange adapter.
pub(super) fn render(frame: &mut Frame, app: &App, area: Rect) {
    render_target(frame, app, area, app.conversation_target(), true);
}

pub(super) fn render_retained(
    frame: &mut Frame,
    app: &App,
    area: Rect,
    target: &super::workspace::OpsTarget,
) {
    render_target(
        frame,
        app,
        area,
        Some(RowTarget::Workspace(target.clone())),
        false,
    );
}

fn render_target(
    frame: &mut Frame,
    app: &App,
    area: Rect,
    target: Option<RowTarget>,
    interactive: bool,
) {
    let look = app.look();
    app.conversation.borrow_mut().header_hits.clear();
    app.conversation.borrow_mut().reply_hits.clear();
    app.conversation.borrow_mut().response_area = None;
    let row = target
        .as_ref()
        .and_then(|target| app.target_row(target))
        .filter(|row| row["id"].as_str().is_some());
    let (Some(target), Some(row)) = (target, row) else {
        app.conversation.borrow_mut().active = None;
        strip::paint_left(
            frame.buffer_mut(),
            area,
            Line::styled(
                "Select an agent to read its response.",
                look.role(Role::Dim),
            ),
        );
        return;
    };
    let identity = (
        app.view.as_ref().and_then(|view| view.me_id.clone()),
        row["id"].as_str().unwrap().to_owned(),
    );
    let input_height = interactive
        .then(|| app.conversation_input_area())
        .flatten()
        .map_or(0, |pane| {
            super::view::waiting::input_lines(app, pane).min(usize::from(area.height)) as u16
        });
    let area = if input_height > 0 {
        app.input_band.set(Some(Rect::new(
            area.x,
            area.bottom() - input_height,
            area.width,
            input_height,
        )));
        Rect {
            height: area.height - input_height,
            ..area
        }
    } else {
        area
    };
    // Chrome replaces the former first header row; keep the existing metadata
    // admission and compact total header budget over the held response area.
    let full = area.height >= 20 && area.width >= 60;
    let header_limit = if area.height < 12 { 4u16 } else { u16::MAX };
    let inside = super::pane_chrome::paint(
        frame,
        app,
        area,
        Pane::Replies,
        &format!(
            "Conversation · {}",
            row["name"].as_str().unwrap_or("unknown agent")
        ),
    );
    let header_limit = header_limit.saturating_sub(inside.y.saturating_sub(area.y));
    let area = inside;
    // Effective bindings depend on logical conversation placement. Acquire them
    // before mutably borrowing the response sessions that own that placement.
    let bindings = if interactive {
        app.bindings()
    } else {
        Default::default()
    };
    let link = link_preview(app, row, &bindings).map(|preview| {
        Line::styled(
            super::view::fit(&preview.value, usize::from(area.width)),
            look.role(Role::Muted),
        )
    });
    // One activity-first label feeds both the header and the conversation
    // status; a target without a member row keeps its task state.
    let tree = app.workspace_rows();
    let tree_row = tree
        .get(app.selected)
        .filter(|row| RowTarget::Workspace(row.target.clone()) == target);
    let mut state = app.conversation.borrow_mut();
    let switched = state.active.as_ref() != Some(&identity);
    state.active = Some(identity.clone());
    let session = state.sessions.entry(identity).or_default();
    let state_words = session.observe_context(app, &target, row);
    let name = sanitize(row["name"].as_str().unwrap_or("unknown agent"));
    let values = tree_row
        .zip(app.workspace_snapshot())
        .map(|(row, snapshot)| super::overview::monitor::values(app, snapshot, row));
    let squad = app
        .target_squad(&target)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "Common pool".into());
    let role = match &target {
        RowTarget::Lead { .. } => "lead",
        _ if tree_row.is_some_and(|row| row.lead) => "lead",
        _ => "member",
    };
    let observed = app.references.agents.get(row["id"].as_str().unwrap());
    let runtime = observed
        .and_then(|agent| agent.runtime.as_deref())
        .or_else(|| observed.filter(|agent| agent.basic).map(|_| "basic"))
        .unwrap_or("–");
    let model = observed
        .and_then(|agent| agent.model.as_deref())
        .or_else(|| row["fields"]["model"].as_str())
        .unwrap_or("–");
    let room = match &target {
        RowTarget::Workspace(super::workspace::OpsTarget::Member { room_id, .. })
            if app.shell.tab != super::shell::OpsTab::Overview =>
        {
            room_id.as_deref()
        }
        _ => None,
    };
    let timers = app.cron_glyphs.label(app.cron.counts.count(
        super::overview::cron_counts::CronCountScope::Owner {
            id: row["id"].as_str().unwrap(),
            room,
        },
    ));
    let timer_text = format!(" · {timers}");
    let runtime_width =
        usize::from(area.width).saturating_sub(if full { 0 } else { timer_text.width() });
    let runtime_text = super::view::fit(
        &format!("{} · {}", sanitize(runtime), sanitize(model)),
        runtime_width,
    );
    let usage = values
        .as_ref()
        .filter(|values| values.tokens.is_some())
        .zip(tree_row)
        .map_or_else(
            || "unknown".into(),
            |(values, row)| values.text(super::overview::columns::Column::TokensPerHour, row),
        );
    let prefix = if full {
        format!("{usage} tok/h · ")
    } else {
        String::new()
    };
    let chip_budget = usize::from(area.width)
        .saturating_sub(prefix.width() + if full { timer_text.width() } else { 0 });
    let mut chips = super::row_chips::of(&app.labels, row, crate::status::now_ms());
    chips.truncate(super::row_chips::prefix(&chips, chip_budget));
    let rule = |title: String, role| {
        let text = if title.width() + 2 <= usize::from(area.width) {
            format!(
                "{title} {}",
                "─".repeat(usize::from(area.width) - title.width() - 1)
            )
        } else {
            title
        };
        Line::styled(text, look.role(role))
    };
    let runtime_known = runtime != "–" || model != "–";
    let state_text = format!("{state_words} · {} · {role}", sanitize(&squad));
    let state_text = if !full && !runtime_known {
        super::view::fit(
            &state_text,
            usize::from(area.width).saturating_sub(timer_text.width()),
        )
    } else {
        state_text
    };
    let state_width = state_text.width();
    let mut state_spans = vec![Span::styled(state_text, look.role(Role::Muted))];
    if !full && !runtime_known {
        let used = state_spans.iter().map(Span::width).sum::<usize>();
        state_spans.push(Span::raw(" ".repeat(
            usize::from(area.width).saturating_sub(used + timer_text.width()),
        )));
        state_spans.push(Span::styled(timer_text.clone(), look.role(Role::Muted)));
    }
    let mut header = Vec::new();
    let state_row = header.len();
    header.push(Line::from(state_spans));
    let runtime_row = runtime_known.then_some(header.len());
    if runtime_known {
        header.push(Line::from(vec![
            Span::styled(runtime_text.clone(), look.role(Role::Accent)),
            Span::styled(
                if full {
                    String::new()
                } else {
                    timer_text.clone()
                },
                look.role(Role::Muted),
            ),
        ]));
    }
    let savings = app.digest_savings(row["id"].as_str());
    let mut savings_inline = false;
    // (header row, x, width) of each painted caption line, for clicks.
    let mut savings_rows: Vec<(usize, usize, usize)> = Vec::new();
    let chip_row = (full || !chips.is_empty()).then_some(header.len());
    if full || !chips.is_empty() {
        let mut spans = vec![Span::styled(prefix.clone(), look.role(Role::Muted))];
        spans.extend(
            chips
                .iter()
                .map(|chip| Span::styled(chip.text.clone(), look.role(chip.role))),
        );
        if full {
            if chips.is_empty() && "Digest unknown".width() <= chip_budget {
                spans.push(Span::styled("Digest unknown", look.role(Role::Dim)));
            }
            let used: usize = spans.iter().map(Span::width).sum();
            let room = usize::from(area.width).saturating_sub(used + timer_text.width() + 3);
            let caption = app.savings_caption(&savings, room);
            if caption.width() <= room {
                savings_rows.push((header.len(), used + " · ".width(), caption.width()));
                spans.push(Span::styled(format!(" · {caption}"), look.role(Role::Dim)));
                savings_inline = true;
            }
            let used: usize = spans.iter().map(Span::width).sum();
            spans.push(Span::raw(" ".repeat(
                usize::from(area.width).saturating_sub(used + timer_text.width()),
            )));
            spans.push(Span::styled(timer_text.clone(), look.role(Role::Muted)));
        }
        header.push(Line::from(spans));
    }
    let controls_row = (interactive && full && tree_row.is_some()).then_some(header.len());
    if controls_row.is_some() {
        header.push(Line::default());
    }
    let host_action = bindings
        .get("j")
        .filter(|action| {
            matches!(
                action.verb,
                crate::action::Verb::Jump | crate::action::Verb::Run
            )
        })
        .map(|action| ("j", action))
        .or_else(|| {
            bindings
                .iter()
                .find(|(_, action)| action.verb == crate::action::Verb::Jump)
                .map(|(key, action)| (key.as_str(), action))
        });
    let host = match host_action {
        Some((key, action)) if action.verb == crate::action::Verb::Jump => {
            let host_name = row["pane"]["location"]["host"]
                .as_str()
                .unwrap_or("unknown");
            let action = if !app.invoker_tmux {
                format!("{key} conversation")
            } else {
                super::overview::actions::host(row, true).map_or_else(
                    |reason| format!("{key} unavailable: {reason}"),
                    |_| format!("{key} host"),
                )
            };
            format!("Host: {host_name} · {action}")
        }
        action => {
            let host_name = row["pane"]["location"]["host"]
                .as_str()
                .unwrap_or("unknown");
            action.map_or_else(
                || format!("Host: {host_name}"),
                |(key, action)| format!("Host: {host_name} · {key} {}", action.description()),
            )
        }
    };
    header.push(Line::styled(
        super::view::fit(&sanitize(&host), usize::from(area.width)),
        look.role(Role::Muted),
    ));
    if let Some(link) = link {
        header.push(link);
    }
    if !savings_inline {
        let caption = app.savings_caption(&savings, usize::from(area.width));
        for line in super::notes::wrap(&caption, usize::from(area.width)) {
            savings_rows.push((header.len(), 0, line.width()));
            header.push(Line::styled(line, look.role(Role::Dim)));
        }
    }
    if let Some(start) = app.digest_savings_start(&savings, usize::from(area.width)) {
        header.extend(
            super::notes::wrap(&start, usize::from(area.width))
                .into_iter()
                .map(|line| Line::styled(line, look.role(Role::Dim))),
        );
    }
    let flow = if session.change == Change::NewOutput {
        "new output · End resumes follow"
    } else if session.change == Change::Unavailable {
        "source unavailable · retained for reading"
    } else if session.follow {
        "following"
    } else {
        "reading · End resumes follow"
    };
    header.push(rule(
        if session.exact_source.is_some() {
            if interactive {
                format!("Exact response source · {name} · Esc back")
            } else {
                format!("Exact response source · {name}")
            }
        } else {
            format!("Response · {flow}")
        },
        if session.change != Change::Current {
            Role::Waiting
        } else {
            Role::Dim
        },
    ));
    // Keep a response viewport even when an input band or low terminal leaves
    // little vertical room. No decorative blank rows consume that budget.
    let header_height = area
        .height
        .saturating_sub(3)
        .max(1)
        .min(area.height)
        .min(header.len() as u16)
        .min(header_limit);
    let retained_prefix = usize::from(header_height)
        .saturating_sub(usize::from(usize::from(header_height) < header.len()));
    let controls_row = controls_row.filter(|index| *index < retained_prefix);
    let timer_row = if full {
        chip_row
    } else {
        Some(runtime_row.unwrap_or(state_row))
    };
    let retained_timer = timer_row.is_some_and(|row| row < retained_prefix);
    let retained_chip = !chips.is_empty() && chip_row.is_some_and(|row| row < retained_prefix);
    if header_height >= 2 && usize::from(header_height) < header.len() {
        let response = header.pop().unwrap();
        header.truncate(usize::from(header_height) - 1);
        header.push(response);
    }
    let mut painted_controls = Vec::new();
    for (index, line) in header
        .into_iter()
        .take(usize::from(header_height))
        .enumerate()
    {
        if interactive && index < retained_prefix {
            for &(_, x, width) in savings_rows.iter().filter(|(row, ..)| *row == index) {
                if x < usize::from(area.width) {
                    app.savings_hit(
                        area.x + x as u16,
                        area.y + index as u16,
                        width,
                        area.width - x as u16,
                    );
                }
            }
        }
        if retained_chip && Some(index) == chip_row {
            if super::row_chips::clock(&app.labels, row, crate::status::now_ms()).is_some() {
                app.frame_clocks
                    .borrow_mut()
                    .push(super::view::ClockSurface::Labels {
                        target: target.clone(),
                        width: chip_budget,
                    });
            }
            let mut x = 0;
            for span in &line.spans {
                let width = span.width();
                if interactive
                    && super::row_chips::offers_choice(&span.content)
                    && x + width <= usize::from(area.width)
                {
                    app.chip_hits.borrow_mut().push((
                        Rect::new(area.x + x as u16, area.y + index as u16, width as u16, 1),
                        app.selected,
                    ));
                }
                x += width;
            }
        }
        if Some(index) == controls_row
            && let RowTarget::Workspace(_) = &target
        {
            let controls = super::buttons::paint(
                app,
                frame.buffer_mut(),
                Rect::new(area.x, area.y + index as u16, area.width, 1),
                &[
                    super::buttons::Button::secondary("Details"),
                    super::buttons::Button::secondary("Links"),
                ],
                tmt_cli_style::grid::Align::Left,
                None,
            );
            for area in controls.into_iter().flatten() {
                painted_controls.push(HeaderHit {
                    area,
                    target: target.clone(),
                });
            }
        }
        if interactive
            && retained_timer
            && Some(index) == timer_row
            && (if full {
                prefix.width()
            } else {
                if runtime_known {
                    runtime_text.width()
                } else {
                    state_width
                }
            }) + timer_text.width()
                <= usize::from(area.width)
            && let RowTarget::Workspace(target) = &target
            && let super::workspace::OpsTarget::Member { identity_id, .. } = target
        {
            app.cron_scope_hits.borrow_mut().push(super::app::TimerHit {
                area: Rect::new(
                    area.right() - timers.width() as u16,
                    area.y + index as u16,
                    timers.width() as u16,
                    1,
                ),
                target: target.clone(),
                room: room.map(str::to_owned),
                owner: Some(identity_id.clone()),
            });
        }
        strip::paint_left(
            frame.buffer_mut(),
            Rect::new(area.x, area.y + index as u16, area.width, 1),
            line,
        );
    }
    let body = Rect::new(
        area.x,
        area.y + header_height,
        area.width,
        area.height - header_height,
    );
    if body.is_empty() {
        return;
    }
    if let Some(source) = &mut session.exact_source {
        source::paint_with_interactivity(frame, app, source, &target, body, look, interactive);
        state.header_hits = painted_controls;
        return;
    }
    let position = (input_height > 0 && body.height > 1)
        .then(|| Rect::new(body.x, body.bottom() - 1, body.width, 1));
    let body = Rect {
        height: body.height - u16::from(position.is_some()),
        ..body
    };
    let rebuilt = session.prepare(usize::from(body.width), look, &app.references);
    let mapped = &session.rendered.as_ref().unwrap().mapped;
    let previous = app.scrolls.offset(Pane::Replies);
    if session.follow {
        app.scrolls.restore([(Pane::Replies, mapped.lines.len())]);
    } else if switched || rebuilt {
        app.scrolls.restore([(
            Pane::Replies,
            session.anchor.visual(&mapped.sources).unwrap_or_default(),
        )]);
    }
    let (offset, shown) = app.scrolls.show_with(
        frame,
        Pane::Replies,
        body,
        &mapped.lines,
        look,
        |index, line| session.feedback_line(mapped.sources[index], line, crate::status::now_ms()),
    );
    if interactive {
        super::member_links::paint_hits(
            frame,
            app,
            body,
            offset,
            shown,
            &mapped.mentions,
            super::member_links::Paint {
                reader: true,
                role: Role::Link,
            },
        );
    }
    if let Some(position) = position {
        let visible = shown.min(mapped.lines.len().saturating_sub(offset));
        strip::paint_left(
            frame.buffer_mut(),
            position,
            Line::styled(
                format!(
                    "{}–{} / {}  ·  select text to annotate",
                    if visible == 0 { 0 } else { offset + 1 },
                    offset + visible,
                    mapped.lines.len()
                ),
                look.role(Role::Dim),
            ),
        );
    }
    let mut reply_hits = Vec::new();
    for index in offset..offset.saturating_add(shown).min(mapped.lines.len()) {
        session.admit_feedback_clock(app, mapped.sources[index], &mapped.lines[index]);
        if interactive
            && mapped.lines[index].width() > 0
            && let Some((key, sequence)) = session.response_key_at(mapped.sources[index])
        {
            reply_hits.push(source::ReplyHit {
                area: Rect::new(body.x, body.y + (index - offset) as u16, body.width, 1),
                target: target.clone(),
                key: key.clone(),
                sequence,
                offset,
            });
        }
    }
    if switched || rebuilt || previous != offset {
        session.anchor.select_visual(
            &session.rendered.as_ref().unwrap().anchor_text,
            &mapped.sources,
            offset,
        );
    }
    state.reply_hits = reply_hits;
    state.response_area = Some(body);
    state.header_hits = painted_controls;
}

#[cfg(test)]
pub(in crate::board) mod tests;

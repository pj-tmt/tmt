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

pub(super) mod layout;

type ConversationKey = (Option<String>, String);

#[derive(Default)]
pub(super) struct State {
    sessions: BTreeMap<ConversationKey, Session>,
    active: Option<ConversationKey>,
    pub area: Rect,
    pub pane: Option<Rect>,
}

struct Rendered {
    width: usize,
    look: Look,
    mapped: Mapped,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Change {
    Current,
    NewOutput,
    Unavailable,
}

struct Session {
    reply: Option<Reply>,
    text: String,
    rendered: Option<Rendered>,
    anchor: NotesCursor,
    follow: bool,
    change: Change,
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
        }
    }
}

impl Session {
    /// A replaced request must not steal the text somebody is reading. It is
    /// admitted when follow resumes; appending to the same source keeps its anchor.
    fn update(&mut self, incoming: Option<Reply>) {
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
        self.anchor.reconcile(&self.text);
        self.reply = incoming;
        self.rendered = None;
    }

    fn prepare(&mut self, width: usize, look: Look) -> bool {
        let changed = self
            .rendered
            .as_ref()
            .is_none_or(|rendered| rendered.width != width || rendered.look != look);
        if changed {
            self.rendered = Some(Rendered {
                width,
                look,
                mapped: markdown::render_mapped(&self.text, width, look),
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
            .filter(|pane| pane.height >= 9)?;
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
        let Some(session) = state.sessions.get_mut(&identity) else {
            return;
        };
        session.follow = matches!(step, Step::Bottom);
        if session.follow {
            session.change = Change::Current;
        } else if let Some(rendered) = &session.rendered {
            session.anchor.select_visual(
                &session.text,
                &rendered.mapped.sources,
                self.scrolls.offset(pane),
            );
        }
    }
}

/// Header and response are shared verbatim between Overview and Agents.
/// The composer remains owned by App, not by either placement of this painter.
pub(super) fn render(frame: &mut Frame, app: &App, area: Rect) {
    let look = app.look();
    let Some(target) = app.conversation_target() else {
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
    let row = app
        .target_row(&target)
        .expect("resolved conversation target");
    let identity = (
        app.view.as_ref().and_then(|view| view.me_id.clone()),
        row["id"].as_str().unwrap().to_owned(),
    );
    let reply = app
        .detail(&Target::Row(target.clone()))
        .and_then(|detail| detail.reply);
    let input_height = app.conversation_input_area().map_or(0, |pane| {
        super::view::waiting::input_lines(app, pane).min(usize::from(area.height.saturating_sub(4)))
            as u16
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
    // Effective bindings depend on logical conversation placement. Acquire them
    // before mutably borrowing the response sessions that own that placement.
    let bindings = app.bindings();
    let activation = bindings.get("o").map(|action| ("o", action)).or_else(|| {
        bindings
            .iter()
            .find(|(_, action)| action.verb == crate::action::Verb::Open)
            .map(|(key, action)| (key.as_str(), action))
    });
    let link = activation
        .filter(|(_, action)| {
            action.verb != crate::action::Verb::Open
                || !action.args.is_empty()
                || crate::effects::default_link(row).is_some()
        })
        .map(|(key, action)| {
            let preview = if action.verb == crate::action::Verb::Open {
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
            let hint = format!(" (press {key} - {})", sanitize(wording));
            let hint = super::view::fit(&hint, hint.width().min(usize::from(area.width)));
            let text = super::view::fit(
                &sanitize(&preview),
                usize::from(area.width).saturating_sub(hint.width()),
            );
            Line::from(vec![
                Span::styled(text, look.role(Role::Link)),
                Span::styled(hint, look.role(Role::Dim)),
            ])
        });
    let mut state = app.conversation.borrow_mut();
    let switched = state.active.as_ref() != Some(&identity);
    state.active = Some(identity.clone());
    let session = state.sessions.entry(identity).or_default();
    session.update(reply);
    let name = sanitize(row["name"].as_str().unwrap_or("unknown agent"));
    let observed = app.references.agents.get(row["id"].as_str().unwrap());
    let runtime = observed
        .and_then(|agent| agent.runtime.as_deref())
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
    let runtime_width = usize::from(area.width).saturating_sub(timer_text.width());
    let runtime_text = super::view::fit(
        &format!("{} · {}", sanitize(runtime), sanitize(model)),
        runtime_width,
    );
    let mut chips = super::row_chips::of(&app.labels, row, crate::status::now_ms());
    // A source name and its first label form one unit. Never paint a bare source
    // or clip the choice mark away from the label whose action it represents.
    chips.truncate(super::row_chips::prefix(&chips, usize::from(area.width)));
    let mut header = vec![
        Line::styled(format!("Conversation · {name}"), look.role(Role::Accent)),
        Line::styled(
            format!(
                "{} · {}",
                sanitize(row["state"].as_str().unwrap_or("unknown")),
                sanitize(row["presence"].as_str().unwrap_or("unknown"))
            ),
            look.role(Role::Muted),
        ),
        Line::from(vec![
            Span::styled(runtime_text.clone(), look.role(Role::Accent)),
            Span::styled(timer_text.clone(), look.role(Role::Muted)),
        ]),
    ];
    if !chips.is_empty() {
        header.push(Line::from(
            chips
                .iter()
                .map(|chip| Span::styled(chip.text.clone(), look.role(chip.role)))
                .collect::<Vec<_>>(),
        ));
    }
    let host_action = bindings.get("j").map(|action| ("j", action)).or_else(|| {
        bindings
            .iter()
            .find(|(_, action)| action.verb == crate::action::Verb::Jump)
            .map(|(key, action)| (key.as_str(), action))
    });
    let host = super::overview::actions::host_preview(row, app.invoker_tmux, host_action);
    header.push(Line::styled(
        super::view::fit(&sanitize(&host), usize::from(area.width)),
        look.role(Role::Muted),
    ));
    if let Some(link) = link {
        header.push(link);
    }
    header.push(Line::styled(
        if session.change == Change::NewOutput {
            "Response · new output · End resumes follow"
        } else if session.change == Change::Unavailable {
            "Response · source unavailable · retained for reading"
        } else if session.follow {
            "Response · following"
        } else {
            "Response · reading · End resumes follow"
        },
        look.role(if session.change != Change::Current {
            Role::Waiting
        } else {
            Role::Dim
        }),
    ));
    // Keep a response viewport even when an input band or low terminal leaves
    // little vertical room. No decorative blank rows consume that budget.
    let header_height = area
        .height
        .saturating_sub(3)
        .max(1)
        .min(area.height)
        .min(header.len() as u16);
    let retained_chip = !chips.is_empty()
        && 3 < usize::from(header_height)
            .saturating_sub(usize::from(usize::from(header_height) < header.len()));
    if header_height >= 2 && usize::from(header_height) < header.len() {
        let response = header.pop().unwrap();
        header.truncate(usize::from(header_height) - 1);
        header.push(response);
    }
    for (index, line) in header
        .into_iter()
        .take(usize::from(header_height))
        .enumerate()
    {
        if retained_chip && index == 3 {
            if super::row_chips::clock(&app.labels, row, crate::status::now_ms()).is_some() {
                app.frame_clocks
                    .borrow_mut()
                    .push(super::view::ClockSurface::Labels {
                        target: target.clone(),
                        width: usize::from(area.width),
                    });
            }
            let mut x = 0;
            for span in &line.spans {
                let width = span.width();
                if super::row_chips::offers_choice(&span.content)
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
        if index == 2
            && runtime_text.width() + timer_text.width() <= usize::from(area.width)
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
    let rebuilt = session.prepare(usize::from(body.width), look);
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
    let (offset, _) = app
        .scrolls
        .show(frame, Pane::Replies, body, &mapped.lines, look);
    if switched || rebuilt || previous != offset {
        session
            .anchor
            .select_visual(&session.text, &mapped.sources, offset);
    }
}

#[cfg(test)]
pub(in crate::board) mod tests;

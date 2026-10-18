//! Final ExchangeView adapter over the existing UUID-owned conversation session.
use super::*;
use crate::board::{empty_state, exchange_view as exchange};

/// The host chooses placement and admission; sessions and scrolling stay here.
pub(in crate::board) enum Surface {
    Overview,
    Agents {
        target: RowTarget,
        interactive: bool,
    },
}
impl Surface {
    fn target(&self, app: &App) -> Option<RowTarget> {
        match self {
            Self::Overview => app.conversation_target(),
            Self::Agents { target, .. } => Some(target.clone()),
        }
    }
    fn interactive(&self) -> bool {
        !matches!(
            self,
            Self::Agents {
                interactive: false,
                ..
            }
        )
    }
    fn is_agents(&self) -> bool {
        matches!(self, Self::Agents { .. })
    }
}

impl Session {
    /// Outgoing turns own their headers. Only an actual earlier exchange needs
    /// a separate incoming heading above them, using that exchange's age.
    fn group_heading_reply(&self) -> Option<&Reply> {
        if !self.finals.is_empty() {
            None
        } else if self.outgoing.is_empty() {
            self.reply.as_ref()
        } else {
            self.before_send
                .as_ref()
                .filter(|reply| !reply.body.is_empty())
        }
    }
}

/// Measure the held group with the same wrapped body the painter uses. Empty
/// observations never reserve a minimum transcript or impose a growth cap.
pub(in crate::board) fn desired_height(app: &App, width: u16) -> u16 {
    desired_height_for(app, width, &Surface::Overview)
}
pub(in crate::board) fn desired_height_for(app: &App, width: u16, surface: &Surface) -> u16 {
    let Some(target) = surface.target(app) else {
        return 1;
    };
    let Some(row) = app.target_row(&target) else {
        return 1;
    };
    let Some(id) = row["id"].as_str() else {
        return 1;
    };
    let identity = (
        app.view.as_ref().and_then(|view| view.me_id.clone()),
        id.to_owned(),
    );
    let has_link =
        !surface.is_agents() && super::link_preview(app, row, &app.workspace_bindings()).is_some();
    let mut state = app.conversation.borrow_mut();
    let session = state.sessions.entry(identity).or_default();
    session.observe_context(app, &target, row);
    if !surface.is_agents() && session.reply.is_none() && session.outgoing.is_empty() {
        return 1;
    }
    session.prepare_grouped(usize::from(width), app.look(), &app.references);
    let waiting = (session.outgoing.is_empty() || session.tail.is_some())
        && session.reply.as_ref().is_some_and(|reply| {
            reply
                .key
                .as_ref()
                .is_some_and(|key| key.kind == super::super::home_leads::Kind::Question)
        });
    (usize::from(session.group_heading_reply().is_some())
        + session.rendered.as_ref().unwrap().mapped.lines.len()
        + usize::from(waiting)
        + usize::from(has_link))
    .min(usize::from(u16::MAX)) as u16
}

pub(in crate::board) fn render(frame: &mut Frame, app: &App, area: Rect) {
    render_for(frame, app, area, &Surface::Overview);
}
pub(in crate::board) fn render_for(frame: &mut Frame, app: &App, area: Rect, surface: &Surface) {
    let look = app.look();
    {
        let mut state = app.conversation.borrow_mut();
        state.header_hits.clear();
        state.fold_hits.clear();
        state.response_area = None;
    }
    let Some(target) = surface.target(app) else {
        app.conversation.borrow_mut().active = None;
        empty_state::paint(
            frame.buffer_mut(),
            area,
            "Select a squad or agent.",
            None,
            look,
        );
        return;
    };
    let Some(row) = app
        .target_row(&target)
        .filter(|row| row["id"].as_str().is_some())
    else {
        app.conversation.borrow_mut().active = None;
        empty_state::paint(
            frame.buffer_mut(),
            area,
            "Selected agent unavailable.",
            None,
            look,
        );
        return;
    };
    let id = row["id"].as_str().expect("member identity");
    let identity = (
        app.view.as_ref().and_then(|view| view.me_id.clone()),
        id.to_owned(),
    );
    let me = app
        .view
        .as_ref()
        .and_then(|view| view.me.as_deref())
        .unwrap_or("you");
    let name = row["name"].as_str().unwrap_or("unknown agent");
    let bindings = app.workspace_bindings();
    let link = (!surface.is_agents())
        .then(|| super::link_preview(app, row, &bindings))
        .flatten();
    let reader_keys = app.workspace_context_bindings(
        app.workspace.selected.as_ref(),
        if surface.is_agents() {
            crate::config::OpsKeyScope::Reader
        } else {
            crate::config::OpsKeyScope::OverviewReader
        },
        row["pane"]["location"]["host"].as_str(),
    );
    let earlier_key = reader_keys
        .iter()
        .find(|(_, action)| {
            action.verb == crate::action::Verb::Ui(crate::action::OpsUiAction::ScrollPageUp)
        })
        .map(|(key, _)| crate::board::view::footer::key_label(key).to_owned())
        .filter(|_| app.input.is_none() && !app.modal_open() && !app.loading());
    let read_only = !surface.interactive()
        || app
            .composer_projection()
            .is_none_or(|projection| projection.key.is_none());
    let mut state = app.conversation.borrow_mut();
    let switched = state.active.as_ref() != Some(&identity);
    state.active = Some(identity.clone());
    let session = state.sessions.entry(identity.clone()).or_default();
    session.observe_context(app, &target, row);
    if !surface.is_agents() && session.reply.is_none() && session.outgoing.is_empty() {
        empty_state::paint(
            frame.buffer_mut(),
            area,
            "No question observed.",
            None,
            look,
        );
        return;
    }
    let question = session.reply.as_ref().is_some_and(|reply| {
        reply
            .key
            .as_ref()
            .is_some_and(|key| key.kind == super::super::home_leads::Kind::Question)
    });
    // Derived from the settled time at paint: a held reply keeps its true age.
    let age = session.group_heading_reply().map_or_else(
        || "—".to_owned(),
        |reply| reply.age_at(crate::status::now_ms()),
    );
    let stamp = age_stamp(&age);
    let stamp = if surface.is_agents() && session.change != Change::Current {
        format!(
            "{stamp} · {}",
            if session.change == Change::NewOutput {
                "new output"
            } else {
                "unavailable"
            }
        )
    } else if session.change != Change::Current {
        format!("{stamp} · End follows")
    } else {
        stamp
    };
    let read_only = read_only || session.change != Change::Current;
    let header = Rect {
        height: area.height.min(u16::from(
            session.group_heading_reply().is_some(),
        )),
        ..area
    };
    strip::paint_left(
        frame.buffer_mut(),
        header,
        exchange::header(
            exchange::Sender::Agent(name),
            me,
            &stamp,
            read_only,
            usize::from(area.width),
            look,
        ),
    );
    let rebuilt = session.prepare_grouped(usize::from(area.width), look, &app.references);
    let footer_rows = u16::from(
        question && (session.outgoing.is_empty() || session.tail.is_some()) && area.height >= 3,
    ) + u16::from(link.is_some() && area.height >= 4);
    // Long observations retain their viewport and paging indicator; optional
    // waiting/link captions yield before the response or the filled input.
    let footer_rows = if session.rendered.as_ref().unwrap().mapped.lines.len()
        + usize::from(header.height + footer_rows)
        > usize::from(area.height)
    {
        0
    } else {
        footer_rows
    };
    let body = Rect::new(
        area.x,
        area.y + header.height,
        area.width,
        area.height.saturating_sub(header.height + footer_rows),
    );
    if body.is_empty() {
        return;
    }
    let mapped = &session.rendered.as_ref().unwrap().mapped;
    if session.follow {
        app.scrolls.restore([(Pane::Replies, mapped.lines.len())]);
    } else if switched || rebuilt {
        app.scrolls.restore([(
            Pane::Replies,
            session
                .anchor
                .visual(&session.rendered.as_ref().unwrap().anchor_sources)
                .unwrap_or_default(),
        )]);
    }
    let (offset, shown) =
        app.scrolls.show_exchange(
            frame,
            body,
            &mapped.lines,
            look,
            earlier_key,
            |index, line| {
                let source = session
                    .rendered
                    .as_ref()
                    .unwrap()
                    .response_sources
                    .as_ref()
                    .map_or(Some(mapped.sources[index]), |sources| {
                        sources.get(mapped.sources[index]).copied().flatten()
                    });
                let Some(source) = source else {
                    return if session.rendered.as_ref().unwrap().final_spans.iter().any(
                        |(range, _)| {
                            session
                                .rendered
                                .as_ref()
                                .unwrap()
                                .anchor_sources
                                .get(index)
                                .is_some_and(|original| range.contains(original))
                        },
                    ) {
                        exchange::body(line.clone(), look)
                    } else {
                        line.clone()
                    };
                };
                match session.authored_lines.get(&source) {
                    Some(AuthoredLine::Outgoing { answer: true }) => {
                        exchange::replier(exchange::Sender::You(me), look)
                    }
                    Some(AuthoredLine::Outgoing { answer: false }) => exchange::header(
                        exchange::Sender::You(me),
                        name,
                        "now",
                        read_only,
                        usize::from(body.width),
                        look,
                    ),
                    Some(AuthoredLine::ReplyHeading) => exchange::replier(
                        exchange::Sender::Agent(line.to_string().trim().trim_end_matches(" · now")),
                        look,
                    ),
                    Some(AuthoredLine::Status { .. }) => exchange::feedback(
                        session.feedback_line(mapped.sources[index], line, crate::status::now_ms()),
                        look,
                    ),
                    None => exchange::body(line.clone(), look),
                }
            },
        );
    let text_area = Rect::new(
        body.x + body.width.min(3),
        body.y,
        body.width.saturating_sub(3),
        body.height,
    );
    if surface.interactive() {
        super::super::member_links::paint_hits(
            frame,
            app,
            text_area,
            offset,
            shown,
            &mapped.mentions,
            super::super::member_links::Paint {
                reader: true,
                role: Role::Text,
            },
        );
    }
    for index in offset..offset.saturating_add(shown).min(mapped.lines.len()) {
        session.admit_feedback_clock(app, mapped.sources[index], &mapped.lines[index]);
    }
    let fold_hits = if surface.interactive() {
        session
            .rendered
            .as_ref()
            .unwrap()
            .fold_controls
            .iter()
            .filter(|(line, _)| **line >= offset && **line < offset + shown)
            .map(|(line, key)| body::Hit {
                area: Rect::new(body.x, body.y + (*line - offset) as u16, body.width, 1),
                conversation: identity.clone(),
                key: key.clone(),
            })
            .collect()
    } else {
        Vec::new()
    };
    let rendered = session.rendered.as_ref().unwrap();
    session
        .anchor
        .select_visual(&rendered.anchor_text, &rendered.anchor_sources, offset);
    let mut y = body.bottom();
    if question && (session.outgoing.is_empty() || session.tail.is_some()) && footer_rows > 0 {
        strip::paint_left(
            frame.buffer_mut(),
            Rect::new(area.x, y, area.width, 1),
            exchange::waiting(look),
        );
        y += 1;
    }
    if let Some(link) = link.filter(|_| footer_rows > 0 && y < area.bottom()) {
        strip::paint_left(
            frame.buffer_mut(),
            Rect::new(area.x, y, area.width, 1),
            exchange::link(
                &link.value,
                if app.input.is_none() { &link.key } else { "" },
                &link.wording,
                usize::from(area.width),
                look,
            ),
        );
    }
    state.fold_hits = fold_hits;
    state.response_area = Some(body);
}

/// Shared exchange/pane-output age wording; unknown and current stay unqualified.
pub(super) fn age_stamp(age: &str) -> String {
    match age {
        "–" => "—".into(),
        "now" | "—" => age.into(),
        _ => format!("{age} ago"),
    }
}

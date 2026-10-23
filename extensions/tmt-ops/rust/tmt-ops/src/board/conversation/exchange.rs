//! Final ExchangeView adapter over the existing UUID-owned conversation session.
use super::*;
use crate::board::{empty_state, exchange_view as exchange};

/// The host chooses placement and admission; sessions and scrolling stay here.
pub(in crate::board) struct Surface {
    pub target: RowTarget,
    pub interactive: bool,
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
pub(in crate::board) fn desired_height(app: &App, width: u16, surface: &Surface) -> u16 {
    let target = surface.target.clone();
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
    let refresh = app.conversation_refresh(&target);
    if refresh == Refresh::Loading {
        return u16::MAX;
    }
    let mut state = app.conversation.borrow_mut();
    let session = state.sessions.entry(identity).or_default();
    if refresh == Refresh::Current {
        session.observe_context(app, &target, row);
    }
    if refresh == Refresh::Current || session.needs_outgoing_prepare() {
        session.prepare_grouped(usize::from(width), app.look(), &app.references);
    }
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
        + usize::from(session.send_now_request().is_some()))
    .min(usize::from(u16::MAX)) as u16
}

pub(in crate::board) fn render(frame: &mut Frame, app: &App, area: Rect, surface: &Surface) {
    let look = app.look();
    {
        let mut state = app.conversation.borrow_mut();
        state.header_hits.clear();
        state.send_now_hit = None;
        state.response_area = None;
    }
    let target = surface.target.clone();
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
    let reader_keys = app.workspace_context_bindings(
        app.workspace.selected.as_ref(),
        crate::config::OpsKeyScope::Reader,
        row["pane"]["location"]["host"].as_str(),
    );
    let earlier_key = reader_keys
        .iter()
        .find(|(_, action)| {
            action.verb == crate::action::Verb::Ui(crate::action::OpsUiAction::ScrollPageUp)
        })
        .map(|(key, _)| crate::board::view::footer::key_label(key).to_owned())
        .filter(|_| app.input.is_none() && !app.modal_open() && !app.loading());
    let read_only = !surface.interactive
        || app
            .composer_projection()
            .is_none_or(|projection| projection.key.is_none());
    let refresh = app.conversation_refresh(&target);
    if refresh == Refresh::Loading {
        let mut state = app.conversation.borrow_mut();
        state.active = Some(identity.clone());
        state.sessions.entry(identity).or_default();
        drop(state);
        loading(frame, app, area);
        return;
    }
    let mut state = app.conversation.borrow_mut();
    let switched = state.active.as_ref() != Some(&identity);
    state.active = Some(identity.clone());
    let send_now_pending = state.send_now_pending.clone();
    let session = state.sessions.entry(identity.clone()).or_default();
    if refresh == Refresh::Current {
        session.observe_context(app, &target, row);
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
    let stamp = if session.change != Change::Current {
        format!(
            "{stamp} · {}",
            if session.change == Change::NewOutput {
                "new output"
            } else {
                "unavailable"
            }
        )
    } else {
        stamp
    };
    let read_only = read_only || session.change != Change::Current;
    let header = Rect {
        height: area
            .height
            .min(u16::from(session.group_heading_reply().is_some())),
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
    let rebuilt = (refresh == Refresh::Current || session.needs_outgoing_prepare())
        && session.prepare_grouped(usize::from(area.width), look, &app.references);
    // The Send now chip owns the last row while Core says the newest own message can go now.
    let send_now = session.send_now_request().map(str::to_owned);
    let chip_rows = u16::from(send_now.is_some() && area.height >= 3);
    let footer_rows = u16::from(
        question && (session.outgoing.is_empty() || session.tail.is_some()) && area.height >= 3,
    );
    // Long observations retain their viewport and paging indicator; optional
    // the waiting caption yields before the response or the filled input.
    let footer_rows = if session.rendered.as_ref().unwrap().mapped.lines.len()
        + usize::from(header.height + footer_rows + chip_rows)
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
        area.height
            .saturating_sub(header.height + footer_rows + chip_rows),
    );
    if body.is_empty() {
        return;
    }
    let mapped = &session.rendered.as_ref().unwrap().mapped;
    let previous = app.scrolls.offset(Pane::Replies);
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
                    Some(AuthoredLine::Outgoing { answer: true, .. }) => {
                        exchange::replier(exchange::Sender::You(me), look)
                    }
                    Some(AuthoredLine::Outgoing {
                        answer: false,
                        sequence,
                    }) => exchange::header(
                        exchange::Sender::You(me),
                        name,
                        &session.outgoing_stamp(app, *sequence),
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
    if surface.interactive {
        super::super::member_links::paint_hits(
            frame,
            app,
            body,
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
    // The anchor follows the viewport only when something moved it, as in the
    // legacy reader: a steady frame never touches the cursor.
    if switched || rebuilt || previous != offset {
        let rendered = session.rendered.as_ref().unwrap();
        session
            .anchor
            .select_visual(&rendered.anchor_text, &rendered.anchor_sources, offset);
    }
    if question && (session.outgoing.is_empty() || session.tail.is_some()) && footer_rows > 0 {
        strip::paint_left(
            frame.buffer_mut(),
            Rect::new(area.x, body.bottom(), area.width, 1),
            exchange::waiting(look),
        );
    }
    let mut send_now_hit = None;
    if chip_rows > 0
        && let Some(request) = &send_now
    {
        let footer = Rect::new(area.x, area.bottom() - 1, area.width, 1);
        let (taken, hit) = super::send_now_chip(
            frame,
            app,
            footer,
            request,
            send_now_pending.as_deref(),
            name,
            SEND_NOW_KEY_HINT.len() as u16 + 2,
        );
        strip::paint_left(
            frame.buffer_mut(),
            Rect {
                width: footer.width.saturating_sub(taken),
                ..footer
            },
            ratatui::text::Line::styled(SEND_NOW_KEY_HINT, look.role(Role::Dim)),
        );
        send_now_hit = hit;
    }
    state.send_now_hit = send_now_hit;
    state.response_area = Some(body);
}

/// The chip's key, listed beside it and in the key help.
const SEND_NOW_KEY_HINT: &str = "Ctrl-S";

/// Shared exchange/pane-output age wording; unknown and current stay unqualified.
pub(super) fn age_stamp(age: &str) -> String {
    match age {
        "–" => "—".into(),
        "now" | "—" => age.into(),
        _ => format!("{age} ago"),
    }
}

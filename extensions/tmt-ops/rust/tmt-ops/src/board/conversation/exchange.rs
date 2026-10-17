//! Final ExchangeView adapter over the existing UUID-owned conversation session.
use super::*;
use crate::board::{empty_state, exchange_view as exchange};

/// Measure the held group with the same wrapped body the painter uses. Empty
/// observations never reserve a minimum transcript or impose a growth cap.
pub(in crate::board) fn desired_height(app: &App, width: u16) -> u16 {
    let Some(target) = app.conversation_target() else {
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
    let has_link = super::link_preview(app, row, &app.workspace_bindings()).is_some();
    let mut state = app.conversation.borrow_mut();
    let session = state.sessions.entry(identity).or_default();
    session.update(
        app.detail(&Target::Row(target))
            .and_then(|detail| detail.reply),
    );
    session.correlate_loaded(app, row["name"].as_str().unwrap_or("unknown agent"));
    session.transcript_context(app, row);
    if session.reply.is_none() && session.outgoing.is_empty() && !session.has_pane_output() {
        return 1;
    }
    session.prepare_exchange(usize::from(width), app.look(), &app.references);
    let waiting = session.outgoing.is_empty()
        && session.reply.as_ref().is_some_and(|reply| {
            reply
                .key
                .as_ref()
                .is_some_and(|key| key.kind == super::super::home_leads::Kind::Question)
        });
    (1 + session.rendered.as_ref().unwrap().mapped.lines.len()
        + usize::from(waiting)
        + usize::from(has_link))
    .min(usize::from(u16::MAX)) as u16
}

pub(in crate::board) fn render(frame: &mut Frame, app: &App, area: Rect) {
    let look = app.look();
    {
        let mut state = app.conversation.borrow_mut();
        state.header_hits.clear();
        state.reply_hits.clear();
        state.response_area = None;
    }
    let Some(target) = app.conversation_target() else {
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
    let row = app.target_row(&target).expect("admitted conversation");
    let id = row["id"].as_str().expect("member identity");
    let identity = (
        app.view.as_ref().and_then(|view| view.me_id.clone()),
        id.to_owned(),
    );
    let incoming = app
        .detail(&Target::Row(target.clone()))
        .and_then(|detail| detail.reply);
    let me = app
        .view
        .as_ref()
        .and_then(|view| view.me.as_deref())
        .unwrap_or("you");
    let name = row["name"].as_str().unwrap_or("unknown agent");
    let bindings = app.workspace_bindings();
    let link = super::link_preview(app, row, &bindings);
    let reader_keys = app.workspace_context_bindings(
        app.workspace.selected.as_ref(),
        crate::config::OpsKeyScope::OverviewReader,
        row["pane"]["location"]["host"].as_str(),
    );
    let earlier_key = reader_keys
        .iter()
        .find(|(_, action)| {
            action.verb == crate::action::Verb::Ui(crate::action::OpsUiAction::ScrollPageUp)
        })
        .map(|(key, _)| crate::board::view::footer::key_label(key).to_owned())
        .filter(|_| app.input.is_none() && !app.modal_open() && !app.loading());
    let read_only = app
        .composer_projection()
        .is_none_or(|projection| projection.key.is_none());
    let mut state = app.conversation.borrow_mut();
    let switched = state.active.as_ref() != Some(&identity);
    state.active = Some(identity.clone());
    let session = state.sessions.entry(identity).or_default();
    session.update(incoming);
    session.correlate_loaded(app, name);
    session.transcript_context(app, row);
    if session.reply.is_none() && session.outgoing.is_empty() && !session.has_pane_output() {
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
    let age = session
        .reply
        .as_ref()
        .map_or("—", |reply| reply.age.as_str());
    let stamp = if age == "–" {
        "—".into()
    } else if age == "now" || age == "—" {
        age.into()
    } else {
        format!("{age} ago")
    };
    let stamp = if session.change != Change::Current {
        format!("{stamp} · End follows")
    } else {
        stamp
    };
    let read_only = read_only || session.change != Change::Current;
    let header = Rect {
        height: area.height.min(1),
        ..area
    };
    strip::paint_left(
        frame.buffer_mut(),
        header,
        exchange::header(name, me, &stamp, read_only, usize::from(area.width), look),
    );
    let rebuilt = session.prepare_exchange(usize::from(area.width), look, &app.references);
    let footer_rows = u16::from(question && session.outgoing.is_empty() && area.height >= 3)
        + u16::from(link.is_some() && area.height >= 4);
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
    if let Some(source) = &mut session.exact_source {
        source::paint(frame, app, source, &target, body, look);
        return;
    }
    let mapped = &session.rendered.as_ref().unwrap().mapped;
    if session.follow {
        app.scrolls.restore([(Pane::Replies, mapped.lines.len())]);
    } else if switched || rebuilt {
        app.scrolls.restore([(
            Pane::Replies,
            session.anchor.visual(&mapped.sources).unwrap_or_default(),
        )]);
    }
    let (offset, shown) = app.scrolls.show_exchange(
        frame,
        body,
        &mapped.lines,
        look,
        earlier_key,
        |index, line| {
            if session
                .rendered
                .as_ref()
                .unwrap()
                .pane_rows
                .contains(&index)
            {
                return line.clone();
            }
            let source = session
                .rendered
                .as_ref()
                .unwrap()
                .response_sources
                .as_ref()
                .and_then(|sources| sources[mapped.sources[index]])
                .unwrap_or(mapped.sources[index]);
            match session.authored_lines.get(&source) {
                Some(AuthoredLine::Outgoing { answer: true }) => exchange::replier(me, look),
                Some(AuthoredLine::Outgoing { answer: false }) => {
                    exchange::header(me, name, "now", read_only, usize::from(body.width), look)
                }
                Some(AuthoredLine::ReplyHeading) => {
                    exchange::replier(line.to_string().trim().trim_end_matches(" · now"), look)
                }
                Some(AuthoredLine::Status) => {
                    let text = line.to_string();
                    let suffix = format!(" · {}", session.agent_mark);
                    exchange::feedback(
                        text.trim_end()
                            .strip_suffix(suffix.trim_end())
                            .unwrap_or(text.trim_end())
                            .trim_end(),
                        look,
                    )
                }
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
    let mut hits = Vec::new();
    for index in offset..offset.saturating_add(shown).min(mapped.lines.len()) {
        if mapped.lines[index].width() > 0
            && let Some((key, sequence)) = session.response_key_at(mapped.sources[index])
        {
            hits.push(source::ReplyHit {
                area: Rect::new(body.x, body.y + (index - offset) as u16, body.width, 1),
                target: target.clone(),
                key: key.clone(),
                sequence,
                offset,
            });
        }
    }
    session.anchor.select_visual(
        &session.rendered.as_ref().unwrap().anchor_text,
        &mapped.sources,
        offset,
    );
    let mut y = body.bottom();
    if question && session.outgoing.is_empty() && footer_rows > 0 {
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
    state.reply_hits = hits;
    state.response_area = Some(body);
}

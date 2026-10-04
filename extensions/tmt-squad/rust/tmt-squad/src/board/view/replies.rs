//! Final responses, cached safe Markdown bodies and pane scrolling.

use super::fit;
use crate::board::app::App;
use crate::board::{
    markdown,
    notes::{sanitize, wrap},
};
use crate::{
    config::Pane,
    requests::{BODIES, age},
};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
};
use serde_json::Value;
use tmt_cli_style::Role;

/// Finals to the user's squad requests, newest first. Bodies are
/// agent-written, so they are sanitized like notes; reading acknowledges
/// nothing.
pub(super) fn reply_lines(
    look: crate::look::Look,
    replies: &[Value],
    width: usize,
    now_ms: u64,
    derived: &mut crate::board::derived::Derived,
) -> Vec<Line<'static>> {
    if derived
        .replies
        .as_ref()
        .is_none_or(|cached| cached.width != width || cached.look != look)
    {
        derived.replies = Some(crate::board::derived::ReplyBodies {
            width,
            look,
            bodies: Default::default(),
        });
    }
    let cached = derived.replies.as_mut().expect("prepared reply bodies");
    let mut lines = Vec::new();
    for (index, reply) in replies.iter().enumerate() {
        let text = |key: &str| sanitize(reply[key].as_str().unwrap_or_default());
        let when = reply["submittedAtMs"]
            .as_u64()
            .map_or(String::new(), |at| format!(" · {}", age(now_ms, at)));
        lines.push(Line::from(Span::styled(
            fit(&format!("{}{when}", text("to")), width),
            Style::new().add_modifier(Modifier::BOLD),
        )));
        // Keep the marker on the first line and align continuation text.
        for (index, prompt) in wrap(&text("prompt"), width.saturating_sub(4))
            .into_iter()
            .enumerate()
        {
            let prefix = if index == 0 { "  › " } else { "    " };
            lines.push(Line::styled(
                format!("{prefix}{prompt}"),
                look.role(Role::Dim),
            ));
        }
        match reply["response"].as_str() {
            Some(response) => {
                let id = reply["requestId"].as_str().unwrap_or_default();
                // A request's final response never changes, so its ID owns the cached body.
                let body = cached.bodies.entry(id.to_owned()).or_insert_with(|| {
                    markdown::render(&sanitize(response), width.saturating_sub(2), look)
                        .into_iter()
                        .map(|mut line| {
                            line.spans.insert(0, Span::raw("  "));
                            line
                        })
                        .collect()
                });
                lines.extend(body.iter().cloned());
            }
            None => {
                let id = reply["requestId"].as_str().unwrap_or_default();
                let hint = if index >= BODIES || reply["status"] == "retained" {
                    format!("  tmt result {id}")
                } else {
                    format!("  (final {})", text("status"))
                };
                lines.push(Line::styled(fit(&hint, width), look.role(Role::Dim)));
            }
        }
        lines.push(Line::from(""));
    }
    lines
}

pub(super) fn render_replies(frame: &mut Frame, app: &App, area: Rect) {
    let look = app.look();
    let Some(view) = &app.view else { return };
    let lines = if view.replies.is_empty() {
        vec![Line::styled(
            if view.me.is_none() {
                "(tmt squad me <name> shows the replies to your requests)"
            } else {
                "(no replies to your squad requests yet)"
            },
            look.role(Role::Dim),
        )]
    } else {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis() as u64);
        reply_lines(
            look,
            &view.replies,
            usize::from(area.width),
            now,
            &mut view.derived.borrow_mut(),
        )
    };
    app.scrolls
        .show(frame, Pane::Replies, area, lines, look.role(Role::Dim));
}

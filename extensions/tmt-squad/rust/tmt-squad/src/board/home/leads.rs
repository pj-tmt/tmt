//! HOME's deferred-lead projection for the shared boxed member-list scene.
use super::controller::HomeEntry;
use crate::{
    board::{
        app::{App, Compose, RowTarget},
        home_leads::{Kind, Lead},
        view::{
            fit,
            member_list::{self, Block, id},
            scene::{Kept, Key, rule},
        },
    },
    look::Look,
};
use ratatui::layout::Rect;
use serde_json::{Value, json};
use tmt_cli_style::{Role, table::escape};

pub(super) struct Section<'a> {
    pub first: usize,
    pub entries: &'a [HomeEntry<'a>],
    pub leads: &'a [&'a Lead],
    pub replies: bool,
}

/// The one line under a heading: the latest exchange, or why there is none.
fn preview(lead: &Lead, width: u16) -> Value {
    let (text, role) = lead.exchange.as_ref().map_or_else(
        || {
            (
                if lead.failure.is_some() {
                    "(exchange unavailable)"
                } else {
                    "–"
                }
                .into(),
                Role::Dim,
            )
        },
        |exchange| match exchange.kind {
            Kind::Question => (format!("asks: {}", exchange.preview), Role::Waiting),
            Kind::Asked => (format!("no reply yet to: {}", exchange.preview), Role::Dim),
            Kind::Reply => (exchange.preview.clone(), Role::Text),
        },
    );
    json!({
        "id": "preview",
        "text": format!(
            "  {}",
            fit(&text, usize::from(width.saturating_sub(5))).trim_end()
        ),
        "role": role.name(),
    })
}

fn heading(lead: &Lead, width: u16, now: u64) -> Value {
    let (mark, role) = match lead.exchange.as_ref().map(|exchange| exchange.kind) {
        Some(Kind::Question) => ("◆", Role::Waiting),
        Some(Kind::Asked) => ("…", Role::Dim),
        Some(Kind::Reply) => ("✓", Role::Working),
        None => (" ", Role::Dim),
    };
    let inner = usize::from(width.saturating_sub(2));
    let age = lead
        .exchange
        .as_ref()
        .and_then(|exchange| exchange.since_ms)
        .map_or_else(|| "–".into(), |at| crate::requests::age(now, at));
    let age = fit(&age, inner.saturating_sub(4)).trim_end().to_owned();
    let available = inner.saturating_sub(4 + unicode_width::UnicodeWidthStr::width(age.as_str()));
    let name_width = available.min(24);
    // Whether the squad column shows at all is the `md` step of the markup;
    // whether any room is left for it is a fit.
    let squad = if available > name_width + 3 {
        format!(
            "   {}",
            fit(&escape(&lead.squad), available - name_width - 3).trim_end()
        )
    } else {
        String::new()
    };
    json!({
        "mark": format!(" {mark} "),
        "mark_role": role.name(),
        "name": fit(&escape(lead.name()), name_width),
        "squad": squad,
        "age": format!("{age} "),
    })
}

/// The section's block, painted again only when its key changed.
pub(super) fn paint(
    app: &App,
    look: Look,
    area: Rect,
    now: u64,
    section: &Section<'_>,
    slot: &mut Kept<Block>,
) -> Block {
    let inner = Rect {
        x: area.x.saturating_add(1),
        width: area.width.saturating_sub(2),
        ..area
    };
    let mut previous_exchange = false;
    let rows =
        section
            .leads
            .iter()
            .enumerate()
            .map(|(local, lead)| {
                let index = section.first + local;
                let target = RowTarget::Home(section.entries[local].target.clone());
                let reading = app.input.as_ref().is_some_and(|input| {
                    matches!(input.compose, Compose::ReadLead { .. })
                        && input
                            .row_send
                            .as_ref()
                            .is_some_and(|send| send.target == target)
                });
                // One blank boxed line keeps a lead with an exchange apart from the
                // next lead, whether that one has an exchange or not.
                let separator = local > 0 && section.replies && previous_exchange;
                previous_exchange = lead.exchange.is_some();
                let mut after = Vec::new();
                if section.replies && lead.exchange.is_some() && !reading {
                    after.push(preview(lead, area.width));
                }
                if app
                    .sent
                    .as_ref()
                    .is_some_and(|feedback| feedback.sent && feedback.target == target)
                {
                    after.push(json!({"id": "sent", "text": "  ✓ sent", "role": "working"}));
                }
                let reserved = crate::board::view::waiting::reserved_lines(app, index, inner)
                    .unwrap_or_default();
                after.extend((0..reserved).map(
                    |line| json!({"id": format!("reserve-{line}"), "text": null, "role": null}),
                ));
                let mut row = heading(lead, area.width, now);
                row["id"] = json!(id(local));
                row["before"] = json!([]);
                for field in ["tag", "state", "model"] {
                    row[field] = json!("");
                }
                row["state_role"] = json!("text");
                row["separator"] = json!(if separator {
                    vec![json!({"id": "gap"})]
                } else {
                    Vec::new()
                });
                row["after"] = json!(after);
                row
            })
            .collect::<Vec<_>>();
    let selected = app
        .selected
        .checked_sub(section.first)
        .filter(|local| *local < section.leads.len())
        .map(id);
    let key = Key {
        width: area.width,
        look,
        selected,
        data: json!({
            "head": [
                {"id": "gap", "text": null, "role": null},
                {
                    "id": "rule",
                    "text": rule(
                        &format!(
                            "leads · latest from each · t {} replies",
                            if section.replies { "hides" } else { "shows" }
                        ),
                        usize::from(area.width),
                    ),
                    "role": Role::Muted.name(),
                },
            ],
            "leads": rows,
            "tail": [],
        }),
    };
    slot.get(key, member_list::build).clone()
}

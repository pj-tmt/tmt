//! HOME's deferred-lead projection for the shared boxed member-list scene.
use super::controller::HomeEntry;
use crate::{
    board::{
        app::{App, RowTarget},
        home_leads::{Kind, Lead},
        row_chips,
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

/// Cells of a lead heading's age (`45s`, `12m`, `3h`, `999d`).
const AGE_CELLS: usize = 4;

pub(super) struct Section<'a> {
    pub first: usize,
    pub entries: &'a [HomeEntry<'a>],
    pub leads: &'a [&'a Lead],
}

fn heading(labels: &crate::labels::Supplied, lead: &Lead, width: u16, now: u64) -> Value {
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
    // The age cell is a fixed width, right-aligned: its text never moves the name.
    let age_cells = if inner < member_list::NARROW {
        0
    } else {
        AGE_CELLS
    };
    let available = inner.saturating_sub(3 + if age_cells == 0 { 0 } else { age_cells + 1 });
    // The name takes at most half the heading, so a digest always has room beside it.
    let name_width = (available / 2).min(24);
    let chips = row_chips::padded(row_chips::pieces(labels, &lead.row, now, available / 2));
    let room = available.saturating_sub(row_chips::pieces_width(&chips));
    // Whether the squad column shows at all is the `md` step of the markup;
    // whether any room is left for it is a fit.
    let squad = if room > name_width + 3 {
        format!(
            "   {}",
            fit(&escape(&lead.squad), room - name_width - 3).trim_end()
        )
    } else {
        String::new()
    };
    json!({
        "chips": chips,
        "chips_visible": row_chips::fitted(labels, &lead.row, now, available / 2).1,
        "mark": format!(" {mark} "),
        "mark_role": role.name(),
        "name": fit(&escape(lead.name()), name_width),
        "name_role": Role::Text.name(),
        "squad": squad,
        "age": member_list::age_cell(&age, age_cells),
        "age_role": Role::Dim.name(),
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
    let rows = section
        .leads
        .iter()
        .enumerate()
        .map(|(local, lead)| {
            let index = section.first + local;
            let target = RowTarget::Home(section.entries[local].target.clone());
            let mut after = Vec::new();
            if let Some(text) = app
                .sent
                .as_ref()
                .and_then(|feedback| feedback.line(&target))
            {
                after.push(json!({"id": "sent", "text": format!("  {text}"), "role": "working"}));
            }
            let reserved =
                crate::board::view::waiting::reserved_lines(app, index, inner).unwrap_or_default();
            after.extend(
                (0..reserved).map(
                    |line| json!({"id": format!("reserve-{line}"), "text": null, "role": null}),
                ),
            );
            let mut row = heading(&app.labels, lead, area.width, now);
            row["id"] = json!(id(local));
            row["before"] = json!([]);
            for field in ["tag", "state", "model"] {
                row[field] = json!("");
            }
            row["model_role"] = json!(Role::Muted.name());
            row["state_role"] = json!("text");
            row["separator"] = json!([]);
            row["after"] = json!(after);
            let visible = if row["chips_visible"] == true {
                vec!["chips"]
            } else {
                vec![]
            };
            row["detail"] = app.detail_value(index, &visible);
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
                        "leads",
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

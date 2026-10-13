//! Retained hidden-name projection for typed HOME detail data. The retired lead
//! painter, its scene cache and its model/age carry state are not retained.
use crate::board::{home_leads::Lead, row_chips, view::fit};
use tmt_cli_style::table::escape;

/// Cells of a lead heading's age (`45s`, `12m`, `3h`, `999d`).
const AGE_CELLS: usize = 4;

fn heading_names(
    labels: &crate::labels::Supplied,
    lead: &Lead,
    width: u16,
    now: u64,
) -> (String, String) {
    let inner = usize::from(width.saturating_sub(2));
    // The age cell is a fixed width, right-aligned: its text never moves the name.
    let age_cells = if inner < 40 { 0 } else { AGE_CELLS };
    let available = inner.saturating_sub(3 + if age_cells == 0 { 0 } else { age_cells + 1 });
    // The name takes at most half the heading, so a digest always has room beside it.
    let name_width = (available / 2).min(24);
    let chips = row_chips::shown(labels, &lead.row, now, available / 2, true);
    let room = available.saturating_sub(chips.width);
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
    (
        fit(&escape(lead.name()), name_width),
        if width >= super::SM.cells {
            squad
        } else {
            String::new()
        },
    )
}

/// Preserve the fitted-name disclosure policy for retained typed detail callers.
pub(in crate::board) fn hidden_metadata(
    labels: &crate::labels::Supplied,
    lead: &Lead,
    width: u16,
    now: u64,
) -> Vec<(&'static str, String)> {
    let (name, shown) = heading_names(labels, lead, width, now);
    let mut fields = Vec::new();
    if name.trim_end() != escape(lead.name()) {
        fields.push(("lead", lead.name().into()));
    }
    let label = escape(&lead.squad);
    if !shown.contains(&label) {
        fields.push(("squad", lead.squad.clone()));
    }
    fields
}

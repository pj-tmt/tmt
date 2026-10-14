//! Shared cell values over admitted workspace rows; callers own scope and actions.
use super::{OpsTarget, OpsTreeRow, observe::OpsWorkspaceSnapshot};
use crate::{
    board::{inventory::Agent, rate::Reading},
    labels::Supplied,
};
use ratatui::layout::Rect;
use tmt_cli_style::Role;

pub use crate::config::OverviewColumn as Column;

impl Column {
    pub fn title(self) -> &'static str {
        match self {
            Self::Agent => "Agent",
            Self::Squad => "Squad",
            Self::State => "State",
            Self::TokensPerHour => "TOK/h",
            Self::Digest => "Digest",
            Self::Cron => "Cron",
            Self::Runtime => "Runtime",
            Self::Model => "Model",
        }
    }

    pub fn minimum(self) -> u16 {
        match self {
            Self::Agent => 12,
            Self::Squad | Self::State => 10,
            Self::TokensPerHour => 7,
            Self::Digest => 11, // The longest supplied primary mode plus its choice mark fits whole.
            Self::Runtime | Self::Model => 8,
            Self::Cron => 9,
        }
    }
}

/// Both tables drop whole fields; scope stays visible ahead of optional metadata.
pub fn visible(width: u16, fields: &[Column]) -> Vec<Column> {
    let mut shown = fields.to_vec();
    for removable in [
        Column::Model,
        Column::Runtime,
        Column::Digest,
        Column::Cron,
        Column::Squad,
    ] {
        let needed = shown.iter().map(|column| column.minimum()).sum::<u16>()
            + shown.len().saturating_sub(1) as u16;
        if needed <= width {
            break;
        }
        shown.retain(|column| *column != removable);
    }
    shown
}

/// One admitted template solves tracks; hosts cache and translate its rectangles.
pub fn solve(width: u16, fields: &[Column]) -> Vec<(Column, Rect)> {
    if width == 0 || fields.is_empty() {
        return vec![];
    }
    let fixed: u16 = fields
        .iter()
        .filter(|column| **column != Column::Agent)
        .map(|column| column.minimum())
        .sum();
    let agent = width.saturating_sub(fixed + fields.len().saturating_sub(1) as u16);
    let mut markup = String::from("<tmt-view version=\"1\"><tmt-row class=\"flex-row gap-1 h-1\">");
    for (index, column) in fields.iter().enumerate() {
        let cells = if *column == Column::Agent {
            agent
        } else {
            column.minimum()
        };
        markup.push_str(&format!(
            "<tmt-text id=\"c{index}\" class=\"w-{cells} shrink-0 truncate\">{}</tmt-text>",
            column.title()
        ));
    }
    markup.push_str("</tmt-row></tmt-view>");
    let file = "ops.workspace.columns.xml";
    let template = crate::board::view::scene::compile(
        file,
        &markup,
        &tmt_tui::binding::Schema::Object(Default::default()),
    );
    let root = template
        .materialize(
            file,
            &serde_json::json!({}),
            &crate::board::picker_surface::Data,
        )
        .expect("admitted workspace columns");
    let solved = tmt_tui::geometry::layout(&root, [width, 1], tmt_tui::text::measure)
        .expect("workspace column geometry");
    fields
        .iter()
        .copied()
        .enumerate()
        .filter_map(|(index, column)| {
            let id = format!("c{index}");
            let cell = solved.iter().find(|cell| {
                cell.node
                    .id
                    .as_ref()
                    .is_some_and(|path| path.last() == Some(&id))
            })?;
            Some((
                column,
                Rect::new(
                    cell.rect.x.max(0) as u16,
                    0,
                    cell.rect.width.min(u32::from(u16::MAX)) as u16,
                    1,
                )
                .intersection(Rect::new(0, 0, width, 1)),
            ))
        })
        .collect()
}

/// The shared sortable header surface. Hosts provide their caption and hit owner.
pub fn paint_header(
    buffer: &mut ratatui::buffer::Buffer,
    area: Rect,
    slots: &[(Column, Rect)],
    look: crate::look::Look,
    order: Option<crate::config::OverviewSort>,
    hover: Option<Column>,
    caption: impl Fn(Column) -> String,
) {
    use ratatui::{style::Modifier, text::Line};
    use tmt_tui::components::strip;
    if area.height == 0 {
        return;
    }
    let row = Rect { height: 1, ..area };
    let base = if look.depth == tmt_cli_style::Depth::None {
        look.role(Role::Muted)
    } else {
        look.role(Role::Track).patch(look.role(Role::Muted))
    }
    .add_modifier(Modifier::BOLD);
    buffer.set_style(row, base);
    for (column, slot) in slots {
        let mark = order.filter(|order| order.column == *column).map_or_else(
            || if hover == Some(*column) { " ↕" } else { "" },
            |order| {
                if order.direction == crate::config::OverviewDirection::Ascending {
                    " ↑"
                } else {
                    " ↓"
                }
            },
        );
        let mark_width = unicode_width::UnicodeWidthStr::width(mark) as u16;
        let label = crate::board::view::fit(
            &caption(*column),
            usize::from(slot.width.saturating_sub(mark_width)),
        );
        let style = if hover == Some(*column) {
            base.patch(look.role(Role::Accent))
        } else {
            base
        };
        strip::paint_left(buffer, *slot, Line::styled(format!("{label}{mark}"), style));
    }
    if area.height > 1 {
        strip::paint_left(
            buffer,
            Rect::new(area.x, area.y + 1, area.width, 1),
            Line::styled("─".repeat(usize::from(area.width)), look.role(Role::Dim)),
        );
    }
}

#[derive(Clone)]
pub struct WorkspaceColumnLabel {
    pub text: String,
    pub role: Role,
    pub actionable: bool,
}

pub struct WorkspaceColumnInput<'a> {
    pub row: &'a OpsTreeRow,
    pub snapshot: &'a OpsWorkspaceSnapshot,
    pub labels: &'a Supplied,
    pub usage: Option<Reading>,
    pub agent: Option<&'a Agent>,
    /// Supplied by the caller's existing global or room/owner count projection.
    pub timers: Option<(u32, u32)>,
}

#[derive(Clone)]
pub struct WorkspaceColumnValues {
    pub squad: Option<String>,
    pub tokens: Option<f64>,
    pub partial: bool,
    pub digest: Option<String>,
    pub digest_labels: Vec<WorkspaceColumnLabel>,
    pub cron: Option<(u32, u32)>,
    pub runtime: Option<String>,
    pub model: Option<String>,
}

impl WorkspaceColumnValues {
    pub fn text(&self, column: Column, row: &OpsTreeRow) -> String {
        match column {
            Column::Agent => row.title.clone(),
            Column::Squad => self.squad.clone().unwrap_or_else(|| "—".into()),
            Column::State => row.state.clone().unwrap_or_else(|| "—".into()),
            Column::TokensPerHour => self.tokens.map_or_else(
                || "—".into(),
                |tokens| {
                    format!(
                        "{}{}",
                        if self.partial { "~" } else { "" },
                        crate::source::tokens(tokens)
                    )
                },
            ),
            Column::Digest => self.digest.clone().unwrap_or_else(|| "—".into()),
            Column::Cron => self
                .cron
                .map_or_else(|| "unknown".into(), |(on, total)| format!("{on}/{total}")),
            Column::Runtime => self.runtime.clone().unwrap_or_else(|| "—".into()),
            Column::Model => self.model.clone().unwrap_or_else(|| "—".into()),
        }
    }
}

pub fn project(input: WorkspaceColumnInput<'_>) -> Option<WorkspaceColumnValues> {
    let OpsTarget::Member {
        room_id,
        identity_id,
    } = &input.row.target
    else {
        return None;
    };
    let squad = room_id
        .as_ref()
        .and_then(|room| {
            input
                .snapshot
                .roster
                .squads
                .iter()
                .find(|squad| &squad.room_id == room)
                .map(|squad| squad.name.clone())
        })
        .or_else(|| room_id.is_none().then(|| "Common pool".into()));
    let verified = input
        .labels
        .digest()
        .is_some_and(|(status, stale)| !stale && status.members.contains_key(identity_id));
    let digest_labels: Vec<_> = input
        .labels
        .of(identity_id)
        .filter(|(source, _)| *source == "digest")
        .map(|(_, label)| WorkspaceColumnLabel {
            text: label.text.clone(),
            role: label.role,
            actionable: label.action.is_some(),
        })
        .collect();
    let digest = verified
        .then(|| digest_labels.first().map(|label| label.text.clone()))
        .flatten();
    // The cell's primary value is the supplier's mode label; secondary labels
    // remain available to the painter and Details without becoming a policy.
    Some(WorkspaceColumnValues {
        squad,
        tokens: input.usage.map(|reading| reading.tokens as f64),
        partial: input.usage.is_some_and(|reading| reading.partial),
        digest,
        digest_labels,
        cron: input.timers,
        runtime: input.agent.and_then(|agent| agent.runtime.clone()),
        model: input.agent.and_then(|agent| agent.model.clone()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_header_surface_hover_arrow_and_no_color_are_independent_of_selection() {
        use ratatui::{
            buffer::Buffer,
            style::{Color, Modifier},
        };
        for base in [
            tmt_cli_style::theme::Base::Tmt,
            tmt_cli_style::theme::Base::TmtLight,
        ] {
            for depth in [tmt_cli_style::Depth::TrueColor, tmt_cli_style::Depth::None] {
                let look = crate::look::Look {
                    theme: tmt_cli_style::Theme::new(base),
                    depth,
                };
                let area = Rect::new(0, 0, 100, 3);
                let slots = solve(100, &Column::ALL);
                let caption = |column: Column| column.title().to_ascii_uppercase();
                let mut buffer = Buffer::empty(area);
                paint_header(
                    &mut buffer,
                    Rect { height: 2, ..area },
                    &slots,
                    look,
                    None,
                    Some(Column::Agent),
                    caption,
                );
                assert!(buffer[(0, 0)].modifier.contains(Modifier::BOLD));
                assert_eq!(
                    buffer[(0, 0)].fg,
                    look.role(Role::Accent).fg.unwrap_or(Color::Reset)
                );
                assert_eq!(
                    buffer[(0, 0)].bg,
                    if depth == tmt_cli_style::Depth::None {
                        Color::Reset
                    } else {
                        look.role(Role::Track).bg.unwrap()
                    }
                );
                assert_eq!(buffer[(0, 1)].symbol(), "─");
                assert_eq!(
                    buffer[(0, 2)].symbol(),
                    " ",
                    "header never paints into the data row"
                );
                let agent = slots[0].1;
                let shown = (agent.x..agent.right())
                    .map(|x| buffer[(x, 0)].symbol())
                    .collect::<String>();
                assert!(shown.contains('↕'));
                let order = Some(crate::config::OverviewSort::initial(Column::Agent));
                paint_header(
                    &mut buffer,
                    Rect { height: 2, ..area },
                    &slots,
                    look,
                    order,
                    Some(Column::Agent),
                    caption,
                );
                let shown = (agent.x..agent.right())
                    .map(|x| buffer[(x, 0)].symbol())
                    .collect::<String>();
                assert!(
                    shown.contains('↑') && !shown.contains('↕'),
                    "active arrow survives hover"
                );
            }
        }
    }

    #[test]
    fn supplier_choice_labels_survive_missing_typed_statistics() {
        let mut app = crate::board::app::workspace::tests::board();
        let rows = app.workspace_rows();
        let row = rows
            .iter()
            .find(|row| matches!(row.target, OpsTarget::Member { .. }))
            .unwrap();
        let OpsTarget::Member { identity_id, .. } = &row.target else {
            unreachable!()
        };
        app.labels = Supplied::from_rows(vec![(
            "digest".into(),
            [(
                identity_id.clone(),
                vec![crate::labels::Label {
                    text: "supplier mode".into(),
                    role: Role::Waiting,
                    action: None,
                }],
            )]
            .into(),
        )]);
        let projected = project(WorkspaceColumnInput {
            row,
            snapshot: app.workspace_snapshot().unwrap(),
            labels: &app.labels,
            usage: None,
            agent: None,
            timers: Some((0, 0)),
        })
        .unwrap();
        assert!(
            projected.digest.is_none(),
            "generic labels cannot invent typed mode evidence"
        );
        assert_eq!(projected.digest_labels[0].text, "supplier mode");
        assert_eq!(projected.digest_labels[0].role, Role::Waiting);
        assert_eq!(
            projected.cron,
            Some((0, 0)),
            "an admitted zero stays distinct from missing"
        );
        assert_eq!(projected.text(Column::Runtime, row), "—");
    }

    #[test]
    fn side_pane_keeps_squad_and_all_monitor_columns() {
        for width in [94, 96, 100, 160] {
            let shown = visible(width, &Column::ALL);
            assert_eq!(shown, Column::ALL);
            let cells = solve(width, &shown);
            assert_eq!(cells.len(), Column::ALL.len());
            assert!(
                cells
                    .iter()
                    .all(|(_, area)| area.right() <= width && area.height == 1)
            );
        }
        let narrow = visible(60, &Column::ALL);
        assert!(narrow.contains(&Column::Squad));
        assert!(narrow.contains(&Column::Agent));
        assert!(narrow.contains(&Column::State));
        assert!(narrow.contains(&Column::TokensPerHour));
        assert!(solve(0, &Column::ALL).is_empty());
    }

    #[test]
    fn agents_tree_uses_the_same_column_tracks() {
        let fields = [Column::Agent, Column::State, Column::Digest, Column::Cron];
        for width in [60, 100, 160] {
            assert_eq!(visible(width, &fields), fields);
            let cells = solve(width, &fields);
            assert_eq!(cells.len(), fields.len());
            assert!(cells.windows(2).all(|pair| pair[0].1.right() < pair[1].1.x));
        }
    }
}

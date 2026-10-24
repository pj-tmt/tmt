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
            Self::Cron => 10,
        }
    }
}

/// Both host profiles materialize their tracks through this one TUI solver.
fn solve_tracks<K: Copy>(width: u16, tracks: &[(K, u16, &str, u16)]) -> Vec<(K, Rect)> {
    if width == 0 || tracks.is_empty() {
        return Vec::new();
    }
    let mut markup = String::from("<tmt-view version=\"1\"><tmt-row class=\"flex-row h-1\">");
    for (index, (_, cells, title, gap)) in tracks.iter().enumerate() {
        markup.push_str(&format!(
            "<tmt-text id=\"c{index}\" class=\"w-{cells} shrink-0 truncate\">{title}</tmt-text>"
        ));
        if index + 1 < tracks.len() {
            markup.push_str(&format!(
                "<tmt-text class=\"w-{gap} shrink-0\"> </tmt-text>"
            ));
        }
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
    tracks
        .iter()
        .enumerate()
        .filter_map(|(index, (column, _, _, _))| {
            let id = format!("c{index}");
            let cell = solved.iter().find(|cell| {
                cell.node
                    .id
                    .as_ref()
                    .is_some_and(|path| path.last() == Some(&id))
            })?;
            Some((
                *column,
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

/// Agents fields are presentation vocabulary, not configurable Overview columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentsField {
    Agent,
    Model,
    State,
    Session,
    Tokens,
    Cron,
}

impl AgentsField {
    pub fn caption(self, glyphs: crate::board::overview::cron_counts::GlyphSet) -> &'static str {
        match self {
            Self::Agent => "SQUAD / AGENT",
            Self::Model => "MODEL / ALERT",
            Self::State => "STATE",
            Self::Session => "HOST / SESSION",
            Self::Tokens => crate::config::TokRange::Hour.caption(),
            Self::Cron => match glyphs {
                crate::board::overview::cron_counts::GlyphSet::Unicode => "◷",
                crate::board::overview::cron_counts::GlyphSet::Ascii => "CRON",
            },
        }
    }
    fn minimum(self) -> u16 {
        match self {
            Self::Agent => 25,
            Self::State => 21,
            Self::Session => 20,
            Self::Tokens => 7,
            Self::Cron => 4,
            Self::Model => 12,
        }
    }
    fn gap(self) -> u16 {
        match self {
            Self::Tokens => 3,
            Self::Cron => 1,
            _ => 1,
        }
    }
}

#[derive(Default)]
pub(in crate::board) struct AgentsLayout {
    held: Option<(u16, Vec<(AgentsField, Rect)>)>,
    roster: Option<(u16, Vec<(RosterField, Rect)>)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RosterField {
    Member,
    State,
    Task,
}

impl RosterField {
    pub fn caption(self) -> &'static str {
        match self {
            Self::Member => "MEMBER",
            Self::State => "STATE",
            Self::Task => "TASK / NEXT ACTION",
        }
    }
}
impl AgentsLayout {
    pub fn roster_slots(&mut self, area: Rect) -> Vec<(RosterField, Rect)> {
        if self
            .roster
            .as_ref()
            .is_none_or(|(width, _)| *width != area.width)
        {
            let member = if area.width >= 66 { 22 } else { 12 };
            let state = 15;
            let task = area.width.saturating_sub(member + state + 2);
            self.roster = Some((
                area.width,
                solve_tracks(
                    area.width,
                    &[
                        (RosterField::Member, member, "MEMBER", 1),
                        (RosterField::State, state, "STATE", 1),
                        (RosterField::Task, task, "TASK / NEXT ACTION", 0),
                    ],
                ),
            ));
        }
        self.roster
            .as_ref()
            .unwrap()
            .1
            .iter()
            .map(|(field, slot)| {
                (
                    *field,
                    Rect::new(area.x + slot.x, area.y, slot.width, 1).intersection(area),
                )
            })
            .collect()
    }
    /// `inside` is the list rectangle after canonical placement and pane chrome.
    pub fn slots(&mut self, inside: Rect) -> Vec<(AgentsField, Rect)> {
        if self
            .held
            .as_ref()
            .is_none_or(|(width, _)| *width != inside.width)
        {
            let width = inside.width;
            let mut fields = vec![
                AgentsField::Agent,
                AgentsField::State,
                AgentsField::Session,
                AgentsField::Tokens,
                AgentsField::Cron,
                AgentsField::Model,
            ];
            let demand = |fields: &[AgentsField]| {
                fields.iter().map(|f| f.minimum()).sum::<u16>()
                    + fields
                        .iter()
                        .take(fields.len().saturating_sub(1))
                        .map(|f| f.gap())
                        .sum::<u16>()
            };
            for optional in [
                AgentsField::Session,
                AgentsField::Model,
                AgentsField::Cron,
                AgentsField::Tokens,
            ] {
                if demand(&fields) <= width {
                    break;
                }
                fields.retain(|f| *f != optional);
            }
            let state_width = AgentsField::State.minimum().min(width.saturating_sub(2));
            let name_width = if fields.len() == 2 {
                width.saturating_sub(state_width + 1)
            } else {
                AgentsField::Agent.minimum()
            };
            let tracks: Vec<_> = fields
                .iter()
                .copied()
                .map(|field| {
                    (
                        field,
                        if field == AgentsField::Agent {
                            name_width
                        } else if field == AgentsField::State {
                            state_width
                        } else {
                            field.minimum()
                        },
                        field.caption(Default::default()),
                        field.gap(),
                    )
                })
                .collect();
            self.held = Some((inside.width, solve_tracks(width, &tracks)));
        }
        self.held
            .as_ref()
            .unwrap()
            .1
            .iter()
            .map(|(field, slot)| {
                // MODEL takes what the other columns leave, so a squad's "now"
                // text has room on a wide list.
                let width = if *field == AgentsField::Model {
                    inside.width.saturating_sub(slot.x)
                } else {
                    slot.width
                };
                (
                    *field,
                    Rect::new(inside.x.saturating_add(slot.x), inside.y, width, 1)
                        .intersection(inside),
                )
            })
            .collect()
    }
}

/// How the TOK column is headed: its saved range, in tokens or in money.
#[derive(Clone, Copy, Default)]
pub struct TokHeading {
    pub range: crate::config::TokRange,
    pub money: bool,
}

impl TokHeading {
    pub fn caption(self) -> &'static str {
        self.range.caption_in(self.money)
    }

    /// The unit word on screen: `TOK` or `≈$`.
    pub fn unit(self) -> &'static str {
        if self.money { "≈$" } else { "TOK" }
    }

    /// The unit a click on the unit word switches to.
    pub fn other_unit(self) -> &'static str {
        Self {
            money: !self.money,
            ..self
        }
        .unit()
    }
}

/// Paints the Agents column header and returns the hit areas of its controls:
/// the TOK unit and window words when the TOK column is present, and the sortable words.
pub fn paint_agents_header(
    buffer: &mut ratatui::buffer::Buffer,
    area: Rect,
    slots: &[(AgentsField, Rect)],
    look: crate::look::Look,
    glyphs: crate::board::overview::cron_counts::GlyphSet,
    tok: TokHeading,
    marks: &crate::board::header_sort::Marks,
) -> AgentsHeaderHits {
    use crate::board::header_sort::{Header, word_hit};
    let mut hits = AgentsHeaderHits::default();
    if area.height == 0 {
        return hits;
    }
    for (field, slot) in slots {
        let sorts = match field {
            AgentsField::Agent => Some((Header::Squads(crate::config::OpsSort::Name), 2)),
            AgentsField::State => Some((Header::Squads(crate::config::OpsSort::Attention), 0)),
            _ => None,
        };
        let base = look.role(Role::Muted);
        let mut word = field.caption(glyphs);
        let mut style = base;
        let caption = if *field == AgentsField::Tokens {
            if let Some(cell) = marks.tokens(None, tok, slot.width, base, look) {
                for (from, size, control) in cell.parts {
                    let hit =
                        Rect::new(slot.x.saturating_add(from), slot.y, size, 1).intersection(*slot);
                    hits.tok.push((hit, control));
                }
                tmt_tui::components::strip::paint_left(
                    buffer,
                    *slot,
                    ratatui::text::Line::from(cell.spans),
                );
                continue;
            }
            word = tok.caption();
            word.to_owned()
        } else if let Some((header, lead)) = sorts {
            if let Some(hit) = word_hit(*slot, lead, word) {
                hits.sorts.push((hit, header));
            }
            style = marks.style(header, base, look);
            let lead = " ".repeat(usize::from(lead));
            match marks.mark(header) {
                Some(mark) => format!("{lead}{word} {mark}"),
                None => format!("{lead}{word}"),
            }
        } else {
            word.to_owned()
        };
        let align = if *field == AgentsField::Tokens {
            tmt_cli_style::grid::Align::Right
        } else {
            tmt_cli_style::grid::Align::Left
        };
        tmt_tui::components::strip::paint_left(
            buffer,
            *slot,
            ratatui::text::Line::styled(
                tmt_tui::text::fit_line(
                    &caption,
                    slot.width,
                    tmt_tui::style::TextFlow::Truncate,
                    align,
                ),
                style,
            ),
        );
    }
    hits
}

/// What the Agents header made clickable.
#[derive(Default)]
pub struct AgentsHeaderHits {
    /// The unit and window words of `TOK 1h`.
    pub tok: Vec<(Rect, crate::board::header_sort::Control)>,
    pub sorts: Vec<(Rect, crate::board::header_sort::Header)>,
}

pub fn paint_roster_header(
    buffer: &mut ratatui::buffer::Buffer,
    area: Rect,
    slots: &[(RosterField, Rect)],
    look: crate::look::Look,
) {
    paint_header_tracks(
        buffer,
        area,
        slots,
        look,
        |field| field.caption().into(),
        |_| ("", false),
    );
}

fn paint_header_tracks<K: Copy>(
    buffer: &mut ratatui::buffer::Buffer,
    area: Rect,
    slots: &[(K, Rect)],
    look: crate::look::Look,
    caption: impl Fn(K) -> String,
    annotation: impl Fn(K) -> (&'static str, bool),
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
    .add_modifier(
        Modifier::BOLD
            | if area.height == 1 {
                Modifier::UNDERLINED
            } else {
                Modifier::empty()
            },
    );
    buffer.set_style(row, base);
    for (column, slot) in slots {
        let (mark, hot) = annotation(*column);
        let mark_width = unicode_width::UnicodeWidthStr::width(mark) as u16;
        let label = crate::board::view::fit(
            &caption(*column),
            usize::from(slot.width.saturating_sub(mark_width)),
        );
        let style = if hot {
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

pub fn state_symbol(state: Option<&str>) -> &'static str {
    match crate::board::state_cell::glyph(state) {
        "?" => "—",
        known => known,
    }
}

/// State semantics apply only to the state cell; other table cells stay neutral.
pub fn state_style(app: &crate::board::app::App, row: &OpsTreeRow) -> ratatui::style::Style {
    let look = app.look();
    let role = match row.state.as_deref() {
        Some("waiting") => Role::Waiting,
        Some("blocked") => Role::Blocked,
        _ => Role::Text,
    };
    app.workspace_snapshot()
        .and_then(|snapshot| snapshot.rows.get(&row.target))
        .and_then(|raw| raw["colors"]["state"].as_str())
        .filter(|_| row.state.as_deref() != Some("waiting"))
        .map_or_else(|| look.role(role), |color| look.named(color))
}

/// The activity glyph: only a request that waits on the user uses attention
/// colour; the `?` of stale or unknown activity is muted.
pub fn activity_style(app: &crate::board::app::App, state: &str) -> ratatui::style::Style {
    app.look().role(match state {
        "waiting" => Role::Waiting,
        "stale" | "unknown" => Role::Muted,
        _ => Role::Text,
    })
}

/// The secondary task state is muted; only a blocked-type (`✗`) task is red.
/// `◆` and `✗` are the board's only attention colours, so no authored task
/// colour applies here.
pub fn task_style(app: &crate::board::app::App, row: &OpsTreeRow) -> ratatui::style::Style {
    let task = row
        .activity
        .as_ref()
        .and_then(|member| member.task.as_deref());
    app.look()
        .role(if crate::board::state_cell::glyph(task) == "✗" {
            Role::Blocked
        } else {
            Role::Muted
        })
}

/// A member's STATE cell in `width` cells: the glyph, the activity words and
/// the task state when it fits. Only the glyph carries request attention colour;
/// the words stay neutral and only a blocked-type task is red. Unread replies
/// replace the glyph and the activity words with the shared unread mark
/// (`● N unread`, blue on the glyph only); the task still follows.
pub fn activity_spans(
    app: &crate::board::app::App,
    row: &OpsTreeRow,
    label: &crate::board::activity::Label,
    width: usize,
    selected: bool,
) -> (
    Vec<ratatui::text::Span<'static>>,
    Option<ratatui::text::Span<'static>>,
    Option<ratatui::text::Span<'static>>,
) {
    use unicode_width::UnicodeWidthStr;
    let (primary, task) = activity_parts(app, row, label, width, selected);
    let used = primary
        .iter()
        .map(|span| span.content.width())
        .sum::<usize>()
        + task.as_ref().map_or(0, |span| span.content.width());
    let suffix = app
        .notes
        .row_label(&row.target)
        .map(|text| format!(" · {text}"))
        .filter(|text| used + text.width() <= width);
    let suffix = suffix.map(|suffix| {
        ratatui::text::Span::styled(
            suffix,
            table_style(app, selected, app.look().role(Role::Muted)),
        )
    });
    (primary, task, suffix)
}

fn activity_parts(
    app: &crate::board::app::App,
    row: &OpsTreeRow,
    label: &crate::board::activity::Label,
    width: usize,
    selected: bool,
) -> (
    Vec<ratatui::text::Span<'static>>,
    Option<ratatui::text::Span<'static>>,
) {
    use ratatui::text::Span;
    use unicode_width::UnicodeWidthStr;
    if width == 0 {
        return (Vec::new(), None);
    }
    let task_span =
        |text: String| Span::styled(text, table_style(app, selected, task_style(app, row)));
    if let Some(unread) =
        crate::board::state_cell::unread(app.unread_target(&row.target), app.look())
    {
        let mut used = 0;
        let mut primary = Vec::new();
        for span in unread.spans {
            let room = width.saturating_sub(used);
            let content = crate::board::view::fit(&span.content, span.content.width().min(room));
            if content.is_empty() {
                break;
            }
            used += content.width();
            primary.push(Span::styled(
                content,
                table_style(app, selected, span.style),
            ));
        }
        let room = width.saturating_sub(used);
        // The unread words replace the activity word, so a task equal to that
        // word still follows; only a repeat of the shown words is left out.
        let shown: String = primary
            .iter()
            .skip(1)
            .map(|span| span.content.as_ref())
            .collect();
        let shown = shown.trim();
        let task = label
            .task
            .as_ref()
            .filter(|task| *task != shown)
            .and_then(|task| {
                let tail = format!(" · {task}");
                // A suffix that does not fit whole is dropped, never cut inside a word.
                (tail.width() <= room).then_some(tail)
            });
        return (primary, task.map(task_span));
    }
    let over = label
        .idle_over
        .then(|| crate::board::state_cell::over(app.cron_glyphs, app.look()));
    let symbol = over.as_ref().map_or_else(
        || crate::board::state_cell::glyph(Some(label.state)),
        |mark| mark.content.as_ref(),
    );
    // Unknown activity has no word: the glyph is the label's own `?`.
    let unknown = label.state == "unknown";
    let (head, task) = label.fit(if unknown {
        width
    } else {
        width.saturating_sub(symbol.width() + 1)
    });
    let words = if unknown {
        String::new()
    } else {
        format!(" {head}")
    };
    let room = width.saturating_sub(symbol.width());
    let words = crate::board::view::fit(&words, words.width().min(room));
    let mut primary = vec![Span::styled(
        symbol.to_owned(),
        table_style(
            app,
            selected,
            over.as_ref()
                .map_or_else(|| activity_style(app, label.state), |mark| mark.style),
        ),
    )];
    if !words.is_empty() {
        primary.push(Span::styled(
            words,
            table_style(app, selected, app.look().role(Role::Text)),
        ));
    }
    (
        primary,
        task.map(|task| Span::styled(task, table_style(app, selected, task_style(app, row)))),
    )
}

/// Paints a member's STATE words into `cell` and records their areas as state
/// cells, so selection keeps their colours. Returns the painted width.
pub fn paint_activity(
    buffer: &mut ratatui::buffer::Buffer,
    app: &crate::board::app::App,
    row: &OpsTreeRow,
    label: &crate::board::activity::Label,
    cell: ratatui::layout::Rect,
    selected: bool,
) -> u16 {
    use ratatui::layout::Rect;
    let (primary, task, todo) = activity_spans(app, row, label, usize::from(cell.width), selected);
    let width = primary.iter().map(|span| span.width()).sum::<usize>() as u16;
    let task_width = task.as_ref().map_or(0, |task| {
        (task.width() as u16).min(cell.width.saturating_sub(width))
    });
    let mut states = app.workspace_state_cells.borrow_mut();
    if width > 0 {
        states.push(Rect::new(cell.x, cell.y, width, 1));
    }
    if task_width > 0 {
        states.push(Rect::new(cell.x + width, cell.y, task_width, 1));
    }
    let todo_width = todo.as_ref().map_or(0, |span| span.width() as u16);
    tmt_tui::components::strip::paint_left(
        buffer,
        cell,
        ratatui::text::Line::from_iter(primary.into_iter().chain(task).chain(todo)),
    );
    width + task_width + todo_width
}

pub fn table_style(
    app: &crate::board::app::App,
    selected: bool,
    style: ratatui::style::Style,
) -> ratatui::style::Style {
    if selected {
        app.look()
            .selection()
            .patch(app.look().row_span(true, style, false))
            .remove_modifier(ratatui::style::Modifier::BOLD)
    } else {
        style
    }
}

#[derive(Clone)]
pub struct WorkspaceColumnLabel {
    pub text: String,
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
    pub cron: Option<(u32, u32)>,
    pub runtime: Option<String>,
    pub model: Option<String>,
    /// A basic driver observes no runtime: say `basic` where the runtime is blank.
    pub basic: bool,
}

/// Dense model cells use the acquired model only while its driver is running.
/// Keep the original inventory value intact for rate attribution and Details.
pub(in crate::board) fn model_text(runtime: Option<&str>, model: Option<&str>) -> String {
    runtime
        .filter(|runtime| !runtime.is_empty())
        .and(model)
        .map(|model| model.strip_prefix("claude-").unwrap_or(model))
        .filter(|model| !model.is_empty())
        .map(super::render::safe)
        .unwrap_or_else(|| "—".into())
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
                        crate::source::tokens(tokens.round() as u128)
                    )
                },
            ),
            Column::Digest => self.digest.clone().unwrap_or_else(|| "—".into()),
            Column::Cron => self
                .cron
                .map_or_else(|| "unknown".into(), |(on, total)| format!("{on}/{total}")),
            Column::Runtime => self
                .runtime
                .clone()
                .unwrap_or_else(|| if self.basic { "basic" } else { "—" }.into()),
            Column::Model => self.model.clone().unwrap_or_else(|| "—".into()),
        }
    }
}

pub fn member_digest_labels(labels: &Supplied, id: &str) -> Vec<WorkspaceColumnLabel> {
    labels
        .of(id)
        .filter(|(source, _)| *source == "digest")
        .map(|(_, label)| WorkspaceColumnLabel {
            text: label.text.clone(),
            actionable: label.action.is_some(),
        })
        .collect()
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
    let digest_labels = member_digest_labels(input.labels, identity_id);
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
        cron: input.timers,
        runtime: input.agent.and_then(|agent| agent.runtime.clone()),
        model: input.agent.and_then(|agent| agent.model.clone()),
        basic: input.agent.is_some_and(|agent| agent.basic),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dense_model_text_requires_a_running_driver_and_shortens_only_the_leading_provider() {
        for (runtime, model, expected) in [
            (Some("claude"), Some("claude-opus-5-5"), "opus-5-5"),
            (Some("codex"), Some("gpt-6.1-sol"), "gpt-6.1-sol"),
            (
                Some("claude"),
                Some("other-claude-model"),
                "other-claude-model",
            ),
            (None, Some("claude-opus-5-5"), "—"),
            (Some(""), Some("gpt-6.1-sol"), "—"),
            (Some("claude"), None, "—"),
            (Some("claude"), Some("claude-"), "—"),
        ] {
            assert_eq!(model_text(runtime, model), expected);
        }
        let shown = model_text(Some("codex"), Some("gpt-6.1-sol"));
        assert_eq!(crate::board::view::fit(&shown, 8), "gpt-6.1…");
    }

    #[test]
    fn agents_tracks_follow_pane_width_and_drop_only_whole_optional_fields() {
        let mut layout = AgentsLayout::default();
        let wide = layout.slots(Rect::new(0, 7, 75, 1));
        assert_eq!(
            wide.iter()
                .map(|(field, slot)| (*field, slot.x, slot.width))
                .collect::<Vec<_>>(),
            vec![
                (AgentsField::Agent, 0, 25),
                (AgentsField::State, 26, 21),
                (AgentsField::Tokens, 48, 7),
                (AgentsField::Cron, 58, 4),
                (AgentsField::Model, 63, 12),
            ]
        );
        for width in 0..=160 {
            let area = Rect::new(4, 8, width, 1);
            let slots = layout.slots(area);
            assert!(
                slots
                    .iter()
                    .all(|(_, slot)| slot.intersection(area) == *slot)
            );
            assert!(
                slots
                    .windows(2)
                    .all(|pair| pair[0].1.right() <= pair[1].1.x)
            );
            assert_eq!(
                slots.iter().any(|(field, _)| *field == AgentsField::Model),
                width >= 75
            );
            for (field, slot) in &slots {
                if matches!(
                    field,
                    AgentsField::Tokens | AgentsField::Cron | AgentsField::Model
                ) {
                    // MODEL, the last field, also takes what the others leave.
                    assert!(slot.width >= field.minimum(), "optional fields fit whole");
                    if *field != AgentsField::Model {
                        assert_eq!(slot.width, field.minimum());
                    }
                }
            }
        }
        assert_eq!(AgentsField::Cron.caption(Default::default()), "◷");
        let narrow = layout.slots(Rect::new(0, 0, 74, 1));
        assert!(!narrow.iter().any(|(field, _)| *field == AgentsField::Model));
        assert!(
            narrow
                .iter()
                .any(|(field, _)| *field == AgentsField::Tokens)
        );
    }

    #[test]
    fn shared_header_stays_bounded_and_neutral_at_every_depth() {
        use ratatui::buffer::Buffer;
        for base in [tmt_cli_style::Base::Tmt, tmt_cli_style::Base::TmtLight] {
            for depth in [tmt_cli_style::Depth::TrueColor, tmt_cli_style::Depth::None] {
                let look = crate::look::Look {
                    theme: tmt_cli_style::Theme::new(base),
                    depth,
                };
                let mut layout = AgentsLayout::default();
                let area = Rect::new(0, 0, 100, 3);
                let slots = layout.slots(area);
                let mut buffer = Buffer::empty(area);
                paint_agents_header(
                    &mut buffer,
                    Rect { height: 2, ..area },
                    &slots,
                    look,
                    Default::default(),
                    Default::default(),
                    &crate::board::header_sort::Marks::new(
                        Default::default(),
                        None,
                        Default::default(),
                    ),
                );
                assert_eq!(
                    buffer[(0, 2)].symbol(),
                    " ",
                    "header cannot paint into its data row"
                );
                assert_eq!(buffer[(0, 1)].symbol(), " ");
                assert!(
                    buffer
                        .content
                        .iter()
                        .all(|cell| !cell.symbol().contains('↕'))
                );
                assert_eq!(
                    buffer[(0, 0)].fg,
                    look.role(Role::Muted).fg.unwrap_or_default()
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
        assert_eq!(
            member_digest_labels(&app.labels, identity_id)[0].text,
            "supplier mode"
        );
        assert_eq!(
            app.labels.of(identity_id).next().unwrap().1.role,
            Role::Waiting
        );
        assert_eq!(
            projected.cron,
            Some((0, 0)),
            "an admitted zero stays distinct from missing"
        );
        assert_eq!(projected.text(Column::Runtime, row), "—");
    }

    #[test]
    fn a_basic_driver_says_basic_where_its_runtime_is_blank_and_nowhere_else() {
        let row = OpsTreeRow {
            target: OpsTarget::Pool,
            title: "pi-1".into(),
            depth: 0,
            lead: false,
            state: None,
            presence: None,
            usage: None,
            activity: None,
            count: None,
            observation: crate::board::workspace::OpsObservation::Complete,
        };
        let values = |runtime: Option<&str>, basic: bool| WorkspaceColumnValues {
            squad: None,
            tokens: None,
            partial: false,
            digest: None,
            cron: None,
            runtime: runtime.map(str::to_owned),
            model: None,
            basic,
        };
        assert_eq!(values(None, true).text(Column::Runtime, &row), "basic");
        assert_eq!(values(None, false).text(Column::Runtime, &row), "—");
        // An observed runtime always wins, and no other cell changes.
        assert_eq!(values(Some("pi"), true).text(Column::Runtime, &row), "pi");
        assert_eq!(values(None, true).text(Column::Model, &row), "—");
        assert_eq!(values(None, true).text(Column::State, &row), "—");
    }
}

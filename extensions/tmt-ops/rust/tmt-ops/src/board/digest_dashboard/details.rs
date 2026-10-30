//! Details for the selected Digest row in three tabs: Impact holds the amounts, Delivery what the
//! supplier holds now, Evidence the basis behind both. Each line belongs to exactly one tab; the
//! Estimate basis modal paints the Evidence lines, so the two never drift apart.
use super::{metrics, model};
use crate::board::{
    app::{App, Effect, Request},
    usage_report::View,
};
use ratatui::{Frame, crossterm::event::Event, layout::Rect};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::OnceLock};
use tmt_tui::{
    binding::{Schema, Schemas, Scopes, Sources},
    components::{
        ScrollState,
        surface::{self, ModalSurface, RenderStyle},
    },
};

/// The Details tab on the Digest tab; session state until slice 5 persists it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::board) enum Tab {
    #[default]
    Impact,
    Delivery,
    Evidence,
}

impl Tab {
    pub(in crate::board) const ALL: [Self; 3] = [Self::Impact, Self::Delivery, Self::Evidence];

    pub(in crate::board) fn label(self) -> &'static str {
        match self {
            Self::Impact => "Impact",
            Self::Delivery => "Delivery",
            Self::Evidence => "Evidence",
        }
    }

    pub(in crate::board) fn next(self) -> Self {
        match self {
            Self::Impact => Self::Delivery,
            Self::Delivery => Self::Evidence,
            Self::Evidence => Self::Impact,
        }
    }
}

pub(super) const METHOD: &str =
    "Method: saved = context each message would have reread if delivered alone";

/// The selected row and the identities it stands for.
struct Selection<'a> {
    title: Option<String>,
    all: bool,
    ids: Vec<&'a str>,
    id: Option<&'a str>,
}

fn selection<'a>(
    rows: &'a [crate::board::workspace::OpsTreeRow],
    app: &App,
    row: Option<&'a Value>,
) -> Selection<'a> {
    let selected = rows
        .iter()
        .find(|row| Some(&row.target) == app.workspace.selected.as_ref());
    Selection {
        title: selected.map(|row| row.title.clone()),
        all: selected.is_some_and(|row| metrics::id(row).is_none()),
        ids: selected.map_or_else(Vec::new, |row| metrics::identities(row, rows)),
        id: row.and_then(|row| row["id"].as_str()),
    }
}

fn zone(app: &App) -> jiff::tz::TimeZone {
    app.local_zone.clone().unwrap_or(jiff::tz::TimeZone::UTC)
}

/// The pane title: what the tab describes and over which window.
pub(super) fn title(app: &App) -> String {
    let rows = app.workspace_rows();
    let name = rows
        .iter()
        .find(|row| Some(&row.target) == app.workspace.selected.as_ref())
        .map_or("none".into(), |row| row.title.clone());
    format!("Details · {name} · {}", app.digest_window().label())
}

/// The amounts: usage, then the saved estimate.
pub(super) fn impact(app: &App, row: Option<&Value>) -> Vec<String> {
    let rows = app.workspace_rows();
    let selected = selection(&rows, app, row);
    let report = metrics::report(app);
    let reading = metrics::tokens(report, &selected.ids);
    let mut text = vec!["USAGE".to_owned()];
    if selected.title.is_none() {
        text.push("Select a visible agent or all".into());
    }
    // Without the supplier the saved amounts below are unavailable; say how to get them.
    if app.labels.digest().is_none()
        && selected
            .id
            .is_none_or(|id| model::labels(&app.labels, id).is_empty())
    {
        text.push("Digest not available".into());
        text.push("Install: tmt extension install digest".into());
    }
    text.push(format!(
        "TOK used {} · {}",
        metrics::token_text(reading),
        match reading {
            Some((_, true)) => format!("{} agents · partial", selected.ids.len()),
            Some(_) => format!("{} agents", selected.ids.len()),
            None => "usage unavailable".into(),
        }
    ));
    text.push(format!(
        "≈ $ used {}",
        metrics::input_price(app, report, &selected.ids).map_or_else(
            || "— usage incomplete".into(),
            crate::board::savings::usd_text
        )
    ));
    if !super::table::price_column_shown(app) {
        text.push(super::table::PRICE_NOTE.into());
    }
    if selected.all {
        text.push(format!("Held now {}", super::held_text(app)));
    } else if let Some(id) = selected.id {
        let held = app
            .labels
            .digest()
            .and_then(|(status, _)| status.members.get(id)?.held_count);
        text.push(format!(
            "Held now {}",
            held.map_or("unavailable".into(), |n| crate::source::tokens(u128::from(
                n
            )))
        ));
        text.push(super::talks(app).busiest(id).map_or(
            "Talks to unavailable".into(),
            |(recipient, count)| {
                format!(
                    "Talks to {} · {} messages",
                    metrics::name(app, recipient),
                    crate::source::tokens(u128::from(count))
                )
            },
        ));
    }
    match app.usage_report.view_for(app.digest_range()) {
        View::Loading => text.push("Usage loading…".into()),
        View::Failed(error) => text.push(format!("Usage unavailable: {error}")),
        View::Report(_) => {}
    }
    if let Some(error) = app.usage_report.refresh_error(app.digest_range()) {
        text.push(format!("Stale usage retained · refresh failed: {error}"));
    }
    text.push(String::new());
    text.push("SAVED · estimate".into());
    // The all row reads the sum its tiles and table row share; a member reads its own.
    let summary = if selected.all {
        super::savings(app)
    } else {
        std::rc::Rc::new(metrics::savings(app, &selected.ids))
    };
    // The same labels as the Agents surfaces, so one estimate reads one way everywhere.
    text.extend(super::summary::fields(
        &summary,
        ["Est. ctx avoided", "List-price equivalent:"],
    ));
    if let Some(points) = summary.percent.value {
        text.push(format!(
            "Estimated input reduction: {} ({}/{} agents)",
            model::detail_percent(points),
            summary.percent.included,
            summary.total
        ));
    } else if summary.percent.conflict {
        text.push("Estimated input reduction: — · intervals differ".into());
    }
    text.push(super::summary::DEFINITION.into());
    text
}

/// What the supplier holds now: modes, held messages and its own clocks.
pub(super) fn delivery(app: &App, now_ms: u64, row: Option<&Value>) -> Vec<String> {
    let rows = app.workspace_rows();
    let selected = selection(&rows, app, row);
    let zone = zone(app);
    let Some((status, stale)) = app.labels.digest() else {
        // Valid supplier labels keep their priority and chooser even without typed stats.
        return selected.id.map_or_else(
            || vec!["Settings and statistics unavailable".into()],
            |id| {
                model::delivery(&app.labels, id, now_ms, &zone, |id| {
                    metrics::name(app, Some(id))
                })
            },
        );
    };
    if let Some(id) = selected.id {
        return model::delivery(&app.labels, id, now_ms, &zone, |id| {
            metrics::name(app, Some(id))
        });
    }
    if !selected.all {
        return vec!["Select a visible agent or all".into()];
    }
    let mut text = Vec::new();
    if let Some(global) = &status.global {
        text.push(global.text.clone());
    }
    if stale {
        text.push("Stale observation · retained values; refresh before acting".into());
    }
    text.push(format!("Held now {}", super::held_text(app)));
    let (mut auto, mut timed, mut off, mut unknown) = (0, 0, 0, 0);
    for id in &selected.ids {
        match status.members.get(*id).map(|member| member.mode) {
            Some(crate::labels::digest::Mode::Auto) => auto += 1,
            Some(crate::labels::digest::Mode::Interval) => timed += 1,
            Some(crate::labels::digest::Mode::Off) => off += 1,
            None => unknown += 1,
        }
    }
    let mut modes = format!("Modes: Auto {auto} · Every {timed} · Off {off}");
    if unknown > 0 {
        modes.push_str(&format!(" · {unknown} not reported"));
    }
    text.push(modes);
    if let Some(notice) = model::tick_notice(&app.labels) {
        text.push(notice);
    }
    if let Some(tick) = status.tick {
        text.push(format!(
            "Last tick: {}",
            model::observation_time(tick.last_at_ms, now_ms, &zone)
        ));
    }
    text.push(format!(
        "Status observed: {}",
        model::observation_time(Some(status.observed_at_ms), now_ms, &zone)
    ));
    text
}

/// The basis behind the figures: source windows, observation times, prices and method.
pub(super) fn evidence(app: &App, now_ms: u64, row: Option<&Value>) -> Vec<String> {
    let rows = app.workspace_rows();
    let selected = selection(&rows, app, row);
    let report = metrics::report(app);
    let zone = zone(app);
    let mut text = Vec::new();
    if let Some(id) = selected.id {
        if let Some(member) = app
            .labels
            .digest()
            .and_then(|(status, _)| status.members.get(id))
        {
            text.extend(model::evidence(member));
        }
        if let Some(seat) = report.and_then(|report| report.seat(id)) {
            text.push(format!(
                "Usage coverage: {}% of {}{}",
                seat.covered_ms.saturating_mul(100) / seat.window_ms.max(1),
                app.digest_range().argument(),
                if seat.complete {
                    " · complete"
                } else {
                    " · partial"
                }
            ));
            let timestamp = |ms: u64, format: &str| {
                i64::try_from(ms)
                    .ok()
                    .and_then(|ms| jiff::Timestamp::from_millisecond(ms).ok())
                    .map_or("unknown".into(), |at| {
                        at.to_zoned(zone.clone()).strftime(format).to_string()
                    })
            };
            text.push(format!(
                "Usage window: {}–{}",
                timestamp(seat.from_ms, "%m-%d %H:%M"),
                timestamp(seat.to_ms, "%m-%d %H:%M")
            ));
            text.push(format!(
                "Usage observed: {}",
                model::observation_time(Some(seat.observed_at_ms), now_ms, &zone)
            ));
            if seat.series.iter().any(|point| !point.complete) {
                text.push("Hourly activity is partial; fills show observed lower bounds".into());
            }
            if let Some(point) = report.and_then(|report| metrics::busiest(report, id)) {
                text.push(format!(
                    "Busiest hour: {}–{} · {}{} TOK",
                    timestamp(point.from_ms, "%H:%M"),
                    timestamp(point.to_ms, "%H:%M"),
                    if point.complete { "" } else { "≥" },
                    crate::source::tokens(u128::from(point.tokens))
                ));
            }
        } else if let Some(unavailable) = report.and_then(|report| {
            report
                .unavailable
                .iter()
                .find(|row| row.who.id.as_deref() == Some(id))
        }) {
            text.push(format!("Usage unavailable: {}", unavailable.reason));
        }
    }
    if let Some(report) = report {
        if selected.id.is_none() {
            text.push(format!(
                "Latest usage observed: {}",
                model::observation_time(Some(report.observed_at_ms), now_ms, &zone)
            ));
            text.push("Totals combine matching source windows only".into());
        }
        text.extend(
            report
                .warnings
                .iter()
                .map(|warning| format!("{}: {}", warning.section, warning.message)),
        );
    }
    if let Some((status, _)) = app.labels.digest() {
        text.extend(model::pricing_details(status));
    }
    text.push(METHOD.into());
    text
}

/// The distinct price-table source addresses, in table order: what `y` copies.
pub(super) fn links(app: &App) -> Vec<String> {
    let mut links: Vec<String> = Vec::new();
    for model in app
        .labels
        .digest()
        .and_then(|(status, _)| status.pricing.as_ref())
        .into_iter()
        .flat_map(|pricing| &pricing.models)
    {
        if !links.contains(&model.source) {
            links.push(model.source.clone());
        }
    }
    links
}

/// The lines of the shown tab, before wrapping.
pub(super) fn lines(app: &App, now_ms: u64, row: Option<&Value>) -> Vec<String> {
    match app.digest_details {
        Tab::Impact => impact(app, row),
        Tab::Delivery => delivery(app, now_ms, row),
        Tab::Evidence => evidence(app, now_ms, row),
    }
}

const FILE: &str = "digest.estimate-basis.xml";
const MARKUP: &str = r#"<tmt-view version="1"><tmt-modal id="estimate-basis" title="Estimate basis · Evidence" placement="body"><tmt-scroll id="estimate-basis-body"><tmt-repeat each="$.lines" as="line"><tmt-text bind="line.text" token="text" wrap="true"/></tmt-repeat></tmt-scroll><tmt-text slot="footer" token="muted" bind="$.footer"/></tmt-modal></tmt-view>"#;
const FOOTER: &str = "↑↓ scroll · y copy links · Esc close";
struct Data;
impl Sources for Data {
    type Source = ();
    fn compile(&self, _: &str, _: &str, _: &Schemas<'_>) -> Result<(), String> {
        Err("the estimate basis has no field sources".into())
    }
    fn resolve(&self, _: &(), _: &Scopes<'_>) -> Result<Option<String>, String> {
        Err("the estimate basis has no field sources".into())
    }
}
fn template() -> &'static surface::Template<()> {
    static TEMPLATE: OnceLock<surface::Template<()>> = OnceLock::new();
    TEMPLATE.get_or_init(|| {
        let parsed = tmt_tui::parse(FILE, MARKUP).expect("embedded estimate basis markup");
        let schema = Schema::Object(BTreeMap::from([
            (
                "lines".into(),
                Schema::Collection(Box::new(Schema::Object(BTreeMap::from([(
                    "text".into(),
                    Schema::Scalar,
                )])))),
            ),
            ("footer".into(), Schema::Scalar),
        ]));
        surface::compile(FILE, &parsed, &schema, &Data).expect("embedded estimate basis schema")
    })
}

/// The read-only Estimate basis modal over the held Evidence lines of the selected row.
#[derive(Default)]
pub(in crate::board) struct Basis {
    scroll: ScrollState,
    scene: Option<(Value, ModalSurface)>,
}

impl Basis {
    pub fn input(&mut self, event: &Event) {
        if self.scroll.viewport().height > 0 {
            self.scroll.input(event);
        }
    }

    pub fn invalidate(&mut self) {
        self.scroll
            .update(Rect::default(), self.scroll.content(), None);
    }

    /// The wrapped text admitted to the last modal viewport, for clock-only repaints.
    pub fn clock_mark(&self, app: &App, now: u64) -> String {
        let area = self.scroll.viewport();
        if area.height == 0 {
            return String::new();
        }
        evidence(app, now, app.selected_row())
            .into_iter()
            .flat_map(|line| crate::board::notes::wrap(&line, usize::from(area.width)))
            .skip(self.scroll.offset())
            .take(usize::from(area.height))
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn render(&mut self, frame: &mut Frame, app: &App, body: Rect) {
        let lines = evidence(app, crate::status::now_ms(), app.selected_row());
        let value = json!({
            "lines": lines.into_iter().map(|text| json!({"text": text})).collect::<Vec<_>>(),
            "footer": FOOTER,
        });
        if self.scene.as_ref().is_none_or(|(old, _)| old != &value) {
            self.scene = Some((
                value.clone(),
                template()
                    .materialize(FILE, &value, &Data)
                    .expect("typed estimate basis"),
            ));
        }
        let look = app.look();
        let outer = surface::render(
            &self.scene.as_ref().unwrap().1,
            &mut self.scroll,
            body,
            frame.buffer_mut(),
            RenderStyle {
                theme: &look.theme,
                depth: look.depth,
            },
            |role| look.role(role),
        )
        .expect("admitted estimate basis surface")
        .areas
        .outer;
        app.peek_areas
            .set(crate::board::app::Overlay::EstimateBasis, Some(outer));
        if self.scroll.viewport().height > 0 {
            app.frame_clocks
                .borrow_mut()
                .push(crate::board::view::ClockSurface::EstimateBasis);
        }
    }
}

impl App {
    /// Opens the Estimate basis modal over the selected row's Evidence.
    pub(in crate::board) fn open_estimate_basis(&mut self) -> Effect {
        self.estimate_basis = Some(Default::default());
        self.invalidate_frame_geometry();
        Effect::None
    }

    /// Esc or q closes; `y` copies the price-table sources; other input scrolls.
    pub(in crate::board) fn estimate_basis_event(&mut self, event: &Event) -> Option<Effect> {
        use ratatui::crossterm::event::KeyCode;
        let Event::Key(key) = event else {
            self.estimate_basis.as_ref()?.borrow_mut().input(event);
            return Some(Effect::None);
        };
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.estimate_basis = None;
                self.invalidate_frame_geometry();
                Some(Effect::None)
            }
            KeyCode::Char('y') if key.modifiers.is_empty() => {
                let links = links(self);
                if links.is_empty() {
                    return Some(self.say("Digest reported no price sources to copy."));
                }
                Some(Effect::Act(Request::Copy {
                    text: links.join("\n"),
                    program: self.view.as_ref().and_then(|view| view.clipboard.clone()),
                    feedback: crate::board::app::CopyFeedback::Destination,
                }))
            }
            _ => {
                self.estimate_basis.as_ref()?.borrow_mut().input(event);
                Some(Effect::None)
            }
        }
    }

    /// Shows a Details tab; its content starts at the top.
    pub(in crate::board) fn show_digest_details(&mut self, tab: Tab) -> Effect {
        self.digest_details = tab;
        self.scrolls.restore([(crate::config::Pane::Replies, 0)]);
        self.invalidate_frame_geometry();
        Effect::None
    }
}

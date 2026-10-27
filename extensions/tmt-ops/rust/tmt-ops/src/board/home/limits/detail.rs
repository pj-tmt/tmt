//! On-demand subscription evidence over the sampler's held observations.
//! The shared modal and scroll components own geometry and event behavior.
use crate::board::{
    app::{App, Effect},
    limits::{Outlook, Standing},
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

const FILE: &str = "squad.quota-details.xml";
const MARKUP: &str = r#"<tmt-view version="1"><tmt-modal id="quota-details" title="Subscriptions · reported windows" placement="body"><tmt-scroll id="quota-body"><tmt-repeat each="$.lines" as="line"><tmt-text bind="line.text" token="text" wrap="true"/></tmt-repeat></tmt-scroll><tmt-text slot="footer" token="muted" bind="$.footer"/></tmt-modal></tmt-view>"#;
struct Data;
impl Sources for Data {
    type Source = ();
    fn compile(&self, _: &str, _: &str, _: &Schemas<'_>) -> Result<(), String> {
        Err("quota details have no field sources".into())
    }
    fn resolve(&self, _: &(), _: &Scopes<'_>) -> Result<Option<String>, String> {
        Err("quota details have no field sources".into())
    }
}
fn template() -> &'static surface::Template<()> {
    static TEMPLATE: OnceLock<surface::Template<()>> = OnceLock::new();
    TEMPLATE.get_or_init(|| {
        let parsed = tmt_tui::parse(FILE, MARKUP).expect("embedded quota details markup");
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
        surface::compile(FILE, &parsed, &schema, &Data).expect("embedded quota details schema")
    })
}
/// A source timestamp as a time in the board's local zone. Its parts are joined with no-break
/// spaces, so a narrow Details wraps before a time and never inside it.
fn stamp(zone: &jiff::tz::TimeZone, at_ms: u64, now_ms: u64) -> String {
    i64::try_from(at_ms)
        .ok()
        .zip(i64::try_from(now_ms).ok())
        .and_then(|(at, now)| crate::board::local_time::time_in(at, now, zone))
        .map_or_else(|| "—".into(), |time| time.replace(' ', "\u{a0}"))
}
fn held_lines(
    standings: Option<&[Standing]>,
    now: u64,
    sampling_enabled: bool,
    zone: &jiff::tz::TimeZone,
) -> Vec<String> {
    let Some(standings) = standings else {
        return vec![if sampling_enabled {
            super::PENDING.into()
        } else {
            "Subscription sampling is disabled; enable usage sampling to acquire readings".into()
        }];
    };
    if standings.is_empty() {
        return vec!["No subscription observations reported".into()];
    }
    let mut lines = Vec::new();
    for standing in standings {
        lines.push(standing.provider.label());
        if let Some(latest) = standing.latest {
            lines.push(format!(
                "Observed: {} · {} ago",
                stamp(zone, latest.observed_at_ms, now),
                super::age(now.saturating_sub(latest.observed_at_ms))
            ));
        }
        match standing.outlook(now) {
            Outlook::Known(figures) => {
                for (window, left) in std::iter::once(("7d", figures.weekly))
                    .chain(figures.short.map(|left| ("5h", left)))
                {
                    lines.push(format!(
                        "{window}: {} remaining · reset {}",
                        super::percent(left.percent),
                        super::reset(left.resets_in_ms)
                    ));
                }
                lines.push(format!("Weekly pace: {}", super::rate_text(&figures)));
                if figures.runs_out_before_reset
                    && let Some(ms) = figures.empties_in_ms()
                {
                    lines.push(format!(
                        "Measured weekly use estimates depletion in {} before reset",
                        super::reset(ms)
                    ));
                }
            }
            Outlook::Unknown { .. } => {
                lines.push("Quota unavailable or stale; no remaining balance is inferred".into())
            }
        }
    }
    lines
}
pub(in crate::board) fn lines(app: &App, now: u64) -> Vec<String> {
    let zone = app.local_zone.clone().unwrap_or(jiff::tz::TimeZone::UTC);
    if app.shell.tab != crate::board::shell::OpsTab::Agents {
        return held_lines(
            app.limits.as_deref(),
            now,
            crate::board::home::limits_enabled(app),
            &zone,
        );
    }
    let Some(standings) = app.limits.as_deref().filter(|rows| !rows.is_empty()) else {
        return held_lines(
            app.limits.as_deref(),
            now,
            crate::board::home::limits_enabled(app),
            &zone,
        );
    };
    let mut lines = vec!["Quota pace: last 2h; trend compared with the preceding 2h. Source timestamps determine elapsed time.".into()];
    for standing in standings {
        lines.push(standing.provider.label());
        if let Some(latest) = standing.latest {
            lines.push(format!(
                "Held source observed: {} · {} ago",
                stamp(&zone, latest.observed_at_ms, now),
                super::age(now.saturating_sub(latest.observed_at_ms))
            ));
        }
        for kind in crate::board::limits::QuotaWindow::ALL {
            let value = crate::board::overview::quota::projection(
                app,
                &standing.provider.board_name(),
                kind,
                now,
            );
            let Some(balance) = value.balance else {
                lines.push(format!(
                    "{}: quota unavailable or stale; no balance or rate inferred",
                    kind.key()
                ));
                continue;
            };
            lines.push(format!(
                "{}: {} remaining · reset {} ({})",
                kind.key(),
                super::percent(balance.left),
                super::reset(balance.resets_at_ms.saturating_sub(now)),
                stamp(&zone, balance.resets_at_ms, now)
            ));
            if let Some(point) = value.pace.latest {
                lines.push(format!(
                    "Observed: {} · {} ago",
                    stamp(&zone, point.observed_at_ms, now),
                    super::age(now.saturating_sub(point.observed_at_ms))
                ));
            }
            lines.push(
                value
                    .pace
                    .per_hour
                    .map_or("2h pace: unknown".into(), |rate| {
                        format!("2h pace: {rate:.3}%/h")
                    }),
            );
            if let Some(exhausted) = value.pace.exhausts_at_ms {
                lines.push(format!(
                    "Projected depletion: {} · {}",
                    stamp(&zone, exhausted, now),
                    value
                        .pace
                        .early_ms()
                        .map_or("on pace through reset".into(), |ms| format!(
                            "{} before reset",
                            super::reset(ms)
                        ))
                ));
            }
        }
    }
    lines
}
#[derive(Default)]
pub(in crate::board) struct Details {
    pub(in crate::board) scroll: ScrollState,
    scene: Option<(Value, ModalSurface)>,
}
impl Details {
    pub fn input(&mut self, event: &Event) {
        if self.scroll.viewport().height > 0 {
            self.scroll.input(event);
        }
    }
    pub fn invalidate(&mut self) {
        self.scroll
            .update(Rect::default(), self.scroll.content(), None);
    }
    pub fn clock_mark(&self, app: &App, now: u64) -> String {
        // Compare only the wrapped text admitted to the last modal viewport.
        let area = self.scroll.viewport();
        if area.height == 0 {
            return String::new();
        }
        lines(app, now)
            .into_iter()
            .flat_map(|line| crate::board::notes::wrap(&line, usize::from(area.width)))
            .skip(self.scroll.offset())
            .take(usize::from(area.height))
            .collect::<Vec<_>>()
            .join("\n")
    }
    pub fn render(&mut self, frame: &mut Frame, app: &App, body: Rect) {
        let value = json!({"lines": lines(app, crate::status::now_ms()).into_iter().map(|text| json!({"text":text})).collect::<Vec<_>>(), "footer":"↑↓ scroll · PgUp/PgDn page · Esc close"});
        if self.scene.as_ref().is_none_or(|(old, _)| old != &value) {
            self.scene = Some((
                value.clone(),
                template()
                    .materialize(FILE, &value, &Data)
                    .expect("typed quota details"),
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
        .expect("admitted quota details surface")
        .areas
        .outer;
        app.peek_areas.set(crate::board::peek::Peek::Quota, outer);
        if self.scroll.viewport().height > 0 {
            app.frame_clocks
                .borrow_mut()
                .push(crate::board::view::ClockSurface::QuotaDetails);
        }
    }
}
impl App {
    pub(in crate::board) fn quota_details_available(&self) -> bool {
        self.workspace_snapshot().is_some()
            && matches!(
                self.shell.tab,
                crate::board::shell::OpsTab::Agents | crate::board::shell::OpsTab::Digest
            )
    }
    pub(in crate::board) fn open_quota_details(&mut self) -> Effect {
        if !self.quota_details_available() || self.loading() {
            return self
                .say("Subscription details are available in Overview and Digest after loading.");
        }
        self.quota_details = Some(Default::default());
        self.invalidate_frame_geometry();
        Effect::None
    }
}

#[cfg(test)]
mod tests;

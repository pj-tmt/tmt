//! Summary, token meter and frame invalidation text.

use crate::board::app::{App, RowTarget};
use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
};
use tmt_cli_style::Role;
use tmt_tui::components::strip;

/// Common shell counts are held snapshot evidence, independent of the active tab.
pub(super) fn render_shell(frame: &mut Frame, app: &App, area: Rect) {
    paint_shell(frame.buffer_mut(), app, area, crate::status::now_ms());
    if !area.is_empty()
        && crate::board::home::limits_enabled(app)
        && app.limits.as_ref().is_some_and(|limits| !limits.is_empty())
    {
        app.frame_clocks
            .borrow_mut()
            .push(ClockSurface::TopBar { width: area.width });
    }
}

fn paint_shell(buffer: &mut ratatui::buffer::Buffer, app: &App, area: Rect, now: u64) {
    let attention = app.workspace_snapshot().map(|snapshot| {
        let counts = &snapshot.shell_counts;
        let complete = counts.complete && snapshot.roster.complete() && app.error.is_none();
        let number = |count: usize| {
            if complete {
                count.to_string()
            } else if count > 0 {
                format!("≥{count}")
            } else {
                "?".into()
            }
        };
        crate::board::top_bar::Attention {
            online: number(counts.online),
            unread: app.unread_global(),
            waiting: number(counts.waiting),
            blocked: number(counts.blocked),
            waiting_positive: counts.waiting > 0,
            blocked_positive: counts.blocked > 0,
            complete,
            ascii: matches!(
                app.cron_glyphs,
                crate::board::overview::cron_counts::GlyphSet::Ascii
            ),
        }
    });
    let context = app.workspace.selected.as_ref().and_then(|target| {
        let crate::board::workspace::OpsTarget::Member { identity_id, .. } = target else {
            return None;
        };
        let snapshot = app.workspace_snapshot()?;
        if !snapshot.roster.contains(target) {
            return None;
        }
        let member = snapshot.roster.members.get(identity_id)?;
        let context = app.references.agents.get(identity_id)?.context?;
        Some((member.name.as_str(), context))
    });
    let observations = crate::board::top_bar::Observations {
        limits: if crate::board::home::limits_enabled(app) {
            app.limits.as_deref().unwrap_or_default()
        } else {
            &[]
        },
        context,
        now,
    };
    use crate::action::{Action, OpsUiAction};
    let key = crate::board::keycap::key(app, OpsUiAction::WorkspaceSnapshots, "w");
    let label = crate::board::keycap::label(key.as_deref(), "Snapshots", false, app.look());
    let control = crate::board::top_bar::paint_with_control(
        buffer,
        area,
        (app.view_layer.name() != "default").then(|| app.view_layer.name()),
        attention.as_ref(),
        Some(&observations),
        app.look(),
        if app.workspace_snapshot().is_some() {
            (label.width() + 2) as u16
        } else {
            0
        },
    );
    if let Some(area) = control {
        let label_text = label.to_string();
        let enabled = !app.loading() && !app.modal_open() && app.input.is_none() && !app.searching;
        let painted = crate::board::buttons::paint(
            app,
            buffer,
            area,
            &[crate::board::buttons::Button::secondary(&label_text)
                .idle(app.look().role(Role::Muted))
                .enabled(enabled)],
            tmt_cli_style::grid::Align::Left,
            None,
        );
        if let Some(rect) = painted[0] {
            if let Some(key) = key.as_deref() {
                strip::paint_left(
                    buffer,
                    Rect::new(rect.x + 1, rect.y, rect.width - 2, 1),
                    Line::from(crate::board::keycap::span(key, false, app.look())),
                );
            }
            if enabled {
                app.workspace_strip_hits.borrow_mut().push((
                    rect,
                    crate::board::app::Choice::Action(
                        Action::parse(OpsUiAction::WorkspaceSnapshots.spec().name)
                            .expect("registered snapshots action"),
                    ),
                ));
            }
        }
    }
}

/// Scope and hidden-room reveal share the shell's existing boundary band.
/// Names are admitted whole; shrinking the terminal never paints a partial name.
pub(super) fn render_visibility(frame: &mut Frame, app: &App, area: Rect) {
    use crate::{
        action::{Action, OpsUiAction, Verb},
        board::app::Choice,
    };
    if area.is_empty() {
        return;
    }
    let Some(snapshot) = app.workspace_snapshot() else {
        return;
    };
    let scope = &app.workspace.visibility.scope;
    let action = if !scope.is_empty() {
        OpsUiAction::ClearScope
    } else {
        if snapshot.hidden_count == 0 {
            return;
        }
        OpsUiAction::RevealHidden
    };
    let action = Action::parse(action.spec().name).expect("visibility action");
    let key = app
        .workspace_bindings()
        .into_iter()
        .find(|(_, bound)| bound.verb == action.verb)
        .map(|(key, _)| key);
    let hint = key.as_deref().map(|key| {
        format!(
            "{} {}",
            super::footer::key_label(key),
            super::footer::hint_word(app, &action)
        )
    });
    let suffix = if matches!(action.verb, Verb::Ui(OpsUiAction::ClearScope)) {
        hint.map_or_else(|| " · clear".into(), |hint| format!(" · {hint}"))
    } else if app.workspace.visibility.reveal_hidden {
        " · shown".into()
    } else {
        " · show".into()
    };
    let width = usize::from(area.width.saturating_sub(2));
    let line = if scope.is_empty() {
        let look = app.look();
        let counts = &snapshot.hidden_attention;
        let mut spans = vec![Span::styled(
            format!("Hidden: {}", snapshot.hidden_count),
            look.role(Role::Muted),
        )];
        if !app.workspace.visibility.reveal_hidden {
            let ascii = matches!(
                app.cron_glyphs,
                crate::board::overview::cron_counts::GlyphSet::Ascii
            );
            for (count, glyph, word, role) in [
                (
                    counts.waiting,
                    if ascii { "!" } else { "◆" },
                    "waiting",
                    Role::Waiting,
                ),
                (
                    counts.blocked,
                    if ascii { "x" } else { "✗" },
                    "blocked",
                    Role::Blocked,
                ),
            ] {
                if count > 0 {
                    spans.push(Span::styled(" · ", look.role(Role::Muted)));
                    spans.push(Span::styled(
                        format!(
                            "{glyph} {}{count} {word}",
                            if counts.complete && app.error.is_none() {
                                ""
                            } else {
                                "≥"
                            }
                        ),
                        look.role(role),
                    ));
                }
            }
        }
        spans.push(Span::styled(suffix, look.role(Role::Muted)));
        Line::from(spans)
    } else {
        let mut names = Vec::new();
        for (index, id) in scope.iter().enumerate() {
            let name = snapshot
                .roster
                .squads
                .iter()
                .find(|squad| &squad.room_id == id)
                .map_or_else(
                    || "(unavailable)".to_owned(),
                    |squad| crate::board::workspace::render::safe(&squad.name),
                );
            let mut candidate = names.clone();
            candidate.push(name);
            let rest = scope.len() - index - 1;
            let extra = if rest == 0 {
                String::new()
            } else {
                format!(" +{rest}")
            };
            let text = format!("Scope: {}{extra}{suffix}", candidate.join(","));
            if unicode_width::UnicodeWidthStr::width(text.as_str()) > width {
                break;
            }
            names = candidate;
        }
        let remaining = scope.len() - names.len();
        let extra = if remaining == 0 {
            String::new()
        } else {
            format!(" +{remaining}")
        };
        Line::styled(
            format!("Scope: {}{extra}{suffix}", names.join(",")),
            app.look().role(Role::Muted),
        )
    };
    if line.width() > width {
        return;
    }
    let rect = Rect::new(area.x + 1, area.y, line.width() as u16, 1);
    strip::paint_left(frame.buffer_mut(), rect, line);
    if matches!(action.verb, Verb::Ui(OpsUiAction::ClearScope))
        || !app.workspace.visibility.reveal_hidden
    {
        app.workspace_strip_hits
            .borrow_mut()
            .push((rect, Choice::Action(action)));
    }
}

/// Shown only when a switch takes long enough to notice.
pub(in crate::board) const SPINNER_DELAY: std::time::Duration =
    std::time::Duration::from_millis(100);
pub(super) const SPINNER_TICK: std::time::Duration = std::time::Duration::from_millis(80);
pub(super) const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Provider feedback shares the shell spinner's frames and cadence.
pub(in crate::board) fn feedback_glyph(now_ms: u64, motion: bool) -> &'static str {
    if motion {
        SPINNER[((now_ms / SPINNER_TICK.as_millis() as u64) % SPINNER.len() as u64) as usize]
    } else {
        "⠹"
    }
}

pub(in crate::board) fn feedback_wait(app: &App, now_ms: u64) -> Option<std::time::Duration> {
    app.frame_clocks.borrow().iter().any(|surface| {
        matches!(surface, ClockSurface::OutgoingFeedback(clock) if clock.animates(now_ms))
    }).then(|| SPINNER_TICK - std::time::Duration::from_millis(now_ms % SPINNER_TICK.as_millis() as u64))
}

pub(in crate::board) fn spinner_frame(app: &App, now: std::time::Instant) -> Option<usize> {
    let elapsed = now.checked_duration_since(app.loading_since?)?;
    (elapsed >= SPINNER_DELAY).then(|| {
        ((elapsed - SPINNER_DELAY).as_millis() / SPINNER_TICK.as_millis()) as usize % SPINNER.len()
    })
}

pub(in crate::board) fn spinner_wait(
    app: &App,
    now: std::time::Instant,
) -> Option<std::time::Duration> {
    let elapsed = now.checked_duration_since(app.loading_since?)?;
    Some(if elapsed < SPINNER_DELAY {
        SPINNER_DELAY - elapsed
    } else {
        SPINNER_TICK
            - std::time::Duration::from_millis(
                ((elapsed - SPINNER_DELAY).as_millis() % SPINNER_TICK.as_millis()) as u64,
            )
    })
}

/// Clock-dependent surfaces actually admitted to the last frame. Stable target
/// keys avoid rebuilding/filtering the whole roster on every UI clock tick.
pub(in crate::board) enum ClockSurface {
    TopBar {
        width: u16,
    },
    Labels {
        target: RowTarget,
        width: usize,
    },
    Limits {
        width: u16,
        rows: usize,
    },
    QuotaDetails,
    DigestDetails {
        target: RowTarget,
        width: u16,
        height: u16,
    },
    UsageChart,
    OverviewUsageQuota {
        width: u16,
    },
    /// Activity cells and counters turn stale at this instant.
    ActivityStale(std::time::Instant),
    /// A painted activity age counts from this core time.
    ActivityAge(u64),
    OutgoingFeedback(crate::board::conversation::FeedbackClock),
    /// A delayed state (R1) changes at this core time: the sending row, or the
    /// conversation's loading line.
    DelayGate {
        until_ms: u64,
    },
}

impl ClockSurface {
    /// Register the repaint wake for a delayed state, once per instant.
    pub(in crate::board) fn admit_delay_gate(app: &App, until_ms: u64) {
        let mut clocks = app.frame_clocks.borrow_mut();
        if !clocks
            .iter()
            .any(|held| matches!(held, Self::DelayGate { until_ms: known } if *known == until_ms))
        {
            clocks.push(Self::DelayGate { until_ms });
        }
    }

    fn mark(&self, app: &App, now: u64) -> String {
        match self {
            Self::TopBar { width } => {
                let area = Rect::new(0, 0, *width, 1);
                let mut buffer = ratatui::buffer::Buffer::empty(area);
                paint_shell(&mut buffer, app, area, now);
                format!("{:?}", buffer.content)
            }
            Self::Labels { target, width } => {
                let Some(row) = app.target_row(target) else {
                    return String::new();
                };
                let chips = crate::board::row_chips::of(&app.labels, row, now);
                chips
                    .iter()
                    .take(crate::board::row_chips::prefix(&chips, *width))
                    .map(|chip| chip.text.as_str())
                    .collect()
            }
            Self::OverviewUsageQuota { width } => {
                super::super::overview::quota::usage_clock_mark(app, now, *width)
            }
            Self::UsageChart => super::super::overview::chart::host::clock_mark(app, now),
            Self::ActivityStale(deadline) => (std::time::Instant::now() > *deadline).to_string(),
            Self::ActivityAge(at) => crate::requests::age(now, *at),
            Self::DigestDetails {
                target,
                width,
                height,
            } => super::super::digest_dashboard::details_clock_mark(
                app, target, *width, *height, now,
            ),
            Self::OutgoingFeedback(clock) => clock.mark(now),
            Self::DelayGate { until_ms } => (now >= *until_ms).to_string(),
            Self::QuotaDetails => app
                .quota_details
                .as_ref()
                .map_or_else(String::new, |details| details.borrow().clock_mark(app, now)),
            Self::Limits { width, rows } => {
                crate::board::home::limit_clock_marks(app.limits.as_deref(), now, *width, *rows)
            }
        }
    }
}

/// No commands or source reads: compare only the visible text/style projections.
pub(in crate::board) fn time_marks(app: &App, now: u64) -> Vec<String> {
    app.frame_clocks
        .borrow()
        .iter()
        .map(|surface| surface.mark(app, now))
        .collect()
}

/// The second header line: the shown squad's summary or delayed loading indicator.
pub(super) fn summary_line(app: &App) -> Line<'_> {
    let look = app.look();
    if app.loading_since.is_some() {
        let target = app
            .current
            .as_deref()
            .map(crate::tabs::label)
            .unwrap_or("your squads");
        let frame = spinner_frame(app, std::time::Instant::now());
        return Line::from(vec![
            Span::styled(
                frame.map_or(" ", |frame| SPINNER[frame]),
                look.role(Role::Dim),
            ),
            Span::styled("  Opening ", look.role(Role::Muted)),
            Span::styled(tmt_cli_style::table::escape(target), look.role(Role::Text)),
        ]);
    }
    let Some(view) = app.view.as_ref().filter(|_| !app.loading()) else {
        return Line::default();
    };
    let lead = match view.document["squad"]["lead"]["name"].as_str() {
        Some(lead) => format!("lead {lead}"),
        None => "no lead".into(),
    };
    // A member can appear in several user sections; count people once.
    let count = view.document["sections"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|section| section["rows"].as_array().into_iter().flatten())
        .filter_map(|row| row["name"].as_str())
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    let plural = |count: usize, one: &str, many: &str| {
        format!("{count} {}", if count == 1 { one } else { many })
    };
    // The built-in tabs have one row per squad: a person leading two squads
    // is two rows on the leads tab.
    let rows = || {
        view.document["sections"][0]["rows"]
            .as_array()
            .map_or(0, Vec::len)
    };
    let summary = if app.current.as_deref() == Some(crate::board::LEADS) {
        plural(rows(), "squad lead", "squad leads")
    } else if app.current.as_deref() == Some(crate::board::ALL) {
        plural(rows(), "squad", "squads")
    } else if app
        .current
        .as_deref()
        .and_then(crate::board::tabs::user_name)
        .is_some()
    {
        plural(count, "member", "members")
    } else {
        format!("{lead} · {}", plural(count, "member", "members"))
    };
    let mut spans = vec![Span::styled(summary, look.role(Role::Text))];
    let waiting = app
        .current
        .as_ref()
        .and_then(|squad| app.attention.get(squad))
        .map_or(0, |attention| attention.waiting);
    if waiting > 0 {
        spans.push(Span::styled(
            format!(" · Needs you {waiting}"),
            look.role(Role::Text),
        ));
    }
    if view.document["olderRequestsNotShown"] == true {
        spans.push(Span::styled(
            " · older requests not shown",
            look.role(Role::Dim),
        ));
    }
    if let Some(notice) = &view.theme_notice {
        spans.push(Span::styled(
            format!(" · {notice}"),
            look.role(Role::Waiting),
        ));
    }
    if view.document["partial"] == true {
        spans.push(Span::styled(
            " · partial: failed reads",
            look.role(Role::Text),
        ));
    }
    Line::from(spans)
}

/// Both meter rows stay reserved while enabled, so gaining/losing coverage
/// does not move member rows or their hit identities.
pub(super) fn meter_enabled(app: &App) -> bool {
    !app.loading()
        && app
            .current
            .as_deref()
            .is_some_and(|tab| !crate::board::tabs::aggregate(tab))
        && app
            .meter
            .as_ref()
            .is_some_and(|meter| meter.settings.enabled)
}

pub(super) fn render_meter_status(frame: &mut Frame, app: &App, area: Rect) {
    let look = app.look();
    if meter_enabled(app)
        && app
            .meter
            .as_ref()
            .is_some_and(|meter| meter.digits().is_none())
    {
        let text = if app.view.as_ref().is_some_and(|view| view.history_pending) {
            "Updating usage…"
        } else {
            "no usage reported yet"
        };
        let width = area.width.min(text.len() as u16);
        strip::paint_left(
            frame.buffer_mut(),
            Rect {
                x: area.right() - width,
                width,
                ..area
            },
            Line::styled(text, look.role(Role::Text)),
        );
    }
}

/// Reserve the current mode first; clip the lead/attention summary if needed.
/// The normal render/diff owns all terminal writes.
pub(super) fn meter_region(
    app: &App,
    summary: Rect,
) -> Option<(Rect, crate::board::meter::Layout)> {
    if !meter_enabled(app) {
        return None;
    }
    let left = summary_line(app).width() + 2;
    let meter = app.meter.as_ref()?;
    let layout = meter
        .layout(usize::from(summary.width).saturating_sub(left))
        .or_else(|| meter.layout(usize::from(summary.width)))?;
    let area = Rect {
        x: summary.right() - layout.width as u16,
        width: layout.width as u16,
        ..summary
    };
    Some((area, layout))
}

pub(super) fn render_meter(frame: &mut Frame, app: &App, summary: Rect) {
    let Some((area, layout)) = meter_region(app, summary) else {
        return;
    };
    let look = app.look();
    let meter = app.meter.as_ref().expect("visible meter");
    let hovered = app.meter_hover.is_some();
    let selected_bar = app.meter_hover.flatten().filter(|_| layout.spark);
    let digits = meter.digits();
    let readout_width = usize::from(area.width).saturating_sub(if layout.spark { 9 } else { 0 });
    let mut spans = if let Some(readout) =
        selected_bar.and_then(|bar| meter.bar_readout(bar, std::time::Instant::now()))
    {
        vec![Span::raw(readout)]
    } else {
        let mut spans = vec![
            Span::raw(digits.as_deref().unwrap_or("–")),
            Span::styled(layout.unit, look.role(Role::Muted)),
        ];
        if let Some(label) = layout.label {
            spans.push(Span::styled(format!(" {label}"), look.role(Role::Muted)));
        }
        spans
    };
    let readout = Line::from(spans.clone()).width();
    spans.insert(
        0,
        Span::raw(" ".repeat(readout_width.saturating_sub(readout))),
    );
    if layout.spark {
        spans.push(Span::raw(" "));
        for (index, bar) in meter.sparkline().chars().enumerate() {
            spans.push(Span::styled(
                bar.to_string(),
                look.role(if selected_bar == Some(index) {
                    Role::Text
                } else {
                    Role::Muted
                }),
            ));
        }
    }
    let mut line = Line::from(spans);
    if hovered {
        line.style = look.selection();
        for span in &mut line.spans {
            span.style = look.row_span(true, span.style, false);
        }
    }
    strip::paint_right(frame.buffer_mut(), area, line);
    let mut hits = app.hits.borrow_mut();
    hits.push(crate::board::app::Hit {
        y: area.y,
        x: area.x,
        width: area.width,
        target: crate::board::app::HitTarget::Meter(None),
    });
    if layout.spark {
        for index in 0..8 {
            hits.push(crate::board::app::Hit {
                y: area.y,
                x: area.right() - 8 + index as u16,
                width: 1,
                target: crate::board::app::HitTarget::Meter(Some(index)),
            });
        }
    }
}

/// Slice bars retain their own tones on the hover background after the general
/// selected-word pass; only the pointed slice changes to the text role.
pub(super) fn finish_meter_styles(frame: &mut Frame, app: &App, summary: Rect) {
    if app.meter_hover.is_none() || app.look().selection().bg.is_none() {
        return;
    }
    let Some((area, _)) = meter_region(app, summary).filter(|(_, layout)| layout.spark) else {
        return;
    };
    let look = app.look();
    for index in 0..8 {
        let role = if app.meter_hover.flatten() == Some(index) {
            Role::Text
        } else {
            Role::Muted
        };
        frame.buffer_mut()[(area.right() - 8 + index as u16, area.y)].set_style(look.role(role));
    }
}

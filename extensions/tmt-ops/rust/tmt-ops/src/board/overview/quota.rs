//! Overview arrangement over the existing quota rules and held readings.
use crate::{
    board::{
        home::limits as rules,
        limits::{Outlook, Standing},
    },
    look::Look,
};
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};
use tmt_cli_style::{Depth, Role};
use unicode_width::UnicodeWidthStr;

/// This is the painted quota rectangle, not terminal width (Overview is 60%).
const TWO_PROVIDER_WIDTH: u16 = 90;
const VISIBLE_PROVIDERS: usize = 4;

fn fill_style(look: Look, role: Role, short: bool) -> Style {
    use tmt_cli_style::theme::screen::{self, Fill};
    let style = look.role(Role::Text).patch(screen::fill(
        &look.theme,
        role,
        look.depth,
        if short { Fill::Lighter } else { Fill::Solid },
    ));
    if style.bg.is_none() {
        style.add_modifier(Modifier::REVERSED)
    } else {
        style
    }
}

fn bar(look: Look, window: &str, percent: f64, width: usize) -> Vec<Span<'static>> {
    let label = crate::board::view::fit(&format!(" {window} {}", rules::percent(percent)), width);
    let role = rules::remaining_role(percent);
    rules::balance_parts(&label, percent, width, role)
        .into_iter()
        .map(|piece| {
            let id = piece["id"].as_str().unwrap();
            let style = if id == "filled" {
                fill_style(look, role, window == "5h")
            } else if look.depth == Depth::None {
                look.role(Role::Text).add_modifier(Modifier::UNDERLINED)
            } else {
                look.role(Role::Text).patch(look.role(Role::Track))
            };
            Span::styled(piece["text"].as_str().unwrap().to_owned(), style)
        })
        .collect()
}
fn provider(standing: &Standing, now: u64, width: u16, look: Look, wide: bool) -> Line<'static> {
    let name = crate::board::notes::sanitize(&standing.provider.label());
    let Outlook::Known(figures) = standing.outlook(now) else {
        let Outlook::Unknown { age_ms } = standing.outlook(now) else {
            unreachable!()
        };
        let reading = age_ms.map_or_else(
            || "no reading yet".into(),
            |age| format!("no reading for {}", rules::age(age)),
        );
        return Line::styled(
            crate::board::view::fit(&format!("{name}  {reading}"), usize::from(width)),
            look.role(Role::Muted),
        );
    };
    let warning = rules::warning(&figures);
    let name_width = UnicodeWidthStr::width(name.as_str()).min(8);
    let available = usize::from(width)
        .saturating_sub(name_width + 1 + UnicodeWidthStr::width(warning.as_str()));
    let count = if figures.short.is_some() { 2 } else { 1 };
    let cells = available.saturating_sub(count - 1) / count;
    let cells = cells.min(if wide { 12 } else { 16 });
    let mut spans = vec![
        Span::styled(
            crate::board::view::fit(&name, name_width),
            look.role(Role::Text),
        ),
        Span::raw(" "),
    ];
    spans.extend(bar(look, "7d", figures.weekly.percent, cells));
    if let Some(short) = figures.short {
        spans.push(Span::raw(" "));
        spans.extend(bar(look, "5h", short.percent, cells));
    }
    spans.push(Span::styled(
        warning,
        look.role(if figures.empties_soon() {
            Role::Blocked
        } else {
            Role::Waiting
        }),
    ));
    let used: usize = spans.iter().map(Span::width).sum();
    if !wide && figures.runs_out_before_reset {
        let extra = format!(
            " · reset {} · {}",
            rules::reset(figures.weekly.resets_in_ms),
            rules::rate_text(&figures)
        );
        if used + UnicodeWidthStr::width(extra.as_str()) <= usize::from(width) {
            spans.push(Span::styled(extra, look.role(Role::Muted)));
        }
    }
    Line::from(spans)
}

pub(super) fn lines(
    standings: Option<&[Standing]>,
    now: u64,
    width: u16,
    look: Look,
) -> Vec<Line<'static>> {
    let Some(standings) = standings else {
        return vec![Line::styled(rules::PENDING, look.role(Role::Muted))];
    };
    let wide = width >= TWO_PROVIDER_WIDTH;
    let cell_width = if wide { (width - 2) / 2 } else { width };
    let cells: Vec<_> = standings
        .iter()
        .take(VISIBLE_PROVIDERS)
        .map(|s| provider(s, now, cell_width, look, wide))
        .collect();
    let mut rows = Vec::new();
    for chunk in cells.chunks(if wide { 2 } else { 1 }) {
        let mut spans = chunk[0].spans.clone();
        if chunk.len() == 2 {
            let used: usize = spans.iter().map(Span::width).sum();
            spans.push(Span::raw(
                " ".repeat(usize::from(cell_width).saturating_sub(used) + 2),
            ));
            spans.extend(chunk[1].spans.clone());
        }
        rows.push(Line::from(spans));
    }
    let hidden = &standings[standings.len().min(VISIBLE_PROVIDERS)..];
    if !hidden.is_empty() {
        let mut low = 0;
        let mut unknown = 0;
        let mut role = Role::Muted;
        let mut alert = Role::Muted;
        for s in hidden {
            match s.outlook(now) {
                Outlook::Known(figures) => {
                    let remaining = rules::remaining_role(figures.weekly.percent);
                    if role == Role::Muted {
                        role = remaining;
                    }
                    if remaining == Role::Blocked {
                        role = Role::Blocked;
                    } else if remaining == Role::Waiting && role != Role::Blocked {
                        role = Role::Waiting;
                    }
                    if remaining == Role::Blocked
                        || remaining == Role::Waiting
                        || figures.runs_out_before_reset
                    {
                        low += 1;
                        if remaining == Role::Blocked || figures.empties_soon() {
                            alert = Role::Blocked;
                        } else if alert != Role::Blocked {
                            alert = Role::Waiting;
                        }
                    }
                }
                Outlook::Unknown { .. } => unknown += 1,
            }
        }
        let mark = match alert {
            Role::Blocked => "✗ ",
            Role::Waiting => "! ",
            _ => "",
        };
        rows.push(
            Line::from(vec![
                Span::styled(format!("+{} more ↓", hidden.len()), look.role(role)),
                Span::styled(format!(" · {mark}{low} low/depleting"), look.role(alert)),
                Span::styled(format!(" · {unknown} unknown"), look.role(Role::Muted)),
            ])
            .style(look.role(role)),
        );
    }
    rows
}

/// A provider's held account windows on its Usage row. Validation and reset/age
/// fences remain in Standing::outlook; no source is acquired here.
pub(in crate::board) fn usage_row(
    standings: Option<&[Standing]>,
    provider: &str,
    now: u64,
    width: u16,
    look: Look,
) -> Line<'static> {
    let figures = standings
        .and_then(|items| items.iter().find(|s| s.provider.id() == provider))
        .and_then(|s| match s.outlook(now) {
            Outlook::Known(figures) => Some(figures),
            _ => None,
        });
    let mut spans = usage_window("7d", figures.map(|f| f.weekly.percent), width, look);
    if let Some(short) = figures.and_then(|f| f.short) {
        spans.push(Span::raw("  "));
        let mut tail = usage_window("5h", Some(short.percent), width, look);
        spans.append(&mut tail);
    } else if let Some(figures) = figures {
        spans.push(Span::styled(
            format!("  resets {}", rules::reset(figures.weekly.resets_in_ms)),
            look.role(Role::Muted),
        ));
    } else {
        spans.push(Span::styled("  5h   —", look.role(Role::Muted)));
    }
    let line = Line::from(spans);
    if line.width() <= usize::from(width) {
        line
    } else {
        Line::styled(
            crate::board::view::fit(&line.to_string(), usize::from(width)),
            look.role(Role::Muted),
        )
    }
}
fn usage_window(name: &str, percent: Option<f64>, width: u16, look: Look) -> Vec<Span<'static>> {
    let mut spans = vec![Span::styled(format!("{name} "), look.role(Role::Muted))];
    if width >= 36 {
        let filled = percent.map_or(0, |value| (value / 10.0).round().clamp(0.0, 10.0) as usize);
        let bar = if percent.is_some() {
            format!("{}{}", "█".repeat(filled), "░".repeat(10 - filled))
        } else {
            "·".repeat(10)
        };
        spans.push(Span::styled(bar, look.role(Role::Muted)));
    }
    spans.push(Span::styled(
        format!("{:>4}", percent.map_or("—".into(), rules::percent)),
        look.role(Role::Text),
    ));
    spans
}

pub(in crate::board) fn usage_clock_mark(
    standings: Option<&[Standing]>,
    now: u64,
    width: u16,
    look: Look,
) -> String {
    let mut providers: Vec<_> = standings
        .into_iter()
        .flatten()
        .map(|standing| standing.provider.id())
        .collect();
    providers.sort_unstable();
    providers.dedup();
    format!(
        "{:?}",
        providers
            .into_iter()
            .map(|provider| usage_row(standings, provider, now, width, look))
            .collect::<Vec<_>>()
    )
}

/// Compare only the text and styles of the same rows admitted by the painter.
pub(in crate::board) fn clock_mark(
    standings: Option<&[Standing]>,
    now: u64,
    width: u16,
    rows: usize,
    look: Look,
) -> String {
    format!(
        "{:?}",
        lines(standings, now, width, look)
            .into_iter()
            .take(rows)
            .collect::<Vec<_>>()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn usage_windows_share_rows_neutral_roles_and_the_existing_time_fences() {
        use crate::board::limits::{Provider, Reading, Window};
        let now = 1_000_000;
        let look = Look::new(tmt_cli_style::Theme::default());
        let mut standings = vec![Standing {
            provider: Provider::named("claude"),
            latest: Some(Reading {
                observed_at_ms: now,
                weekly: Window {
                    left: 82.0,
                    resets_at_ms: now + 86_400_000,
                },
                short: Some(Window {
                    left: 98.0,
                    resets_at_ms: now + 3_600_000,
                }),
            }),
            burn: None,
            trend: None,
        }];
        let known = usage_row(Some(&standings), "claude", now, 36, look);
        assert_eq!(known.to_string(), "7d ████████░░ 82%  5h ██████████ 98%");
        assert_eq!(known.width(), 36);
        assert!(
            known
                .spans
                .iter()
                .filter(|span| !span.content.trim().is_empty())
                .all(|span| span.style == look.role(Role::Text)
                    || span.style == look.role(Role::Muted))
        );
        assert_eq!(
            usage_row(Some(&standings), "claude", now, 16, look).to_string(),
            "7d  82%  5h  98%"
        );
        standings[0].latest.as_mut().unwrap().weekly.left = 0.0;
        assert!(
            usage_row(Some(&standings), "claude", now, 36, look)
                .to_string()
                .starts_with("7d ░░░░░░░░░░  0%")
        );
        standings[0].latest.as_mut().unwrap().weekly.left = 100.0;
        standings[0]
            .latest
            .as_mut()
            .unwrap()
            .short
            .as_mut()
            .unwrap()
            .left = 100.0;
        let full = usage_row(Some(&standings), "claude", now, 36, look);
        assert_eq!(full.width(), 36);
        assert!(full.to_string().ends_with("100%"));
        for time in [now + 86_400_000, now + 7_200_001] {
            let stale = usage_row(Some(&standings), "claude", time, 36, look).to_string();
            assert!(stale.contains("—") && !stale.contains("82%") && !stale.contains("0%"));
        }
        let missing = usage_row(None, "codex", now, 36, look).to_string();
        assert!(missing.contains("—") && !missing.contains('%'));
        standings[0].latest.as_mut().unwrap().short = None;
        let reset = usage_row(Some(&standings), "claude", now, 36, look).to_string();
        assert!(reset.contains("resets 1d00h") && !reset.contains("5h"));
    }
    #[test]
    fn provider_width_boundary_and_unknowns_preserve_order() {
        let providers: Vec<_> = ["claude", "codex", "gemini"]
            .into_iter()
            .map(|name| Standing {
                provider: crate::board::limits::Provider::named(name),
                latest: None,
                burn: None,
                trend: None,
            })
            .collect();
        for width in [60, 80, 89, 90, 91, 100, 140, 160] {
            let lines = lines(
                Some(&providers),
                1,
                width,
                Look::new(tmt_cli_style::Theme::default()),
            );
            assert_eq!(lines.len(), if width >= 90 { 2 } else { 3 });
            assert!(lines.iter().all(|line| line.width() <= usize::from(width)));
            let words = lines
                .iter()
                .flat_map(|l| l.spans.iter())
                .map(|s| s.content.as_ref())
                .collect::<String>();
            assert!(words.find("Claude").unwrap() < words.find("Codex").unwrap());
            assert!(words.contains("no reading yet"));
            assert!(!words.contains("0%"));
        }
    }
    #[test]
    fn both_window_fills_use_the_shared_role_owner_at_every_depth() {
        use tmt_cli_style::theme::screen::{self, Fill};
        for depth in [Depth::TrueColor, Depth::Ansi16, Depth::None] {
            let look = Look {
                theme: tmt_cli_style::Theme::default(),
                depth,
            };
            for role in [Role::Working, Role::Waiting, Role::Blocked] {
                for short in [false, true] {
                    let mut expected = look.role(Role::Text).patch(screen::fill(
                        &look.theme,
                        role,
                        depth,
                        if short { Fill::Lighter } else { Fill::Solid },
                    ));
                    if depth == Depth::None {
                        expected = expected.add_modifier(Modifier::REVERSED);
                    }
                    assert_eq!(fill_style(look, role, short), expected);
                }
            }
        }
    }

    fn known(name: &str, percent: f64, short: Option<f64>, now: u64) -> Standing {
        Standing {
            provider: crate::board::limits::Provider::named(name),
            latest: Some(crate::board::limits::Reading {
                observed_at_ms: now,
                weekly: crate::board::limits::Window {
                    left: percent,
                    resets_at_ms: now + 7 * 24 * 60 * 60 * 1000,
                },
                short: short.map(|left| crate::board::limits::Window {
                    left,
                    resets_at_ms: now + 5 * 60 * 60 * 1000,
                }),
            }),
            burn: Some(1.0),
            trend: Some(crate::board::limits::Trend::Up),
        }
    }
    #[test]
    fn windows_labels_and_hidden_warnings_do_not_become_zero_readings() {
        let now = 1_791_600_000_000;
        let look = Look {
            theme: tmt_cli_style::Theme::default(),
            depth: Depth::None,
        };
        let mut providers = vec![
            known("claude", 95.0, Some(98.0), now),
            known("codex", 0.5, None, now),
        ];
        providers.extend([
            known("gemini", 65.0, None, now),
            known("other", 65.0, None, now),
            known("low", 5.0, None, now),
        ]);
        providers.push(Standing {
            provider: crate::board::limits::Provider::named("unknown"),
            latest: None,
            burn: None,
            trend: None,
        });
        for width in [60, 80, 89, 90, 91, 100, 140, 160] {
            let rows = lines(Some(&providers), now, width, look);
            assert!(rows.iter().all(|line| line.width() <= usize::from(width)));
            let text = rows
                .iter()
                .flat_map(|line| line.spans.iter())
                .map(|span| span.content.as_ref())
                .collect::<String>();
            assert!(text.contains("7d 95%"));
            assert!(text.contains("5h 98%"));
            assert!(text.contains("7d <1%"));
            assert!(text.contains("+2 more ↓ · ✗ 1 low/depleting · 1 unknown"));
            assert_eq!(rows.last().unwrap().style, look.role(Role::Blocked));
        }
    }
    #[test]
    fn clock_mark_is_exact_painted_row_builder_including_clipping_and_styles() {
        let now = 1_791_600_000_000;
        let providers = vec![
            known("claude", 95.0, Some(98.0), now),
            known("codex", 55.0, None, now),
        ];
        let look = Look {
            theme: tmt_cli_style::Theme::default(),
            depth: Depth::TrueColor,
        };
        for width in [60, 100, 160] {
            for count in [0, 1, 2] {
                let painted = lines(Some(&providers), now, width, look)
                    .into_iter()
                    .take(count)
                    .collect::<Vec<_>>();
                assert_eq!(
                    clock_mark(Some(&providers), now, width, count, look),
                    format!("{painted:?}")
                );
            }
        }
        // At zero painted rows a source age transition cannot invalidate the frame.
        assert_eq!(
            clock_mark(Some(&providers), now, 60, 0, look),
            clock_mark(Some(&providers), now + 60_000, 60, 0, look)
        );
        let old = clock_mark(Some(&providers), now, 80, 2, look);
        assert_ne!(old, clock_mark(Some(&providers), now + 60_000, 80, 2, look));
    }
    #[test]
    fn provider_admission_keeps_unicode_out_and_long_ids_fit_without_partial_cells() {
        assert!(crate::board::limits::Provider::from_driver("代理").is_none());
        assert!(crate::board::limits::Provider::from_driver("e\u{301}").is_none());
        let now = 1_791_600_000_000;
        let providers = vec![known("long-provider-name-12345", 65.0, Some(80.0), now)];
        for width in [60, 80, 89, 90, 91, 100, 140, 160] {
            let rows = lines(
                Some(&providers),
                now,
                width,
                Look::new(tmt_cli_style::Theme::default()),
            );
            assert!(rows.iter().all(|line| line.width() <= usize::from(width)));
        }
    }
    #[test]
    fn collapsed_high_remaining_provider_still_discloses_urgent_depletion() {
        let now = 1_791_600_000_000;
        let mut hidden = known("fast", 95.0, None, now);
        hidden.burn = Some(20.0);
        let mut providers = vec![known("claude", 95.0, None, now); 4];
        providers.push(hidden);
        let look = Look {
            theme: tmt_cli_style::Theme::default(),
            depth: Depth::TrueColor,
        };
        let rows = lines(Some(&providers), now, 96, look);
        let more = rows.last().unwrap();
        assert_eq!(
            more.style,
            look.role(Role::Working),
            "remaining hue stays independent of the warning"
        );
        assert!(
            more.spans
                .iter()
                .any(|span| span.content.contains("✗ 1 low/depleting")
                    && span.style == look.role(Role::Blocked))
        );
    }
}

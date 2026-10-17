//! One selected account window and fixed-horizon pace on shared Usage rows.
use crate::{
    board::{
        app::App,
        home::limits as rules,
        limits::{
            Outlook, QuotaWindow, Trend, Window,
            pace::{self, Pace},
        },
    },
    look::Look,
};
use ratatui::text::{Line, Span};
use tmt_cli_style::Role;

#[derive(Clone, Copy, Debug, Default)]
pub(in crate::board) struct Projection {
    pub balance: Option<Window>,
    pub pace: Pace,
}

pub(in crate::board) fn projection(
    app: &App,
    provider: &str,
    kind: QuotaWindow,
    now: u64,
) -> Projection {
    let reports: Vec<_> = app
        .view
        .as_ref()
        .into_iter()
        .flat_map(|view| &view.home_rate)
        .filter(|(_, rate)| rate.settings.enabled)
        .filter_map(|(name, _)| app.home_meter(name))
        .flat_map(|meter| meter.history().quota_reports())
        .filter(|(driver, _)| *driver == provider)
        .map(|(_, report)| report)
        .collect();
    let pace = pace::project(reports.iter().copied(), kind, now);
    let balance = if reports.is_empty() {
        app.limits
            .as_deref()
            .and_then(|items| items.iter().find(|s| s.provider.id() == provider))
            .and_then(|s| match s.outlook(now) {
                Outlook::Known(figures) => match kind {
                    QuotaWindow::Weekly => Some(figures.weekly),
                    QuotaWindow::Short => figures.short,
                },
                _ => None,
            })
            .map(|left| Window {
                left: left.percent,
                resets_at_ms: now.saturating_add(left.resets_in_ms),
            })
    } else {
        pace.latest.map(|point| point.balance())
    };
    Projection { balance, pace }
}

pub(in crate::board) fn usage_row(
    value: Projection,
    now: u64,
    width: u16,
    look: Look,
) -> Line<'static> {
    let Some(balance) = value.balance else {
        return Line::styled(
            if width >= 18 {
                "—  no limit source"
            } else {
                "—"
            },
            look.role(Role::Dim),
        );
    };
    let percent = rules::percent(balance.left);
    let filled = ((balance.left / 10.0).round().clamp(0.0, 10.0) as usize)
        .max(usize::from(balance.left > 0.0));
    let bar = format!("{}{}", "█".repeat(filled), "░".repeat(10 - filled));
    let trend = value
        .pace
        .trend
        .map(|t| match t {
            Trend::Up => " ↑",
            Trend::Down => " ↓",
            Trend::Same => " →",
        })
        .unwrap_or("");
    let rate = value
        .pace
        .per_hour
        .map_or("—".into(), |rate| format!("+{rate:.1}%/h{trend}"));
    let mut spans = Vec::new();
    let base = format!("{bar}{percent:>4}");
    if base.chars().count() + 2 + rate.chars().count() <= usize::from(width) {
        spans.push(Span::styled(bar, look.role(Role::Muted)));
        spans.push(Span::styled(format!("{percent:>4}"), look.role(Role::Text)));
    } else {
        spans.push(Span::styled(percent.clone(), look.role(Role::Text)));
    }
    let base_width = spans
        .iter()
        .map(|s| s.content.chars().count())
        .sum::<usize>();
    if base_width + 2 + rate.chars().count() > usize::from(width) {
        return Line::from(spans);
    }
    spans.push(Span::styled(format!("  {rate:<9}"), look.role(Role::Text)));
    let used = spans
        .iter()
        .map(|s| s.content.chars().count())
        .sum::<usize>();
    let remaining = usize::from(width).saturating_sub(used + 2);
    if let Some(early) = value.pace.early_ms() {
        let full = format!("runs out {} early", duration(early, true));
        let compact = format!("{} early", duration(early, false));
        let words = if full.chars().count() + 2 <= remaining {
            Some(full)
        } else if compact.chars().count() + 2 <= remaining {
            Some(compact)
        } else {
            None
        };
        if let Some(words) = words {
            spans.push(Span::raw("  "));
            spans.push(Span::styled("◆", look.role(Role::Waiting)));
            spans.push(Span::styled(format!(" {words}"), look.role(Role::Muted)));
        }
    } else if value.pace.exhausts_at_ms.is_some() {
        let full = format!(
            "on pace · resets in {}",
            duration(balance.resets_at_ms.saturating_sub(now), true)
        );
        if full.chars().count() <= remaining {
            spans.push(Span::styled(format!("  {full}"), look.role(Role::Muted)));
        } else if remaining >= 7 {
            spans.push(Span::styled("  on pace", look.role(Role::Muted)));
        }
    }
    Line::from(spans)
}

fn duration(ms: u64, two: bool) -> String {
    let minutes = ms / 60_000;
    let (days, hours) = (minutes / 1440, minutes / 60 % 24);
    if days > 0 {
        if two {
            format!("{days}d {hours}h")
        } else {
            format!("{days}d")
        }
    } else if minutes >= 60 {
        if two {
            format!("{}h {}m", minutes / 60, minutes % 60)
        } else {
            format!("{}h", minutes / 60)
        }
    } else {
        format!("{minutes}m")
    }
}

pub(in crate::board) fn usage_clock_mark(app: &App, now: u64, width: u16) -> String {
    let kind = app.usage_chart.borrow().quota_window();
    let mut providers: Vec<_> = app
        .limits
        .iter()
        .flatten()
        .map(|s| s.provider.id())
        .collect();
    providers.sort_unstable();
    providers.dedup();
    format!(
        "{:?}",
        providers
            .into_iter()
            .map(|provider| usage_row(projection(app, provider, kind, now), now, width, app.look()))
            .collect::<Vec<_>>()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::limits::observation::Point;
    use tmt_cli_style::{Depth, Theme};
    const NOW: u64 = 1_791_600_000_000;
    fn value(left: f64, rate: Option<f64>, early: Option<u64>) -> Projection {
        let reset = NOW + 6 * 86_400_000;
        Projection {
            balance: Some(Window {
                left,
                resets_at_ms: reset,
            }),
            pace: Pace {
                latest: Some(Point {
                    observed_at_ms: NOW,
                    window_minutes: 10_080,
                    used_percent: 100.0 - left,
                    resets_at_ms: reset,
                }),
                per_hour: rate,
                trend: rate.map(|_| Trend::Up),
                exhausts_at_ms: early.map(|ms| reset - ms),
            },
        }
    }
    #[test]
    fn quota_tracks_have_mock_fences_complete_numbers_and_only_the_warning_mark_coloured() {
        for depth in [Depth::TrueColor, Depth::Ansi16, Depth::None] {
            let look = Look {
                theme: Theme::default(),
                depth,
            };
            let projection = value(30.0, Some(0.6), Some(3 * 86_400_000 + 3_600_000));
            assert_eq!(
                usage_row(projection, NOW, 37, look).to_string(),
                "███░░░░░░░ 30%  +0.6%/h ↑  ◆ 3d early"
            );
            assert_eq!(
                usage_row(projection, NOW, 49, look).to_string(),
                "███░░░░░░░ 30%  +0.6%/h ↑  ◆ runs out 3d 1h early"
            );
            let styled = usage_row(projection, NOW, 37, look);
            assert!(styled.spans.iter().any(|span| span.content.contains("30%") && span.style == look.role(Role::Text)));
            assert!(styled.spans.iter().any(
                |span| span.content.contains("+0.6%/h") && span.style == look.role(Role::Text)
            ));
            for width in [4, 16, 20, 37, 49] {
                let row = usage_row(projection, NOW, width, look);
                assert!(row.width() <= usize::from(width));
                for span in row.spans {
                    assert!(
                        span.style == look.role(Role::Muted)
                            || span.style == look.role(Role::Text)
                            || span.style == Default::default()
                            || (span.content == "◆" && span.style == look.role(Role::Waiting))
                    );
                }
                for percent in [0.0, 0.5, 100.0] {
                    let row = usage_row(value(percent, Some(12345.6), None), NOW, width, look);
                    assert!(row.width() <= usize::from(width));
                    assert!(row.to_string().contains(&rules::percent(percent)));
                }
            }
        }
    }
    #[test]
    fn unavailable_windows_and_unknown_rates_do_not_invent_zero_or_warning_or_trend() {
        let look = Look::default();
        for width in [16, 37, 49] {
            let missing = usage_row(Projection::default(), NOW, width, look).to_string();
            assert!(missing.contains('—') && !missing.contains('%'));
            let row = usage_row(value(82.0, None, None), NOW, width, look).to_string();
            assert!(row.contains("82%") && row.contains('—'));
            assert!(!row.contains('◆') && !row.contains('↑'));
        }
        let mut known = value(50.0, Some(0.5), None);
        known.pace.trend = None;
        known.pace.exhausts_at_ms = Some(NOW + 8 * 86_400_000);
        let row = usage_row(known, NOW, 49, look).to_string();
        assert!(row.contains("on pace") && !row.contains('↑') && !row.contains('◆'));
    }
    #[test]
    fn held_balances_remain_available_without_claiming_an_api_backed_rate() {
        crate::status::with_now_ms(NOW, || {
            let app = crate::board::home::tests::oracle::metered(Look::default());
            let standing = &app.limits.as_ref().unwrap()[0];
            let before = projection(&app, standing.provider.id(), QuotaWindow::Weekly, NOW);
            assert!(before.balance.is_some());
            assert_eq!(before.pace.per_hour, None);
            let later = projection(
                &app,
                standing.provider.id(),
                QuotaWindow::Weekly,
                NOW + 7_200_001,
            );
            assert!(later.balance.is_none());
            assert_ne!(
                usage_clock_mark(&app, NOW, 49),
                usage_clock_mark(&app, NOW + 7_200_001, 49)
            );
        });
    }
}

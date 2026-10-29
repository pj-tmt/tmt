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
    pub notice: Option<&'static str>,
    pub pace: Pace,
}

/// `name` is a row's name on the board: a driver ID (every account of it, or, beside several
/// accounts, the sessions that name none), or `driver.key` for one account of a driver that has
/// several (#5275).
pub(in crate::board) fn projection(
    app: &App,
    name: &str,
    kind: QuotaWindow,
    now: u64,
) -> Projection {
    let (driver, account) = name
        .split_once('.')
        .map_or((name, None), |(d, k)| (d, Some(k)));
    let several = app.limits.as_deref().is_some_and(|items| {
        items
            .iter()
            .filter(|s| s.provider.id() == driver && s.provider.names_account())
            .count()
            > 0
    });
    let reports: Vec<_> = app
        .view
        .as_ref()
        .into_iter()
        .flat_map(|view| &view.home_rate)
        .filter(|(_, rate)| rate.settings.enabled)
        .filter_map(|(name, _)| app.home_meter(name))
        .flat_map(|meter| meter.history().quota_reports())
        .filter(|(reported, report)| {
            *reported == driver
                && match (account, several) {
                    (Some(key), _) => report.account.as_ref().is_some_and(|a| a.key == key),
                    // Beside several accounts the plain driver is the unnamed sessions only.
                    (None, true) => report.account.is_none(),
                    (None, false) => true,
                }
        })
        .map(|(_, report)| report)
        .collect();
    let standing = app.limits.as_deref().and_then(|items| {
        items.iter().find(|s| match account {
            Some(key) => s.provider.id() == driver && s.provider.account_key() == Some(key),
            None if several => s.provider.id() == driver && s.provider.account_key().is_none(),
            None => s.provider.id() == driver,
        })
    });
    if let Some(standing) = standing.filter(|standing| standing.current.is_some()) {
        let balance = standing.window(kind, now);
        let pace = match standing.current.as_ref() {
            Some(crate::board::limits::Current::Reported(current)) => {
                let mut admitted: Vec<_> = reports
                    .iter()
                    .copied()
                    .filter(|report| {
                        report.observed_at_ms < current.observed_at_ms
                            && report.account.as_ref().map(|account| account.key.as_str())
                                == standing.provider.account_key()
                    })
                    .collect();
                admitted.push(current);
                pace::project(admitted, kind, now)
            }
            _ => Pace::default(),
        };
        return Projection {
            balance,
            pace,
            notice: standing.notice(),
        };
    }
    let pace = pace::project(reports.iter().copied(), kind, now);
    let balance = if reports.is_empty() {
        app.limits
            .as_deref()
            .and_then(|items| {
                items.iter().find(|s| match account {
                    Some(key) => s.provider.id() == driver && s.provider.account_key() == Some(key),
                    None if several => {
                        s.provider.id() == driver && s.provider.account_key().is_none()
                    }
                    None => s.provider.id() == driver,
                })
            })
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
    Projection {
        balance,
        pace,
        notice: None,
    }
}

pub(in crate::board) fn usage_row(
    value: Projection,
    now: u64,
    width: u16,
    look: Look,
) -> Line<'static> {
    let Some(balance) = value.balance else {
        return Line::styled(
            match value.notice {
                Some(text) if text.chars().count() <= usize::from(width) => text,
                Some(text)
                    if text.split(" · ").next().unwrap_or(text).chars().count()
                        <= usize::from(width) =>
                {
                    text.split(" · ").next().unwrap_or(text)
                }
                Some(_) => "—",
                None if width >= 18 => "—  no limit source",
                None => "—",
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
        .map(|s| s.provider.board_name())
        .collect();
    providers.sort_unstable();
    providers.dedup();
    format!(
        "{:?}",
        providers
            .into_iter()
            .map(|provider| usage_row(
                projection(app, &provider, kind, now),
                now,
                width,
                app.look()
            ))
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
            notice: None,
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
    fn an_account_row_reads_its_own_balance_and_the_plain_driver_beside_several_only_the_unnamed() {
        use crate::board::limits::Provider;
        const WORK: &str = "0123456789ab";
        const HOME: &str = "ba9876543210";
        crate::status::with_now_ms(NOW, || {
            let mut app = crate::board::home::tests::oracle::metered(Look::default());
            let template = app.limits.as_ref().unwrap()[0].clone();
            let standing = |provider: Provider, left: f64| {
                let mut standing = template.clone();
                standing.provider = provider;
                standing.latest.as_mut().unwrap().weekly.left = left;
                standing
            };
            let account = |key: &str, label: &str, shown: bool| {
                Provider::from_key(&format!("newcomer.{key}"))
                    .unwrap()
                    .labelled(label)
                    .showing_account(shown)
            };
            let plain = |unknown: bool| {
                Provider::from_driver("newcomer")
                    .unwrap()
                    .showing_unknown_account(unknown)
            };
            let left = |app: &App, name: &str| {
                projection(app, name, QuotaWindow::Weekly, NOW)
                    .balance
                    .map(|balance| balance.left.round())
            };
            app.limits = Some(vec![
                standing(plain(true), 90.0),
                standing(account(WORK, "work", true), 70.0),
                standing(account(HOME, "home", true), 30.0),
            ]);
            let names: Vec<_> = app
                .limits
                .iter()
                .flatten()
                .map(|standing| standing.provider.board_name())
                .collect();
            assert_eq!(
                names,
                [
                    "newcomer".to_owned(),
                    format!("newcomer.{WORK}"),
                    format!("newcomer.{HOME}")
                ]
            );
            assert_eq!(left(&app, "newcomer"), Some(90.0), "the unnamed sessions");
            assert_eq!(left(&app, &format!("newcomer.{WORK}")), Some(70.0));
            assert_eq!(left(&app, &format!("newcomer.{HOME}")), Some(30.0));
            assert_eq!(left(&app, "newcomer.ffffffffffff"), None);
            // One account is the whole driver and is asked for by the driver's name.
            app.limits = Some(vec![standing(account(WORK, "work", false), 70.0)]);
            assert_eq!(
                app.limits.as_ref().unwrap()[0].provider.board_name(),
                "newcomer"
            );
            assert_eq!(left(&app, "newcomer"), Some(70.0));
        });
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

#[cfg(test)]
mod account_tests {
    use super::*;
    use crate::board::limits::{
        Current, Provider, Standing,
        observation::{Account, Report},
    };
    use serde_json::json;
    #[test]
    fn null_account_states_suppress_balance_and_rate_in_the_shared_usage_row() {
        let mut app = crate::board::app::workspace::tests::board();
        let report=Report::read(&json!({"observedAtMs":1000,"account":{"key":"0123456789ab","label":"work"},"windows":[{"windowMinutes":300,"usedPercent":8.0,"resetsAtMs":10000}]})).unwrap();
        let provider = Provider::named("claude")
            .with_account(&Account::read(&json!({"key":"0123456789ab","label":"work"})).unwrap());
        app.limits = Some(vec![Standing {
            current: Some(Current::Reported(report)),
            provider,
            latest: None,
            burn: None,
            trend: None,
            rates: Default::default(),
        }]);
        let value = projection(&app, "claude", QuotaWindow::Short, 2000);
        assert_eq!(value.balance.unwrap().left, 92.0);
        for (state, word) in [(Current::Waiting, "Waiting"), (Current::Expired, "Stale")] {
            app.limits.as_mut().unwrap()[0].current = Some(state);
            let value = projection(&app, "claude", QuotaWindow::Short, 2000);
            assert!(value.balance.is_none());
            assert_eq!(value.pace, Pace::default());
            let text = usage_row(value, 2000, 80, Look::default()).to_string();
            assert!(text.contains(word), "{text}");
            assert!(!text.contains('%'));
        }
    }
}

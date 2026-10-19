//! Compact text from held limits and the admitted selected member's usage.
use crate::{
    board::{
        home::limits::percent,
        inventory::ContextUsage,
        limits::{Outlook, QuotaWindow, Standing},
    },
    look::Look,
};
use ratatui::text::{Line, Span};
use tmt_cli_style::Role;
use unicode_width::UnicodeWidthStr;

pub(in crate::board) struct Observations<'a> {
    pub limits: &'a [Standing],
    pub context: Option<(&'a str, ContextUsage)>,
    pub now: u64,
}

impl Observations<'_> {
    pub(super) fn line(
        &self,
        weekly: bool,
        window: bool,
        width: Option<usize>,
        look: Look,
    ) -> Line<'static> {
        let mut spans = Vec::new();
        for standing in self.limits {
            let Outlook::Known(figures) = standing.outlook(self.now) else {
                continue;
            };
            if figures.short.is_none() && !weekly {
                continue;
            }
            spans.push(Span::styled(
                format!("  {}", standing.provider.label()),
                look.role(Role::Muted),
            ));
            for (kind, value) in [
                (QuotaWindow::Short, figures.short),
                (QuotaWindow::Weekly, weekly.then_some(figures.weekly)),
            ] {
                if let Some(value) = value {
                    let label = kind.key();
                    let join = if kind == QuotaWindow::Weekly && figures.short.is_some() {
                        " ·"
                    } else {
                        ""
                    };
                    spans.push(Span::styled(
                        format!("{join} {label} "),
                        look.role(Role::Muted),
                    ));
                    spans.push(Span::styled(percent(value.percent), look.role(Role::Text)));
                }
            }
        }
        if let Some((name, usage)) = self.context {
            let role = if usage
                .window_tokens
                .is_some_and(|total| u128::from(usage.used_tokens) * 100 >= u128::from(total) * 80)
            {
                Role::Waiting
            } else {
                Role::Text
            };
            let mut value = crate::source::tokens(u128::from(usage.used_tokens));
            if let Some(total) = usage.window_tokens.filter(|_| window) {
                value.push_str(&format!(" / {}", crate::source::tokens(u128::from(total))));
            }
            let name = crate::board::notes::single_line(name);
            let name = width.map_or_else(
                || name.clone(),
                |width| {
                    let provider_width: usize = spans.iter().map(|span| span.content.width()).sum();
                    let room = width.saturating_sub(provider_width + 9 + value.width());
                    crate::board::view::fit(&name, room.max(1).min(name.width()))
                },
            );
            spans.push(Span::styled(
                format!("  │ {name} ctx "),
                look.role(Role::Muted),
            ));
            spans.push(Span::styled(value, look.role(role)));
        }
        Line::from(spans)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::limits::{Provider, Reading, Window};

    #[test]
    fn only_live_windows_paint_and_weekly_parts_yield_together() {
        let now = 10_000_000;
        let limit = Standing {
            provider: Provider::named("claude"),
            latest: Some(Reading {
                observed_at_ms: now,
                weekly: Window {
                    left: 41.0,
                    resets_at_ms: now + 10_000,
                },
                short: Some(Window {
                    left: 62.0,
                    resets_at_ms: now + 5_000,
                }),
            }),
            burn: None,
            trend: None,
        };
        let limits = [limit];
        let mut held = Observations {
            limits: &limits,
            context: None,
            now,
        };
        let look = Look::default();
        assert_eq!(
            held.line(true, true, None, look).to_string(),
            "  Claude 5h 62% · 7d 41%"
        );
        assert_eq!(
            held.line(false, true, None, look).to_string(),
            "  Claude 5h 62%"
        );
        held.now += 10_001;
        assert!(held.line(true, true, None, look).spans.is_empty());
    }

    #[test]
    fn context_keeps_zero_unknown_windows_and_the_exact_eighty_percent_boundary() {
        let look = Look {
            theme: tmt_cli_style::Theme::new(tmt_cli_style::Base::Tmt),
            depth: tmt_cli_style::Depth::TrueColor,
        };
        assert_ne!(look.role(Role::Waiting), look.role(Role::Text));
        let mut held = Observations {
            limits: &[],
            context: None,
            now: 1,
        };
        assert!(held.line(true, true, None, look).spans.is_empty());
        for (used_tokens, window_tokens, role, expected) in [
            (0, None, Role::Text, Some("0")),
            (79, Some(100), Role::Text, Some("79 / 100")),
            (80, Some(100), Role::Waiting, Some("80 / 100")),
            (u64::MAX, Some(u64::MAX), Role::Waiting, None),
        ] {
            held.context = Some((
                "worker",
                ContextUsage {
                    used_tokens,
                    window_tokens,
                    observed_at_ms: 1,
                },
            ));
            let line = held.line(true, true, None, look);
            assert_eq!(line.spans.last().unwrap().style, look.role(role));
            if let Some(expected) = expected {
                assert_eq!(line.to_string(), format!("  │ worker ctx {expected}"));
            }
            assert!(
                !held
                    .line(true, false, None, look)
                    .to_string()
                    .contains(" / ")
            );
        }
    }
}

//! HOME's three strips outside the body: the summary under the tab line, the
//! usage line under it and the key line in the footer. Each is a literal template
//! whose width steps (`lg`, `md`) are a `tmt-switch`; what each step shows is built
//! here, and the content fit (the member name that shrinks, the hints that drop)
//! stays measurement that feeds the branch it belongs to.
use super::{
    Counts, Home,
    scene::{self, Kept, Key, Painted, Part},
};
use crate::{
    board::{
        app::{HomeHeaderUsage, UsageShare},
        view::fit,
    },
    look::Look,
};
use ratatui::{
    style::Style,
    text::{Line, Span},
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::OnceLock};
use tmt_cli_style::{Role, grid::Align, table::escape};
use tmt_tui::binding::{Schema, Template};
use unicode_width::UnicodeWidthStr;

type Piece = (String, Option<Role>);

/// One branch of a strip: a single row of role-tagged pieces. A piece without a
/// role is plain text with no style of its own.
fn row(branch: &str) -> String {
    format!(
        r#"<tmt-row id="line" class="w-full h-1"><tmt-repeat each="$.{branch}" as="piece"><tmt-text id-bind="piece.id" bind="piece.text" token-bind="piece.role" class="shrink-0"/></tmt-repeat></tmt-row>"#
    )
}

fn schema(branches: &[&str]) -> Schema {
    let piece = Schema::Object(BTreeMap::from([
        ("id".to_owned(), Schema::StableId),
        ("text".to_owned(), Schema::Scalar),
        ("role".to_owned(), Schema::Scalar),
    ]));
    Schema::Object(
        branches
            .iter()
            .map(|branch| {
                (
                    (*branch).to_owned(),
                    Schema::Collection(Box::new(piece.clone())),
                )
            })
            .collect(),
    )
}

fn data(branches: &[(&str, Vec<Piece>)]) -> Value {
    Value::Object(
        branches
            .iter()
            .map(|(branch, pieces)| {
                (
                    (*branch).to_owned(),
                    Value::Array(
                        pieces
                            .iter()
                            .enumerate()
                            .map(|(at, (text, role))| {
                                json!({
                                    "id": format!("{}{at}", if role.is_some() { "p" } else { "raw" }),
                                    "text": text,
                                    "role": role.map(Role::name),
                                })
                            })
                            .collect(),
                    ),
                )
            })
            .collect(),
    )
}

/// A plain piece (`raw…`) takes no style; a tagged one (`p…`) takes its role's.
fn paint(key: &Key, file: &str, template: &Template<()>) -> Painted {
    let look = key.look;
    scene::paint(
        file,
        template,
        &key.data,
        key.width,
        &mut |Part { id, role, .. }| {
            let style = match id.and_then(<[String]>::last) {
                Some(piece) if piece.starts_with('p') => look.role(role),
                _ => Style::new(),
            };
            (style, Align::Left)
        },
    )
}

fn count_pieces(counts: &Counts, words: bool) -> Vec<Piece> {
    [
        ("◆", counts.waiting, Role::Waiting, "waiting on you"),
        ("✗", counts.blocked, Role::Blocked, "blocked"),
        ("◐", counts.review, Role::Review, "review"),
        ("●", counts.working, Role::Working, "working"),
        ("○", counts.idle, Role::Dim, "idle"),
    ]
    .into_iter()
    .flat_map(|(mark, count, role, label)| {
        let suffix = if words {
            format!(" {label}")
        } else {
            String::new()
        };
        [
            (
                mark.to_owned(),
                Some(if count == 0 { Role::Dim } else { role }),
            ),
            (format!(" {count}{suffix}"), Some(Role::Text)),
            ("  ".into(), Some(Role::Dim)),
        ]
    })
    .collect()
}

/// The strip's line: its content, without the unpainted tail of the row.
fn lines(painted: Painted) -> Option<Line<'static>> {
    let mut line = painted.lines.into_iter().next()?;
    let blank = ratatui::buffer::Cell::default().style();
    while let Some(last) = line.spans.last_mut() {
        if last.style != blank {
            break;
        }
        let kept = last.content.trim_end().len();
        if kept == 0 {
            line.spans.pop();
        } else {
            last.content.to_mut().truncate(kept);
            break;
        }
    }
    Some(line)
}

/// The summary: squads and members in words from `lg`, squads and the five
/// counts alone below it.
#[cfg(test)]
pub(crate) fn summary(home: &Home, width: u16, look: Look) -> Line<'static> {
    summary_in(&mut Kept::default(), home, width, look)
}

/// The summary, painted again only when its key changed.
pub(super) fn summary_in(
    slot: &mut Kept<Line<'static>>,
    home: &Home,
    width: u16,
    look: Look,
) -> Line<'static> {
    static TEMPLATE: OnceLock<Template<()>> = OnceLock::new();
    const FILE: &str = "squad.home.summary.xml";
    let template = TEMPLATE.get_or_init(|| {
        let markup = format!(
            r#"<tmt-view version="1"><tmt-switch><tmt-case min="lg">{}</tmt-case><tmt-default>{}</tmt-default></tmt-switch></tmt-view>"#,
            row("wide"),
            row("narrow")
        );
        scene::compile(FILE, &markup, &schema(&["wide", "narrow"]))
    });
    let mut wide = vec![(
        format!(
            "{} squads · {} members   ",
            home.squads.len(),
            home.summary.members
        ),
        Some(Role::Text),
    )];
    wide.extend(count_pieces(&home.summary, true));
    let mut narrow = vec![(format!("{} squads  ", home.squads.len()), Some(Role::Text))];
    narrow.extend(count_pieces(&home.summary, false));
    let key = Key {
        width,
        look,
        selected: None,
        data: data(&[("wide", wide), ("narrow", narrow)]),
    };
    slot.get(key, |key| {
        lines(paint(key, FILE, template)).unwrap_or_default()
    })
    .clone()
}

fn percent(share: &UsageShare) -> String {
    format!(
        "{}{:.0}%",
        if share.partial { "~" } else { "" },
        share.fraction * 100.0
    )
}

/// One step of the usage line. `wide` is the `lg` step: it adds the first window,
/// the models and the members without data. The member name shrinks until the
/// step fits `width`; that is a fit, not a step.
fn usage_pieces(usage: &HomeHeaderUsage<'_>, width: u16, wide: bool) -> Vec<Piece> {
    let width = usize::from(width);
    let windows = (usize::from(!wide)..3)
        .map(|index| {
            let number = usage.totals[index].map_or_else(
                || "–".into(),
                |reading| {
                    let number = crate::source::tokens(reading.tokens as f64);
                    format!("{}{number}", if reading.partial { "~" } else { "" })
                },
            );
            format!("{} {number}", usage.windows[index].label())
        })
        .collect::<Vec<_>>()
        .join(" · ");
    let windows = format!("tok {windows}");
    let unreported = wide.then(|| {
        format!(
            "{} {} without data",
            usage.unreported,
            if usage.unreported == 1 {
                "member"
            } else {
                "members"
            }
        )
    });
    let reserved = unreported.as_ref().map_or(0, |text| text.len() + 3);
    let mut member_width = 18;
    let top = loop {
        let top = usage.top.as_ref().map_or_else(
            || format!("share {}: –", usage.windows[2].label()),
            |top| {
                format!(
                    "share {}: {} {}",
                    usage.windows[2].label(),
                    fit(&escape(top.member), member_width).trim_end(),
                    percent(&top.share)
                )
            },
        );
        let used = windows.width()
            + top.width()
            + 3
            + reserved
            + if wide { " · models –".width() } else { 0 };
        if used <= width || member_width == 1 {
            break top;
        }
        member_width -= 1;
    };
    let mut pieces: Vec<Piece> = vec![
        (windows, Some(Role::Text)),
        (" · ".into(), Some(Role::Dim)),
        (top, Some(Role::Text)),
    ];
    if let Some(unreported) = unreported {
        let used: usize = pieces.iter().map(|(text, _)| text.width()).sum();
        let available = width.saturating_sub(used + reserved + 3);
        let mut models = String::from("models ");
        for model in &usage.models {
            let one = format!(
                "{} {}",
                fit(
                    &escape(model.model.map(crate::source::model_name).unwrap_or("–")),
                    12
                )
                .trim_end(),
                percent(&model.share)
            );
            let separator = if models == "models " { "" } else { ", " };
            let next = format!("{models}{separator}{one}");
            if next.width() > available {
                break;
            }
            models = next;
        }
        if models == "models " {
            models.push('–');
        }
        if models.width() <= available {
            pieces.push((" · ".into(), Some(Role::Dim)));
            pieces.push((models, Some(Role::Text)));
        }
        pieces.push((" · ".into(), Some(Role::Dim)));
        pieces.push((unreported, Some(Role::Text)));
    }
    pieces
}

/// The usage line: nothing below `md`, the last two windows and the top member's
/// share from `md`, and from `lg` all three windows, the models and the members
/// without data. `None` when the switch selects nothing or nothing was observed.
#[cfg(test)]
pub(crate) fn usage(usage: &HomeHeaderUsage<'_>, width: u16, look: Look) -> Option<Line<'static>> {
    usage_in(&mut Kept::default(), usage, width, look)
}

/// The usage line, painted again only when its key changed.
pub(super) fn usage_in(
    slot: &mut Kept<Option<Line<'static>>>,
    usage: &HomeHeaderUsage<'_>,
    width: u16,
    look: Look,
) -> Option<Line<'static>> {
    static TEMPLATE: OnceLock<Template<()>> = OnceLock::new();
    const FILE: &str = "squad.home.usage.xml";
    if usage.totals.iter().all(Option::is_none) {
        return None;
    }
    let template = TEMPLATE.get_or_init(|| {
        let markup = format!(
            r#"<tmt-view version="1"><tmt-switch><tmt-case min="lg">{}</tmt-case><tmt-case min="md">{}</tmt-case><tmt-default/></tmt-switch></tmt-view>"#,
            row("wide"),
            row("medium")
        );
        scene::compile(FILE, &markup, &schema(&["wide", "medium"]))
    });
    let key = Key {
        width,
        look,
        selected: None,
        data: data(&[
            ("wide", usage_pieces(usage, width, true)),
            ("medium", usage_pieces(usage, width, false)),
        ]),
    };
    slot.get(key, |key| lines(paint(key, FILE, template)))
        .clone()
}

/// The key line: whole hints drop from the end until the line and the two exit
/// hints fit (a fit, not a step).
fn key_line(width: usize, overflow: bool) -> String {
    // The other keys (tabs, replies, cron, refresh, ...) are in `?` help;
    // `s switch` comes back only while the tab line hides tabs.
    let mut optional = vec!["↑↓ move", "⏎ open", "e expand", "/ search"];
    if overflow {
        optional.push("s switch");
    }
    loop {
        let text = optional
            .iter()
            .copied()
            .chain(["? more", "q quit"])
            .collect::<Vec<_>>()
            .join("  ");
        if text.width() <= width || optional.is_empty() {
            return text;
        }
        optional.pop();
    }
}

#[cfg(test)]
pub(crate) fn hints(width: usize, overflow: bool) -> String {
    hints_in(&mut Kept::default(), width, overflow, true)
}

/// The key line, painted again only when its key changed.
pub(super) fn hints_in(
    slot: &mut Kept<String>,
    width: usize,
    overflow: bool,
    _receiving: bool,
) -> String {
    static TEMPLATE: OnceLock<Template<()>> = OnceLock::new();
    const FILE: &str = "squad.home.keys.xml";
    let template = TEMPLATE.get_or_init(|| {
        scene::compile(
            FILE,
            r#"<tmt-view version="1"><tmt-text id="line" bind="$.line" token="muted" class="w-full h-1"/></tmt-view>"#,
            &schema(&["line"]),
        )
    });
    let width = width.min(usize::from(u16::MAX)) as u16;
    let key = Key {
        width,
        look: Look::default(),
        selected: None,
        data: json!({"line": key_line(usize::from(width), overflow)}),
    };
    slot.get(key, |key| {
        let painted = scene::paint(FILE, template, &key.data, key.width, &mut |_| {
            (Style::new(), Align::Left)
        });
        painted
            .lines
            .first()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span: &Span<'_>| span.content.as_ref())
                    .collect::<String>()
                    .trim_end()
                    .to_owned()
            })
            .unwrap_or_default()
    })
    .clone()
}

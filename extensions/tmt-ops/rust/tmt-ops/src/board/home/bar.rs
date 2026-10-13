//! Overview summary and usage strips over held data. Named width steps and
//! content budgets are projected here before shared scene painting. Footer
//! actions are generated separately from Config's effective scoped key table.
use super::{
    Home,
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

pub(super) type Piece = (String, Option<Role>);

/// One branch of a strip: a single row of role-tagged pieces. A piece without a
/// role is plain text with no style of its own.
pub(super) fn row(branch: &str) -> String {
    format!(
        r#"<tmt-row id="line" class="w-full h-1"><tmt-repeat each="$.{branch}" as="piece"><tmt-text id-bind="piece.id" bind="piece.text" token-bind="piece.role" class="shrink-0"/></tmt-repeat></tmt-row>"#
    )
}

pub(super) fn schema(branches: &[&str]) -> Schema {
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

pub(super) fn data(branches: &[(&str, Vec<Piece>)]) -> Value {
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
pub(super) fn paint(key: &Key, file: &str, template: &Template<()>) -> Painted {
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

fn summary_pieces(home: &Home, width: u16) -> Vec<Piece> {
    let names = home
        .squads
        .iter()
        .filter(|squad| squad.counts.waiting > 0)
        .map(|squad| escape(&squad.squad))
        .collect::<Vec<_>>();
    let needs = if names.is_empty() {
        if home.failures.is_empty() && !home.incomplete {
            "✓ Nothing needs you".into()
        } else {
            "Needs you: partial reading".into()
        }
    } else if width < super::SM.cells {
        format!(
            "◆ {} need{} you {}",
            names.len(),
            if names.len() == 1 { "s" } else { "" },
            names.join(" ")
        )
    } else {
        format!("◆ Needs you  {}", names.join(" · "))
    };
    let totals = if width >= super::MD.cells {
        format!(
            "● {} working  ◌ {} idle  ✗ {} blocked",
            home.summary.working, home.summary.idle, home.summary.blocked
        )
    } else if width >= super::SM.cells {
        format!("● {} ◌ {}", home.summary.working, home.summary.idle)
    } else {
        String::new()
    };
    let available =
        usize::from(width).saturating_sub(totals.width() + usize::from(!totals.is_empty()) * 2);
    let needs = fit(&needs, available).trim_end().to_owned();
    let gap = if totals.is_empty() {
        String::new()
    } else {
        " ".repeat(usize::from(width).saturating_sub(needs.width() + totals.width()))
    };
    let (mark, words, role) = if let Some(words) = needs.strip_prefix("◆") {
        ("◆", words, Role::Waiting)
    } else if let Some(words) = needs.strip_prefix("✓") {
        ("✓", words, Role::Muted)
    } else {
        ("", needs.as_str(), Role::Text)
    };
    let mut pieces = vec![
        (mark.into(), Some(role)),
        (words.into(), Some(Role::Text)),
        (gap, None),
    ];
    for (index, group) in totals
        .split("  ")
        .filter(|group| !group.is_empty())
        .enumerate()
    {
        if index > 0 {
            pieces.push(("  ".into(), None));
        }
        let (mark, words) = group.split_at(group.chars().next().unwrap().len_utf8());
        let role = match mark {
            "●" => Role::Working,
            "◌" => Role::Muted,
            _ => Role::Blocked,
        };
        pieces.push((mark.into(), Some(role)));
        pieces.push((words.into(), Some(Role::Text)));
    }
    pieces
}

/// Only names omitted by the actual fitted summary belong in overview detail.
pub(in crate::board) fn hidden_needs(home: &Home, width: u16) -> Option<String> {
    let shown = summary_pieces(home, width)
        .into_iter()
        .map(|(text, _)| text)
        .collect::<String>();
    let names = home
        .squads
        .iter()
        .filter(|squad| squad.counts.waiting > 0)
        .collect::<Vec<_>>();
    let mut prefix = if width < super::SM.cells {
        format!(
            "◆ {} need{} you ",
            names.len(),
            if names.len() == 1 { "s" } else { "" }
        )
    } else {
        "◆ Needs you  ".into()
    };
    let mut hidden = Vec::new();
    for (index, squad) in names.iter().enumerate() {
        if index > 0 {
            prefix.push_str(if width < super::SM.cells { " " } else { " · " });
        }
        prefix.push_str(&escape(&squad.squad));
        if !shown.starts_with(&prefix) {
            hidden.push(squad.squad.as_str());
        }
    }
    (!hidden.is_empty()).then(|| hidden.join(" · "))
}

/// The strip's line: its content, without the unpainted tail of the row.
pub(super) fn lines(painted: Painted) -> Option<Line<'static>> {
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

/// Needs-you names lead; state totals use the remaining space.
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
        let markup = format!(r#"<tmt-view version="1">{}</tmt-view>"#, row("summary"));
        scene::compile(FILE, &markup, &schema(&["summary"]))
    });
    let pieces = summary_pieces(home, width);
    let key = Key {
        width,
        look,
        selected: None,
        data: data(&[("summary", pieces)]),
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

/// Accepted evidence owns the numeric budget, independently of animation.
fn usage_number(usage: &HomeHeaderUsage<'_>, index: usize) -> String {
    usage.totals[index].map_or_else(
        || "–".into(),
        |reading| {
            let number = crate::source::tokens(reading.tokens as f64);
            format!("{}{number}", if reading.partial { "~" } else { "" })
        },
    )
}

/// The longest configured window stays visible. The top member drops below
/// 80 cells; short windows and model shares belong in the shared detail.
fn usage_pieces(
    usage: &HomeHeaderUsage<'_>,
    width: u16,
    digits: Option<&[String; 3]>,
) -> Vec<Piece> {
    let available = width.saturating_sub(2);
    let accepted = if usage.totals[2].is_some() {
        usage_number(usage, 2)
    } else {
        "no reading".into()
    };
    let number = digits
        .filter(|digits| usage.totals[2].is_some() && digits[2].width() <= accepted.width())
        .map_or_else(
            || accepted.clone(),
            |digits| format!("{:>budget$}", digits[2], budget = accepted.width()),
        );
    let mut text = format!(
        "{}{} {number}{}",
        if width >= super::MD.cells {
            "Usage "
        } else {
            ""
        },
        usage.windows[2].label(),
        if usage.totals[2].is_some() {
            " tok"
        } else {
            ""
        }
    );
    if width >= super::SM.cells
        && let Some(top) = &usage.top
    {
        let suffix = format!(" {}", percent(&top.share));
        let room = usize::from(available)
            .saturating_sub(text.width() + " · top ".width() + suffix.width());
        text.push_str(&format!(
            " · top {}{suffix}",
            fit(&escape(top.member), room).trim_end()
        ));
    }
    vec![("  ".into(), Some(Role::Muted)), (text, Some(Role::Muted))]
}

pub(in crate::board) fn hidden_usage(
    usage: &HomeHeaderUsage<'_>,
    width: u16,
) -> Vec<(&'static str, String)> {
    let shown = usage_pieces(usage, width, None)
        .into_iter()
        .map(|(text, _)| text)
        .collect::<String>();
    let mut fields = (0..2)
        .map(|index| {
            (
                "usage",
                format!(
                    "{} {}",
                    usage.windows[index].label(),
                    if usage.totals[index].is_some() {
                        format!("{} tok", usage_number(usage, index))
                    } else {
                        "no reading".into()
                    }
                ),
            )
        })
        .collect::<Vec<_>>();
    fields.push((
        "coverage",
        format!(
            "{} {} without data",
            usage.unreported,
            if usage.unreported == 1 {
                "member"
            } else {
                "members"
            }
        ),
    ));
    if let Some(top) = &usage.top {
        let full = format!("{} {}", escape(top.member), percent(&top.share));
        if !shown.contains(&full) {
            fields.push(("top", full));
        }
    }
    let models = usage
        .models
        .iter()
        .map(|model| {
            format!(
                "{} {}",
                model
                    .model
                    .map(crate::source::model_name)
                    .unwrap_or("model not known"),
                percent(&model.share)
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    if !models.is_empty() {
        fields.push(("models", models));
    }
    fields
}

/// The longest-window usage line, including explicit missing-data wording.
#[cfg(test)]
pub(crate) fn usage(usage: &HomeHeaderUsage<'_>, width: u16, look: Look) -> Option<Line<'static>> {
    usage_in(&mut Kept::default(), usage, width, look, None)
}

/// The usage line, painted again only when its key changed.
pub(super) fn usage_in(
    slot: &mut Kept<Option<Line<'static>>>,
    usage: &HomeHeaderUsage<'_>,
    width: u16,
    look: Look,
    digits: Option<&[String; 3]>,
) -> Option<Line<'static>> {
    static TEMPLATE: OnceLock<Template<()>> = OnceLock::new();
    const FILE: &str = "squad.home.usage.xml";
    let template = TEMPLATE.get_or_init(|| {
        let markup = format!(r#"<tmt-view version="1">{}</tmt-view>"#, row("usage"));
        scene::compile(FILE, &markup, &schema(&["usage"]))
    });
    let key = Key {
        width,
        look,
        selected: None,
        data: data(&[("usage", usage_pieces(usage, width, digits))]),
    };
    slot.get(key, |key| lines(paint(key, FILE, template)))
        .clone()
}

/// Numeric cells of the admitted header branch, before viewport placement.
pub(super) fn usage_slots(usage: &HomeHeaderUsage<'_>, width: u16) -> Vec<(usize, u16, u16)> {
    let prefix = format!(
        "{}{} ",
        if width >= super::MD.cells {
            "Usage "
        } else {
            ""
        },
        usage.windows[2].label()
    );
    let x = 2 + prefix.width() as u16;
    vec![(
        2,
        x,
        (usage_number(usage, 2).width() as u16).min(width.saturating_sub(x)),
    )]
}

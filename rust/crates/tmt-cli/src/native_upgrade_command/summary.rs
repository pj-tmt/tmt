//! The plain-text account of one `tmt upgrade`: a table of the components that changed or need
//! attention, one muted line for those already current, a one-line summary and one list of next
//! steps. It reads the rows the JSON document carries, so the two never disagree and `--json` does
//! not change with this view.

use serde_json::Value;
use std::{
    collections::BTreeMap,
    io::{self, Write},
};
use tmt_cli_style::{
    Terminal, Token,
    mark::Mark,
    message,
    table::{Cell, Column, Table},
};

/// Where a component stands after this upgrade.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Standing {
    Updated,
    Current,
    Pinned,
    /// An update is available and waits for the reader's approval.
    Declined,
    Failed,
}

impl Standing {
    fn of(status: &str) -> Self {
        match status {
            "changed" => Self::Updated,
            "skippedPinned" => Self::Pinned,
            "consentRequired" => Self::Declined,
            "failed" => Self::Failed,
            _ => Self::Current,
        }
    }

    fn mark(self) -> Option<Mark> {
        match self {
            Self::Updated => Some(Mark::Done),
            Self::Pinned | Self::Declined => Some(Mark::Warning),
            Self::Failed => Some(Mark::Failed),
            Self::Current => None,
        }
    }

    fn word(self) -> &'static str {
        match self {
            Self::Updated => "updated",
            Self::Current => "current",
            Self::Pinned => "pinned",
            Self::Declined => "needs approval",
            Self::Failed => "failed",
        }
    }
}

struct Component<'a> {
    name: &'a str,
    from: Option<&'a str>,
    to: Option<&'a str>,
    standing: Standing,
}

impl<'a> Component<'a> {
    fn of(row: &'a Value, before: Option<&'a BTreeMap<String, String>>) -> Self {
        let product = row["product"].as_str().unwrap_or("extension");
        let standing = Standing::of(row["status"].as_str().unwrap_or("failed"));
        let version = row["version"].as_str();
        let before = before
            .and_then(|before| before.get(product))
            .map(String::as_str);
        let (from, to) = match standing {
            Standing::Updated => (before, version),
            Standing::Declined => (version, row["availableVersion"].as_str()),
            Standing::Failed => (before, version.filter(|version| Some(*version) != before)),
            Standing::Pinned | Standing::Current => (None, version),
        };
        Self {
            name: if product == "cli" { "tmt" } else { product },
            from,
            to,
            standing,
        }
    }

    fn versions(&self) -> String {
        match (self.from, self.to) {
            (Some(from), Some(to)) if from != to => format!("{from} → {to}"),
            (_, Some(version)) | (Some(version), None) => version.to_owned(),
            (None, None) => String::new(),
        }
    }
}

/// What the reader still has to do (`required`) and what only helps (`optional`), each said once.
#[derive(Default)]
struct Next {
    required: Vec<String>,
    optional: Vec<String>,
}

impl Next {
    fn required(&mut self, text: impl Into<String>) {
        let text = text.into();
        if !self.required.contains(&text) {
            self.required.push(text);
        }
    }

    fn optional(&mut self, text: impl Into<String>) {
        let text = text.into();
        if !self.optional.contains(&text) {
            self.optional.push(text);
        }
    }

    fn write(&self, output: &mut impl Write, terminal: Terminal) -> io::Result<()> {
        let optional = self
            .optional
            .iter()
            .filter(|text| !self.required.contains(text));
        if self.required.is_empty() && optional.clone().next().is_none() {
            return Ok(());
        }
        writeln!(output, "{}", terminal.paint(Token::Title, "Next:"))?;
        for text in &self.required {
            writeln!(output, "  {}", tmt_cli_style::table::escape(text))?;
        }
        for text in optional {
            writeln!(
                output,
                "  {}",
                terminal.paint(Token::Dim, &tmt_cli_style::table::escape(text))
            )?;
        }
        Ok(())
    }
}

/// What the upgrade tells the reader beyond its products. Required notices need an action to
/// finish the upgrade; optional ones only help.
pub(super) struct Followups<'a> {
    /// Pane option cutover and database notices: unfinished work.
    pub required: &'a [String],
    /// The missing prompt picker, the retained former prefix and the retired Squad.
    pub optional: &'a [&'a str],
    /// A skill target kept with its own content.
    pub conflict_hints: Vec<String>,
    /// Said only when a skill's content changed, so an upgrade that updated no guidance does not
    /// ask for a reload; existing conversations read it with `tmt learn --skill`.
    pub reload_hint: Option<&'static str>,
}

pub(super) struct Account<'a> {
    /// Products as the JSON document carries them, the CLI first.
    pub products: &'a [Value],
    /// Whether the CLI row has a report; without one only the failure is told.
    pub has_report: bool,
    pub skipped_pinned: bool,
    /// Versions installed before the upgrade, by product name (`cli` for the CLI).
    pub before: Option<&'a BTreeMap<String, String>>,
    pub skills: &'a Value,
    /// The command that repeats this invocation; the first line of `Next:` after a component
    /// failed, so the retry is never a guess.
    pub repeat: Option<&'a str>,
    pub followups: Followups<'a>,
}

pub(super) fn write(
    output: &mut impl Write,
    terminal: Terminal,
    account: &Account<'_>,
) -> io::Result<()> {
    if !account.has_report {
        // Nothing was installed or told; the failure carries the whole account.
        return Ok(());
    }
    let shown = account
        .products
        .iter()
        .map(|row| Component::of(row, account.before))
        .collect::<Vec<_>>();
    let attention = shown
        .iter()
        .filter(|component| component.standing != Standing::Current);
    let mut table = Table::new(&[Column::Fixed, Column::Fixed, Column::Fixed]);
    for component in attention {
        let mark = component.standing.mark().expect("attention has a mark");
        table.row([
            Cell::from(component.name),
            Cell::from(component.versions()),
            Cell::styled(
                format!("{} {}", mark.symbol(), component.standing.word()),
                mark.token(),
            ),
        ]);
    }
    table.write(output, terminal)?;
    let mut next = Next::default();
    let retry = account.repeat.filter(|_| {
        account
            .products
            .iter()
            .skip(1)
            .any(|row| row["status"] == "failed")
    });
    if let Some(repeat) = retry {
        next.required(repeat);
    }
    for row in account.products.iter().skip(1) {
        extension_details(output, terminal, row, retry.is_some(), &mut next)?;
    }
    let current = shown
        .iter()
        .filter(|component| component.standing == Standing::Current)
        .map(|component| match component.to {
            Some(version) => format!("{} {version}", component.name),
            None => component.name.to_owned(),
        })
        .collect::<Vec<_>>();
    if !current.is_empty() {
        let line = format!("Up to date: {}", current.join(", "));
        writeln!(output, "{}", terminal.paint(Token::Dim, &line))?;
    }
    if let Some(message) = account.skills["warning"]["message"].as_str() {
        message::warning(output, terminal, message, None)?;
    }
    summary(output, terminal, account, &shown)?;
    for text in account.followups.required {
        next.required(text);
    }
    for conflict in &account.followups.conflict_hints {
        next.required(conflict);
    }
    for text in account.followups.optional {
        next.optional(*text);
    }
    if let Some(hint) = account.followups.reload_hint
        && skills_changed(account.skills)
    {
        next.optional(hint);
    }
    next.write(output, terminal)
}

/// A component's own failure and warnings stay beside the table in their old words; what the
/// reader must do about it goes to the final list.
fn extension_details(
    output: &mut impl Write,
    terminal: Terminal,
    row: &Value,
    retrying: bool,
    next: &mut Next,
) -> io::Result<()> {
    // The generic `tmt upgrade` retry is the repeated invocation's job once that is known.
    let generic = |hint: &str| retrying && hint == super::GENERIC_RETRY;
    if row["status"] == "failed" {
        message::error(
            output,
            terminal,
            &format!(
                "{}: {}: {}",
                row["product"].as_str().unwrap_or("extension"),
                row["error"]["code"]
                    .as_str()
                    .unwrap_or("EXTENSION_UPGRADE_FAILED"),
                row["error"]["message"]
                    .as_str()
                    .unwrap_or("Extension update failed")
            ),
            None,
        )?;
    }
    if let Some(message) = row["details"]["skills"]["warning"]["message"].as_str() {
        message::warning(output, terminal, message, None)?;
    }
    if let Some(hint) = row["details"]["restartHint"]
        .as_str()
        .or_else(|| row["error"]["suggestion"].as_str())
        && !generic(hint)
    {
        next.required(hint);
    }
    match row["status"].as_str() {
        Some("consentRequired") => {
            if let Some(hint) = row["hint"].as_str() {
                next.required(hint);
            }
        }
        Some("skippedPinned") => {
            if let Some(text) = row["message"].as_str().or_else(|| row["hint"].as_str()) {
                next.optional(text);
            }
        }
        _ => {
            if let Some(hint) = row["hint"].as_str().filter(|hint| !generic(hint)) {
                next.required(hint);
            }
        }
    }
    Ok(())
}

fn summary(
    output: &mut impl Write,
    terminal: Terminal,
    account: &Account<'_>,
    shown: &[Component<'_>],
) -> io::Result<()> {
    let count = |standing| {
        shown
            .iter()
            .filter(|component| component.standing == standing)
            .count()
    };
    let (updated, failed, declined, pinned) = (
        count(Standing::Updated),
        count(Standing::Failed),
        count(Standing::Declined),
        count(Standing::Pinned),
    );
    let mut parts = Vec::new();
    let mark = if failed > 0 {
        Mark::Failed
    } else if updated == 0 && (declined > 0 || pinned > 0) {
        Mark::Warning
    } else {
        Mark::Done
    };
    if failed > 0 {
        // What went wrong leads; what still worked follows.
        parts.push(format!("{failed} failed"));
        if updated > 0 {
            parts.push(format!("updated {updated}"));
        }
    } else if updated > 0 {
        parts.push(format!("Updated {updated}"));
    } else if declined == 0 && pinned == 0 {
        parts.push("Up to date".into());
    } else {
        parts.push("Nothing changed".into());
    }
    if declined > 0 {
        parts.push(format!("{declined} waiting for approval"));
    }
    if pinned > 0 {
        parts.push(format!("{pinned} pinned"));
    }
    if !account.skipped_pinned && account.skills["refreshed"].is_array() {
        parts.push(if skills_changed(account.skills) {
            "skills refreshed".into()
        } else {
            "skills current".into()
        });
        let missing = account.skills["skipped"].as_array().map_or(0, |skipped| {
            skipped.iter().filter(|t| core_skill(t)).count()
        });
        if missing > 0 {
            parts.push(format!("{missing} skill{} missing", plural(missing)));
        }
        let conflicts = account.skills["conflicts"].as_array().map_or(0, Vec::len);
        if conflicts > 0 {
            parts.push(format!(
                "{conflicts} skill conflict{} preserved",
                plural(conflicts)
            ));
        }
    }
    writeln!(
        output,
        "{} {}",
        terminal.paint(mark.token(), mark.symbol()),
        parts.join(" · ")
    )
}

fn skills_changed(skills: &Value) -> bool {
    skills["refreshed"]
        .as_array()
        .is_some_and(|refreshed| refreshed.iter().any(|skill| skill["changed"] == true))
}

fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

/// Only core's own skills that went missing are worth the reader's attention: a skill of an
/// extension that is frozen or not installed is not a fault, and the JSON report keeps every
/// skipped path.
fn core_skill(target: &Value) -> bool {
    target
        .as_str()
        .and_then(|target| std::path::Path::new(target).file_name())
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            tmt_core::skill_catalog::BUNDLED.iter().any(|skill| {
                skill.name == name && skill.group == tmt_core::skill_catalog::Group::Core
            })
        })
}

#[cfg(test)]
mod tests;

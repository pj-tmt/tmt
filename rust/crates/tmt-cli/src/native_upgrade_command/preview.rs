//! Whole-run metadata presentation and consent before native activation.

use super::*;
use crate::{caller_context, consent, extension_install_command::upgrade_all};
use tmt_adapters::native_install::{MigrationPreview, PreparedUpgrade};
use tmt_cli_style::{
    Terminal, Token,
    table::{Cell, Column, Table},
};

#[derive(Debug)]
pub(super) struct Approval {
    pub proceed: bool,
    pub extensions: bool,
}

pub(super) fn approve(
    cli: &PreparedUpgrade,
    extensions: &upgrade_all::Plan,
    explicit: bool,
    yes: bool,
    mode: OutputMode,
) -> Result<Approval, Failure> {
    let human = caller_context::may_offer_install(tmt_cli_style::Interaction::detect(mode.json));
    let version = cli.current().state.version.to_string();
    let selected = cli.selected_version().map(ToString::to_string);
    let changes = explicit
        || selected
            .as_ref()
            .is_some_and(|selected| selected != &version)
        || !extensions.pending.is_empty();
    approve_with(
        human,
        yes,
        changes,
        || {
            let rows = rows(
                &version,
                selected.as_deref(),
                explicit,
                cli.migration(),
                extensions,
            );
            let mut output = tmt_cli_style::stream::stdout(false);
            let terminal = output.terminal();
            write_rows(&mut output, terminal, &rows).map_err(presentation_failure)
        },
        || {
            consent::ask_defaulting_yes(
                &mut tmt_cli_style::stream::stdout(false),
                false,
                mode,
                consent::Consent {
                    code: "NATIVE_UPGRADE_CONSENT_REQUIRED",
                    refusal: "Upgrade approval requires a human terminal.",
                    question: "  Upgrade",
                    declined: "  No components were changed.",
                },
                presentation_failure,
            )
        },
    )
}

fn approve_with(
    human: bool,
    yes: bool,
    changes: bool,
    show: impl FnOnce() -> Result<(), Failure>,
    ask: impl FnOnce() -> Result<bool, Failure>,
) -> Result<Approval, Failure> {
    if !human {
        return Ok(Approval {
            proceed: true,
            extensions: yes,
        });
    }
    show()?;
    let accepted = yes || !changes || ask()?;
    Ok(Approval {
        proceed: accepted,
        extensions: accepted,
    })
}

fn presentation_failure(error: io::Error) -> Failure {
    Failure::new(
        "NATIVE_UPGRADE_FAILED",
        "Could not present upgrade approval.",
        1,
    )
    .caused_by(error)
}

struct Row {
    name: String,
    versions: String,
    detail: String,
}

fn rows(
    cli_version: &str,
    cli_selected: Option<&str>,
    explicit: bool,
    migration: MigrationPreview,
    plan: &upgrade_all::Plan,
) -> Vec<Row> {
    let mut rows = vec![row(
        "tmt",
        cli_version,
        cli_selected,
        match cli_selected {
            None => "skippedPinned",
            Some(selected) if selected == cli_version && !explicit => "unchanged",
            _ => "pending",
        },
        migration,
    )];
    for (product, (status, version, selected)) in super::extensions::targets(plan) {
        rows.push(row(
            &product,
            version.as_str().unwrap_or("unknown"),
            selected.as_str(),
            &status,
            MigrationPreview::Unknown,
        ));
    }
    rows
}

fn row(name: &str, from: &str, to: Option<&str>, status: &str, migration: MigrationPreview) -> Row {
    let versions = match to {
        Some(to) if to != from => format!("{from} → {to}"),
        _ => from.to_owned(),
    };
    let detail = match status {
        "pending" => match migration {
            MigrationPreview::Required => "migration needed (data kept)",
            MigrationPreview::NotRequired => "no migration needed",
            MigrationPreview::Unknown => "migration unknown",
        },
        "skippedPinned" => "pinned",
        "failed" => "preview unavailable",
        _ => "up to date",
    };
    Row {
        name: name.into(),
        versions,
        detail: detail.into(),
    }
}

fn write_rows(out: &mut impl Write, terminal: Terminal, rows: &[Row]) -> io::Result<()> {
    writeln!(out, "\n  {}\n", terminal.paint(Token::Title, "Upgrade"))?;
    let mut table = Table::new(&[Column::Name, Column::Detail, Column::Detail]);
    for row in rows {
        table.row([
            Cell::styled(&row.name, Token::Title),
            Cell::from(&row.versions),
            Cell::styled(&row.detail, Token::Dim),
        ]);
    }
    table.write(out, terminal)?;
    writeln!(out)?;
    for note in [
        "Application data is kept.",
        "Bundled agent skills are included.",
    ] {
        for line in tmt_cli_style::grid::fit_lines(
            note,
            terminal
                .width
                .map_or(note.len(), |width| usize::from(width).saturating_sub(2)),
            tmt_cli_style::grid::Align::Left,
            tmt_cli_style::grid::Truncate::End,
            tmt_cli_style::grid::Overflow::Wrap { max_lines: 6 },
        ) {
            writeln!(out, "  {}", line.trim_end())?;
        }
    }
    writeln!(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn a_decline_happens_after_preview_and_authorises_no_component() {
        let calls = RefCell::new(Vec::new());
        let approval = approve_with(
            true,
            false,
            true,
            || {
                calls.borrow_mut().push("preview");
                Ok(())
            },
            || {
                calls.borrow_mut().push("question");
                Ok(false)
            },
        )
        .unwrap();
        assert_eq!(*calls.borrow(), ["preview", "question"]);
        assert!(!approval.proceed);
        assert!(!approval.extensions);
    }

    #[test]
    fn machine_runs_preserve_cli_behaviour_and_never_ask() {
        for yes in [false, true] {
            let approval = approve_with(
                false,
                yes,
                true,
                || panic!("no human screen"),
                || panic!("no agent, JSON or pipe prompt"),
            )
            .unwrap();
            assert!(approval.proceed);
            assert_eq!(approval.extensions, yes);
        }
    }

    #[test]
    fn yes_and_no_op_show_the_human_preview_without_a_question() {
        for (yes, changes) in [(true, true), (false, false)] {
            let shown = RefCell::new(false);
            let approval = approve_with(
                true,
                yes,
                changes,
                || {
                    *shown.borrow_mut() = true;
                    Ok(())
                },
                || panic!("no second question"),
            )
            .unwrap();
            assert!(*shown.borrow());
            assert!(approval.proceed && approval.extensions);
        }
    }

    #[test]
    fn a_failed_preview_cannot_authorise_activation() {
        let error = approve_with(
            true,
            false,
            true,
            || Err(Failure::new("NATIVE_UPGRADE_FAILED", "output failed", 1)),
            || panic!("a failed screen cannot ask for approval"),
        )
        .unwrap_err();
        assert_eq!(error.message, "output failed");
    }
    #[test]
    fn plain_preview_keeps_versions_migration_evidence_and_the_plan_bytes() {
        let document = serde_json::json!({"products": [
            {"product": "ops", "status": "unchanged", "version": "0.1.0-alpha.1"},
            {"product": "remote", "status": "skippedPinned", "version": "0.1.0-alpha.2"}
        ], "pending": [{"product": "colab", "version": "0.1.0-alpha.1", "selected": "0.1.0-alpha.2"}]});
        let bytes = serde_json::to_vec(&document).unwrap();
        let plan = upgrade_all::Plan::parse(&bytes).unwrap();
        let rows = rows(
            "5.0.0-alpha.1",
            Some("5.0.0-alpha.2"),
            false,
            MigrationPreview::Required,
            &plan,
        );
        let mut output = Vec::new();
        write_rows(&mut output, Terminal::PLAIN, &rows).unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("5.0.0-alpha.1 → 5.0.0-alpha.2"));
        assert!(text.contains("migration needed (data kept)"));
        assert!(text.contains("migration unknown"));
        assert!(text.contains("up to date") && text.contains("pinned"));
        assert!(!text.contains('\u{1b}'));
        assert_eq!(serde_json::to_vec(&plan.document()).unwrap(), bytes);
    }

    #[test]
    fn narrow_preview_uses_the_shared_table_and_never_interprets_controls() {
        let rows = vec![Row {
            name: "tmt".into(),
            versions: "1.0.0\u{1b}[31m".into(),
            detail: "migration unknown".into(),
        }];
        for width in [20, 40] {
            let mut output = Vec::new();
            write_rows(
                &mut output,
                Terminal {
                    width: Some(width),
                    ..Terminal::PLAIN
                },
                &rows,
            )
            .unwrap();
            let text = String::from_utf8(output).unwrap();
            assert!(!text.contains('\u{1b}'));
            assert!(
                text.lines()
                    .all(|line| line.chars().count() <= usize::from(width)),
                "{text}"
            );
        }
    }
}

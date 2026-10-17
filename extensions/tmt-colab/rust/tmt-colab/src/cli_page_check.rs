//! `tmt colab page check`, and the warnings `page create` and `page write` print from the same
//! check. The rules live in `tmt_colab::page_check`; this is the command's adaptation to it.

use std::{
    io::Write,
    path::PathBuf,
    time::{Duration, Instant},
};

use clap::{Arg, ArgAction, ArgMatches, Command};
use serde_json::{Value, json};
use tmt_cli_style::{CommandSpec, Example, OutputModes};
use tmt_colab::{
    Result,
    decoder::Decoder,
    keyring::Keyring,
    limits::{
        ATTACHMENT_REFS, DATA_IMAGE_BYTES, DATA_IMAGES_TOTAL_BYTES, GROUP_LABEL_POINTS,
        GROUP_QUESTIONS, PAGE_DEPTH, QUESTION_AGENT_POINTS, QUESTION_CHOICE_POINTS,
        QUESTION_CHOICES, QUESTION_PAGE, QUESTION_TITLE_POINTS, READING_PAGE, REVIEW_PAGE,
    },
    page_check::{self, Finding, Report},
    store::Store,
};

use crate::cli_management::{ManagementFault, fail};

const CHECK: CommandSpec = CommandSpec {
    name: "check",
    summary: "Check page markup before publishing",
    examples: &[Example {
        command: "tmt colab page check page.html --json",
        note: "Report ignored question blocks and other markup problems with a line and a reason code",
    }],
    outputs: OutputModes::HumanAndJson,
    details: "Offline and fast: reads one file (or - for bounded UTF-8 stdin) and needs no running serve and no local state. Reports every block the browser ignores (question blocks, with a reason code and line), data: images that make the page heavy and colab-attachment: references that cannot show. Exits 1 when it finds anything. With --page, references are also resolved against that page's current document attachments in the local store. page create and page write run the same check and print the findings as warnings.",
};

/// Where `page check` and its siblings look for the attachments a reference names.
pub enum Attachments<'a> {
    /// Check the reference grammar and count only.
    Unchecked,
    /// A page being created has no attachments yet: every reference is unresolved.
    NewPage,
    /// Resolve against this page's current document attachments.
    Page {
        store: &'a Store,
        key: &'a Keyring,
        page: &'a str,
        decoder: &'a mut Decoder,
    },
}

pub fn command() -> Command {
    tmt_cli_style::command(&CHECK)
        .arg(
            Arg::new("file")
                .required(true)
                .index(1)
                .value_name("file|-")
                .value_parser(clap::value_parser!(PathBuf))
                .help("Page source to check"),
        )
        .arg(
            crate::cli_grammar::page_option()
                .help("Also resolve colab-attachment: references against this page"),
        )
}

/// `--strict` on create and write: refuse, before anything is committed, when the check finds
/// anything.
pub fn strict() -> Arg {
    Arg::new("strict")
        .long("strict")
        .action(ArgAction::SetTrue)
        .help("Refuse to publish when the page markup check finds a problem")
}

fn kib(bytes: usize) -> usize {
    bytes / 1024
}

/// The sentence for one reason code. The code is the stable part; this wording may improve.
fn message(code: &str) -> String {
    match code {
        "page-too-deep" => format!(
            "The page nests deeper than {PAGE_DEPTH} elements, where a browser stops nesting; nothing after this element was checked."
        ),
        "question-page-limit" => format!(
            "More than {QUESTION_PAGE} questions on the page: the browser shows none of them. Keep at most {QUESTION_PAGE}."
        ),
        "question-id-invalid" => "data-id must be a lowercase UUID such as 10000000-0000-4000-8000-000000000001; this question is ignored.".into(),
        "question-id-duplicate" => "Another question or group uses the same data-id, so every holder is ignored. Give each its own UUID.".into(),
        "question-nested" => "A question inside another question is ignored. Put it beside the outer one.".into(),
        "question-heading-missing" => "No direct child has data-question to hold the question text, so this question is ignored.".into(),
        "question-heading-multiple" => "More than one direct child has data-question. Keep exactly one.".into(),
        "question-title-empty" => "The data-question text is empty.".into(),
        "question-title-too-long" => format!("The data-question text is over {QUESTION_TITLE_POINTS} characters."),
        "question-title-control" => "The data-question text contains a control character.".into(),
        "question-kind-invalid" => "data-kind must be single, multiple, short or long.".into(),
        "question-choices-too-many" => format!("More than {QUESTION_CHOICES} data-choice children."),
        "question-choice-empty" => "A data-choice is empty.".into(),
        "question-choice-too-long" => format!("A data-choice is over {QUESTION_CHOICE_POINTS} characters."),
        "question-choice-control" => "A data-choice contains a control character.".into(),
        "question-choice-duplicate" => "Two data-choice texts are the same once whitespace is normalized.".into(),
        "question-choices-missing" => "A single or multiple question needs at least one direct data-choice child.".into(),
        "question-choices-unexpected" => "A short or long question takes no data-choice children.".into(),
        "question-other-unexpected" => "data-other applies only to single and multiple questions.".into(),
        "question-agent-invalid" => format!("data-agent is over {QUESTION_AGENT_POINTS} characters or contains a control character."),
        "question-group-child-invalid" => "A question in a tmt-questions group must be a direct child of it; inside another element it is ignored and never becomes a lone question.".into(),
        "group-id-invalid" => "data-id must be a lowercase UUID; this group and everything in it is ignored.".into(),
        "group-id-duplicate" => "Another group or question uses the same data-id, so every holder is ignored. Give each its own UUID.".into(),
        "group-nested" => "A tmt-questions group inside another group or a question is ignored with everything in it. Put it beside the outer one.".into(),
        "group-title-missing" => "A group needs exactly one direct <p data-title> child; none was found, so the group is ignored.".into(),
        "group-title-multiple" => "A group needs exactly one direct <p data-title> child; more than one was found.".into(),
        "group-title-empty" => "The group's data-title text is empty.".into(),
        "group-title-too-long" => format!("The group's data-title text is over {QUESTION_TITLE_POINTS} characters."),
        "group-title-control" => "The group's data-title text contains a control character.".into(),
        "group-questions-too-many" => format!("More than {GROUP_QUESTIONS} direct tmt-question children; the group is ignored."),
        "group-empty" => "No question of this group is shown, so the group is ignored.".into(),
        "group-label-too-long" => format!("data-label is over {GROUP_LABEL_POINTS} characters; the tab shows Q<n> instead."),
        "group-label-control" => "data-label contains a control character; the tab shows Q<n> instead.".into(),
        "group-form-invalid" | "question-form-invalid" => "data-form has one value, chip (lowercase); the block is shown as the full block.".into(),
        "question-form-member" => "data-form on a question inside a group is ignored; put it on the tmt-questions group.".into(),
        "reading-page-limit" => format!("More than {READING_PAGE} reading anchors: none supplies reading controls. Keep at most {READING_PAGE}."),
        "reading-id-invalid" => "A reading data-id must be a canonical lowercase UUID.".into(),
        "reading-id-duplicate" => "Another reading, review, question or group holds this UUID; use a unique data-id.".into(),
        "reading-nested" => "A reading anchor cannot contain or sit inside another reading, review, question or question group.".into(),
        "reading-heading-missing" => "A reading anchor needs exactly one immediate data-title child.".into(),
        "reading-heading-multiple" => "A reading anchor has more than one immediate data-title child.".into(),
        "reading-title-empty" => "The reading title is empty.".into(),
        "reading-title-too-long" => format!("The reading title is over {QUESTION_TITLE_POINTS} code points."),
        "reading-title-control" => "The reading title contains a control character.".into(),
        "reading-agent-invalid" => format!("The reading data-agent is required, at most {QUESTION_AGENT_POINTS} code points, without control characters."),
        "reading-why-invalid" => format!("The reading data-why is required, at most {QUESTION_TITLE_POINTS} code points, without control characters."),
        "review-page-limit" => format!(
            "More than {REVIEW_PAGE} review sections on the page: the browser shows none of them. Keep at most {REVIEW_PAGE}."
        ),
        "review-id-invalid" => "data-id must be a lowercase UUID such as 10000000-0000-4000-8000-000000000301; this section is ignored.".into(),
        "review-id-duplicate" => "Another section, group or question uses the same data-id, so every holder is ignored. Give each its own UUID.".into(),
        "review-nested" => "A tmt-review inside another section, a question or a group is ignored. Put it beside the outer one.".into(),
        "review-heading-missing" => "No direct child has data-title to hold the section heading, so this section is ignored.".into(),
        "review-heading-multiple" => "More than one direct child has data-title. Keep exactly one.".into(),
        "review-title-empty" => "The section's data-title text is empty.".into(),
        "review-title-too-long" => format!("The section's data-title text is over {QUESTION_TITLE_POINTS} characters."),
        "review-title-control" => "The section's data-title text contains a control character.".into(),
        "review-agent-invalid" => format!("data-agent is over {QUESTION_AGENT_POINTS} characters or contains a control character."),
        "question-marker-misplaced" => "data-question and data-choice count only on direct children of a tmt-question; here the element is plain content.".into(),
        "image-data-oversized" => format!(
            "This data: image is over {} KiB. Attach the file and reference it with colab-attachment: instead.",
            kib(DATA_IMAGE_BYTES)
        ),
        "image-data-total" => format!(
            "The data: images on this page add up to more than {} KiB.",
            kib(DATA_IMAGES_TOTAL_BYTES)
        ),
        "attachment-ref-invalid" => "A colab-attachment: reference is the scheme plus 8 lowercase hex digits or a full attachment ID.".into(),
        "attachment-ref-too-many" => format!(
            "More than {ATTACHMENT_REFS} distinct colab-attachment: references; one render resolves at most {ATTACHMENT_REFS}."
        ),
        "attachment-ref-unresolved" => "No current document attachment of this page matches this reference, so a placeholder shows.".into(),
        "attachment-ref-ambiguous" => "This reference matches more than one current document attachment; use the full attachment ID.".into(),
        _ => "See the reason code in the colab-v1 page markup check.".into(),
    }
}

fn finding_json(finding: &Finding) -> Value {
    let mut value = json!({
        "code": finding.code,
        "severity": finding.severity.as_str(),
        "element": finding.element,
        "ordinal": finding.ordinal,
        "id": finding.id,
        "line": finding.line,
        "message": message(finding.code),
    });
    if let Some(skipped) = finding.questions_skipped {
        value["questionsSkipped"] = json!(skipped);
    }
    value
}

fn findings_json(findings: &[Finding]) -> Value {
    Value::Array(findings.iter().map(finding_json).collect())
}

fn report_json(report: &Report, bytes: usize) -> Value {
    json!({
        "format": "tmt-colab-page-check",
        "version": 1,
        "ok": report.findings.is_empty(),
        "bytes": bytes,
        "questions": {
            "admitted": report.questions.len(),
            "ignored": report.question_blocks.saturating_sub(report.questions.len()),
        },
        "readings": {
            "admitted": report.readings.len(),
            "ignored": report.reading_blocks.saturating_sub(report.readings.len()),
        },
        "reviews": {
            "admitted": report.reviews.len(),
            "ignored": report.review_blocks.saturating_sub(report.reviews.len()),
        },
        "findings": findings_json(&report.findings),
    })
}

fn describe(finding: &Finding) -> String {
    let id = finding
        .id
        .as_deref()
        .map_or_else(String::new, |id| format!(" ({id})"));
    let skipped = finding.questions_skipped.map_or_else(String::new, |count| {
        let plural = if count == 1 { "" } else { "s" };
        format!(" {count} question{plural} skipped with it.")
    });
    format!(
        "{} <{}> #{}{}: {}{}",
        finding.code,
        finding.element,
        finding.ordinal,
        id,
        message(finding.code),
        skipped
    )
}

fn refusal(report: &Report, bytes: usize, what: &str) -> Box<dyn std::error::Error + Send + Sync> {
    let count = report.findings.len();
    let plural = if count == 1 { "" } else { "s" };
    Box::new(ManagementFault {
        code: "COLAB_PAGE_CHECK",
        message: format!(
            "{what} has {count} markup problem{plural}; the first is on line {}.",
            report.findings[0].line
        ),
        correlation: report_json(report, bytes),
        source: None,
    })
}

/// Run the check on one source, resolving references as `attachments` says.
pub fn inspect(source: &str, attachments: Attachments<'_>) -> Result<Report> {
    let mut report = page_check::check(source)
        .map_err(|_| fail("COLAB_CAPACITY", "Page source is over the page size cap."))?;
    let tokens: Vec<String> = report.references.iter().map(|r| r.token.clone()).collect();
    match attachments {
        Attachments::Unchecked => {}
        Attachments::NewPage => report.resolve_references(&vec![0; tokens.len()]),
        Attachments::Page {
            store,
            key,
            page,
            decoder,
        } if !tokens.is_empty() => {
            let tokens: Vec<&str> = tokens.iter().map(String::as_str).collect();
            let deadline = Instant::now() + Duration::from_secs(15);
            let matches = tmt_colab::attachments::document_matches(
                store, key, page, &tokens, decoder, deadline,
            )?;
            report.resolve_references(&matches);
        }
        Attachments::Page { .. } => {}
    }
    Ok(report)
}

/// The check `page create` and `page write` run before publishing: under `--strict` any finding
/// refuses the command before anything is committed; otherwise the findings come back to be
/// printed as warnings after the page is published.
pub fn gate(source: &str, args: &ArgMatches, attachments: Attachments<'_>) -> Result<Vec<Finding>> {
    let report = inspect(source, attachments)?;
    if args.get_flag("strict") && !report.findings.is_empty() {
        return Err(refusal(&report, source.len(), "The page"));
    }
    Ok(report.findings)
}

/// Warnings on stderr in human mode, for a command that already published.
pub fn print_warnings(findings: &[Finding]) -> Result<()> {
    for finding in findings {
        let mut err = tmt_cli_style::stream::stderr();
        let terminal = err.terminal();
        tmt_cli_style::message::warning(
            &mut err,
            terminal,
            &format!("line {}: {}", finding.line, describe(finding)),
            None,
        )?;
    }
    Ok(())
}

/// The `warnings` member of a create or write result: always an array, empty when clean.
pub fn warnings_json(findings: &[Finding]) -> Value {
    findings_json(findings)
}

/// `tmt colab page check`; `store` is the local state, present only when `--page` needs it.
pub fn run(state: Option<(&Store, &Keyring, &str, &mut Decoder)>, args: &ArgMatches) -> Result<()> {
    let file = args.get_one::<PathBuf>("file").expect("required file");
    let source = crate::page_source(file)?;
    let attachments = match state {
        Some((store, key, page, decoder)) => Attachments::Page {
            store,
            key,
            page,
            decoder,
        },
        None => Attachments::Unchecked,
    };
    let report = inspect(&source, attachments)?;
    let name = if file.as_os_str() == "-" {
        "stdin".to_owned()
    } else {
        file.display().to_string()
    };
    if args.get_flag("json") {
        if report.findings.is_empty() {
            let mut out = tmt_cli_style::stream::stdout(true);
            writeln!(out, "{}", report_json(&report, source.len()))?;
            return Ok(());
        }
        // The shared failure path prints this report with the error attached.
        return Err(refusal(&report, source.len(), &name));
    }
    let mut out = tmt_cli_style::stream::stdout(false);
    let terminal = out.terminal();
    let ignored = report
        .question_blocks
        .saturating_sub(report.questions.len());
    let mut rows = vec![
        ("file", name.clone()),
        (
            "questions",
            format!("{} shown, {} ignored", report.questions.len(), ignored),
        ),
    ];
    if report.reading_blocks > 0 {
        rows.push((
            "readings",
            format!(
                "{} shown, {} ignored",
                report.readings.len(),
                report.reading_blocks.saturating_sub(report.readings.len())
            ),
        ));
    }
    // Only a page that has sections mentions them, so a page without any reads as before.
    if report.review_blocks > 0 {
        rows.push((
            "reviews",
            format!(
                "{} shown, {} ignored",
                report.reviews.len(),
                report.review_blocks.saturating_sub(report.reviews.len())
            ),
        ));
    }
    let lines: Vec<String> = report
        .findings
        .iter()
        .map(|f| format!("line {}", f.line))
        .collect();
    for (finding, line) in report.findings.iter().zip(&lines) {
        rows.push((line.as_str(), describe(finding)));
    }
    tmt_cli_style::detail::write(&mut out, terminal, "PAGE CHECK", &rows)?;
    if report.findings.is_empty() {
        Ok(())
    } else {
        Err(refusal(&report, source.len(), &name))
    }
}

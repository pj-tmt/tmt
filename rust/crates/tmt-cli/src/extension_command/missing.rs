//! A human-only offer over the existing official extension installer.

use crate::{
    caller_context, consent, extension_install_command, invocation::OutputMode, output::Failure,
};
use std::{
    cell::RefCell,
    ffi::OsString,
    io::{self, Write},
    time::{Duration, Instant},
};
use tmt_adapters::native_install::{self, Product, ProgressEvent, ReleasePreview};
use tmt_cli_style::{
    Interaction, Terminal, Token,
    progress::{Progress, State, Step, StepProgress, Transfer},
};

pub(super) fn official(name: &str) -> Option<Product> {
    extension_install_command::INSTALLABLE_EXTENSIONS
        .iter()
        .copied()
        .find(|product| {
            product
                .links()
                .iter()
                .any(|link| link.strip_prefix("tmt-") == Some(name))
        })
}

fn missing(product: Product) -> Failure {
    Failure::new(
        "EXTENSION_NOT_INSTALLED",
        format!(
            "tmt {} needs the official extension \"{}\", which is not installed.",
            product.as_str(),
            product.as_str()
        ),
        3,
    )
    .suggestion(format!("tmt extension install {} --yes", product.as_str()))
}

pub(super) fn execute(
    product: Product,
    original: &[OsString],
    mode: OutputMode,
    help: bool,
) -> io::Result<u8> {
    let interaction = Interaction::detect(mode.json);
    // Help is read-only, including help for an absent official extension.
    if help || !caller_context::may_offer_install(interaction) {
        return missing(product).publish(mode);
    }
    let result = offer(product, original, interaction);
    match result {
        Ok(()) => Ok(0),
        Err(failure) => failure.publish(mode),
    }
}

fn offer(product: Product, original: &[OsString], interaction: Interaction) -> Result<(), Failure> {
    // A non-UTF8 argument cannot be faithfully printed as a runnable command.
    let command = original_command(original).ok_or_else(|| missing(product))?;
    let preview =
        tmt_core::native_install::native_target(std::env::consts::OS, std::env::consts::ARCH)
            .and_then(|target| native_install::preview_release(product, target).ok());
    let prefix = native_install::default_install_prefix().map_err(|error| {
        Failure::new("EXTENSION_INSTALL_FAILED", error.to_string(), 1).caused_by(error)
    })?;
    let mut output = tmt_cli_style::stream::stdout(false);
    let terminal = output.terminal();
    write_question(
        &mut output,
        terminal,
        product,
        preview.as_ref(),
        &tmt_cli_style::value::home_path(&prefix, std::env::home_dir().as_deref()),
    )
    .map_err(io_failure)?;
    let accepted = consent::ask_defaulting_yes(
        &mut output,
        false,
        OutputMode::default(),
        consent::Consent {
            code: "EXTENSION_NOT_INSTALLED",
            refusal: "Installation requires a human terminal.",
            question: "  Install it now",
            declined: "  No changes made.",
        },
        io_failure,
    )?;
    if !accepted {
        return Ok(());
    }
    drop(output);
    let presentation = RefCell::new(InstallProgress::new(product, interaction));
    let observed = |event: ProgressEvent<'_>| presentation.borrow_mut().observe(event);
    let (document, notices) = match extension_install_command::install_offered(product, &observed) {
        Ok(document) => document,
        Err(failure) => {
            presentation.borrow_mut().failed();
            return Err(failure);
        }
    };
    let warning = document["skills"]["status"] == "warning";
    presentation.borrow_mut().settled(warning);
    notices.write_notices().map_err(io_failure)?;
    let mut output = tmt_cli_style::stream::stdout(false);
    let terminal = output.terminal();
    let version = document["version"].as_str().unwrap_or("unknown version");
    let agents = document["skills"]["published"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| item["agent"].as_str())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        .join(", ");
    let skills = if warning {
        "skills need attention".into()
    } else if agents.is_empty() {
        "bundled skills settled".into()
    } else {
        format!("skills for {agents}")
    };
    tmt_cli_style::message::success(
        &mut output,
        terminal,
        &format!(
            "{} {version} installed   verified · {skills}",
            product.as_str()
        ),
    )
    .map_err(io_failure)?;
    writeln!(
        output,
        "\n  Run it again: {}",
        terminal.paint(Token::Literal, &command)
    )
    .map_err(io_failure)
}

fn original_command(original: &[OsString]) -> Option<String> {
    let words = original
        .iter()
        .map(|word| {
            word.to_str()
                .filter(|text| !text.chars().any(char::is_control))
                .map(crate::output::shell_word)
        })
        .collect::<Option<Vec<_>>>()?;
    Some(format!("tmt {}", words.join(" ")))
}

fn write_question(
    out: &mut impl Write,
    terminal: Terminal,
    product: Product,
    preview: Option<&ReleasePreview>,
    prefix: &str,
) -> io::Result<()> {
    let name = product.as_str();
    writeln!(
        out,
        "\n  {} {} comes from the official {} extension, which is not installed.\n",
        terminal.paint(
            tmt_cli_style::mark::Mark::Decision.token(),
            tmt_cli_style::mark::Mark::Decision.symbol()
        ),
        terminal.paint(Token::Literal, &format!("tmt {name}")),
        terminal.paint(Token::Literal, name)
    )?;
    let version = preview.map_or("version unknown", |preview| preview.version.as_str());
    let size = preview.map_or_else(
        || "size unknown".into(),
        |preview| tmt_cli_style::progress::bytes(preview.archive_bytes),
    );
    writeln!(
        out,
        "    {name} {version}\n    {}\n",
        terminal.paint(
            Token::Dim,
            &format!(
                "verified release · {} · {size} · bundled agent skills included",
                tmt_cli_style::table::escape(prefix)
            )
        )
    )
}

fn io_failure(error: io::Error) -> Failure {
    Failure::new(
        "EXTENSION_IO_ERROR",
        "Could not present extension installation.",
        1,
    )
    .caused_by(error)
}

struct InstallProgress {
    progress: Progress,
    product: Product,
    started: Instant,
    last_draw: Option<Instant>,
    step: Step,
    output_failed: bool,
    transfer: Option<Transfer>,
}

impl InstallProgress {
    fn new(product: Product, interaction: Interaction) -> Self {
        Self {
            progress: Progress::new(
                &format!("Installing {}", product.as_str()),
                interaction.stdout && !interaction.dumb,
            ),
            product,
            started: Instant::now(),
            last_draw: None,
            step: Step::Download,
            output_failed: false,
            transfer: None,
        }
    }

    fn observe(&mut self, event: ProgressEvent<'_>) {
        let (step, state, transfer, detail) = match event {
            ProgressEvent::Selected {
                version,
                archive_bytes,
            } => {
                self.progress
                    .title(&format!("Installing {} {version}", self.product.as_str()));
                self.started = Instant::now();
                (
                    Step::Download,
                    State::Running,
                    Some(Transfer {
                        received_bytes: 0,
                        total_bytes: Some(archive_bytes),
                        elapsed: Duration::ZERO,
                    }),
                    String::new(),
                )
            }
            ProgressEvent::Download {
                received_bytes,
                total_bytes,
            } => (
                Step::Download,
                State::Running,
                Some(Transfer {
                    received_bytes,
                    total_bytes,
                    elapsed: self.started.elapsed(),
                }),
                String::new(),
            ),
            ProgressEvent::Downloaded => (
                Step::Download,
                State::Done,
                self.transfer,
                "archive downloaded".into(),
            ),
            ProgressEvent::Verifying => (
                Step::Verify,
                State::Running,
                None,
                "checking release record and archive".into(),
            ),
            ProgressEvent::Verified => (
                Step::Verify,
                State::Done,
                None,
                "checksum matches the release record; archive checked".into(),
            ),
            ProgressEvent::Activating => (
                Step::Activate,
                State::Running,
                None,
                "publishing verified release".into(),
            ),
            ProgressEvent::Activated => (
                Step::Activate,
                State::Running,
                None,
                "settling bundled agent skills".into(),
            ),
        };
        self.step = step;
        if step == Step::Download {
            self.transfer = transfer;
        }
        self.progress.update(StepProgress {
            step,
            state,
            transfer,
            detail: &detail,
        });
        let throttled = matches!(event, ProgressEvent::Download { .. })
            && self
                .last_draw
                .is_some_and(|last| last.elapsed() < Duration::from_millis(100));
        if !throttled {
            self.draw();
        }
    }

    fn settled(&mut self, warning: bool) {
        self.progress.update(StepProgress {
            step: Step::Activate,
            state: if warning { State::Warning } else { State::Done },
            transfer: None,
            detail: if warning {
                "installed; skills need attention"
            } else {
                "release and bundled skills ready"
            },
        });
        self.draw();
    }

    fn failed(&mut self) {
        self.progress.update(StepProgress {
            step: self.step,
            state: State::Failed,
            transfer: None,
            detail: "stopped; inspect the reported installation outcome",
        });
        self.draw();
    }

    fn draw(&mut self) {
        if self.output_failed {
            return;
        }
        let mut output = tmt_cli_style::stream::stdout(false);
        let terminal = output.terminal();
        self.output_failed = self.progress.write(&mut output, terminal).is_err();
        self.last_draw = Some(Instant::now());
    }
}

#[cfg(test)]
pub(crate) const PRINTED_HINTS: &[crate::cli_style_tests::HintSpec] = &[
    crate::cli_style_tests::HintSpec::core(
        "tmt extension install {} --yes",
        &[" --yes"],
        &[("{}", "ops")],
    ),
    crate::cli_style_tests::HintSpec::skipped(
        "tmt {} needs the official extension \"{}\", which is not installed.",
        "Names the missing external command, not a runnable command hint.",
    ),
    crate::cli_style_tests::HintSpec::skipped(
        "tmt {}",
        "Original opaque external argv is shell-quoted by original_command; extension grammar is external.",
    ),
    crate::cli_style_tests::HintSpec::skipped(
        "tmt {name}",
        "Names the missing external command; extension grammar is external.",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_arguments_are_quoted_and_terminal_controls_are_refused() {
        let words = ["ops", "sq", "two words", "owner's", ""].map(OsString::from);
        assert_eq!(
            original_command(&words).unwrap(),
            "tmt ops sq 'two words' 'owner'\\''s' ''"
        );
        assert!(original_command(&["ops".into(), "\x1b[2J".into()]).is_none());
    }
    #[test]
    fn missing_failure_has_the_exact_code_status_and_next_command() {
        for product in extension_install_command::INSTALLABLE_EXTENSIONS {
            assert_eq!(official(product.as_str()), Some(*product));
            let failure = missing(*product);
            assert_eq!(failure.code, "EXTENSION_NOT_INSTALLED");
            assert_eq!(
                failure.document()["error"]["suggestion"],
                format!("tmt extension install {} --yes", product.as_str())
            );
        }
        assert_eq!(official("opss"), None);
        assert_eq!(official("office"), None);
    }
    #[test]
    fn unavailable_preview_still_presents_unknown_version_and_size() {
        let mut output = Vec::new();
        write_question(&mut output, Terminal::PLAIN, Product::Ops, None, "~/.local").unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("version unknown"));
        assert!(text.contains("size unknown"));
        assert!(!text.contains("signature"));
    }
}

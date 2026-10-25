//! Official extension selection and ordered setup outcomes.

use crate::{
    extension_install_command::setup, invocation::OutputMode, native_progress::InstallProgress,
    output::Failure,
};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    io::{self, Read, Write},
};
use tmt_cli_style::{Interaction, Terminal, message, stream::Stream};

use tmt_adapters::native_install::Product;

pub(super) const PRODUCTS: [Product; 4] = [
    Product::Ops,
    Product::Digest,
    Product::Remote,
    Product::Colab,
];

/// Colab's dependency is visible in the selection; it is never an extra install.
#[derive(Clone, Debug)]
pub(super) struct Selection([bool; 4]);

impl Default for Selection {
    fn default() -> Self {
        Self([true; 4])
    }
}

impl Selection {
    pub(super) fn contains(&self, product: Product) -> bool {
        PRODUCTS
            .iter()
            .position(|item| *item == product)
            .is_some_and(|index| self.0[index])
    }

    pub(super) fn toggle(&mut self, product: Product) -> Option<&'static str> {
        let index = PRODUCTS.iter().position(|item| *item == product)?;
        self.0[index] = !self.0[index];
        if product == Product::Remote && !self.0[index] && self.0[3] {
            self.0[3] = false;
            return Some("Colab was unchecked because it needs Remote.");
        }
        if product == Product::Colab && self.0[index] && !self.0[2] {
            self.0[2] = true;
            return Some("Remote was checked because Colab needs it.");
        }
        None
    }
}

pub(super) fn dependency_ready(product: Product, remote_ready: bool) -> bool {
    product != Product::Colab || remote_ready
}

pub(super) struct Plan {
    pub(super) selection: Selection,
    rows: Vec<(Product, Result<setup::SetupState, Failure>)>,
}

impl Plan {
    pub(super) fn read() -> Self {
        Self {
            selection: Selection::default(),
            rows: PRODUCTS
                .into_iter()
                .map(|product| (product, setup::plan(product)))
                .collect(),
        }
    }

    pub(super) fn is_current(&self) -> bool {
        self.rows.iter().all(|(product, state)| {
            !self.selection.contains(*product)
                || state
                    .as_ref()
                    .is_ok_and(|state| state.installed && state.skills_current)
        })
    }

    fn rows(&self, output: &mut impl Write, cursor: Option<usize>) -> io::Result<()> {
        for (index, (product, state)) in self.rows.iter().enumerate() {
            let description = match state {
                Ok(state) if state.installed && state.skills_current => {
                    format!("{} ready", state.version.as_deref().unwrap_or(""))
                }
                Ok(state) if state.installed => "installed; bundled skills need setup".into(),
                Ok(_) => "install with bundled skills".into(),
                Err(_) => "needs attention; existing installation will be preserved".into(),
            };
            let label = if *product == Product::Colab {
                "colab (needs remote)"
            } else {
                product.as_str()
            };
            let line = format!(
                "{} [{}] {:7} {}",
                if cursor == Some(index) { ">" } else { " " },
                if self.selection.contains(*product) {
                    "x"
                } else {
                    " "
                },
                label,
                description
            );
            writeln!(
                output,
                "{}",
                if cursor.is_some() {
                    fitted(&line)
                } else {
                    line
                }
            )?;
        }
        Ok(())
    }

    /// Selection changes only memory. The one consent question follows after raw mode is restored.
    pub(super) fn select(
        &mut self,
        output: &mut Stream<impl Write>,
        yes: bool,
        mode: OutputMode,
    ) -> io::Result<bool> {
        writeln!(
            output,
            "Official extensions (bundled agent skills included)"
        )?;
        let terminal = output.terminal();
        for (product, state) in &self.rows {
            if let Err(error) = state {
                message::warning(
                    output,
                    terminal,
                    &format!("{}: {}", product.as_str(), error.message),
                    None,
                )?;
            }
        }
        let interaction = Interaction::detect(mode.json);
        if yes || interaction.view() != tmt_cli_style::interaction::Mode::Interactive {
            return self.rows(output, None).map(|()| true);
        }
        let _raw = RawMode::enter()?;
        let mut cursor = 0;
        let mut reason = "↑/↓ move · space select · Enter review · Ctrl-C cancel";
        let mut input = io::stdin().lock();
        let mut keys = Keys::default();
        loop {
            let mut screen = Vec::new();
            self.rows(&mut screen, Some(cursor))?;
            writeln!(screen, "{}", fitted(reason))?;
            let screen = String::from_utf8(screen).map_err(io::Error::other)?;
            write!(output, "{}", screen.replace('\n', "\r\n"))?;
            output.flush()?;
            let key = loop {
                let mut byte = [0];
                if input.read(&mut byte)? == 0 {
                    return Ok(false);
                }
                if let Some(key) = keys.feed(byte[0]) {
                    break key;
                }
            };
            match key {
                Key::Review => return Ok(true),
                Key::Cancel => return Ok(false),
                Key::Up => cursor = (cursor + PRODUCTS.len() - 1) % PRODUCTS.len(),
                Key::Down => cursor = (cursor + 1) % PRODUCTS.len(),
                Key::Toggle => {
                    reason = self
                        .selection
                        .toggle(PRODUCTS[cursor])
                        .unwrap_or("↑/↓ move · space select · Enter review · Ctrl-C cancel")
                }
            }
            crossterm::queue!(
                output,
                crossterm::cursor::MoveUp(5),
                crossterm::terminal::Clear(crossterm::terminal::ClearType::FromCursorDown)
            )?;
        }
    }

    pub(super) fn current_document(&self) -> Vec<Value> {
        self.rows.iter().map(|(product, state)| match state {
            Ok(state) => json!({"extension":product.as_str(), "selected": self.selection.contains(*product),
                "installed": state.installed, "version":state.version,
                "status":if self.selection.contains(*product) { "current" } else { "not_selected" },
                "skills":{"status":if state.skills_current { "current" } else { "pending" }}}),
            Err(error) => json!({"extension":product.as_str(), "selected":self.selection.contains(*product), "status":if self.selection.contains(*product) { "failed" } else { "not_selected" }, "error":error.document()["error"]}),
        }).collect()
    }

    pub(super) fn apply(
        &self,
        output: &mut impl Write,
        terminal: Terminal,
        mode: OutputMode,
    ) -> Result<Vec<Value>, Failure> {
        self.apply_with(output, terminal, mode, setup::apply)
    }

    fn apply_with(
        &self,
        output: &mut impl Write,
        terminal: Terminal,
        mode: OutputMode,
        mut install: impl FnMut(
            Product,
            tmt_adapters::native_install::ProgressObserver<'_>,
        ) -> Result<Value, Failure>,
    ) -> Result<Vec<Value>, Failure> {
        let interaction = Interaction::detect(mode.json);
        let mut rows = Vec::new();
        let mut remote_ready = false;
        for (product, planned) in &self.rows {
            if !self.selection.contains(*product) {
                rows.push(json!({"extension":product.as_str(), "selected":false, "status":"not_selected"}));
                continue;
            }
            let progress = RefCell::new(InstallProgress::new("Set up", *product, interaction));
            let result = if !dependency_ready(*product, remote_ready) {
                json!({"extension":product.as_str(),"selected":true,"status":"skipped",
                    "reason":"Remote is not ready; Colab needs Remote. Rerun: tmt setup --yes"})
            } else {
                let result = match planned {
                    Err(error) => Err(Failure::new(&error.code, &error.message, error.status)),
                    Ok(_) => install(*product, &|event| progress.borrow_mut().observe(event)),
                };
                match result {
                    Ok(mut document) => {
                        let warning = document["skills"]["status"] == "warning";
                        let changed = document["changed"] == true;
                        if changed {
                            progress.borrow_mut().settled(warning);
                        }
                        document["selected"] = json!(true);
                        document["status"] = json!(if warning {
                            "needs_attention"
                        } else if changed {
                            "installed"
                        } else {
                            "current"
                        });
                        if *product == Product::Remote {
                            remote_ready = true;
                        }
                        document
                    }
                    Err(error) => {
                        progress.borrow_mut().failed();
                        json!({"extension":product.as_str(),"selected":true,"status":"failed",
                            "error":error.document()["error"],"retry":"tmt setup --yes"})
                    }
                }
            };
            if !mode.json {
                let text = format!(
                    "{}: {}",
                    product.as_str(),
                    result["status"]
                        .as_str()
                        .unwrap_or("unknown")
                        .replace('_', " ")
                );
                if matches!(
                    result["status"].as_str(),
                    Some("failed" | "skipped" | "needs_attention")
                ) {
                    let detail = result["error"]["message"]
                        .as_str()
                        .or_else(|| result["reason"].as_str())
                        .or_else(|| result["skills"]["warning"]["message"].as_str())
                        .unwrap_or("Rerun: tmt setup --yes");
                    message::warning(output, terminal, &format!("{text}: {detail}"), None)
                        .map_err(super::failure)?;
                } else {
                    message::success(output, terminal, &text).map_err(super::failure)?;
                }
                // The installer owns whether these notices apply; setup preserves them.
                for field in ["restartHint", "retiredSquadHint"] {
                    if let Some(notice) = result[field].as_str() {
                        message::hint(output, terminal, notice).map_err(super::failure)?;
                    }
                }
            }
            rows.push(result);
        }
        Ok(rows)
    }
}

/// Leave one display cell spare so the terminal does not soft-wrap our fixed rows.
fn fitted(text: &str) -> String {
    let width = crossterm::terminal::size().map_or(80, |(width, _)| usize::from(width));
    tmt_cli_style::grid::fit(
        text,
        width.saturating_sub(1),
        tmt_cli_style::grid::Align::Left,
        tmt_cli_style::grid::Truncate::End,
    )
}

#[derive(Debug, PartialEq, Eq)]
enum Key {
    Up,
    Down,
    Toggle,
    Review,
    Cancel,
}

/// Only the two cursor sequences and explicit control keys affect selection.
#[derive(Default)]
struct Keys {
    escape: u8,
}
impl Keys {
    fn feed(&mut self, byte: u8) -> Option<Key> {
        if matches!(byte, 3 | 4) {
            self.escape = 0;
            return Some(Key::Cancel);
        }
        let state = self.escape;
        self.escape = 0;
        match (state, byte) {
            (0, 27) => {
                self.escape = 1;
                None
            }
            (1, b'[' | b'O') => {
                self.escape = 2;
                None
            }
            (2, b'A') => Some(Key::Up),
            (2, b'B') => Some(Key::Down),
            (0, b' ') => Some(Key::Toggle),
            (0, b'\r' | b'\n') => Some(Key::Review),
            _ => None,
        }
    }
}

struct RawMode;
impl RawMode {
    fn enter() -> io::Result<Self> {
        crossterm::terminal::enable_raw_mode()?;
        Ok(Self)
    }
}
impl Drop for RawMode {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

pub(super) fn failed(rows: &[Value]) -> bool {
    rows.iter().any(|row| {
        matches!(
            row["status"].as_str(),
            Some("failed" | "skipped" | "needs_attention")
        )
    })
}

#[cfg(test)]
pub(crate) const PRINTED_HINTS: &[crate::cli_style_tests::HintSpec] = &[
    crate::cli_style_tests::HintSpec::core(
        "Remote is not ready; Colab needs Remote. Rerun: tmt setup --yes",
        &[""],
        &[],
    ),
    crate::cli_style_tests::HintSpec::core("Rerun: tmt setup --yes", &[""], &[]),
    crate::cli_style_tests::HintSpec::core("tmt setup --yes", &[""], &[]),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_select_all_four_official_extensions_in_dependency_order() {
        let selection = Selection::default();
        assert_eq!(
            PRODUCTS.map(Product::as_str),
            ["ops", "digest", "remote", "colab"]
        );
        assert!(
            PRODUCTS
                .into_iter()
                .all(|product| selection.contains(product))
        );
    }

    #[test]
    fn unchecking_remote_also_unchecks_colab_with_an_explanation() {
        let mut selection = Selection::default();
        let reason = selection.toggle(Product::Remote);
        assert!(!selection.contains(Product::Remote));
        assert!(!selection.contains(Product::Colab));
        assert!(selection.contains(Product::Ops));
        assert!(selection.contains(Product::Digest));
        assert_eq!(reason, Some("Colab was unchecked because it needs Remote."));
    }

    #[test]
    fn checking_colab_selects_remote_but_unchecking_colab_keeps_remote() {
        let mut selection = Selection::default();
        selection.toggle(Product::Remote);
        assert_eq!(
            selection.toggle(Product::Colab),
            Some("Remote was checked because Colab needs it.")
        );
        assert!(selection.contains(Product::Remote));
        assert!(selection.contains(Product::Colab));
        assert_eq!(selection.toggle(Product::Colab), None);
        assert!(selection.contains(Product::Remote));
        assert!(!selection.contains(Product::Colab));
    }

    #[test]
    fn a_remote_failure_blocks_only_colab_and_healthy_remote_satisfies_it() {
        assert!(!dependency_ready(Product::Colab, false));
        assert!(dependency_ready(Product::Colab, true));
        for product in [Product::Ops, Product::Digest, Product::Remote] {
            assert!(dependency_ready(product, false));
        }
    }
    fn missing_plan() -> Plan {
        Plan {
            selection: Selection::default(),
            rows: PRODUCTS
                .into_iter()
                .map(|product| {
                    (
                        product,
                        Ok(setup::SetupState {
                            installed: false,
                            skills_current: false,
                            version: None,
                        }),
                    )
                })
                .collect(),
        }
    }

    #[test]
    fn partial_apply_preserves_independent_success_and_skips_dependent_colab() {
        let plan = missing_plan();
        let mut called = Vec::new();
        let mode = OutputMode { json: true };
        let mut output = Vec::new();
        let rows = plan.apply_with(&mut output, Terminal::PLAIN, mode, |product, _| {
            called.push(product);
            if product == Product::Remote { return Err(Failure::new("EXTENSION_INSTALL_FAILED", "fixture refuses before activation", 1)); }
            Ok(json!({"extension":product.as_str(),"installed":true,"changed":true,"skills":{"status":"current"}}))
        }).unwrap();
        assert_eq!(called, [Product::Ops, Product::Digest, Product::Remote]);
        assert_eq!(
            rows.iter()
                .map(|row| row["status"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["installed", "installed", "failed", "skipped"]
        );
        assert!(failed(&rows));
        assert!(
            output.is_empty(),
            "JSON must contain no progress or human lines"
        );
    }

    #[test]
    fn unselected_extensions_are_never_installed() {
        let mut plan = missing_plan();
        plan.selection.toggle(Product::Remote);
        let mut called = Vec::new();
        let mut output = Vec::new();
        let rows = plan.apply_with(&mut output, Terminal::PLAIN, OutputMode { json:true }, |product, _| {
            called.push(product);
            Ok(json!({"extension":product.as_str(),"installed":true,"changed":true,"skills":{"status":"current"}}))
        }).unwrap();
        assert_eq!(called, [Product::Ops, Product::Digest]);
        assert_eq!(rows[2]["status"], "not_selected");
        assert_eq!(rows[3]["status"], "not_selected");
        assert!(!failed(&rows));
    }

    #[test]
    fn the_installer_notices_remain_visible_in_the_setup_summary() {
        let plan = missing_plan();
        let mut output = Vec::new();
        plan.apply_with(&mut output, Terminal::PLAIN, OutputMode { json:false }, |product, _| {
            Ok(json!({"extension":product.as_str(),"installed":true,"changed":false,
                "skills":{"status":"current"},"restartHint":"Restart the open board to use this release",
                "retiredSquadHint":"The retired directory was kept"}))
        }).unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("Restart the open board to use this release"));
        assert!(text.contains("The retired directory was kept"));
    }

    #[test]
    fn cursor_sequences_are_bounded_and_only_explicit_keys_change_selection() {
        let mut keys = Keys::default();
        let decoded: Vec<_> = b"\x1b[A\x1b[B \r\x03"
            .iter()
            .filter_map(|byte| keys.feed(*byte))
            .collect();
        assert_eq!(
            decoded,
            [Key::Up, Key::Down, Key::Toggle, Key::Review, Key::Cancel]
        );
        assert_eq!(keys.feed(b'x'), None);
        assert_eq!(keys.feed(4), Some(Key::Cancel));
    }
}

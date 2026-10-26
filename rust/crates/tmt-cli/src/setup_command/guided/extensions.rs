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

use setup::PRODUCTS;

/// Colab's dependency is visible in the selection; it is never an extra install.
#[derive(Clone, Debug)]
pub(super) struct Selection([bool; PRODUCTS.len()]);

impl Default for Selection {
    fn default() -> Self {
        Self([true; PRODUCTS.len()])
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
        let remote = PRODUCTS.iter().position(|p| *p == Product::Remote)?;
        let colab = PRODUCTS.iter().position(|p| *p == Product::Colab)?;
        if product == Product::Remote && !self.0[remote] && self.0[colab] {
            self.0[colab] = false;
            return Some("Colab was unchecked because it needs Remote.");
        }
        if product == Product::Colab && self.0[colab] && !self.0[remote] {
            self.0[remote] = true;
            return Some("Remote was checked because Colab needs it.");
        }
        None
    }
}

pub(super) fn dependency_ready(product: Product, remote_ready: bool) -> bool {
    !setup::dependencies(product).contains(&Product::Remote) || remote_ready
}

pub(super) struct Plan {
    pub(super) selection: Selection,
    rows: Vec<(Product, Result<setup::SetupState, Failure>)>,
}

impl Plan {
    pub(super) fn read() -> Self {
        Self {
            selection: Selection::default(),
            rows: match tmt_adapters::native_install::default_install_prefix() {
                Ok(prefix) => setup::inventory(&prefix)
                    .iter()
                    .map(|entry| (entry.product, entry.plan()))
                    .collect(),
                Err(error) => PRODUCTS
                    .into_iter()
                    .map(|product| {
                        (
                            product,
                            Err(Failure::new(
                                "EXTENSION_INSTALL_FAILED",
                                error.to_string(),
                                1,
                            )),
                        )
                    })
                    .collect(),
            },
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
        let mut labels = Vec::new();
        self.rows(&mut labels, None)?;
        let labels = String::from_utf8(labels)
            .map_err(io::Error::other)?
            .lines()
            .map(|line| line.get(6..).unwrap_or(line).to_owned())
            .collect::<Vec<_>>();
        let mut selected = PRODUCTS
            .iter()
            .map(|p| self.selection.contains(*p))
            .collect::<Vec<_>>();
        let accepted = choose(output, &labels, &mut selected, |index, selected| {
            let reason = self.selection.toggle(PRODUCTS[index]);
            for (index, product) in PRODUCTS.iter().enumerate() {
                selected[index] = self.selection.contains(*product);
            }
            reason.map(str::to_owned)
        })?;
        Ok(accepted)
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
            let result = if interrupted(&rows) {
                json!({"extension":product.as_str(),"selected":true,"status":"skipped",
                    "reason":"Setup was interrupted. Rerun: tmt setup --yes"})
            } else if !dependency_ready(*product, remote_ready) {
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
                        json!({"extension":product.as_str(),"selected":true,"status":if error.status == 130 {"interrupted"} else {"failed"},
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
                    Some("failed" | "interrupted" | "skipped" | "needs_attention")
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

/// Shared read-only selection screen. Raw mode is restored before the consent question.
pub(crate) fn choose(
    output: &mut Stream<impl Write>,
    labels: &[String],
    selected: &mut [bool],
    mut toggle: impl FnMut(usize, &mut [bool]) -> Option<String>,
) -> io::Result<bool> {
    if labels.is_empty() {
        return Ok(true);
    }
    let _raw = RawMode::enter()?;
    let mut cursor = 0;
    let mut reason = "↑/↓ move · space select · Enter review · Ctrl-C cancel".to_owned();
    let mut input = io::stdin().lock();
    let mut keys = Keys::default();
    loop {
        for (index, label) in labels.iter().enumerate() {
            write!(
                output,
                "{}\r\n",
                fitted(&format!(
                    "{} [{}] {}",
                    if cursor == index { ">" } else { " " },
                    if selected[index] { "x" } else { " " },
                    label
                ))
            )?;
        }
        write!(output, "{}\r\n", fitted(&reason))?;
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
            Key::Up => cursor = (cursor + labels.len() - 1) % labels.len(),
            Key::Down => cursor = (cursor + 1) % labels.len(),
            Key::Toggle => {
                reason = toggle(cursor, selected).unwrap_or_else(|| {
                    "↑/↓ move · space select · Enter review · Ctrl-C cancel".into()
                })
            }
        }
        crossterm::queue!(
            output,
            crossterm::cursor::MoveUp((labels.len() + 1) as u16),
            crossterm::terminal::Clear(crossterm::terminal::ClearType::FromCursorDown)
        )?;
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

pub(super) fn interrupted(rows: &[Value]) -> bool {
    rows.iter().any(|row| row["status"] == "interrupted")
}

pub(super) fn failed(rows: &[Value]) -> bool {
    rows.iter().any(|row| {
        matches!(
            row["status"].as_str(),
            Some("failed" | "interrupted" | "skipped" | "needs_attention")
        )
    })
}

#[cfg(test)]
pub(crate) const PRINTED_HINTS: &[crate::cli_style_tests::HintSpec] = &[
    crate::cli_style_tests::HintSpec::core(
        "Setup was interrupted. Rerun: tmt setup --yes",
        &[""],
        &[],
    ),
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
            ["ops", "remote", "colab", "digest"]
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
        for product in [Product::Ops, Product::Remote, Product::Digest] {
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
        assert_eq!(called, [Product::Ops, Product::Remote, Product::Digest]);
        assert_eq!(
            rows.iter()
                .map(|row| row["status"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["installed", "failed", "skipped", "installed"]
        );
        assert!(failed(&rows));
        assert!(
            output.is_empty(),
            "JSON must contain no progress or human lines"
        );
    }

    #[test]
    fn ordinary_failure_continues_with_later_independent_products() {
        let plan = missing_plan();
        let mut called = Vec::new();
        let rows = plan.apply_with(&mut Vec::new(), Terminal::PLAIN, OutputMode { json:true }, |product, _| {
            called.push(product);
            if product == Product::Ops {
                return Err(Failure::new("EXTENSION_INSTALL_FAILED", "fixture download failure", 1));
            }
            Ok(json!({"extension":product.as_str(),"installed":true,"changed":true,"skills":{"status":"current"}}))
        }).unwrap();
        assert_eq!(called, PRODUCTS);
        assert_eq!(rows[0]["status"], "failed");
        assert!(rows[1..].iter().all(|row| row["status"] == "installed"));
        assert!(!interrupted(&rows));
    }

    #[test]
    fn interruption_keeps_completed_outcomes_and_stops_later_installation() {
        let plan = missing_plan();
        let mut called = Vec::new();
        let mut output = Vec::new();
        let rows = plan.apply_with(&mut output, Terminal::PLAIN, OutputMode { json:true }, |product, _| {
            called.push(product);
            if product == Product::Remote {
                return Err(Failure::new("EXTENSION_INSTALL_FAILED", "interrupted before activation", 130));
            }
            Ok(json!({"extension":product.as_str(),"installed":true,"changed":true,"skills":{"status":"current"}}))
        }).unwrap();
        assert_eq!(called, [Product::Ops, Product::Remote]);
        assert_eq!(rows[0]["status"], "installed");
        assert_eq!(rows[1]["status"], "interrupted");
        for row in &rows[2..] {
            assert_eq!(row["status"], "skipped");
            assert_eq!(
                row["reason"],
                "Setup was interrupted. Rerun: tmt setup --yes"
            );
        }
        assert!(interrupted(&rows));
        assert!(failed(&rows));
        assert!(output.is_empty());
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
        assert_eq!(rows[1]["status"], "not_selected");
        assert_eq!(rows[2]["status"], "not_selected");
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

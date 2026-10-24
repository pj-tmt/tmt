//! The parent freezes metadata and consent; the new executable owns extension admission.
use super::*;
use crate::extension_install_command::upgrade_all::{self, Plan};
const EXTENSION_BUDGET: Duration = Duration::from_secs(180);

pub(super) fn upgrade_frozen(
    executable: &Path,
    approved: &Plan,
    consented: bool,
    mode: OutputMode,
    runner: &impl CommandRunner,
) -> Vec<Value> {
    upgrade_with_plan(
        executable,
        runner,
        |_| Ok(consented),
        |remote_pending| show_remote_progress(remote_pending, mode),
        Some(approved),
    )
}

fn show_remote_progress(pending: bool, mode: OutputMode) {
    if remote_progress(
        pending,
        mode.json,
        tmt_cli_style::Interaction::detect(mode.json).stderr,
    ) {
        let _ = writeln!(tmt_cli_style::stream::stderr(), "Upgrading Remote…");
    }
}

fn remote_progress(remote_pending: bool, json: bool, stderr_terminal: bool) -> bool {
    remote_pending && !json && stderr_terminal
}

#[cfg(test)]
fn upgrade_with(
    executable: &Path,
    runner: &impl CommandRunner,
    consent: impl FnOnce(&Plan) -> Result<bool, Failure>,
) -> Vec<Value> {
    upgrade_with_progress(executable, runner, consent, |_| {})
}
#[cfg(test)]
fn upgrade_with_progress(
    executable: &Path,
    runner: &impl CommandRunner,
    consent: impl FnOnce(&Plan) -> Result<bool, Failure>,
    progress: impl FnOnce(bool),
) -> Vec<Value> {
    upgrade_with_plan(executable, runner, consent, progress, None)
}

fn upgrade_with_plan(
    executable: &Path,
    runner: &impl CommandRunner,
    consent: impl FnOnce(&Plan) -> Result<bool, Failure>,
    progress: impl FnOnce(bool),
    approved: Option<&Plan>,
) -> Vec<Value> {
    let plan =
        match child(executable, runner, true, &[], EXTENSION_BUDGET).and_then(|(bytes, failed)| {
            if failed {
                return Err(invalid_report());
            }
            Plan::parse(&bytes).ok_or_else(invalid_report)
        }) {
            Ok(plan) => plan,
            Err(error) => return vec![phase_failure(error)],
        };
    if approved.is_some_and(|approved| targets(approved) != targets(&plan)) {
        return vec![phase_failure(io::Error::other(
            "The installed extensions or selected targets changed after the upgrade preview. No extension was changed.",
        ))];
    }
    if plan.pending.is_empty() {
        return plan.products;
    }
    let accepted = consent(&plan);
    if !matches!(accepted, Ok(true)) {
        return upgrade_all::without_consent(plan, accepted);
    }
    let Plan {
        mut products,
        pending,
    } = plan;
    let apply = Plan {
        products: Vec::new(),
        pending,
    };
    let remote_pending = apply
        .pending
        .iter()
        .any(|(product, _, _)| *product == native_install::Product::Remote);
    let budget = EXTENSION_BUDGET
        + if remote_pending {
            native_install::POST_UPGRADE_RESTART_BOUND
        } else {
            Duration::ZERO
        };
    progress(remote_pending);
    let input = serde_json::to_vec(&apply.document()).expect("JSON plan");
    match child(executable, runner, false, &input, budget).and_then(|(bytes, failed)| {
        let rows = upgrade_all::parse_results(&bytes).ok_or_else(invalid_report)?;
        if failed != rows.iter().any(|row| row["status"] == "failed")
            || rows.len() != apply.pending.len()
            || !apply.pending.iter().all(|(product, _, selected)| {
                rows.iter().any(|row| {
                    row["product"] == product.as_str()
                        && (row["status"] == "failed"
                            || row["status"] == "skippedPinned"
                            || row["version"] == *selected)
                })
            })
        {
            return Err(invalid_report());
        }
        Ok(rows)
    }) {
        Ok(rows) => products.extend(rows),
        Err(error) => {
            for (product, _, _) in apply.pending {
                let mut row = phase_failure(io::Error::other(error.to_string()));
                row["product"] = product.as_str().into();
                products.push(row);
            }
        }
    }
    products
}

/// The new executable retains inventory authority. Presentation/error wording
/// may change across versions; consent covers product, state and exact target.
pub(super) fn targets(plan: &Plan) -> BTreeMap<String, (String, Value, Value)> {
    plan.products
        .iter()
        .map(|row| {
            (
                row["product"]
                    .as_str()
                    .expect("validated product")
                    .to_owned(),
                (
                    row["status"].as_str().expect("validated status").to_owned(),
                    row["version"].clone(),
                    Value::Null,
                ),
            )
        })
        .chain(plan.pending.iter().map(|(product, version, selected)| {
            (
                product.as_str().to_owned(),
                (
                    "pending".into(),
                    version.clone().into(),
                    selected.clone().into(),
                ),
            )
        }))
        .collect()
}

fn child(
    executable: &Path,
    runner: &impl CommandRunner,
    planning: bool,
    input: &[u8],
    budget: Duration,
) -> io::Result<(Vec<u8>, bool)> {
    let result = runner.execute(CommandRequest {
        program: executable.as_os_str(),
        args: &[
            "__native-upgrade-extensions".into(),
            "--json".into(),
            if planning { "--plan" } else { "--yes" }.into(),
        ],
        input,
        // Keep the existing acquisition/activation/skills allowance. Only a
        // pending Remote replacement adds its single owner-defined lifecycle bound.
        deadline: Instant::now() + budget,
        max_output_bytes: upgrade_all::LIMIT,
    });
    let (output, failed) = match result {
        Ok(output) => (output, false),
        Err(mut error) => {
            if !matches!(
                error.kind,
                CommandFailure::Exit {
                    code: Some(1),
                    signal: None
                }
            ) || error.cleanup_failed()
            {
                return Err(io::Error::other(error));
            }
            let output = error.output.take().ok_or_else(|| io::Error::other(error))?;
            (output, true)
        }
    };
    if !output.stderr.is_empty() {
        return Err(invalid_report());
    }
    Ok((output.stdout, failed))
}

fn invalid_report() -> io::Error {
    io::Error::other("The installed executable returned an invalid extension upgrade report.")
}

fn phase_failure(error: io::Error) -> Value {
    json!({
        "product": "extensions",
        "status": "failed",
        "error": {
            "code": "EXTENSION_UPGRADE_FAILED",
            "message": format!("The installed executable could not complete extension upgrades: {error} Run tmt upgrade again; no fallback to the previous executable was attempted."),
        },
        "hint": "tmt upgrade",
    })
}

#[cfg(test)]
mod tests;

// Source-checked command samples for the printed-command guard.
#[cfg(test)]
pub(crate) const PRINTED_HINTS: &[crate::cli_style_tests::HintSpec] = &[
    crate::cli_style_tests::HintSpec::core(
        "The installed executable could not complete extension upgrades: {error} Run tmt upgrade again; no fallback to the previous executable was attempted.",
        &[" again"],
        &[],
    ),
    crate::cli_style_tests::HintSpec::core("tmt upgrade", &[""], &[]),
];

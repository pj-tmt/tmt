//! The parent owns consent; only the newly installed executable plans/applies extensions.
use super::*;
use crate::extension_install_command::upgrade_all::{self, Plan};
const EXTENSION_BUDGET: Duration = Duration::from_secs(180);

pub(super) fn upgrade(
    executable: &Path,
    yes: bool,
    mode: OutputMode,
    runner: &impl CommandRunner,
) -> Vec<Value> {
    upgrade_with_progress(
        executable,
        runner,
        |plan| upgrade_all::ask(plan, yes, mode),
        |remote_pending| {
            let mut stderr = tmt_cli_style::stream::stderr();
            if remote_pending && !mode.json && tmt_cli_style::Interaction::detect(mode.json).stderr
            {
                let _ = writeln!(stderr, "Upgrading Remote…");
            }
        },
    )
}

#[cfg(test)]
fn upgrade_with(
    executable: &Path,
    runner: &impl CommandRunner,
    consent: impl FnOnce(&Plan) -> Result<bool, Failure>,
) -> Vec<Value> {
    upgrade_with_progress(executable, runner, consent, |_| {})
}
fn upgrade_with_progress(
    executable: &Path,
    runner: &impl CommandRunner,
    consent: impl FnOnce(&Plan) -> Result<bool, Failure>,
    progress: impl FnOnce(bool),
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

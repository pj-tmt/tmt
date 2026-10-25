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
    upgrade_with_observations(
        executable,
        runner,
        |_| Ok(consented),
        |remote_pending| show_remote_progress(remote_pending, mode),
        Some(approved),
        Some(mode),
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

#[cfg(test)]
fn upgrade_with_plan(
    executable: &Path,
    runner: &impl CommandRunner,
    consent: impl FnOnce(&Plan) -> Result<bool, Failure>,
    progress: impl FnOnce(bool),
    approved: Option<&Plan>,
) -> Vec<Value> {
    upgrade_with_observations(executable, runner, consent, progress, approved, None)
}

fn upgrade_with_observations(
    executable: &Path,
    runner: &impl CommandRunner,
    consent: impl FnOnce(&Plan) -> Result<bool, Failure>,
    progress: impl FnOnce(bool),
    approved: Option<&Plan>,
    mode: Option<OutputMode>,
) -> Vec<Value> {
    let (plan, live) = match plan_child(executable, runner, mode.is_some()) {
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
    if !live {
        progress(remote_pending);
    }
    let input = serde_json::to_vec(&apply.document()).expect("JSON plan");
    let result = if live {
        observed_child(executable, runner, &input, budget, mode.unwrap_or_default())
    } else {
        child(executable, runner, false, &input, budget)
    };
    match result.and_then(|(bytes, failed)| {
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

/// Negotiate only during read-only planning. An old parser's exit 2 permits one
/// legacy planning call; an apply is never replayed or used as a capability probe.
fn plan_child(
    executable: &Path,
    runner: &impl CommandRunner,
    observations: bool,
) -> io::Result<(Plan, bool)> {
    if observations {
        let probe = invoke(
            executable,
            runner,
            true,
            true,
            &[],
            EXTENSION_BUDGET,
            &mut |_| {},
        );
        match probe {
            Err(error)
                if !error.cleanup_failed()
                    && matches!(
                        error.kind,
                        CommandFailure::Exit {
                            code: Some(2),
                            signal: None
                        }
                    ) => {}
            result => {
                let (output, failed) = completed(result)?;
                if failed || output.stderr != crate::native_progress::child::CAPABILITY {
                    return Err(invalid_report());
                }
                return Plan::parse(&output.stdout)
                    .map(|plan| (plan, true))
                    .ok_or_else(invalid_report);
            }
        }
    }
    let (bytes, failed) = child(executable, runner, true, &[], EXTENSION_BUDGET)?;
    if failed {
        return Err(invalid_report());
    }
    Plan::parse(&bytes)
        .map(|plan| (plan, false))
        .ok_or_else(invalid_report)
}

fn child(
    executable: &Path,
    runner: &impl CommandRunner,
    planning: bool,
    input: &[u8],
    budget: Duration,
) -> io::Result<(Vec<u8>, bool)> {
    let (output, failed) = completed(invoke(
        executable,
        runner,
        planning,
        false,
        input,
        budget,
        &mut |_| {},
    ))?;
    if !output.stderr.is_empty() {
        return Err(invalid_report());
    }
    Ok((output.stdout, failed))
}

fn observed_child(
    executable: &Path,
    runner: &impl CommandRunner,
    input: &[u8],
    budget: Duration,
    mode: OutputMode,
) -> io::Result<(Vec<u8>, bool)> {
    let mut decoder = crate::native_progress::child::Decoder::default();
    let mut presentations = BTreeMap::new();
    let mut observe =
        |product: native_install::Product,
         event: crate::native_progress::child::UpgradeProgress| {
            let presentation = presentations.entry(product.as_str()).or_insert_with(|| {
                crate::native_progress::InstallProgress::new(
                    "Upgrading",
                    product,
                    tmt_cli_style::Interaction::detect(mode.json),
                )
            });
            event.present(presentation);
        };
    let result = invoke(
        executable,
        runner,
        false,
        true,
        input,
        budget,
        &mut |bytes| decoder.feed(bytes, &mut observe),
    );
    let (output, failed) = completed(result)?;
    if !decoder.complete() {
        return Err(invalid_report());
    }
    Ok((output.stdout, failed))
}

fn invoke(
    executable: &Path,
    runner: &impl CommandRunner,
    planning: bool,
    progress: bool,
    input: &[u8],
    budget: Duration,
    observer: &mut dyn FnMut(&[u8]),
) -> Result<tmt_adapters::process::CommandOutput, tmt_adapters::process::CommandError> {
    let mut args = vec![
        "__native-upgrade-extensions".into(),
        "--json".into(),
        if planning { "--plan" } else { "--yes" }.into(),
    ];
    if progress {
        args.push("--progress".into());
    }
    let request = CommandRequest {
        program: executable.as_os_str(),
        args: &args,
        input,
        deadline: Instant::now() + budget,
        max_output_bytes: upgrade_all::LIMIT,
    };
    if progress && !planning {
        runner.execute_observed(request, crate::native_progress::child::LIMIT, observer)
    } else {
        runner.execute(request)
    }
}

fn completed(
    result: Result<tmt_adapters::process::CommandOutput, tmt_adapters::process::CommandError>,
) -> io::Result<(tmt_adapters::process::CommandOutput, bool)> {
    match result {
        Ok(output) => Ok((output, false)),
        Err(mut error)
            if !error.cleanup_failed()
                && matches!(
                    error.kind,
                    CommandFailure::Exit {
                        code: Some(1),
                        signal: None
                    }
                ) =>
        {
            let output = error.output.take().ok_or_else(|| io::Error::other(error))?;
            Ok((output, true))
        }
        Err(error) => Err(io::Error::other(error)),
    }
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

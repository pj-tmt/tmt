//! The activated CLI owns approved installation/setup effects. Capability probing is read-only.
use super::*;
use crate::extension_install_command::upgrade_all::{self, complete::Plan};

#[derive(Default)]
pub(super) struct Outcome {
    pub(super) products: Vec<Value>,
    pub(super) providers: Vec<Value>,
    pub(super) interrupted: bool,
}

pub(super) fn run(
    executable: &Path,
    approved: &Plan,
    consented: bool,
    mode: OutputMode,
    runner: &impl CommandRunner,
) -> Outcome {
    let has_actions = !approved.installed.pending.is_empty()
        || approved
            .missing
            .iter()
            .chain(&approved.providers)
            .any(|row| row["selected"] == true && row["status"] == "pending");
    if !consented || !has_actions {
        let plan = Plan::parse(&serde_json::to_vec(&approved.document()).expect("plan JSON"))
            .expect("own plan");
        let (products, providers) = plan.without_consent();
        return Outcome {
            products,
            providers,
            interrupted: false,
        };
    }
    let remote_pending = approved
        .installed
        .pending
        .iter()
        .any(|(product, _, _)| *product == native_install::Product::Remote);
    let budget = Duration::from_secs(180)
        + if remote_pending {
            native_install::POST_UPGRADE_RESTART_BOUND
        } else {
            Duration::ZERO
        };
    let probe =
        extensions::invoke_complete(executable, runner, true, true, &[], budget, &mut |_| {});
    match probe {
        Err(error)
            if !error.cleanup_failed()
                && matches!(
                    error.kind,
                    CommandFailure::Exit {
                        code: Some(2),
                        signal: None
                    }
                ) =>
        {
            // The old target can upgrade installed products, but receives no missing-item apply.
            let products =
                extensions::upgrade_frozen(executable, &approved.installed, true, mode, runner);
            let mut fallback =
                Plan::parse(&serde_json::to_vec(&approved.document()).expect("plan JSON"))
                    .expect("own plan");
            fallback.installed.products.clear();
            fallback.installed.pending.clear();
            let (mut missing, providers) = fallback.without_consent();
            for row in &mut missing {
                row["message"] =
                    "This CLI does not support missing-item upgrade plans; rerun from a newer CLI."
                        .into();
            }
            return Outcome {
                products: products.into_iter().chain(missing).collect(),
                providers,
                interrupted: false,
            };
        }
        Ok(output) if output.stderr == upgrade_all::complete::CAPABILITY => {
            let current = Plan::parse(&output.stdout);
            if !current
                .as_ref()
                .is_some_and(|current| approved.same_scope(current))
            {
                return failed(
                    approved,
                    "The installed components, provider scopes or selected targets changed after approval. Run tmt upgrade again.",
                );
            }
        }
        _ => {
            return failed(
                approved,
                "The activated CLI did not confirm whole-upgrade planning; inspect the installation before retrying.",
            );
        }
    }
    let input = serde_json::to_vec(&approved.document()).expect("plan JSON");
    let mut decoder = crate::native_progress::child::Decoder::default();
    let mut presentations = BTreeMap::new();
    let mut observe =
        |product: native_install::Product,
         event: crate::native_progress::child::UpgradeProgress| {
            let verb = if approved
                .missing
                .iter()
                .any(|row| row["product"] == product.as_str() && row["action"] == "install")
            {
                "Installing"
            } else {
                "Upgrading"
            };
            let progress = presentations.entry(product.as_str()).or_insert_with(|| {
                crate::native_progress::InstallProgress::new(
                    verb,
                    product,
                    tmt_cli_style::Interaction::detect(mode.json),
                )
            });
            event.present(progress);
        };
    let result = extensions::invoke_complete(
        executable,
        runner,
        false,
        true,
        &input,
        budget,
        &mut |bytes| decoder.feed(bytes, &mut observe),
    );
    let (output, status) = match result {
        Ok(output) => (output, 0),
        Err(mut error)
            if !error.cleanup_failed()
                && matches!(
                    error.kind,
                    CommandFailure::Exit {
                        code: Some(1 | 130),
                        signal: None
                    }
                ) =>
        {
            let status = match error.kind {
                CommandFailure::Exit {
                    code: Some(code), ..
                } => code,
                _ => unreachable!(),
            };
            let Some(output) = error.output.take() else {
                return failed(
                    approved,
                    "Missing apply report; effects may have completed. Inspect before retrying.",
                );
            };
            (output, status)
        }
        _ => {
            return failed(
                approved,
                "The apply outcome is unconfirmed; effects may have completed. Inspect before retrying. No apply was replayed.",
            );
        }
    };
    if !decoder.complete() {
        return failed(
            approved,
            "Invalid progress report; inspect before retrying. No apply was replayed.",
        );
    }
    parse_results(&output.stdout, status, approved).unwrap_or_else(|| {
        failed(
            approved,
            "Invalid apply report; inspect before retrying. No apply was replayed.",
        )
    })
}

fn parse_results(bytes: &[u8], status: u32, approved: &Plan) -> Option<Outcome> {
    if bytes.len() > upgrade_all::LIMIT {
        return None;
    }
    let document: Value = serde_json::from_slice(bytes).ok()?;
    if document.as_object()?.len() != 3 {
        return None;
    }
    let products = document["products"].as_array()?.clone();
    let providers = document["providers"].as_array()?.clone();
    let interrupted = document["interrupted"].as_bool()?;
    let mut expected = extensions::targets(&approved.installed)
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    for row in &approved.missing {
        let name = row["product"].as_str()?.to_owned();
        if !expected.contains(&name) {
            expected.push(name);
        }
    }
    if expected.len() != products.len() || providers.len() != approved.providers.len() {
        return None;
    }
    let mut seen = Vec::new();
    for row in &products {
        let name = row["product"].as_str()?.to_owned();
        if !expected.contains(&name) || seen.contains(&name) {
            return None;
        }
        seen.push(name.clone());
        if !matches!(
            row["status"].as_str(),
            Some("changed" | "unchanged" | "skippedPinned" | "consentRequired" | "failed")
        ) {
            return None;
        }
        if row["status"] == "changed" {
            let selected = approved
                .missing
                .iter()
                .find(|r| r["product"] == name)
                .map(|r| (r["selected"] == true, r["version"].clone()))
                .or_else(|| {
                    approved
                        .installed
                        .pending
                        .iter()
                        .find(|(p, _, _)| p.as_str() == name)
                        .map(|(_, _, v)| (true, json!(v)))
                })?;
            if !selected.0 || row["version"] != selected.1 {
                return None;
            }
        }
    }
    let mut seen = Vec::new();
    for row in &providers {
        let name = row["provider"].as_str()?;
        let prior = approved.providers.iter().find(|r| r["provider"] == name)?;
        if seen.contains(&name)
            || !matches!(
                row["status"].as_str(),
                Some("changed" | "consentRequired" | "needsApproval" | "skipped" | "failed")
            )
        {
            return None;
        }
        seen.push(name);
        if row["status"] == "changed" && prior["selected"] != true {
            return None;
        }
    }
    let failed = products
        .iter()
        .chain(&providers)
        .any(|row| row["status"] == "failed");
    if status != if interrupted { 130 } else { u32::from(failed) } {
        return None;
    }
    Some(Outcome {
        products,
        providers,
        interrupted,
    })
}

fn failed(plan: &Plan, message: &str) -> Outcome {
    let mut products = plan.installed.products.clone();
    for (product, _, _) in &plan.installed.pending {
        products.push(json!({"product":product.as_str(),"status":"failed","message":message,"hint":"tmt upgrade"}));
    }
    for row in &plan.missing {
        products.retain(|prior| prior["product"] != row["product"]);
        let mut row = row.clone();
        row["status"] = "failed".into();
        row["message"] = message.into();
        products.push(row);
    }
    let providers = plan
        .providers
        .iter()
        .map(|row| {
            let mut row = row.clone();
            row["status"] = if row["selected"] == true {
                "failed"
            } else {
                "consentRequired"
            }
            .into();
            row["message"] = message.into();
            row
        })
        .collect();
    Outcome {
        products,
        providers,
        interrupted: false,
    }
}

#[cfg(test)]
pub(crate) const PRINTED_HINTS: &[crate::cli_style_tests::HintSpec] = &[
    crate::cli_style_tests::HintSpec::core(
        "The installed components, provider scopes or selected targets changed after approval. Run tmt upgrade again.",
        &[" again."],
        &[],
    ),
    crate::cli_style_tests::HintSpec::core("tmt upgrade", &[""], &[]),
];

#[cfg(test)]
#[path = "complete/tests.rs"]
mod tests;

//! One consent decision over installed official extensions; activation keeps its existing owner.
use super::{INSTALLABLE_EXTENSIONS, ask_declining, failure, installed, list_upgrade::upgrade_at};
use crate::{consent, invocation::OutputMode, output::Failure};
use serde_json::{Value, json};
use std::{
    io::{self, Read, Write},
    path::Path,
};

mod protocol;
pub(crate) use protocol::{LIMIT, Plan, parse_results};
use tmt_adapters::native_install::{self, Product};

fn plan(prefix: &Path) -> Plan {
    let mut outcomes = Vec::new();
    let mut pending = Vec::new();
    for &product in INSTALLABLE_EXTENSIONS {
        let result: Result<Option<Value>, Failure> = (|| {
            if !installed(product, prefix)? {
                return Ok(None);
            }
            let current = native_install::inspect_product_prefix(product, prefix)
                .map_err(|error| failure("EXTENSION_INSTALLATION_INVALID", error))?;
            let version = current.state.version.to_string();
            if current.state.pinned_version.is_some() {
                return Ok(Some(pinned(product, &version)));
            }
            let latest = native_install::latest_release_version(product, current.state.channel)
                .map_err(|error| failure("EXTENSION_UPGRADE_FAILED", error))?;
            if latest <= current.state.version {
                return Ok(Some(unchanged(product, &version)));
            }
            pending.push((product, version, latest.to_string()));
            Ok(None)
        })();
        match result {
            Ok(Some(row)) => outcomes.push(row),
            Ok(None) => {}
            Err(error) => outcomes.push(failed(product, error)),
        }
    }
    Plan {
        products: outcomes,
        pending,
    }
}

pub(crate) fn ask(plan: &Plan, yes: bool, mode: OutputMode) -> Result<bool, Failure> {
    if !yes && !consent::interactive(mode, &tmt_cli_style::stream::stdout(mode.json)) {
        return Ok(false);
    }
    ask_declining(
        yes,
        mode,
        &question(&plan.pending),
        "No extension changes made.",
    )
}

pub(crate) fn without_consent(plan: Plan, consent: Result<bool, Failure>) -> Vec<Value> {
    settle(
        plan.products,
        plan.pending,
        |_| consent,
        |_, _| unreachable!("no apply without consent"),
    )
}

pub(crate) fn execute(planning: bool, mode: OutputMode) -> io::Result<u8> {
    let result: Result<(Value, u8), Failure> = (|| {
        let executable =
            std::env::current_exe().map_err(|error| failure("EXTENSION_UPGRADE_FAILED", error))?;
        let current = native_install::inspect(&executable)
            .map_err(|error| failure("EXTENSION_INSTALLATION_INVALID", error))?;
        if planning {
            return Ok((plan(current.prefix()).document(), 0));
        }
        let mut bytes = Vec::new();
        io::stdin()
            .take((LIMIT + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|error| failure("EXTENSION_UPGRADE_FAILED", error))?;
        let plan = Plan::parse(&bytes)
            .filter(|plan| plan.products.is_empty())
            .ok_or_else(|| {
                Failure::new(
                    "EXTENSION_UPGRADE_PLAN_INVALID",
                    "Invalid extension upgrade plan; run tmt upgrade again.",
                    1,
                )
            })?;
        let products = settle(
            Vec::new(),
            plan.pending,
            |_| Ok(true),
            |product, selected| {
                upgrade_at(product, current.prefix(), None, None, false, Some(selected))
            },
        );
        let status = u8::from(products.iter().any(|product| product["status"] == "failed"));
        Ok((json!({ "products": products }), status))
    })();
    match result {
        Ok((document, status)) => {
            writeln!(tmt_cli_style::stream::stdout(mode.json), "{document}")?;
            Ok(status)
        }
        Err(error) => error.publish(mode),
    }
}

fn question(pending: &[(Product, String, String)]) -> String {
    let changes = pending
        .iter()
        .map(|(product, old, new)| format!("{} {old} -> {new}", product.as_str()))
        .collect::<Vec<_>>()
        .join(", ");
    format!("Update installed extensions: {changes}")
}

fn settle(
    mut outcomes: Vec<Value>,
    pending: Vec<(Product, String, String)>,
    consent: impl FnOnce(&str) -> Result<bool, Failure>,
    mut upgrade: impl FnMut(Product, &str) -> Result<super::Outcome, Failure>,
) -> Vec<Value> {
    if pending.is_empty() {
        return outcomes;
    }
    let question = question(&pending);
    let accepted = consent(&question);
    for (product, version, selected) in pending {
        match &accepted {
            Ok(false) => outcomes.push(consent_required(product, &version, &selected)),
            Err(error) => outcomes.push(failed_ref(product, error)),
            Ok(true) => match upgrade(product, &selected) {
                Ok(Some((document, _))) => outcomes.push(upgraded(product, document)),
                Ok(None) => unreachable!("consented upgrade always reports"),
                Err(error) => outcomes.push(failed(product, error)),
            },
        }
    }
    outcomes
}
fn pinned(product: Product, version: &str) -> Value {
    let name = product.as_str();
    let hint = format!("tmt extension upgrade {name} --unpin");
    json!({
        "product": name,
        "status": "skippedPinned",
        "version": version,
        "hint": hint,
        "message": format!("{name} {version} is pinned; nothing changed. Clear the pin with: {hint}"),
    })
}

fn consent_required(product: Product, version: &str, selected: &str) -> Value {
    json!({
        "product": product.as_str(),
        "status": "consentRequired",
        "version": version,
        "availableVersion": selected,
        "hint": "tmt upgrade --yes",
    })
}

fn upgraded(product: Product, document: Value) -> Value {
    if document["skippedPinned"] == true {
        return pinned(
            product,
            document["version"].as_str().expect("upgrade version"),
        );
    }
    let status = if document["changed"] == true {
        "changed"
    } else {
        "unchanged"
    };
    json!({
        "product": product.as_str(),
        "status": status,
        "version": document["version"],
        "details": document,
    })
}

fn failed(product: Product, error: Failure) -> Value {
    failed_ref(product, &error)
}

fn failed_ref(product: Product, error: &Failure) -> Value {
    json!({
        "product": product.as_str(),
        "status": "failed",
        "error": error.document()["error"],
    })
}

#[cfg(test)]
mod tests;

fn unchanged(product: Product, version: &str) -> Value {
    json!({
        "product": product.as_str(),
        "status": "unchanged",
        "version": version,
    })
}

// Source-checked command samples for the printed-command guard.
#[cfg(test)]
pub(crate) const PRINTED_HINTS: &[crate::cli_style_tests::HintSpec] = &[
    crate::cli_style_tests::HintSpec::core(
        "Invalid extension upgrade plan; run tmt upgrade again.",
        &[" again"],
        &[],
    ),
    crate::cli_style_tests::HintSpec::core(
        "tmt extension upgrade {name} --unpin",
        &[""],
        &[
            ("{name}", "ops"),
            ("{}", "ops"),
            ("{SUGGESTED_EXTENSION}", "ops"),
        ],
    ),
    crate::cli_style_tests::HintSpec::core("tmt upgrade --yes", &[""], &[]),
];

//! One consent decision over installed official extensions; activation keeps its existing owner.
use super::{ask_declining, failure, installed, list_upgrade::upgrade_at};
use crate::{consent, invocation::OutputMode, output::Failure};
use serde_json::{Value, json};
use std::path::Path;
use tmt_adapters::native_install::{self, Product};

pub(crate) fn upgrade_installed(prefix: &Path, yes: bool, mode: OutputMode) -> Vec<Value> {
    let mut outcomes = Vec::new();
    let mut pending = Vec::new();
    for product in Product::ALL.into_iter().filter(|p| *p != Product::Cli) {
        let result = (|| {
            if !installed(product, prefix)? {
                return Ok(None);
            }
            let current = native_install::inspect_product_prefix(product, prefix)
                .map_err(|e| failure("EXTENSION_INSTALLATION_INVALID", e))?;
            let version = current.state.version.to_string();
            if current.state.pinned_version.is_some() {
                return Ok(Some(pinned(product, &version)));
            }
            let latest = native_install::latest_release_version(product, current.state.channel)
                .map_err(|e| failure("EXTENSION_UPGRADE_FAILED", e))?;
            if latest <= current.state.version {
                return Ok(Some(
                    json!({"product":product.as_str(),"status":"unchanged","version":version}),
                ));
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
    settle(
        outcomes,
        pending,
        |question| {
            if !yes && !consent::interactive(mode, &tmt_cli_style::stream::stdout(mode.json)) {
                Ok(false)
            } else {
                ask_declining(yes, mode, question, "No extension changes made.")
            }
        },
        |product, selected| upgrade_at(product, prefix, None, None, false, Some(selected)),
    )
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
    let question = format!(
        "Update installed extensions: {}",
        pending
            .iter()
            .map(|(p, old, new)| format!("{} {old} -> {new}", p.as_str()))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let accepted = consent(&question);
    for (product, version, selected) in pending {
        match &accepted {
            Ok(false) => outcomes.push(json!({"product":product.as_str(),"status":"consentRequired",
                "version":version,"availableVersion":selected,"hint":"tmt upgrade --yes"})),
            Err(error) => outcomes.push(json!({"product":product.as_str(),"status":"failed","error":error.document()["error"]})),
            Ok(true) => match upgrade(product, &selected) {
                Ok(Some((document,_))) => {
                    if document["skippedPinned"] == true {
                        outcomes.push(pinned(product, document["version"].as_str().expect("upgrade version")));
                    } else {
                        outcomes.push(json!({"product":product.as_str(),
                            "status":if document["changed"]==true {"changed"} else {"unchanged"},
                            "version":document["version"],"details":document}));
                    }
                },
                Ok(None) => unreachable!("consented upgrade always reports"),
                Err(error) => outcomes.push(failed(product,error)),
            },
        }
    }
    outcomes
}
fn pinned(product: Product, version: &str) -> Value {
    json!({"product":product.as_str(), "status":"skippedPinned", "version":version,
        "hint":format!("tmt extension upgrade {} --unpin",product.as_str()),
        "message":format!("{} {version} is pinned; nothing changed. Clear the pin with: tmt extension upgrade {} --unpin",product.as_str(),product.as_str())})
}

fn failed(product: Product, error: Failure) -> Value {
    json!({"product":product.as_str(),"status":"failed","error":error.document()["error"]})
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn pending(product: Product) -> (Product, String, String) {
        (product, "1.0.0".into(), "1.1.0".into())
    }

    #[test]
    fn one_consent_lists_all_versions_and_failure_does_not_stop_squad() {
        let asked = Cell::new(0);
        let mut applied = Vec::new();
        let rows = settle(
            vec![],
            vec![pending(Product::Office), pending(Product::Squad)],
            |question| {
                asked.set(asked.get() + 1);
                assert!(question.contains("office 1.0.0 -> 1.1.0"));
                assert!(question.contains("squad 1.0.0 -> 1.1.0"));
                Ok(true)
            },
            |product, selected| {
                applied.push(product);
                assert_eq!(selected, "1.1.0");
                if product == Product::Office {
                    return Err(Failure::new("EXTENSION_UPGRADE_FAILED", "refused", 1));
                }
                Ok(Some((
                    json!({"changed":true,"version":selected}),
                    super::super::Human::plain(String::new()),
                )))
            },
        );
        assert_eq!(asked.get(), 1);
        assert_eq!(applied, vec![Product::Office, Product::Squad]);
        assert_eq!(rows[0]["status"], "failed");
        assert_eq!(rows[0]["error"]["code"], "EXTENSION_UPGRADE_FAILED");
        assert_eq!(rows[1]["status"], "changed");
    }

    #[test]
    fn missing_consent_changes_no_extension_and_returns_exact_rerun_hint() {
        let rows = settle(
            vec![],
            vec![pending(Product::Squad)],
            |_| Ok(false),
            |_, _| panic!("must not mutate"),
        );
        assert_eq!(rows[0]["status"], "consentRequired");
        assert_eq!(rows[0]["hint"], "tmt upgrade --yes");
    }

    #[test]
    fn nothing_installed_neither_prompts_nor_installs() {
        let rows = settle(
            vec![],
            vec![],
            |_| panic!("no prompt"),
            |_, _| panic!("no installation"),
        );
        assert!(rows.is_empty());
    }
}

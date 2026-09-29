//! `tmt extension hooks`: record, remove and list consent for extension hooks.
//! Verification, probing and storage belong to `tmt_adapters::extension_hooks`.

use crate::invocation::{ExtensionHooksRequest, OutputMode};
use crate::output::Failure;
use serde_json::json;
use std::io::{self, Write};
use tmt_adapters::{config::ConfigPaths, extension_hooks};

fn failure(error: extension_hooks::ExtensionHookError) -> Failure {
    Failure::new(error.code(), error.to_string(), 1)
}

fn consent_json(consent: &extension_hooks::Consent) -> serde_json::Value {
    json!({
        "name": consent.name,
        "path": consent.path,
        "digest": consent.digest,
        "capabilities": consent.capabilities,
        "consentedAtMs": consent.consented_at_ms,
    })
}

pub fn execute(request: ExtensionHooksRequest, mode: OutputMode) -> io::Result<u8> {
    // The document, whether the human text reports a change, and the text.
    let result = (|| -> Result<(serde_json::Value, bool, String), Failure> {
        let paths = ConfigPaths::discover()?;
        match request {
            ExtensionHooksRequest::Enable(name) => {
                let tmt = std::env::current_exe().map_err(|error| {
                    Failure::new("EXTENSION_HOOKS_UNAVAILABLE", error.to_string(), 1)
                })?;
                let search = std::env::var_os("PATH").unwrap_or_default();
                let consent =
                    extension_hooks::enable(&paths, &name, &search, &tmt).map_err(failure)?;
                let text = format!(
                    "Enabled hooks for {} ({}): {}.",
                    consent.name,
                    consent.path.display(),
                    consent.capabilities.join(", ")
                );
                Ok((json!({"enabled": consent_json(&consent)}), true, text))
            }
            ExtensionHooksRequest::Disable(name) => {
                let changed = extension_hooks::disable(&paths, &name).map_err(failure)?;
                let text = if changed {
                    format!("Disabled hooks for {name}.")
                } else {
                    format!("Hooks for {name} were not enabled.")
                };
                Ok((json!({"name": name, "changed": changed}), changed, text))
            }
            ExtensionHooksRequest::List => {
                let consents = extension_hooks::list_consents(&paths).map_err(failure)?;
                let text = if consents.is_empty() {
                    "No extension hooks are enabled.".to_owned()
                } else {
                    consents
                        .iter()
                        .map(|consent| {
                            format!(
                                "{}  {}  {}",
                                consent.name,
                                consent.path.display(),
                                consent.capabilities.join(",")
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                };
                Ok((
                    json!({"extensions": consents.iter().map(consent_json).collect::<Vec<_>>()}),
                    false,
                    text,
                ))
            }
        }
    })();
    match result {
        Ok((value, changed, text)) => {
            let mut output = tmt_cli_style::stream::stdout(mode.json);
            let terminal = output.terminal();
            if mode.json {
                writeln!(output, "{value}")?;
            } else if changed {
                tmt_cli_style::message::success(&mut output, terminal, &text)?;
            } else {
                writeln!(output, "{text}")?;
            }
            Ok(0)
        }
        Err(error) => error.publish(mode),
    }
}

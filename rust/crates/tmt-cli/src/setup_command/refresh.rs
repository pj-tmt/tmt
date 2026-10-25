//! New-executable entrypoint for refreshing previously consented integrations.
use crate::invocation::OutputMode;
use std::io::{self, Write};
use tmt_adapters::{
    config::ConfigPaths,
    setup::{self, IntegrationRefreshFailure, IntegrationRefreshReport, RefreshReason},
};

pub(crate) fn unavailable(message: String) -> IntegrationRefreshReport {
    IntegrationRefreshReport {
        integrations: Vec::new(),
        error: Some(IntegrationRefreshFailure {
            reason: RefreshReason::RefreshUnavailable,
            message: message.chars().take(2048).collect(),
            next_step: "tmt setup".into(),
        }),
    }
}

pub(crate) fn parse(bytes: &[u8]) -> Option<IntegrationRefreshReport> {
    // Deserialize the typed document first so duplicate fields are refused,
    // rather than being collapsed by Value's object representation.
    let report: IntegrationRefreshReport = serde_json::from_slice(bytes).ok()?;
    let value: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    if value.as_object()?.len() != 2
        || !value["integrations"]
            .as_array()?
            .iter()
            .all(|row| row.as_object().is_some_and(|row| row.len() == 6))
    {
        return None;
    }
    if report.integrations.len() > 64
        || report.integrations.iter().any(|row| {
            row.provider.len() > 64
                || !row.settings_path.is_absolute()
                || row.message.chars().count() > 2048
                || row.next_step.as_ref().is_some_and(|step| step.len() > 128)
                || row
                    .settings_path
                    .to_str()
                    .is_none_or(|path| path.len() > 16_384)
        })
        || report.error.as_ref().is_some_and(|error| {
            error.message.chars().count() > 2048 || error.next_step.len() > 128
        })
    {
        return None;
    }
    Some(report)
}

pub(crate) fn execute(mode: OutputMode) -> io::Result<u8> {
    // Enter before automatic host registration/reminders. The child emits only
    // its bounded JSON contract, including directory-cutover failures.
    let report = match ConfigPaths::discover_with_move_report(|_, _| {}) {
        Ok(paths) => std::env::current_exe()
            .and_then(|executable| {
                tmt_adapters::native_install::with_active_release(&executable, || {
                    setup::refresh_recorded(&paths.global_dir)
                })
            })
            .unwrap_or_else(|error| unavailable(error.to_string())),
        Err(error) => unavailable(error.to_string()),
    };
    writeln!(
        tmt_cli_style::stream::stdout(mode.json),
        "{}",
        serde_json::to_string(&report)?
    )?;
    Ok(0)
}

#[cfg(test)]
pub(crate) const PRINTED_HINTS: &[crate::cli_style_tests::HintSpec] =
    &[crate::cli_style_tests::HintSpec::core(
        "tmt setup",
        &[""],
        &[],
    )];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integration_reports_require_the_owned_shape_and_bounded_rows() {
        let report = unavailable("failed".into());
        assert!(parse(&serde_json::to_vec(&report).unwrap()).is_some());
        assert!(parse(br#"{"integrations":[],"error":null,"extra":true}"#).is_none());
        assert!(parse(br#"{"integrations":[]}"#).is_none());
        assert!(parse(br#"{"integrations":[],"integrations":[],"error":null}"#).is_none());
        assert!(parse(br#"{"integrations":[{"status":"done"}],"error":null}"#).is_none());
        let mut value = serde_json::to_value(report).unwrap();
        value["error"]["message"] = "x".repeat(2049).into();
        assert!(parse(&serde_json::to_vec(&value).unwrap()).is_none());
        let row = serde_json::json!({"provider":"claude","settingsPath":"/private/settings.json","status":"unchanged","reason":"current","message":"","nextStep":null});
        for (count, admitted) in [(64, true), (65, false)] {
            let value = serde_json::json!({"integrations": vec![row.clone(); count], "error":null});
            assert_eq!(
                parse(&serde_json::to_vec(&value).unwrap()).is_some(),
                admitted
            );
        }
    }
}

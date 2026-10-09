//! Carry one observed product notice through success and partial failure.

use super::Human;
use crate::output::Failure;
use serde_json::Value;
use tmt_adapters::native_install::{self, InstallReport, Product};

#[derive(Default)]
pub(super) struct PostUpgradeNotice(Option<&'static str>);

impl PostUpgradeNotice {
    pub fn observe(product: Product, report: &InstallReport, previous: Option<&str>) -> Self {
        let core = std::env::current_exe().ok();
        let replaced = previous.is_some_and(|old| old != report.version);
        Self(native_install::post_upgrade_hint(
            product,
            report,
            replaced,
            core.as_deref(),
        ))
    }

    pub fn after_failure(product: Product, error: &std::io::Error, previous: Option<&str>) -> Self {
        native_install::activated_report(error)
            .map(|report| Self::observe(product, report, previous))
            .unwrap_or_default()
    }

    pub fn record(&self, document: &mut Value, human: &mut Human) {
        if let Some(hint) = self.0 {
            document["restartHint"] = hint.into();
            human.push(hint.into());
        }
    }

    pub fn retain(&self, error: Failure) -> Failure {
        match self.0 {
            Some(hint) => {
                let existing = error.document()["error"]["suggestion"]
                    .as_str()
                    .map(str::to_owned);
                error
                    .suggestion(existing.map_or_else(|| hint.into(), |old| format!("{old} {hint}")))
            }
            None => error,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn initial_and_metadata_only_changes_stay_quiet() {
        let report = InstallReport {
            executable: "/unselected/tmt-remote".into(),
            active_executable: "/must-not-run/tmt-remote".into(),
            version: "0.1.0-alpha.2".into(),
            changed: true,
        };
        let previous = report.version.as_str();
        assert!(
            PostUpgradeNotice::observe(Product::Remote, &report, None)
                .0
                .is_none()
        );
        assert!(
            PostUpgradeNotice::observe(Product::Remote, &report, Some(previous))
                .0
                .is_none()
        );
        assert!(
            PostUpgradeNotice::after_failure(
                Product::Remote,
                &std::io::Error::other("before activation"),
                Some(previous)
            )
            .0
            .is_none()
        );
    }

    #[test]
    fn one_notice_survives_success_or_settlement_failure_without_replacing_the_error() {
        let notice = PostUpgradeNotice(Some("one restart notice"));
        let mut document = json!({"changed":true,"version":"0.1.0-alpha.2"});
        let mut human = Human::done("Updated product.".into());
        notice.record(&mut document, &mut human);
        assert_eq!(document["restartHint"], "one restart notice");
        let mut bytes = Vec::new();
        human
            .write(&mut bytes, tmt_cli_style::Terminal::PLAIN)
            .unwrap();
        assert_eq!(
            String::from_utf8(bytes)
                .unwrap()
                .matches("one restart notice")
                .count(),
            1
        );
        let error = notice.retain(
            Failure::new(
                "EXTENSION_SKILLS_FAILED",
                "already activated; skills refused",
                1,
            )
            .suggestion("existing recovery".into()),
        );
        assert_eq!(error.code, "EXTENSION_SKILLS_FAILED");
        assert_eq!(error.status, 1);
        assert_eq!(error.message, "already activated; skills refused");
        assert_eq!(
            error.document()["error"]["suggestion"],
            "existing recovery one restart notice"
        );
    }
}

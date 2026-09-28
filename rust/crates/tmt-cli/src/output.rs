//! Shared failure presentation and close-before-publication ordering.

pub(crate) mod table;

use crate::invocation::OutputMode;
use std::{
    error::Error,
    fmt,
    io::{self, Write},
    path::Path,
};
use tmt_adapters::storage::{StorageError, StorageErrorCode};

#[derive(Debug)]
pub struct Failure {
    pub code: &'static str,
    pub message: String,
    pub status: u8,
    diagnostics: Option<Box<Diagnostics>>,
    suggestion: Option<String>,
    request: Option<Box<(String, Option<&'static str>)>>,
    target: Option<Box<TargetDetails>>,
    stage: Option<&'static str>,
}

#[derive(Debug, Default)]
struct Diagnostics {
    cause: Option<Box<dyn Error>>,
    secondary: Vec<Box<dyn Error>>,
}

#[derive(Debug)]
struct TargetDetails {
    target: String,
    pane: Option<String>,
    identity: Option<(String, String)>,
}

impl Failure {
    pub fn storage_access(
        error: StorageError,
        directory: &Path,
        effect: &str,
        fallback_code: &'static str,
        fallback_message: &str,
    ) -> Self {
        if error.code == StorageErrorCode::NotWritable {
            Self::new(
                "STORAGE_NOT_WRITABLE",
                format!(
                    "TMT cannot write its data directory {}. An agent sandbox may be blocking it: allow that directory or use the provider's escalation for the next authorized command. {effect}",
                    directory.display()
                ),
                1,
            )
            .caused_by(error)
        } else {
            Self::new(fallback_code, fallback_message, 1).caused_by(error)
        }
    }
    pub fn new(code: &'static str, message: impl Into<String>, status: u8) -> Self {
        Self {
            code,
            message: message.into(),
            status,
            diagnostics: None,
            suggestion: None,
            request: None,
            target: None,
            stage: None,
        }
    }

    pub fn caused_by(mut self, cause: impl Error + 'static) -> Self {
        self.diagnostics.get_or_insert_default().cause = Some(Box::new(cause));
        self
    }

    pub fn suggestion(mut self, suggestion: String) -> Self {
        self.suggestion = Some(suggestion);
        self
    }

    pub fn with_secondary_error(mut self, error: impl Error + 'static) -> Self {
        self.diagnostics
            .get_or_insert_default()
            .secondary
            .push(Box::new(error));
        self
    }

    /// Only explicit public correlation is carried into an error document;
    /// bodies, receipt proofs and endpoint evidence are never included.
    pub fn with_request(mut self, request_id: String, status: Option<&'static str>) -> Self {
        self.request = Some(Box::new((request_id, status)));
        self
    }

    pub fn with_target(
        mut self,
        target: &str,
        pane: &str,
        identity: Option<&tmt_core::identity::Identity>,
    ) -> Self {
        self.target = Some(Box::new(TargetDetails {
            target: target.into(),
            pane: Some(pane.into()),
            identity: identity
                .map(|identity| (identity.name.clone(), identity.canonical_name.clone())),
        }));
        self
    }

    pub fn with_inbox_target(
        mut self,
        target: &str,
        identity: Option<&tmt_core::identity::Identity>,
    ) -> Self {
        self.target = Some(Box::new(TargetDetails {
            target: target.into(),
            pane: None,
            identity: identity
                .map(|identity| (identity.name.clone(), identity.canonical_name.clone())),
        }));
        self
    }

    pub fn at_stage(mut self, stage: &'static str) -> Self {
        self.stage = Some(stage);
        self
    }

    /// Feature reports may attach their own bounded partial-effect projection;
    /// this remains the sole shared error-envelope formatter.
    pub fn document(&self) -> serde_json::Value {
        let mut document =
            serde_json::json!({"error": {"code": self.code, "message": self.message}});
        if let Some(suggestion) = &self.suggestion {
            document["error"]["suggestion"] = suggestion.clone().into();
        }
        if let Some(request) = &self.request {
            let (request_id, status) = request.as_ref();
            document["requestId"] = request_id.clone().into();
            if let Some(status) = status {
                document["status"] = (*status).into();
            }
        }
        if let Some(target) = &self.target {
            document["target"] = target.target.clone().into();
            if let Some(pane) = &target.pane {
                document["pane"] = pane.clone().into();
            }
            if let Some((name, canonical_name)) = &target.identity {
                document["identity"] =
                    serde_json::json!({"name": name, "canonicalName": canonical_name});
            }
        }
        if let Some(stage) = self.stage {
            document["error"]["stage"] = stage.into();
        }
        document
    }

    pub fn publish(&self, mode: OutputMode) -> io::Result<u8> {
        if mode.json {
            writeln!(io::stdout().lock(), "{}", self.document())?;
        } else {
            writeln!(io::stderr().lock(), "{}", self.message)?;
            if let Some(suggestion) = &self.suggestion {
                writeln!(io::stderr().lock(), "{suggestion}")?;
            }
        }
        Ok(self.status)
    }
}

pub fn identity_document(identity: &tmt_core::identity::Identity) -> serde_json::Value {
    serde_json::json!({"id": identity.id, "name": identity.name,
        "canonicalName": identity.canonical_name, "lifetime": identity.lifetime.as_str()})
}

impl fmt::Display for Failure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for Failure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.diagnostics
            .as_ref()
            .and_then(|details| details.cause.as_deref())
    }
}

/// Cleanup runs even when the operation failed. A committed operation is not
/// rolled back by failed cleanup; discard its pending success projection and
/// preserve an existing primary failure, including its status and cause.
pub fn after_cleanup<T, E: Error + 'static>(
    pending: Result<T, Failure>,
    cleanup: impl FnOnce() -> Result<(), E>,
) -> Result<T, Failure> {
    let cleanup = cleanup();
    match pending {
        Err(primary) => Err(match cleanup {
            Ok(()) => primary,
            Err(secondary) => primary.with_secondary_error(secondary),
        }),
        Ok(report) => cleanup.map(|()| report).map_err(|error| {
            Failure::new(
                "CLEANUP_ERROR",
                "Could not clean up command resources. Effects may already have occurred.",
                1,
            )
            .caused_by(error)
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn cleanup_runs_once_before_a_success_can_be_published() {
        let calls = Cell::new(0);
        let report = after_cleanup(Ok("pending identity"), || {
            calls.set(calls.get() + 1);
            Ok::<_, io::Error>(())
        })
        .unwrap();
        assert_eq!(calls.get(), 1);
        assert_eq!(report, "pending identity");
    }

    #[test]
    fn cleanup_failure_discards_the_success_without_exposing_its_data_or_cause() {
        let failure = after_cleanup(Ok("private identity data"), || {
            Err(io::Error::other("private storage cause"))
        })
        .unwrap_err();
        assert_eq!(failure.code, "CLEANUP_ERROR");
        assert_eq!(failure.status, 1);
        assert!(
            failure
                .message
                .contains("Effects may already have occurred")
        );
        assert!(!failure.message.contains("private"));
        assert_eq!(
            failure.source().unwrap().to_string(),
            "private storage cause"
        );
    }

    #[test]
    fn primary_failure_survives_successful_and_failed_cleanup() {
        for fail_cleanup in [false, true] {
            let calls = Cell::new(0);
            let primary = Failure::new("NAME_NOT_FOUND", "Missing identity", 3)
                .caused_by(io::Error::other("primary cause"));
            let failure = after_cleanup::<(), _>(Err(primary), || {
                calls.set(calls.get() + 1);
                if fail_cleanup {
                    Err(io::Error::other("cleanup cause"))
                } else {
                    Ok(())
                }
            })
            .unwrap_err();
            assert_eq!(calls.get(), 1);
            assert_eq!(failure.code, "NAME_NOT_FOUND");
            assert_eq!(failure.message, "Missing identity");
            assert_eq!(failure.status, 3);
            assert_eq!(failure.source().unwrap().to_string(), "primary cause");
            let diagnostics = failure.diagnostics.as_ref().unwrap();
            assert_eq!(diagnostics.secondary.len(), usize::from(fail_cleanup));
            if fail_cleanup {
                assert_eq!(diagnostics.secondary[0].to_string(), "cleanup cause");
            }
        }
    }
}

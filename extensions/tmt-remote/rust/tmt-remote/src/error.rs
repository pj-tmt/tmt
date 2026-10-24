use std::{fmt, io};

/// Internal provenance only. Both cases retain the same unavailable wire refusal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OwnedReadFailure {
    Unavailable,
    Store,
}

#[derive(Debug, PartialEq, Eq)]
pub struct RemoteError {
    pub code: String,
    pub message: String,
    /// The next step, when there is one. Human output prints it as its own `hint:` line.
    pub hint: Option<String>,
    /// Active session cap for a signed eviction refusal.
    pub limit: Option<usize>,
    // These origins are mutually exclusive; keep one typed detail without growing every error.
    detail: Option<Box<ErrorDetail>>,
}
impl RemoteError {
    pub fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            hint: None,
            limit: None,
            detail: None,
        }
    }
    pub(crate) fn with_owned_read(mut self, failure: OwnedReadFailure) -> Self {
        self.detail = Some(Box::new(ErrorDetail::OwnedRead(failure)));
        self
    }
    pub fn json_value(&self) -> serde_json::Value {
        let mut value =
            serde_json::json!({"error":{"code":self.code,"message":self.json_message()}});
        match self.detail.as_deref() {
            Some(ErrorDetail::DeployInputs(required)) => {
                value["error"]["missing"] = serde_json::json!(required.missing);
                value["error"]["permissions"] = serde_json::json!(required.permissions);
                value["error"]["creates"] = serde_json::json!(required.creates);
            }
            Some(ErrorDetail::DeployProvider(provider)) => {
                value["error"]["provider"] = serde_json::json!(provider)
            }
            Some(ErrorDetail::CloudProvider(provider)) => {
                value["error"]["provider"] = serde_json::json!(provider)
            }
            Some(ErrorDetail::CloudDocument(reason)) => {
                value["error"]["reason"] = serde_json::json!(reason)
            }
            _ => {}
        }
        value
    }
    pub(crate) fn owned_read(&self) -> Option<OwnedReadFailure> {
        match self.detail.as_deref() {
            Some(ErrorDetail::OwnedRead(failure)) => Some(*failure),
            _ => None,
        }
    }
    pub(crate) fn with_deploy_provider(
        mut self,
        provider: Option<DeployProviderDiagnostic>,
    ) -> Self {
        self.detail = provider.map(|provider| Box::new(ErrorDetail::DeployProvider(provider)));
        self
    }
    pub(crate) fn with_cloud_provider(mut self, provider: CloudProviderDiagnostic) -> Self {
        self.detail = Some(Box::new(ErrorDetail::CloudProvider(provider)));
        self
    }
    pub(crate) fn with_cloud_document(mut self, reason: CloudDocumentReason) -> Self {
        self.detail = Some(Box::new(ErrorDetail::CloudDocument(reason)));
        self
    }
    pub fn with_hint(mut self, hint: &str) -> Self {
        self.hint = Some(hint.into());
        self
    }
    /// Shared missing-input refusal. The CLI's ordinary error publisher owns exit 1;
    /// guidance is human-only and never adds provider/user strings to JSON's message.
    pub fn deploy_inputs(required: crate::deploy_input::RequiredInputs, rerun: &str) -> Self {
        let mut error = Self::new("REMOTE_DEPLOY_INPUT_REQUIRED", &required.message())
            .with_hint(&required.guidance(rerun));
        error.detail = Some(Box::new(ErrorDetail::DeployInputs(required)));
        error
    }
    /// The one-string `message` of JSON output, which has no hint field.
    pub fn json_message(&self) -> String {
        if matches!(self.detail.as_deref(), Some(ErrorDetail::DeployInputs(_))) {
            return self.message.clone();
        }
        match &self.hint {
            Some(hint) => format!("{}: {hint}", self.message),
            None => self.message.clone(),
        }
    }
}
impl fmt::Display for RemoteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.message, self.code)
    }
}
impl std::error::Error for RemoteError {}
impl From<io::Error> for RemoteError {
    fn from(_: io::Error) -> Self {
        Self::new("REMOTE_IO", "Remote I/O failed; the door is closed.")
    }
}

/// Sanitized HTTP evidence for a rejected deploy read. No provider strings cross the boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeployOperation {
    Identity,
    Project,
    Database,
    OwnerRegistration,
    Fields,
    Indexes,
    RulesRelease,
    RulesSet,
    WebApps,
    WebAppConfig,
    WebAppOperation,
    SignInConfig,
    GoogleSignInConfig,
    HostingSite,
    HostingChannel,
    HostingVersion,
    HostingFiles,
    HostingRelease,
    HostingUpload,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeployProviderDiagnostic {
    pub operation: DeployOperation,
    pub status: u16,
}
impl DeployProviderDiagnostic {
    pub(crate) fn valid(self) -> bool {
        (100..=599).contains(&self.status) && !(200..=299).contains(&self.status)
    }
}
impl DeployOperation {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Identity => "the signed-in account",
            Self::Project => "the project",
            Self::Database => "the database",
            Self::OwnerRegistration => "machine owner registration",
            Self::Fields => "Firestore field configurations",
            Self::Indexes => "Firestore composite indexes",
            Self::RulesRelease => "the Rules release",
            Self::RulesSet => "the Rules source",
            Self::WebApps => "web apps",
            Self::WebAppConfig => "web app configuration",
            Self::WebAppOperation => "web app provisioning",
            Self::SignInConfig => "sign-in configuration",
            Self::GoogleSignInConfig => "Google sign-in configuration",
            Self::HostingSite => "the Hosting site",
            Self::HostingChannel => "the Hosting channel",
            Self::HostingVersion => "the Hosting version",
            Self::HostingFiles => "Hosting files",
            Self::HostingRelease => "the Hosting release",
            Self::HostingUpload => "the Hosting upload",
        }
    }
}

/// Fixed cloud HTTP evidence only; raw provider messages and fields are never retained.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CloudProviderOperation {
    InboxRead,
    ResponseRead,
    InboxQuery,
    InboxCreate,
    ResponseCreate,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CloudProviderStatus {
    Cancelled,
    Unknown,
    InvalidArgument,
    DeadlineExceeded,
    NotFound,
    AlreadyExists,
    PermissionDenied,
    ResourceExhausted,
    FailedPrecondition,
    Aborted,
    OutOfRange,
    Unimplemented,
    Internal,
    Unavailable,
    DataLoss,
    Unauthenticated,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct CloudProviderDiagnostic {
    pub operation: CloudProviderOperation,
    pub status: u16,
    #[serde(rename = "errorStatus", skip_serializing_if = "Option::is_none")]
    pub error_status: Option<CloudProviderStatus>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ErrorDetail {
    DeployInputs(crate::deploy_input::RequiredInputs),
    OwnedRead(OwnedReadFailure),
    DeployProvider(DeployProviderDiagnostic),
    CloudProvider(CloudProviderDiagnostic),
    CloudDocument(CloudDocumentReason),
}

/// Closed codec stages; contains no provider values, identifiers or ciphertext.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CloudDocumentReason {
    QueryShape,
    NameShape,
    FieldsShape,
    PeerShape,
    CreateTime,
    IndexedTime,
    TimeDelta,
    Encoding,
    Size,
}

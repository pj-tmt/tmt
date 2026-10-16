use std::{fmt, io};

#[derive(Debug, PartialEq, Eq)]
pub struct RemoteError {
    pub code: String,
    pub message: String,
    /// The next step, when there is one. Human output prints it as its own `hint:` line.
    pub hint: Option<String>,
    /// Active session cap for a signed eviction refusal.
    pub limit: Option<usize>,
    /// Fixed operation and numeric HTTP status for deployment preflight failures only.
    pub provider: Option<DeployProviderDiagnostic>,
}
impl RemoteError {
    pub fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            hint: None,
            limit: None,
            provider: None,
        }
    }
    pub fn json_value(&self) -> serde_json::Value {
        let mut value =
            serde_json::json!({"error":{"code":self.code,"message":self.json_message()}});
        if let Some(provider) = self.provider {
            value["error"]["provider"] = serde_json::json!(provider);
        }
        value
    }
    pub fn with_hint(mut self, hint: &str) -> Self {
        self.hint = Some(hint.into());
        self
    }
    /// The one-string `message` of JSON output, which has no hint field.
    pub fn json_message(&self) -> String {
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

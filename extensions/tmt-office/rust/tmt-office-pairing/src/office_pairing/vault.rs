//! Explicit OS-store selection. No global default, plaintext or mock fallback.

use super::OfficeError;
use keyring_core::{Entry, Error, api::CredentialStoreApi};

const SERVICE: &str = "org.tmux-team.office.v1";
pub(super) const RECORD_LIMIT: usize = 32 * 1024;

/// Call only inside the parent's bounded companion process. Platform providers
/// can block on a locked collection or an OS prompt; they own no TMT daemon.
pub struct ProtectedEntry(Entry);

impl ProtectedEntry {
    pub fn open(scope: &str) -> Result<Self, OfficeError> {
        if !tmt_core::content_digest::is_sha256(scope) {
            return Err(OfficeError::CredentialsInvalid);
        }
        #[cfg(target_os = "linux")]
        let store = zbus_secret_service_keyring_store::Store::new().map_err(failure)?;
        #[cfg(target_os = "macos")]
        let store = apple_native_keyring_store::keychain::Store::new().map_err(failure)?;
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        return store.build(SERVICE, scope, None).map(Self).map_err(failure);
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        Err(OfficeError::CredentialsUnavailable)
    }

    pub fn read(&self) -> Result<Option<Vec<u8>>, OfficeError> {
        match self.0.get_secret() {
            Ok(bytes) if bytes.len() <= RECORD_LIMIT => Ok(Some(bytes)),
            Ok(_) => Err(OfficeError::CredentialsInvalid),
            Err(Error::NoEntry) => Ok(None),
            Err(error) => Err(failure(error)),
        }
    }

    pub fn write(&self, bytes: &[u8]) -> Result<(), OfficeError> {
        if bytes.is_empty() || bytes.len() > RECORD_LIMIT {
            return Err(OfficeError::CredentialsInvalid);
        }
        self.0.set_secret(bytes).map_err(failure)?;
        // Do not report publication from a successful write alone. A later
        // uncertain failure retains the same scope for explicit recovery.
        if self.read()?.as_deref() != Some(bytes) {
            return Err(OfficeError::CredentialsUnavailable);
        }
        Ok(())
    }
}

fn failure(error: Error) -> OfficeError {
    // Provider errors may embed secret bytes or identifying attributes.
    // Never retain them as an output/source error or format them for diagnostics.
    match error {
        Error::BadEncoding(_) | Error::BadDataFormat(_, _) | Error::Ambiguous(_) => {
            OfficeError::CredentialsInvalid
        }
        _ => OfficeError::CredentialsUnavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_scope_is_rejected_before_opening_any_platform_store() {
        for scope in ["", "secret-user-value", "../path", &"A".repeat(64)] {
            assert!(matches!(
                ProtectedEntry::open(scope),
                Err(OfficeError::CredentialsInvalid)
            ));
        }
    }

    #[test]
    fn provider_errors_never_retain_or_format_secret_material() {
        assert_eq!(
            failure(Error::BadEncoding(b"private-material".to_vec())).to_string(),
            "OFFICE_CREDENTIALS_INVALID"
        );
        assert_eq!(
            failure(Error::Invalid(
                "private-account".into(),
                "private-material".into()
            ))
            .to_string(),
            "OFFICE_CREDENTIALS_UNAVAILABLE"
        );
    }
}

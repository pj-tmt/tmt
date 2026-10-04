//! Optional cross-extension uses (#1575): a read-only answer from installed
//! receipts. It reads no network, application storage or extension process.

use super::{Fault, Request, invalid};
use crate::native_install::{self, CheckError};
use serde::Deserialize;
use std::path::Path;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UsesInput {
    extension: String,
    feature: String,
}

pub(super) fn decode(input: &[u8]) -> Result<Request, Fault> {
    let value: UsesInput = serde_json::from_slice(input).map_err(|_| invalid())?;
    if native_install::extension_named(&value.extension, None).is_none()
        || !native_install::valid_feature(&value.feature)
    {
        return Err(invalid());
    }
    Ok(Request::ExtensionUse {
        extension: value.extension,
        feature: value.feature,
    })
}

/// The caller's own receipt names the use; the named extension's receipt and
/// version decide availability. `prefix` is the installation prefix the CLI
/// uses without `--prefix`.
pub(super) fn uses(prefix: &Path, extension: &str, feature: &str) -> Result<Vec<u8>, Fault> {
    let caller = native_install::extension_named(extension, None).ok_or_else(invalid)?;
    let status =
        native_install::check_use(prefix, caller, feature).map_err(|error| match error {
            CheckError::Undeclared(message) => Fault::detailed("EXTENSION_USE_UNDECLARED", message),
            CheckError::Io(_) => Fault::unavailable(),
        })?;
    let mut body = status.to_json();
    body["hint"] = status.hint(None).into();
    serde_json::to_vec(&body).map_err(|_| Fault::unavailable())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn decoded(input: &str) -> Result<Request, Fault> {
        decode(input.as_bytes())
    }

    #[test]
    fn names_and_features_are_bounded_canonical_data() {
        assert!(matches!(
            decoded(r#"{"extension":"colab","feature":"browser-access"}"#),
            Ok(Request::ExtensionUse { .. })
        ));
        for input in [
            r#"{"extension":"cli","feature":"a"}"#,
            r#"{"extension":"office","feature":"a"}"#,
            r#"{"extension":"nothing","feature":"a"}"#,
            r#"{"extension":"colab","feature":"Browser"}"#,
            r#"{"extension":"colab","feature":""}"#,
            r#"{"extension":"colab","feature":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#,
            r#"{"extension":"colab"}"#,
            r#"{"extension":"colab","feature":"a","extra":1}"#,
        ] {
            assert_eq!(
                decoded(input).err().map(|fault| fault.code()),
                Some("API_INPUT_INVALID"),
                "{input}"
            );
        }
    }

    #[test]
    fn a_caller_without_a_managed_installation_is_undeclared_and_nothing_is_created() {
        let directory = crate::test_support::TestDirectory::new();
        let prefix = directory.path.join("prefix");
        let fault = uses(&prefix, "colab", "browser-access").unwrap_err();
        assert_eq!(fault.code(), "EXTENSION_USE_UNDECLARED");
        assert!(!prefix.exists());
        let body: Value = serde_json::from_slice(&fault.encode()).unwrap();
        assert_eq!(body["error"]["code"], "EXTENSION_USE_UNDECLARED");
    }

    #[test]
    fn the_operation_is_advertised_by_capabilities() {
        let capabilities: Value = serde_json::from_slice(&super::super::capabilities()).unwrap();
        assert!(
            capabilities["operations"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!("extensions.uses"))
        );
    }
}

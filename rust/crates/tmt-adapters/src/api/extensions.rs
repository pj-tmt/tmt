//! Optional cross-extension uses (#1575): a read-only answer from installed
//! receipts. It reads no network, application storage or extension process.

use super::{Fault, Request, invalid};
use crate::native_install::{self, CheckError, Product};
use serde::Deserialize;
use serde_json::json;
use std::path::Path;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UsesInput {
    extension: String,
    feature: String,
}

/// An extension name is a fixed official product; a feature id is bounded
/// lower-case data the declarer chose.
fn product(name: &str) -> Option<Product> {
    Product::ALL
        .into_iter()
        .filter(|product| !matches!(product, Product::Cli | Product::Office))
        .find(|product| product.as_str() == name)
}

fn feature_id(text: &str) -> bool {
    let mut bytes = text.bytes();
    text.len() <= 32
        && bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

pub(super) fn decode(input: &[u8]) -> Result<Request, Fault> {
    let value: UsesInput = serde_json::from_slice(input).map_err(|_| invalid())?;
    if product(&value.extension).is_none() || !feature_id(&value.feature) {
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
    let caller = product(extension).ok_or_else(invalid)?;
    let status =
        native_install::check_use(prefix, caller, feature).map_err(|error| match error {
            CheckError::Undeclared(message) => Fault::detailed("EXTENSION_USE_UNDECLARED", message),
            CheckError::Io(_) => Fault::unavailable(),
        })?;
    let declared = &status.declared;
    serde_json::to_vec(&json!({
        "feature": declared.feature,
        "label": declared.label,
        "extension": declared.extension.as_str(),
        "requires": format!(">={}", declared.minimum),
        "available": status.available(),
        "installed": status.installed.as_ref().map(ToString::to_string),
        "reason": status.unavailable.map(native_install::Unavailable::as_str),
        "hint": status.hint(None),
    }))
    .map_err(|_| Fault::unavailable())
}

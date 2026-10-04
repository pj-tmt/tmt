//! Optional cross-extension uses (#1575): the `TMT-USES.json` an extension
//! release may carry, and the use-time answer about the extension it names.
//! Core never interprets a feature; it only compares an installed version with a
//! declared minimum. Nothing here installs, executes or reads application state.

use super::{Product, inspect_product_prefix};
use crate::bounded_file;
use semver::Version;
use serde_json::{Map, Value};
use std::{
    io,
    path::{Path, PathBuf},
};

/// The release file that declares uses (see `Product::optional_files`).
pub(super) const FILE: &str = "TMT-USES.json";
const LIMIT: usize = 4 * 1024;
const MAXIMUM_USES: usize = 8;
const MINIMUM_PREFIX: &str = ">=";

/// One declared use: `feature` of the owning extension needs `extension` at
/// least at `minimum`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Use {
    pub feature: String,
    pub label: String,
    pub extension: Product,
    pub minimum: Version,
}

/// Why a use is not available now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unavailable {
    Missing,
    TooOld,
    /// Installed but failing verification.
    Damaged,
}

impl Unavailable {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::TooOld => "tooOld",
            Self::Damaged => "damaged",
        }
    }
}

/// A declared use and what is installed for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UseStatus {
    pub declared: Use,
    pub installed: Option<Version>,
    pub unavailable: Option<Unavailable>,
}

impl UseStatus {
    pub fn available(&self) -> bool {
        self.unavailable.is_none()
    }

    /// One actionable line, empty when available. `prefix` is added to the
    /// printed commands when the installation is not at the default one.
    pub fn hint(&self, prefix: Option<&Path>) -> String {
        let name = self.declared.extension.as_str();
        let title = capitalized(name);
        let label = &self.declared.label;
        let suffix = prefix.map_or_else(String::new, |prefix| {
            format!(" --prefix {}", shell_word(&prefix.to_string_lossy()))
        });
        match self.unavailable {
            None => String::new(),
            Some(Unavailable::Missing) => {
                format!(
                    "{label} needs the {title} extension: tmt extension install {name} --yes{suffix}"
                )
            }
            Some(Unavailable::TooOld) => format!(
                "{label} needs {title} {} or newer (installed {}): tmt extension upgrade {name} --yes{suffix}",
                self.declared.minimum,
                self.installed
                    .as_ref()
                    .map_or_else(String::new, ToString::to_string)
            ),
            Some(Unavailable::Damaged) => format!(
                "{label} needs the {title} extension, which is installed but damaged: run tmt extension ls{suffix} for its repair"
            ),
        }
    }
}

/// Why `check` could not answer.
#[derive(Debug)]
pub enum CheckError {
    /// The caller has no managed installation, or declares no such feature.
    Undeclared(String),
    /// Reading the caller's own installation failed.
    Io(io::Error),
}

fn capitalized(name: &str) -> String {
    let mut chars = name.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

fn shell_word(text: &str) -> String {
    if !text.is_empty()
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"/._-:=@%+,".contains(&byte))
    {
        text.to_owned()
    } else {
        format!("'{}'", text.replace('\'', "'\\''"))
    }
}

/// Safe on one terminal line: no control, bidirectional-override or
/// line-separator characters.
fn printable(character: char) -> bool {
    !character.is_control()
        && !matches!(
            character,
            '\u{061C}'
                | '\u{200E}'
                | '\u{200F}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2066}'..='\u{2069}'
        )
}

fn feature_id(text: &str) -> bool {
    let mut bytes = text.bytes();
    text.len() <= 32
        && bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

/// An installable official extension other than `owner` (Office is frozen).
fn target(name: &str, owner: Product) -> Option<Product> {
    Product::ALL
        .into_iter()
        .filter(|product| !matches!(product, Product::Cli | Product::Office))
        .find(|product| product.as_str() == name && *product != owner)
}

fn minimum(text: &str) -> Option<Version> {
    let version = text.strip_prefix(MINIMUM_PREFIX)?;
    let parsed = Version::parse(version).ok()?;
    (text.len() <= 64 && parsed.build.is_empty()).then_some(parsed)
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("Invalid {FILE}: {message}"),
    )
}

fn exactly(object: &Map<String, Value>, keys: &[&str]) -> bool {
    object.len() == keys.len() && keys.iter().all(|key| object.contains_key(*key))
}

/// Parse and validate a release's declaration of uses for `owner`.
pub(super) fn parse(owner: Product, bytes: &[u8]) -> io::Result<Vec<Use>> {
    if bytes.len() > LIMIT {
        return Err(invalid("larger than 4 KiB"));
    }
    let document: Value = serde_json::from_slice(bytes).map_err(|_| invalid("not JSON"))?;
    let object = document
        .as_object()
        .filter(|object| exactly(object, &["version", "uses"]))
        .ok_or_else(|| invalid("expected exactly version and uses"))?;
    if object["version"] != 1 {
        return Err(invalid("unsupported version"));
    }
    let entries = object["uses"]
        .as_array()
        .filter(|entries| entries.len() <= MAXIMUM_USES)
        .ok_or_else(|| invalid("uses must be a list of at most 8"))?;
    let mut uses = Vec::new();
    for entry in entries {
        let entry = entry
            .as_object()
            .filter(|entry| exactly(entry, &["feature", "label", "extension", "requires"]))
            .ok_or_else(|| invalid("each use needs exactly feature, label, extension, requires"))?;
        let text = |key: &str| entry[key].as_str();
        let feature = text("feature")
            .filter(|feature| feature_id(feature))
            .ok_or_else(|| invalid("feature must match [a-z][a-z0-9-]{0,31}"))?;
        let label = text("label")
            .filter(|label| {
                (1..=48).contains(&label.chars().count()) && label.chars().all(printable)
            })
            .ok_or_else(|| invalid("label must be 1 to 48 printable characters"))?;
        let extension = text("extension")
            .and_then(|name| target(name, owner))
            .ok_or_else(|| invalid("extension must be another installable official extension"))?;
        let minimum = text("requires")
            .and_then(minimum)
            .ok_or_else(|| invalid("requires must be >=X.Y.Z or >=X.Y.Z-pre"))?;
        if uses.iter().any(|used: &Use| used.feature == feature) {
            return Err(invalid("duplicate feature"));
        }
        uses.push(Use {
            feature: feature.to_owned(),
            label: label.to_owned(),
            extension,
            minimum,
        });
    }
    Ok(uses)
}

/// The uses an installed, verified release declares; empty without the file.
/// An error means the product has no readable managed installation.
pub fn declared(product: Product, prefix: &Path) -> io::Result<Vec<Use>> {
    let installation = inspect_product_prefix(product, prefix)?;
    let release: PathBuf = installation
        .active_executable
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| invalid("release directory is unknown"))?;
    let file = release.join(FILE);
    match std::fs::symlink_metadata(&file) {
        Ok(_) => {
            let bytes = bounded_file::read_no_follow(&file, LIMIT).map_err(io::Error::other)?;
            parse(product, &bytes)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

/// What is installed for one declared use.
pub fn status(declared: Use, prefix: &Path) -> UseStatus {
    // Without its command link the extension cannot be used, whether it was
    // never installed or its commands were removed.
    let command = prefix.join("bin").join(declared.extension.executable());
    let linked = !matches!(
        std::fs::symlink_metadata(&command),
        Err(error) if error.kind() == io::ErrorKind::NotFound
    );
    let (installed, unavailable) = match linked
        .then(|| inspect_product_prefix(declared.extension, prefix))
        .unwrap_or_else(|| Err(io::Error::from(io::ErrorKind::NotFound)))
    {
        Ok(installation) => {
            let version = installation.state.version;
            let unavailable = (version < declared.minimum).then_some(Unavailable::TooOld);
            (Some(version), unavailable)
        }
        // No directory or receipt for it: it was never installed by TMT.
        Err(error) if error.kind() == io::ErrorKind::NotFound => (None, Some(Unavailable::Missing)),
        Err(_) => (None, Some(Unavailable::Damaged)),
    };
    UseStatus {
        declared,
        installed,
        unavailable,
    }
}

/// Every declared use of an installed extension with its current status.
pub fn statuses(product: Product, prefix: &Path) -> io::Result<Vec<UseStatus>> {
    Ok(declared(product, prefix)?
        .into_iter()
        .map(|declared| status(declared, prefix))
        .collect())
}

/// The use-time answer for `feature` of `caller`.
pub fn check(prefix: &Path, caller: Product, feature: &str) -> Result<UseStatus, CheckError> {
    let uses = declared(caller, prefix).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            CheckError::Undeclared(format!(
                "{} has no managed installation to read its optional uses from.",
                caller.as_str()
            ))
        } else {
            CheckError::Io(error)
        }
    })?;
    uses.into_iter()
        .find(|used| used.feature == feature)
        .map(|declared| status(declared, prefix))
        .ok_or_else(|| {
            CheckError::Undeclared(format!(
                "{} declares no optional use named {feature}.",
                caller.as_str()
            ))
        })
}

/// A feature of an installed extension that stops working.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Affected {
    pub extension: Product,
    pub feature: String,
    pub label: String,
}

/// The features of other installed extensions that work now through `target`
/// and would stop if `target` were removed (`result` is `None`) or replaced by
/// the exact version `result`.
pub fn affected(prefix: &Path, target: Product, result: Option<&Version>) -> Vec<Affected> {
    Product::ALL
        .into_iter()
        .filter(|product| !matches!(product, Product::Cli | Product::Office) && *product != target)
        .flat_map(|owner| {
            statuses(owner, prefix)
                .unwrap_or_default()
                .into_iter()
                .filter(|used| used.declared.extension == target && used.available())
                .filter(|used| result.is_none_or(|version| *version < used.declared.minimum))
                .map(move |used| Affected {
                    extension: owner,
                    feature: used.declared.feature,
                    label: used.declared.label,
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"{"version":1,"uses":[{"feature":"browser-access","label":"Browser access","extension":"remote","requires":">=0.1.0-alpha.1"}]}"#;

    fn parsed(text: &str) -> io::Result<Vec<Use>> {
        parse(Product::Colab, text.as_bytes())
    }

    #[test]
    fn a_valid_declaration_parses_with_a_prerelease_aware_minimum() {
        let uses = parsed(GOOD).unwrap();
        assert_eq!(uses.len(), 1);
        assert_eq!(uses[0].extension, Product::Remote);
        assert_eq!(uses[0].minimum, Version::parse("0.1.0-alpha.1").unwrap());
        assert!(Version::parse("0.1.0-alpha.2").unwrap() >= uses[0].minimum);
        assert!(Version::parse("0.1.0-alpha.0").unwrap() < uses[0].minimum);
        assert!(Version::parse("0.1.0").unwrap() >= uses[0].minimum);
        assert_eq!(parsed(r#"{"version":1,"uses":[]}"#).unwrap(), []);
    }

    #[test]
    fn every_rule_rejects_the_whole_declaration() {
        let with = |from: &str, to: &str| GOOD.replace(from, to);
        let long_label = "x".repeat(49);
        let cases = [
            ("not json", "nope".to_owned()),
            ("unknown top key", with("\"uses\"", "\"extra\":1,\"uses\"")),
            ("wrong version", with("\"version\":1", "\"version\":2")),
            (
                "unknown use key",
                with("\"label\"", "\"extra\":1,\"label\""),
            ),
            ("bad feature", with("browser-access", "Browser_Access")),
            ("feature too long", with("browser-access", &"a".repeat(33))),
            ("empty label", with("Browser access", "")),
            ("long label", with("Browser access", &long_label)),
            (
                "control in label",
                with("Browser access", "Browser\\u0007access"),
            ),
            (
                "bidi in label",
                with("Browser access", "Browser\\u202eaccess"),
            ),
            (
                "line separator in label",
                with("Browser access", "Browser\\u2028access"),
            ),
            ("unknown extension", with("remote", "nothing")),
            ("the cli is not an extension", with("remote", "cli")),
            ("frozen extension", with("remote", "office")),
            ("itself", with("remote", "colab")),
            ("range operator", with(">=0.1.0-alpha.1", "^0.1.0")),
            ("exact version", with(">=0.1.0-alpha.1", "0.1.0")),
            ("build metadata", with(">=0.1.0-alpha.1", ">=0.1.0+build")),
            ("not semver", with(">=0.1.0-alpha.1", ">=latest")),
        ];
        for (name, text) in cases {
            assert!(parsed(&text).is_err(), "{name} was accepted");
        }
        let one = r#"{"feature":"a","label":"A","extension":"remote","requires":">=0.1.0"}"#;
        let nine = format!(
            r#"{{"version":1,"uses":[{}]}}"#,
            (0..9)
                .map(|n| one.replace("\"a\"", &format!("\"f{n}\"")))
                .collect::<Vec<_>>()
                .join(",")
        );
        assert!(parsed(&nine).is_err(), "nine uses were accepted");
        let duplicate = format!(r#"{{"version":1,"uses":[{one},{one}]}}"#);
        assert!(parsed(&duplicate).is_err(), "duplicate feature accepted");
        let padded = format!("{GOOD}{}", " ".repeat(LIMIT));
        assert!(parsed(&padded).is_err(), "oversized file accepted");
    }

    #[test]
    fn hints_are_one_line_and_name_the_exact_command() {
        let declared = parsed(GOOD).unwrap().remove(0);
        let missing = UseStatus {
            declared: declared.clone(),
            installed: None,
            unavailable: Some(Unavailable::Missing),
        };
        assert_eq!(
            missing.hint(None),
            "Browser access needs the Remote extension: tmt extension install remote --yes"
        );
        assert!(
            missing
                .hint(Some(Path::new("/tmp/a b")))
                .ends_with("tmt extension install remote --yes --prefix '/tmp/a b'")
        );
        let old = UseStatus {
            declared: declared.clone(),
            installed: Some(Version::parse("0.1.0-alpha.0").unwrap()),
            unavailable: Some(Unavailable::TooOld),
        };
        assert_eq!(
            old.hint(None),
            "Browser access needs Remote 0.1.0-alpha.1 or newer (installed 0.1.0-alpha.0): tmt extension upgrade remote --yes"
        );
        let damaged = UseStatus {
            declared: declared.clone(),
            installed: None,
            unavailable: Some(Unavailable::Damaged),
        };
        assert!(damaged.hint(None).contains("tmt extension ls"));
        let fine = UseStatus {
            declared,
            installed: Some(Version::parse("0.1.0-alpha.1").unwrap()),
            unavailable: None,
        };
        assert!(fine.available());
        assert_eq!(fine.hint(None), "");
    }
}

//! Guided setup composes the verified installer without upgrading healthy releases.

use super::{Human, failure, prefix, skills};
use crate::output::Failure;
use serde_json::{Value, json};
use std::{fs, io, path::Path};
use tmt_adapters::{
    native_install::{self, Product},
    skill_installation::{self, ProviderEnvironment},
};

pub(crate) struct SetupState {
    pub(crate) installed: bool,
    pub(crate) skills_current: bool,
    pub(crate) version: Option<String>,
}

/// The official inventory has one product owner; dependencies determine apply order.
pub(crate) const PRODUCTS: [Product; Product::ALL.len() - 1] = {
    let mut products = [Product::Cli; Product::ALL.len() - 1];
    let mut source = 0;
    let mut target = 0;
    while source < Product::ALL.len() {
        if !matches!(Product::ALL[source], Product::Cli) {
            products[target] = Product::ALL[source];
            target += 1;
        }
        source += 1;
    }
    products
};

pub(crate) fn dependencies(product: Product) -> &'static [Product] {
    match product {
        Product::Colab => &[Product::Remote],
        _ => &[],
    }
}

/// Read-only installation facts shared by the setup planner and public listing.
pub(crate) struct Entry {
    pub(crate) product: Product,
    pub(crate) installation: Installation,
}

pub(crate) enum Installation {
    Missing,
    Managed(Box<native_install::ManagedInstallation>),
    PartiallyRemoved,
    Invalid(io::Error),
}

pub(crate) fn inventory(prefix: &Path) -> Vec<Entry> {
    PRODUCTS
        .into_iter()
        .map(|product| Entry::read(product, prefix))
        .collect()
}

impl Entry {
    fn read(product: Product, prefix: &Path) -> Self {
        let observe = || -> io::Result<Installation> {
            if exists(&prefix.join(product.namespace()).join("current"))? {
                for link in product.links() {
                    if !exists(&prefix.join("bin").join(link))? {
                        return Ok(Installation::PartiallyRemoved);
                    }
                }
            }
            if !exists(&prefix.join("bin").join(product.executable()))? {
                return Ok(Installation::Missing);
            }
            native_install::inspect_product_prefix(product, prefix)
                .map(Box::new)
                .map(Installation::Managed)
        };
        Self {
            product,
            installation: observe().unwrap_or_else(Installation::Invalid),
        }
    }

    pub(crate) fn facts(&self) -> Value {
        let version = match &self.installation {
            Installation::Managed(installation) => Some(installation.state.version.to_string()),
            _ => None,
        };
        json!({"name":self.product.as_str(), "official":true,
            "installed":version.is_some(), "version":version,
            "missing":matches!(self.installation, Installation::Missing),
            "dependencies":dependencies(self.product).iter().map(|product| product.as_str()).collect::<Vec<_>>()})
    }

    pub(crate) fn plan(&self) -> Result<SetupState, Failure> {
        let product = self.product;
        let installation = match &self.installation {
            Installation::Missing => {
                return Ok(SetupState {
                    installed: false,
                    skills_current: false,
                    version: None,
                });
            }
            Installation::Managed(installation) => installation,
            Installation::PartiallyRemoved => {
                return Err(Failure::new(
                    "EXTENSION_INSTALLATION_INVALID",
                    format!(
                        "{} has an activation but missing command links. Finish removal with: tmt extension rm {} --yes",
                        product.as_str(),
                        product.as_str()
                    ),
                    1,
                ));
            }
            Installation::Invalid(error) => {
                return Err(recovery(
                    product,
                    io::Error::new(error.kind(), error.to_string()),
                ));
            }
        };
        let executable = &installation.active_executable;
        let bundled = native_install::release_skills(product, executable)
            .map_err(|error| recovery(product, error))?;
        let env = ProviderEnvironment::capture()
            .map_err(|error| failure("EXTENSION_SKILLS_FAILED", error))?;
        let current = skill_installation::owned_skills_current(
            &env,
            &skills::global_dir()?,
            product.as_str(),
            &bundled,
        )
        .map_err(|error| failure("EXTENSION_SKILLS_FAILED", error))?;
        Ok(SetupState {
            installed: true,
            skills_current: current,
            version: Some(installation.state.version.to_string()),
        })
    }
}

fn exists(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn recovery(product: Product, error: std::io::Error) -> Failure {
    Failure::new("EXTENSION_INSTALLATION_INVALID", format!(
        "{} could not be verified: {error}. Inspect with: tmt extension ls; repair with: tmt extension install {} --repair --yes",
        product.as_str(), product.as_str()), 1).caused_by(error)
}

/// Recheck after consent. Existing binaries are never passed to the upgrade path.
pub(crate) fn apply(
    product: Product,
    progress: native_install::ProgressObserver<'_>,
) -> Result<Value, Failure> {
    apply_at(product, &prefix(None)?, None, progress)
}

pub(crate) fn apply_at(
    product: Product,
    prefix: &Path,
    selected: Option<&str>,
    progress: native_install::ProgressObserver<'_>,
) -> Result<Value, Failure> {
    let entry = Entry::read(product, prefix);
    let state = entry.plan()?;
    if !state.installed {
        return super::install_offered_at(product, Some(prefix), selected, progress)
            .map(|(document, _)| document);
    }
    let executable = prefix.join("bin").join(product.executable());
    let mut document = json!({"extension": product.as_str(), "installed": true, "changed": false,
        "version": state.version, "executable": executable,
        "skills": {"status": "current"}});
    if !state.skills_current {
        let mut human = Human::plain(String::new());
        let Installation::Managed(current) = &entry.installation else {
            unreachable!("verified installed release");
        };
        skills::settle_skills(
            product,
            &current.active_executable,
            &[],
            &mut document,
            &mut human,
        )?;
    }
    Ok(document)
}

#[cfg(test)]
pub(crate) const PRINTED_HINTS: &[crate::cli_style_tests::HintSpec] = &[
    crate::cli_style_tests::HintSpec::core(
        "{} has an activation but missing command links. Finish removal with: tmt extension rm {} --yes",
        &[""],
        &[("{}", "ops")],
    ),
    crate::cli_style_tests::HintSpec::core(
        "{} could not be verified: {error}. Inspect with: tmt extension ls; repair with: tmt extension install {} --repair --yes",
        &["; repair with:", ""],
        &[("{}", "ops")],
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_inventory_and_setup_share_missing_facts_and_dependencies_without_writes() {
        let root = std::env::temp_dir().join(format!("tmt-setup-inventory-{}", std::process::id()));
        assert!(!root.exists());
        let inventory = inventory(&root);
        let (listing, _) =
            super::super::listing(&root, false, root.join("bin").as_os_str()).unwrap();
        assert_eq!(inventory.len(), 4);
        for entry in inventory {
            let facts = entry.facts();
            let public = listing["extensions"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["name"] == entry.product.as_str())
                .unwrap();
            for (key, value) in facts.as_object().unwrap() {
                assert_eq!(&public[key], value, "{key}");
            }
            let planned = entry.plan().unwrap();
            assert_eq!(facts["installed"], planned.installed);
            assert_eq!(facts["version"], json!(planned.version));
            assert_eq!(facts["missing"], true);
            assert_eq!(
                facts["dependencies"],
                if entry.product == Product::Colab {
                    json!(["remote"])
                } else {
                    json!([])
                }
            );
        }
        assert!(
            !root.exists(),
            "read-only inventory creates no prefix or provider roots"
        );
    }
}

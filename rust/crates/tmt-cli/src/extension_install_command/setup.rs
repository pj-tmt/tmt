//! Guided setup composes the verified installer without upgrading healthy releases.

use super::{Human, failure, installed, prefix, skills};
use crate::output::Failure;
use serde_json::{Value, json};
use tmt_adapters::{
    native_install::{self, Product},
    skill_installation::{self, ProviderEnvironment},
};

pub(crate) struct SetupState {
    pub(crate) installed: bool,
    pub(crate) skills_current: bool,
    pub(crate) version: Option<String>,
}

pub(crate) fn plan(product: Product) -> Result<SetupState, Failure> {
    let prefix = prefix(None)?;
    if !installed(product, &prefix)? {
        return Ok(SetupState {
            installed: false,
            skills_current: false,
            version: None,
        });
    }
    let installation = native_install::inspect_product_prefix(product, &prefix)
        .map_err(|error| recovery(product, error))?;
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
    let state = plan(product)?;
    if !state.installed {
        return super::install_offered(product, progress).map(|(document, _)| document);
    }
    let executable = prefix(None)?.join("bin").join(product.executable());
    let mut document = json!({"extension": product.as_str(), "installed": true, "changed": false,
        "version": state.version, "executable": executable,
        "skills": {"status": "current"}});
    if !state.skills_current {
        let mut human = Human::plain(String::new());
        skills::settle_skills(product, &executable, &[], &mut document, &mut human)?;
    }
    Ok(document)
}

#[cfg(test)]
pub(crate) const PRINTED_HINTS: &[crate::cli_style_tests::HintSpec] = &[
    crate::cli_style_tests::HintSpec::core(
        "{} could not be verified: {error}. Inspect with: tmt extension ls; repair with: tmt extension install {} --repair --yes",
        &["; repair with:", ""],
        &[("{}", "ops")],
    ),
];

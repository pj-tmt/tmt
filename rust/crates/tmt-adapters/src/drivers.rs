//! Each driver's behavior, keyed by its core descriptor
//! (`tmt_core::driver::descriptor`): where it keeps its files and its runtime
//! when it has one. Setup, detection, skill targets and `run` iterate a
//! [`Registry`]; nothing outside the descriptors and these modules names a
//! driver.

pub mod agy;
pub mod claude;
pub mod codex;
pub mod gemini;
pub mod opencode;
pub mod pi;

use crate::{
    runtime::{RuntimeCommand, RuntimeError, lifecycle::RuntimeLifecycle},
    skill_installation::ProviderEnvironment,
};
use std::{fmt, path::PathBuf};
use tmt_core::{
    binding::BindingEntry,
    driver::{ActionResult, Driver, caller::RuntimeCaller},
};

use tmt_core::driver::descriptor;
pub use tmt_core::driver::descriptor::{DriverDescriptor, HookFormat, Hue};

/// The runtime port a registered driver implements.
pub type RuntimeDriver =
    dyn Driver<Target = BindingEntry, Error = RuntimeError, Launch = RuntimeCommand>;

/// Where a driver keeps its files, resolved against one captured environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Locations {
    /// Directories whose presence means the driver is installed.
    pub config_dirs: Vec<PathBuf>,
    /// The root its skills are read from; a skill lives at `<root>/<name>`.
    pub skills: PathBuf,
    /// Earlier TMT guidance locations, kept for migration checks.
    pub legacy_skills: Vec<PathBuf>,
    /// The settings file its lifecycle hooks go in, when it has hooks.
    pub hook_settings: Option<PathBuf>,
}

/// A driver's runtime: launch and resume, lifecycle observation, and caller
/// recognition when the runtime can tell which conversation invoked TMT.
pub struct Runtime {
    pub driver: fn() -> Box<RuntimeDriver>,
    pub lifecycle: fn() -> Box<dyn RuntimeLifecycle>,
    pub identify_caller: Option<fn() -> ActionResult<RuntimeCaller, ()>>,
}

pub struct DriverDefinition {
    pub descriptor: &'static DriverDescriptor,
    /// Environment variables `locate` reads, captured once per invocation.
    pub env: &'static [&'static str],
    pub locate: fn(&ProviderEnvironment) -> Locations,
    pub runtime: Option<Runtime>,
}

impl DriverDefinition {
    pub fn name(&self) -> &'static str {
        self.descriptor.name
    }
}

impl fmt::Debug for DriverDefinition {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

impl PartialEq for DriverDefinition {
    fn eq(&self, other: &Self) -> bool {
        self.name() == other.name()
    }
}

impl Eq for DriverDefinition {}

/// One adapter module per core descriptor.
static MODULES: [&DriverDefinition; 6] = [
    &claude::DRIVER,
    &codex::DRIVER,
    &gemini::DRIVER,
    &agy::DRIVER,
    &pi::DRIVER,
    &opencode::DRIVER,
];

/// An ordered set of drivers with unique names. Production uses
/// [`Registry::builtin`]; a test can add a descriptor with [`Registry::with`].
#[derive(Debug, Clone)]
pub struct Registry {
    drivers: Vec<&'static DriverDefinition>,
}

impl Registry {
    /// The built-in drivers in the core descriptors' order.
    ///
    /// # Panics
    /// When a core descriptor has no adapter module: a build bug the
    /// registry test catches.
    pub fn builtin() -> Self {
        Self {
            drivers: descriptor::ALL
                .iter()
                .map(|descriptor| {
                    MODULES
                        .into_iter()
                        .find(|module| std::ptr::eq(module.descriptor, *descriptor))
                        .expect("every core descriptor has an adapter module")
                })
                .collect(),
        }
    }

    /// # Panics
    /// When the name is invalid or already registered: a caller bug.
    pub fn with(mut self, driver: &'static DriverDefinition) -> Self {
        assert!(
            DriverDescriptor::is_valid_name(driver.name()) && self.find(driver.name()).is_none(),
            "a driver name must be valid and unique"
        );
        self.drivers.push(driver);
        self
    }

    pub fn iter(&self) -> impl Iterator<Item = &'static DriverDefinition> + '_ {
        self.drivers.iter().copied()
    }

    /// The driver with this name, ignoring ASCII case.
    pub fn find(&self, name: &str) -> Option<&'static DriverDefinition> {
        self.iter()
            .find(|driver| driver.name().eq_ignore_ascii_case(name))
    }

    pub fn names(&self) -> Vec<&'static str> {
        self.iter().map(DriverDefinition::name).collect()
    }

    /// Drivers `tmt setup` can install lifecycle hooks for.
    pub fn with_hooks(&self) -> impl Iterator<Item = &'static DriverDefinition> + '_ {
        self.iter()
            .filter(|driver| driver.descriptor.hooks.is_some())
    }

    /// Every environment variable some driver reads.
    pub fn env_vars(&self) -> Vec<&'static str> {
        let mut names: Vec<&'static str> = self
            .iter()
            .flat_map(|driver| driver.env.iter().copied())
            .collect();
        names.sort_unstable();
        names.dedup();
        names
    }

    /// The first runtime that recognizes this invocation's caller.
    pub fn identify_caller(&self) -> ActionResult<RuntimeCaller, ()> {
        self.iter()
            .filter_map(|driver| driver.runtime.as_ref()?.identify_caller)
            .fold(ActionResult::Unsupported, |found, identify| {
                found.or_unsupported(identify)
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_core_descriptor_has_exactly_one_adapter_module_and_back() {
        for descriptor in descriptor::ALL {
            let modules = MODULES
                .iter()
                .filter(|module| std::ptr::eq(module.descriptor, descriptor))
                .count();
            assert_eq!(modules, 1, "{}", descriptor.name);
        }
        for module in MODULES {
            assert!(
                descriptor::ALL
                    .iter()
                    .any(|descriptor| std::ptr::eq(module.descriptor, *descriptor)),
                "{} has no core descriptor",
                module.name()
            );
        }
        assert_eq!(
            Registry::builtin().names(),
            descriptor::ALL.map(|descriptor| descriptor.name)
        );
    }
}

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

use crate::process::{CommandFailure, CommandRequest, CommandRunner};
use crate::{
    runtime::{RuntimeCommand, RuntimeError, lifecycle::RuntimeLifecycle},
    skill_installation::ProviderEnvironment,
};
use std::{
    fmt,
    path::PathBuf,
    time::{Duration, Instant},
};
use tmt_core::{
    binding::BindingEntry,
    driver::{ActionResult, Driver, caller::RuntimeCaller},
};

use tmt_core::driver::descriptor;
use tmt_core::driver::detection::{self, VersionProbe};
pub use tmt_core::driver::{
    descriptor::{DriverDescriptor, HookFormat, Hue},
    detection::Detection,
};

/// How long a driver's `--version` may run, and how much it may print.
const PROBE_DEADLINE: Duration = Duration::from_secs(5);
const PROBE_OUTPUT: usize = 4096;

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

    /// Each driver's detection: configuration directories on disk, then a
    /// bounded `--version` of the first of its executables on `PATH`.
    ///
    /// This starts the agent. `--version` is not read-only for every agent
    /// (Codex 0.159 creates `~/.codex/tmp`), so only a flow the user started to
    /// set up agents runs it; status commands and skill installation do not.
    pub fn detect(
        &self,
        environment: &ProviderEnvironment,
        runner: &impl CommandRunner,
    ) -> Vec<(&'static DriverDefinition, Detection)> {
        self.detect_within(environment, runner, PROBE_DEADLINE)
    }

    fn detect_within(
        &self,
        environment: &ProviderEnvironment,
        runner: &impl CommandRunner,
        deadline: Duration,
    ) -> Vec<(&'static DriverDefinition, Detection)> {
        self.iter()
            .map(|driver| {
                let configured = environment
                    .locations(driver)
                    .config_dirs
                    .iter()
                    .any(|directory| environment.resolve(directory).is_dir());
                let probe = driver
                    .descriptor
                    .executables
                    .iter()
                    .find_map(|name| environment.find_command(name))
                    .map_or(VersionProbe::NotFound, |executable| {
                        probe(runner, &executable, deadline)
                    });
                (driver, detection::detection_of(configured, &probe))
            })
            .collect()
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

fn probe(
    runner: &impl CommandRunner,
    executable: &std::path::Path,
    deadline: Duration,
) -> VersionProbe {
    let result = runner.execute(CommandRequest {
        program: executable.as_os_str(),
        args: &["--version".into()],
        input: &[],
        deadline: Instant::now() + deadline,
        max_output_bytes: PROBE_OUTPUT,
    });
    match result {
        Ok(output) => VersionProbe::Exited {
            success: true,
            stdout: output.stdout,
        },
        Err(error) => match error.kind {
            CommandFailure::Exit { .. } => VersionProbe::Exited {
                success: false,
                stdout: Vec::new(),
            },
            // A version longer than the bound still ran; keep what was read.
            CommandFailure::OutputLimit => VersionProbe::Exited {
                success: true,
                stdout: error.output.map(|output| output.stdout).unwrap_or_default(),
            },
            CommandFailure::Timeout => VersionProbe::TimedOut,
            CommandFailure::Spawn | CommandFailure::Io => VersionProbe::Unstartable,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{process::UnixCommandRunner, test_support::TestDirectory};
    use std::{fs, os::unix::fs::PermissionsExt};

    fn script(directory: &std::path::Path, name: &str, body: &str, mode: u32) {
        let path = directory.join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
    }

    #[test]
    fn detection_runs_a_bounded_version_check_for_each_driver() {
        let directory = TestDirectory::new();
        let home = directory.path.join("home");
        let bin = directory.path.join("bin");
        fs::create_dir_all(&bin).unwrap();
        let [present, failing, hanging, unstartable, configured, absent] =
            descriptor::ALL.map(|descriptor| descriptor.executables[0]);
        script(&bin, present, "echo 'fixture 1.2.3'", 0o755);
        script(&bin, failing, "exit 1", 0o755);
        script(&bin, hanging, "sleep 5", 0o755);
        script(&bin, unstartable, "echo never", 0o644);
        let registry = Registry::builtin();
        let config_only = registry.find(configured).unwrap();
        let environment = ProviderEnvironment::from_parts(&home, &home, vec![bin.clone()], []);
        for directory in environment.locations(config_only).config_dirs {
            fs::create_dir_all(environment.resolve(&directory)).unwrap();
        }
        let detected: Vec<(&str, Detection)> = registry
            .detect_within(
                &environment,
                &UnixCommandRunner,
                Duration::from_millis(1500),
            )
            .into_iter()
            .map(|(driver, detection)| (driver.name(), detection))
            .collect();
        let broken = |reason: &str| Detection::Broken {
            reason: reason.into(),
        };
        assert_eq!(
            detected,
            [
                (
                    present,
                    Detection::Present {
                        version: Some("fixture 1.2.3".into())
                    }
                ),
                (failing, broken("its version check failed")),
                (hanging, broken("its version check did not finish")),
                (unstartable, broken("it could not be started")),
                (configured, Detection::ConfigOnly),
                (absent, Detection::Absent),
            ]
        );
    }

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

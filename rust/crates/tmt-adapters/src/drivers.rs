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
    fmt, fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    time::{Duration, Instant},
};
use tmt_core::{
    binding::BindingEntry,
    driver::{ActionResult, Driver, caller::RuntimeCaller},
};

use tmt_core::driver::descriptor;
use tmt_core::driver::detection::{self, OnPath, VersionProbe};
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
    /// A channel to hand messages to a running agent without terminal paste,
    /// when the provider has a supported one.
    pub channel: Option<fn() -> Box<dyn crate::runtime::channel::RuntimeChannel>>,
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

/// A registered driver's name when it is a whole word of a pane's command,
/// such as `codex` in `/usr/local/bin/codex`. Whole words keep a short name
/// from matching inside another command. Every host suggests names this way.
pub(crate) fn suggested_name(command: &str) -> Option<String> {
    let command = command.to_lowercase();
    let words: Vec<&str> = command
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .collect();
    Registry::builtin()
        .names()
        .into_iter()
        .find(|name| words.contains(name))
        .map(str::to_owned)
}

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

    /// Each driver's detection from the filesystem alone: configuration
    /// directories and the first of its executables on `PATH`, with its
    /// execute permission. It never starts an agent.
    pub fn detect(
        &self,
        environment: &ProviderEnvironment,
    ) -> Vec<(&'static DriverDefinition, Detection)> {
        self.iter()
            .map(|driver| {
                let configured = environment
                    .locations(driver)
                    .config_dirs
                    .iter()
                    .any(|directory| environment.resolve(directory).is_dir());
                let on_path = executable(driver, environment).map_or(OnPath::Missing, |path| {
                    if fs::metadata(path)
                        .is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0)
                    {
                        OnPath::Executable
                    } else {
                        OnPath::NotExecutable
                    }
                });
                (driver, detection::detection_of(configured, on_path))
            })
            .collect()
    }

    /// [`Registry::detect`], then a bounded `--version` of each present
    /// driver, for diagnostics only. This runs the agents, and some write
    /// under `HOME` when they do (Codex 0.159 creates `~/.codex/tmp`); setup,
    /// install and status commands never call it.
    pub fn probe_versions(
        &self,
        environment: &ProviderEnvironment,
        runner: &impl CommandRunner,
    ) -> Vec<(&'static DriverDefinition, Detection)> {
        self.probe_within(environment, runner, PROBE_DEADLINE)
    }

    fn probe_within(
        &self,
        environment: &ProviderEnvironment,
        runner: &impl CommandRunner,
        deadline: Duration,
    ) -> Vec<(&'static DriverDefinition, Detection)> {
        self.detect(environment)
            .into_iter()
            .map(
                |(driver, detection)| match (&detection, executable(driver, environment)) {
                    (Detection::Present { .. }, Some(path)) => (
                        driver,
                        detection::with_version(&probe(runner, &path, deadline)),
                    ),
                    _ => (driver, detection),
                },
            )
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

/// The first of the driver's executables on `PATH`.
fn executable(driver: &DriverDefinition, environment: &ProviderEnvironment) -> Option<PathBuf> {
    driver
        .descriptor
        .executables
        .iter()
        .find_map(|name| environment.find_command(name))
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
    use crate::{
        process::{CommandError, CommandOutput, UnixCommandRunner},
        test_support::TestDirectory,
    };

    struct ScriptRunner;

    impl CommandRunner for ScriptRunner {
        fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
            // Read the fixture instead of execing an inode another fork may hold writable.
            let mut args = vec![request.program.to_owned()];
            args.extend_from_slice(request.args);
            UnixCommandRunner.execute(CommandRequest {
                program: std::ffi::OsStr::new("/bin/sh"),
                args: &args,
                ..request
            })
        }
    }

    fn script(directory: &std::path::Path, name: &str, body: &str, mode: u32) {
        let path = directory.join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
    }

    /// Six fixture executables, one per built-in driver, and the
    /// environment that finds them.
    fn fixture(directory: &TestDirectory) -> (ProviderEnvironment, [&'static str; 6]) {
        let home = directory.path.join("home");
        let bin = directory.path.join("bin");
        fs::create_dir_all(&bin).unwrap();
        let names = descriptor::ALL.map(|descriptor| descriptor.executables[0]);
        let [present, failing, hanging, unexecutable, configured, _absent] = names;
        // Each script records that it ran, so detection can prove it did not.
        script(
            &bin,
            present,
            "touch \"$0.ran\"; echo 'fixture 1.2.3'",
            0o755,
        );
        script(&bin, failing, "touch \"$0.ran\"; exit 1", 0o755);
        script(&bin, hanging, "touch \"$0.ran\"; sleep 5", 0o755);
        script(&bin, unexecutable, "echo never", 0o644);
        let environment = ProviderEnvironment::from_parts(&home, &home, vec![bin], []);
        let config_only = Registry::builtin().find(configured).unwrap();
        for directory in environment.locations(config_only).config_dirs {
            fs::create_dir_all(environment.resolve(&directory)).unwrap();
        }
        (environment, names)
    }

    fn named(found: Vec<(&'static DriverDefinition, Detection)>) -> Vec<(&'static str, Detection)> {
        found
            .into_iter()
            .map(|(driver, detection)| (driver.name(), detection))
            .collect()
    }

    fn broken(reason: &str) -> Detection {
        Detection::Broken {
            reason: reason.into(),
        }
    }

    #[test]
    fn detection_reads_the_filesystem_and_never_starts_an_agent() {
        let directory = TestDirectory::new();
        let (environment, [present, failing, hanging, unexecutable, configured, absent]) =
            fixture(&directory);
        let unversioned = Detection::Present { version: None };
        assert_eq!(
            named(Registry::builtin().detect(&environment)),
            [
                (present, unversioned.clone()),
                (failing, unversioned.clone()),
                (hanging, unversioned),
                (unexecutable, broken("it is not executable")),
                (configured, Detection::ConfigOnly),
                (absent, Detection::Absent),
            ]
        );
        let ran = fs::read_dir(directory.path.join("bin"))
            .unwrap()
            .filter(|entry| {
                entry
                    .as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".ran")
            })
            .count();
        assert_eq!(ran, 0);
    }

    #[test]
    fn the_explicit_probe_runs_each_present_driver_once_within_its_bound() {
        let directory = TestDirectory::new();
        let (environment, [present, failing, hanging, unexecutable, configured, absent]) =
            fixture(&directory);
        assert_eq!(
            named(Registry::builtin().probe_within(
                &environment,
                &ScriptRunner,
                Duration::from_millis(1500)
            )),
            [
                (
                    present,
                    Detection::Present {
                        version: Some("fixture 1.2.3".into())
                    }
                ),
                (failing, broken("its version check failed")),
                (hanging, broken("its version check did not finish")),
                (unexecutable, broken("it is not executable")),
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

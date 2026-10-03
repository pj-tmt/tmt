//! The environment a driver's file locations are resolved against, captured
//! once per invocation. Each driver's own module (`crate::drivers`) says where
//! it keeps skills; this module only resolves paths and detects presence.

use crate::drivers::{DriverDefinition, Locations, Registry};
use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
};

/// The directory name of TMT's core skill under a skills root.
pub const SKILL_NAME: &str = super::catalog::MAIN;

#[derive(Debug, Clone)]
pub struct ProviderEnvironment {
    home: PathBuf,
    cwd: PathBuf,
    path: Vec<PathBuf>,
    /// The non-empty environment variables the registered drivers read.
    vars: BTreeMap<String, PathBuf>,
}

impl ProviderEnvironment {
    /// Capture process state once at the invocation boundary.
    pub fn capture() -> std::io::Result<Self> {
        Ok(Self::from_parts(
            env::home_dir().ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "Cannot determine the home directory",
                )
            })?,
            env::current_dir()?,
            env::var_os("PATH")
                .map(|value| env::split_paths(&value).collect())
                .unwrap_or_default(),
            Registry::builtin()
                .env_vars()
                .into_iter()
                .filter_map(|name| Some((name, PathBuf::from(env::var_os(name)?)))),
        ))
    }

    /// Empty variable values count as unset.
    pub fn from_parts<'a>(
        home: impl Into<PathBuf>,
        cwd: impl Into<PathBuf>,
        path: Vec<PathBuf>,
        vars: impl IntoIterator<Item = (&'a str, PathBuf)>,
    ) -> Self {
        Self {
            home: home.into(),
            cwd: cwd.into(),
            path,
            vars: vars
                .into_iter()
                .filter(|(_, value)| !value.as_os_str().is_empty())
                .map(|(name, value)| (name.to_owned(), value))
                .collect(),
        }
    }

    pub fn home(&self) -> &Path {
        &self.home
    }

    /// A captured variable, unresolved.
    pub fn var(&self, name: &str) -> Option<&Path> {
        self.vars.get(name).map(PathBuf::as_path)
    }

    /// A path as the process would open it: relative to the working
    /// directory, normalized.
    pub fn resolve(&self, path: &Path) -> PathBuf {
        crate::config::normalize(&self.cwd.join(path))
    }

    pub fn locations(&self, driver: &DriverDefinition) -> Locations {
        (driver.locate)(self)
    }

    /// The built-in drivers present on this machine, in registry order.
    pub fn detect(&self) -> Vec<&'static DriverDefinition> {
        self.detect_in(&Registry::builtin())
    }

    /// A driver is present when one of its configuration directories exists or
    /// one of its executables is on `PATH`.
    pub fn detect_in(&self, registry: &Registry) -> Vec<&'static DriverDefinition> {
        registry
            .iter()
            .filter(|driver| {
                self.locations(driver)
                    .config_dirs
                    .iter()
                    .any(|directory| self.resolve(directory).exists())
                    || driver
                        .descriptor
                        .executables
                        .iter()
                        .any(|name| self.command_exists(name))
            })
            .collect()
    }

    /// Where the driver reads TMT's core skill.
    pub fn target(&self, driver: &DriverDefinition) -> PathBuf {
        self.locations(driver).skills.join(SKILL_NAME)
    }

    /// The shared skills root several drivers read.
    pub fn universal_skills(&self) -> PathBuf {
        self.home.join(".agents/skills")
    }

    pub fn universal_target(&self) -> PathBuf {
        self.universal_skills().join(SKILL_NAME)
    }

    pub fn custom_target(&self, root: impl AsRef<Path>) -> PathBuf {
        self.resolve(root.as_ref()).join(SKILL_NAME)
    }

    pub fn legacy_targets(&self, driver: &DriverDefinition) -> Vec<PathBuf> {
        self.locations(driver).legacy_skills
    }

    fn command_exists(&self, command: &str) -> bool {
        self.find_command(command).is_some()
    }

    /// The first regular file with this name on `PATH`, executable or not.
    pub fn find_command(&self, command: &str) -> Option<PathBuf> {
        self.path
            .iter()
            .map(|directory| self.resolve(&directory.join(command)))
            .find(|candidate| fs::metadata(candidate).is_ok_and(|metadata| metadata.is_file()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDirectory;

    fn environment(directory: &TestDirectory) -> ProviderEnvironment {
        let home = directory.path.join("home");
        let cwd = directory.path.join("cwd");
        fs::create_dir(&home).unwrap();
        fs::create_dir(&cwd).unwrap();
        ProviderEnvironment::from_parts(home, cwd, Vec::new(), [])
    }

    #[test]
    fn target_paths_match_the_canonical_provider_locations() {
        let directory = TestDirectory::new();
        let environment = environment(&directory);
        let home = directory.path.join("home");
        let expected = [
            (
                &crate::drivers::claude::DRIVER,
                home.join(".claude/skills/tmux-team"),
            ),
            (
                &crate::drivers::codex::DRIVER,
                home.join(".agents/skills/tmux-team"),
            ),
            (
                &crate::drivers::gemini::DRIVER,
                home.join(".agents/skills/tmux-team"),
            ),
            (
                &crate::drivers::agy::DRIVER,
                home.join(".gemini/config/skills/tmux-team"),
            ),
            (
                &crate::drivers::pi::DRIVER,
                home.join(".pi/agent/skills/tmux-team"),
            ),
            (
                &crate::drivers::opencode::DRIVER,
                home.join(".agents/skills/tmux-team"),
            ),
        ];
        for (provider, target) in expected {
            assert_eq!(environment.target(provider), target, "{provider:?}");
        }
        assert_eq!(
            environment.universal_target(),
            home.join(".agents/skills/tmux-team")
        );
        assert_eq!(
            environment.custom_target("custom skills"),
            directory.path.join("cwd/custom skills/tmux-team")
        );
    }

    #[test]
    fn claude_config_directory_owns_detection_settings_and_skills() {
        use crate::drivers::claude;
        let directory = TestDirectory::new();
        let home = directory.path.join("home");
        let cwd = directory.path.join("cwd");
        fs::create_dir_all(home.join(".claude")).unwrap();
        fs::create_dir(&cwd).unwrap();
        for (configured, expected) in [
            (PathBuf::new(), home.join(".claude")),
            (
                PathBuf::from("relative-claude"),
                cwd.join("relative-claude"),
            ),
            (
                directory.path.join("custom-claude"),
                directory.path.join("custom-claude"),
            ),
        ] {
            let environment = ProviderEnvironment::from_parts(
                &home,
                &cwd,
                Vec::new(),
                [("CLAUDE_CONFIG_DIR", configured)],
            );
            let locations = environment.locations(&claude::DRIVER);
            assert_eq!(
                locations.hook_settings,
                Some(expected.join("settings.json"))
            );
            assert_eq!(locations.skills, expected.join("skills"));
            assert_eq!(
                locations.legacy_skills,
                vec![expected.join("commands/team.md")]
            );
            assert_eq!(locations.config_dirs, vec![expected.clone()]);
            if expected != home.join(".claude") {
                assert!(!environment.detect().contains(&&claude::DRIVER));
                fs::create_dir_all(&expected).unwrap();
            }
            assert!(environment.detect().contains(&&claude::DRIVER));
        }
    }

    #[test]
    fn pi_directory_expands_home_forms_and_relative_overrides_from_cwd() {
        let directory = TestDirectory::new();
        let base = environment(&directory);
        let home = directory.path.join("home");
        let cwd = directory.path.join("cwd");
        for (configured, expected) in [
            (PathBuf::from("~"), home.join("skills/tmux-team")),
            (
                PathBuf::from("~/custom-pi"),
                home.join("custom-pi/skills/tmux-team"),
            ),
            (
                PathBuf::from("relative-pi"),
                cwd.join("relative-pi/skills/tmux-team"),
            ),
        ] {
            let environment = ProviderEnvironment::from_parts(
                &home,
                &cwd,
                Vec::new(),
                [("PI_CODING_AGENT_DIR", configured)],
            );
            assert_eq!(environment.target(&crate::drivers::pi::DRIVER), expected);
        }
        assert_eq!(
            base.target(&crate::drivers::pi::DRIVER),
            home.join(".pi/agent/skills/tmux-team")
        );
    }

    #[test]
    fn legacy_targets_preserve_claude_and_codex_migration_candidates() {
        let directory = TestDirectory::new();
        let home = directory.path.join("home");
        let cwd = directory.path.join("cwd");
        fs::create_dir(&home).unwrap();
        fs::create_dir(&cwd).unwrap();
        let codex_home = home.join("custom-codex");
        let environment = ProviderEnvironment::from_parts(
            &home,
            &cwd,
            Vec::new(),
            [("CODEX_HOME", codex_home.clone())],
        );
        assert_eq!(
            environment.legacy_targets(&crate::drivers::claude::DRIVER),
            vec![home.join(".claude/commands/team.md")]
        );
        assert_eq!(
            environment.legacy_targets(&crate::drivers::codex::DRIVER),
            vec![
                codex_home.join("skills/tmux-team"),
                home.join(".codex/skills/tmux-team"),
            ]
        );

        let shared = ProviderEnvironment::from_parts(
            &home,
            &cwd,
            Vec::new(),
            [("CODEX_HOME", home.join(".agents"))],
        );
        assert_eq!(
            shared.legacy_targets(&crate::drivers::codex::DRIVER),
            vec![home.join(".codex/skills/tmux-team")]
        );
    }

    #[test]
    fn detection_is_ordered_and_shared_agents_state_is_neutral() {
        let directory = TestDirectory::new();
        let environment = environment(&directory);
        let home = directory.path.join("home");
        fs::create_dir(home.join(".agents")).unwrap();
        assert!(environment.detect().is_empty());

        fs::create_dir(home.join(".claude")).unwrap();
        fs::create_dir(home.join(".codex")).unwrap();
        fs::create_dir(home.join(".gemini")).unwrap();
        fs::create_dir_all(home.join(".gemini/config")).unwrap();
        fs::create_dir_all(home.join(".pi/agent")).unwrap();
        fs::create_dir_all(home.join(".config/opencode")).unwrap();
        assert_eq!(
            environment.detect(),
            vec![
                &crate::drivers::claude::DRIVER,
                &crate::drivers::codex::DRIVER,
                &crate::drivers::gemini::DRIVER,
                &crate::drivers::agy::DRIVER,
                &crate::drivers::pi::DRIVER,
                &crate::drivers::opencode::DRIVER,
            ]
        );
    }

    #[test]
    fn detection_supports_executables_and_provider_directory_overrides() {
        let directory = TestDirectory::new();
        let home = directory.path.join("home");
        let cwd = directory.path.join("cwd");
        let bin = directory.path.join("bin");
        fs::create_dir(&home).unwrap();
        fs::create_dir(&cwd).unwrap();
        fs::create_dir(&bin).unwrap();
        fs::write(bin.join("opencode"), b"regular executable marker").unwrap();
        let executable = ProviderEnvironment::from_parts(&home, &cwd, vec![bin.clone()], []);
        assert_eq!(executable.detect(), vec![&crate::drivers::opencode::DRIVER]);

        let pi_root = directory.path.join("pi-override");
        let pi = ProviderEnvironment::from_parts(
            &home,
            &cwd,
            Vec::new(),
            [("PI_CODING_AGENT_DIR", pi_root.clone())],
        );
        fs::create_dir(&pi_root).unwrap();
        assert_eq!(pi.detect(), vec![&crate::drivers::pi::DRIVER]);
        assert_eq!(
            pi.target(&crate::drivers::pi::DRIVER),
            pi_root.join("skills/tmux-team")
        );

        let opencode_root = directory.path.join("opencode-override");
        let opencode = ProviderEnvironment::from_parts(
            &home,
            &cwd,
            Vec::new(),
            [("OPENCODE_CONFIG_DIR", opencode_root.clone())],
        );
        fs::create_dir(&opencode_root).unwrap();
        assert_eq!(opencode.detect(), vec![&crate::drivers::opencode::DRIVER]);

        let xdg_root = directory.path.join("xdg");
        fs::create_dir_all(xdg_root.join("opencode")).unwrap();
        let xdg = ProviderEnvironment::from_parts(
            &home,
            &cwd,
            Vec::new(),
            [("XDG_CONFIG_HOME", xdg_root)],
        );
        assert_eq!(xdg.detect(), vec![&crate::drivers::opencode::DRIVER]);
    }

    #[test]
    fn codex_home_under_shared_agents_does_not_create_codex_detection() {
        let directory = TestDirectory::new();
        let home = directory.path.join("home");
        let cwd = directory.path.join("cwd");
        fs::create_dir(&home).unwrap();
        fs::create_dir(&cwd).unwrap();
        let shared = home.join(".agents");
        fs::create_dir(&shared).unwrap();
        let environment =
            ProviderEnvironment::from_parts(&home, &cwd, Vec::new(), [("CODEX_HOME", shared)]);
        assert!(environment.detect().is_empty());
    }
}

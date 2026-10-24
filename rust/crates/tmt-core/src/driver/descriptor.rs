//! What each driver is, as declarative data: the one place a driver's name is
//! spelled. Parsing, completion and style read [`ALL`] directly; each driver's
//! behavior (where it keeps files, its runtime) lives in its adapter module,
//! `tmt-adapters/src/drivers/<name>.rs`, keyed by this descriptor. Nothing
//! else names a driver.

/// The display color a driver's addresses use, independent of any terminal
/// library. `Neutral` renders dimmed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hue {
    Magenta,
    Cyan,
    Neutral,
}

/// How a driver's session lifecycle hooks are written: `SessionStart` and
/// `SessionEnd` command entries in its JSON settings file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookFormat {
    SessionHooksJson,
}

/// Turn-end context and consumption collection. Default setup installs it
/// unless the setup record preserves a disabled installation. Keep is the
/// internal, settings-only inspection/removal policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UsageHook {
    #[default]
    Default,
    Keep,
    Install,
    Remove,
}

/// How much of a driver's session TMT can observe. `Full` drivers declare
/// lifecycle hooks and a runtime, so activity, model, consumption, resume and
/// channels exist; `Basic` drivers are detected and given skills only, so those
/// fields stay blank rather than idle. A registry test pins
/// each level to the declared capabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Support {
    Full,
    Basic,
}

impl Support {
    /// The level's stable spelling in `tmt ls --json` and human output.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Basic => "basic",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DriverDescriptor {
    /// The stable driver ID: the harness ID in storage and the provider name
    /// on the command line.
    pub name: &'static str,
    /// Executable names on `PATH` that indicate the driver is installed.
    pub executables: &'static [&'static str],
    /// Present when `tmt setup` can install lifecycle hooks for it.
    pub hooks: Option<HookFormat>,
    pub hue: Hue,
    pub support: Support,
    /// Whether text typed into the pane while a turn is running is taken into that turn, so the
    /// turn's final message answers it (#4912). Declared only for a driver where that has been
    /// measured: a driver that runs queued text as the next turn would have it marked answered by
    /// the wrong turn. Undeclared drivers keep prompt matching.
    pub folds_queued_text: bool,
}

pub static CLAUDE: DriverDescriptor = DriverDescriptor {
    name: "claude",
    executables: &["claude"],
    hooks: Some(HookFormat::SessionHooksJson),
    hue: Hue::Magenta,
    support: Support::Full,
    folds_queued_text: true,
};

pub static CODEX: DriverDescriptor = DriverDescriptor {
    name: "codex",
    executables: &["codex"],
    hooks: Some(HookFormat::SessionHooksJson),
    hue: Hue::Cyan,
    support: Support::Full,
    folds_queued_text: false,
};

pub static GEMINI: DriverDescriptor = DriverDescriptor {
    name: "gemini",
    executables: &["gemini"],
    hooks: None,
    hue: Hue::Neutral,
    support: Support::Basic,
    folds_queued_text: false,
};

pub static AGY: DriverDescriptor = DriverDescriptor {
    name: "agy",
    executables: &["agy"],
    hooks: None,
    hue: Hue::Neutral,
    support: Support::Basic,
    folds_queued_text: false,
};

pub static PI: DriverDescriptor = DriverDescriptor {
    name: "pi",
    executables: &["pi"],
    hooks: None,
    hue: Hue::Neutral,
    support: Support::Basic,
    folds_queued_text: false,
};

pub static OPENCODE: DriverDescriptor = DriverDescriptor {
    name: "opencode",
    executables: &["opencode"],
    hooks: None,
    hue: Hue::Neutral,
    support: Support::Basic,
    folds_queued_text: false,
};

pub static MUSE: DriverDescriptor = DriverDescriptor {
    name: "muse",
    executables: &["muse"],
    hooks: None,
    hue: Hue::Neutral,
    support: Support::Basic,
    folds_queued_text: false,
};

/// Every built-in driver, in detection and `install all` order.
pub static ALL: [&DriverDescriptor; 7] = [&CLAUDE, &CODEX, &GEMINI, &AGY, &PI, &OPENCODE, &MUSE];

/// The built-in driver with this name, ignoring ASCII case.
pub fn named(name: &str) -> Option<&'static DriverDescriptor> {
    ALL.into_iter()
        .find(|driver| driver.name.eq_ignore_ascii_case(name))
}

/// The built-in driver whose executable name is this foreground command,
/// ignoring any directory. An exact match, not a word search, so `pi-calc` is
/// not Pi.
pub fn for_executable(command: &str) -> Option<&'static DriverDescriptor> {
    let name = command.rsplit('/').next().unwrap_or(command);
    ALL.into_iter()
        .find(|driver| driver.executables.contains(&name))
}

impl DriverDescriptor {
    /// A driver name is a lowercase bare word, usable as a harness ID and a
    /// command-line value.
    pub fn is_valid_name(name: &str) -> bool {
        !name.is_empty()
            && name.len() <= 64
            && name
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    }
}

#[cfg(test)]
mod tests {
    use super::{ALL, DriverDescriptor, Support, for_executable, named};

    #[test]
    fn built_in_names_are_valid_unique_and_found_ignoring_case() {
        let mut names: Vec<&str> = ALL.iter().map(|driver| driver.name).collect();
        assert!(
            names
                .iter()
                .all(|name| DriverDescriptor::is_valid_name(name))
        );
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), ALL.len());
        for driver in ALL {
            assert_eq!(named(&driver.name.to_uppercase()), Some(driver));
            assert_eq!(driver.executables, [driver.name]);
        }
        assert_eq!(named("all"), None);
    }

    #[test]
    fn names_are_lowercase_bare_words() {
        for valid in ["claude", "open-code2"] {
            assert!(DriverDescriptor::is_valid_name(valid), "{valid}");
        }
        for invalid in ["", "Claude", "a b", "a/b", &"a".repeat(65)] {
            assert!(!DriverDescriptor::is_valid_name(invalid), "{invalid}");
        }
    }

    #[test]
    fn levels_are_stable_spellings_and_only_hook_drivers_are_full() {
        assert_eq!(Support::Full.as_str(), "full");
        assert_eq!(Support::Basic.as_str(), "basic");
        for driver in ALL {
            assert_eq!(
                driver.support == Support::Full,
                driver.hooks.is_some(),
                "{}",
                driver.name
            );
        }
        assert_eq!(named("claude").unwrap().support, Support::Full);
        assert_eq!(named("codex").unwrap().support, Support::Full);
        for basic in ["gemini", "agy", "pi", "opencode", "muse"] {
            assert_eq!(named(basic).unwrap().support, Support::Basic, "{basic}");
        }
    }

    #[test]
    fn only_a_driver_measured_to_fold_queued_text_declares_it() {
        for driver in ALL {
            assert_eq!(
                driver.folds_queued_text,
                driver.name == "claude",
                "{}",
                driver.name
            );
        }
    }

    #[test]
    fn a_foreground_command_matches_only_a_declared_executable_exactly() {
        for (command, expected) in [
            ("gemini", Some("gemini")),
            ("/usr/local/bin/codex", Some("codex")),
            ("claude", Some("claude")),
            ("pi", Some("pi")),
            ("pi-calc", None),
            ("python3", None),
            ("node", None),
            ("Gemini", None),
            ("zsh", None),
            ("-zsh", None),
            ("", None),
            ("gemini/", None),
        ] {
            assert_eq!(
                for_executable(command).map(|driver| driver.name),
                expected,
                "{command:?}"
            );
        }
    }
}

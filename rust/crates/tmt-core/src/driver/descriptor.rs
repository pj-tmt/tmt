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
}

pub static CLAUDE: DriverDescriptor = DriverDescriptor {
    name: "claude",
    executables: &["claude"],
    hooks: Some(HookFormat::SessionHooksJson),
    hue: Hue::Magenta,
};

pub static CODEX: DriverDescriptor = DriverDescriptor {
    name: "codex",
    executables: &["codex"],
    hooks: Some(HookFormat::SessionHooksJson),
    hue: Hue::Cyan,
};

pub static GEMINI: DriverDescriptor = DriverDescriptor {
    name: "gemini",
    executables: &["gemini"],
    hooks: None,
    hue: Hue::Neutral,
};

pub static AGY: DriverDescriptor = DriverDescriptor {
    name: "agy",
    executables: &["agy"],
    hooks: None,
    hue: Hue::Neutral,
};

pub static PI: DriverDescriptor = DriverDescriptor {
    name: "pi",
    executables: &["pi"],
    hooks: None,
    hue: Hue::Neutral,
};

pub static OPENCODE: DriverDescriptor = DriverDescriptor {
    name: "opencode",
    executables: &["opencode"],
    hooks: None,
    hue: Hue::Neutral,
};

/// Every built-in driver, in detection and `install all` order.
pub static ALL: [&DriverDescriptor; 6] = [&CLAUDE, &CODEX, &GEMINI, &AGY, &PI, &OPENCODE];

/// The built-in driver with this name, ignoring ASCII case.
pub fn named(name: &str) -> Option<&'static DriverDescriptor> {
    ALL.into_iter()
        .find(|driver| driver.name.eq_ignore_ascii_case(name))
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
    use super::{ALL, DriverDescriptor, named};

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
}

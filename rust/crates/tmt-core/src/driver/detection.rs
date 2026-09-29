//! Whether a driver is installed and runnable, as a pure decision over what
//! the adapters observed. Setup and the guided install (#333) read this; skill
//! installation keeps its filesystem-only presence check and never runs an
//! agent.

/// What running a driver's `--version` showed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionProbe {
    /// None of its executables is on `PATH`.
    NotFound,
    /// It exited within the deadline.
    Exited { success: bool, stdout: Vec<u8> },
    /// It did not exit within the deadline and was stopped.
    TimedOut,
    /// It could not be started, such as a file without execute permission.
    Unstartable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Detection {
    /// An executable was found and its version check succeeded. `version`
    /// is `None` only when the output held no version line.
    Present {
        version: Option<String>,
    },
    /// Configuration directories exist but no executable is on `PATH`. Skills
    /// may still be installed (the agent may live elsewhere); hooks may not.
    ConfigOnly,
    Absent,
    /// On `PATH` but not runnable; `reason` is short and user-facing.
    Broken {
        reason: String,
    },
}

/// The longest version string kept, in characters.
pub const VERSION_LIMIT: usize = 64;

pub fn detection_of(configured: bool, probe: &VersionProbe) -> Detection {
    match probe {
        VersionProbe::NotFound if configured => Detection::ConfigOnly,
        VersionProbe::NotFound => Detection::Absent,
        VersionProbe::Exited {
            success: true,
            stdout,
        } => Detection::Present {
            version: version(stdout),
        },
        VersionProbe::Exited { success: false, .. } => broken("its version check failed"),
        VersionProbe::TimedOut => broken("its version check did not finish"),
        VersionProbe::Unstartable => broken("it could not be started"),
    }
}

fn broken(reason: &str) -> Detection {
    Detection::Broken {
        reason: reason.to_owned(),
    }
}

/// The first non-empty line, without control characters, bounded.
fn version(stdout: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(stdout);
    let line = text.lines().map(str::trim).find(|line| !line.is_empty())?;
    let clean: String = line
        .chars()
        .filter(|ch| !ch.is_control())
        .take(VERSION_LIMIT)
        .collect();
    (!clean.is_empty()).then_some(clean)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exited(success: bool, stdout: &str) -> VersionProbe {
        VersionProbe::Exited {
            success,
            stdout: stdout.as_bytes().to_vec(),
        }
    }

    #[test]
    fn a_runnable_executable_is_present_with_its_first_version_line() {
        assert_eq!(
            detection_of(false, &exited(true, "\n  2.1.3 (Claude Code)\nextra\n")),
            Detection::Present {
                version: Some("2.1.3 (Claude Code)".into())
            }
        );
        assert_eq!(
            detection_of(true, &exited(true, "")),
            Detection::Present { version: None }
        );
        let long = format!("v{}\u{1b}[31m", "9".repeat(100));
        let Detection::Present {
            version: Some(version),
        } = detection_of(false, &exited(true, &long))
        else {
            panic!("present");
        };
        assert_eq!(version.chars().count(), VERSION_LIMIT);
        assert!(!version.contains('\u{1b}'));
    }

    #[test]
    fn configuration_alone_is_config_only_and_nothing_is_absent() {
        assert_eq!(
            detection_of(true, &VersionProbe::NotFound),
            Detection::ConfigOnly
        );
        assert_eq!(
            detection_of(false, &VersionProbe::NotFound),
            Detection::Absent
        );
    }

    #[test]
    fn an_executable_that_does_not_run_is_broken_even_when_configured() {
        for probe in [
            exited(false, "2.0"),
            VersionProbe::TimedOut,
            VersionProbe::Unstartable,
        ] {
            for configured in [false, true] {
                assert!(
                    matches!(detection_of(configured, &probe), Detection::Broken { .. }),
                    "{probe:?} {configured}"
                );
            }
        }
    }
}

//! Whether a driver is installed, as a pure decision over what the adapters
//! observed. Detection reads only the filesystem and never starts an agent;
//! [`with_version`] refines it from an explicit `--version` probe, which is
//! for diagnostics only because running an agent can write under `HOME`.

/// What `PATH` holds for a driver's executables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnPath {
    Missing,
    /// A regular file without execute permission.
    NotExecutable,
    Executable,
}

/// What running a driver's `--version` showed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionProbe {
    /// It exited within the deadline.
    Exited { success: bool, stdout: Vec<u8> },
    /// It did not exit within the deadline and was stopped.
    TimedOut,
    /// It could not be started.
    Unstartable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Detection {
    /// An executable is on `PATH`. `version` is known only after an explicit
    /// version probe, and then only when its output had a version line.
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

/// The filesystem-only decision: configuration directories and `PATH`.
pub fn detection_of(configured: bool, on_path: OnPath) -> Detection {
    match on_path {
        OnPath::Executable => Detection::Present { version: None },
        OnPath::NotExecutable => broken("it is not executable"),
        OnPath::Missing if configured => Detection::ConfigOnly,
        OnPath::Missing => Detection::Absent,
    }
}

/// A present driver refined by its version probe.
pub fn with_version(probe: &VersionProbe) -> Detection {
    match probe {
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
    fn detection_reads_only_the_filesystem() {
        for configured in [false, true] {
            assert_eq!(
                detection_of(configured, OnPath::Executable),
                Detection::Present { version: None }
            );
            assert!(matches!(
                detection_of(configured, OnPath::NotExecutable),
                Detection::Broken { .. }
            ));
        }
        assert_eq!(detection_of(true, OnPath::Missing), Detection::ConfigOnly);
        assert_eq!(detection_of(false, OnPath::Missing), Detection::Absent);
    }

    #[test]
    fn a_successful_probe_keeps_the_first_version_line_bounded() {
        assert_eq!(
            with_version(&exited(true, "\n  2.1.3 (Claude Code)\nextra\n")),
            Detection::Present {
                version: Some("2.1.3 (Claude Code)".into())
            }
        );
        assert_eq!(
            with_version(&exited(true, "")),
            Detection::Present { version: None }
        );
        let long = format!("v{}\u{1b}[31m", "9".repeat(100));
        let Detection::Present {
            version: Some(version),
        } = with_version(&exited(true, &long))
        else {
            panic!("present");
        };
        assert_eq!(version.chars().count(), VERSION_LIMIT);
        assert!(!version.contains('\u{1b}'));
    }

    #[test]
    fn a_probe_that_does_not_run_is_broken() {
        for probe in [
            exited(false, "2.0"),
            VersionProbe::TimedOut,
            VersionProbe::Unstartable,
        ] {
            assert!(
                matches!(with_version(&probe), Detection::Broken { .. }),
                "{probe:?}"
            );
        }
    }
}

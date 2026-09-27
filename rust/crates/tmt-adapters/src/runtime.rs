//! Registered runtime recognition and exact resume commands.
//! Explicit launches bypass argument planning entirely: they preserve argv.

use std::{
    ffi::{OsStr, OsString},
    fmt,
    path::Path,
};
use tmt_core::{
    binding::session::{HarnessId, RememberedSession},
    driver::{ActionResult, Driver, HarnessResume, HarnessStart},
};

/// Tokens shared with first-party hook mappings; mode belongs to the runtime,
/// not the host interface. Codex embedded mode is selected by `--no-daemon`.
pub const CLAUDE_MODE_DEFAULT: &str = "default";
pub const CODEX_MODE_SHARED: &str = "shared";
pub const CODEX_MODE_EMBEDDED: &str = "embedded";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeCommand {
    pub executable: OsString,
    pub args: Vec<OsString>,
}

impl RuntimeCommand {
    pub fn verbatim(command: &[OsString]) -> Option<Self> {
        let (executable, args) = command.split_first()?;
        Some(Self {
            executable: executable.clone(),
            args: args.to_vec(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeError {
    InvalidSession,
    InvalidRegistration,
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidSession => "The remembered runtime session is not an exact UUID.",
            Self::InvalidRegistration => {
                "Runtime registration requires a unique harness ID and a bare command name."
            }
        })
    }
}
impl std::error::Error for RuntimeError {}

type RegisteredDriver = dyn Driver<Target = (), Error = RuntimeError, Launch = RuntimeCommand>;

struct Registration {
    harness: HarnessId,
    executable: OsString,
    priority: i32,
    driver: Box<RegisteredDriver>,
}

/// First-party and community drivers use the same registration boundary.
/// Higher priority wins; equal priority is ordered by stable harness ID.
/// This registry does not discover, install or execute extension code.
/// Future external registration must require explicit trust before overriding
/// a first-party claim; priority alone is not permission to load driver code.
#[derive(Default)]
pub struct RuntimeRegistry {
    registrations: Vec<Registration>,
}

impl RuntimeRegistry {
    pub fn first_party() -> Self {
        let mut registry = Self::default();
        for (name, driver) in [("claude", FirstParty::Claude), ("codex", FirstParty::Codex)] {
            registry
                .register(
                    HarnessId::new(name).expect("built-in harness ID"),
                    name,
                    0,
                    driver,
                )
                .expect("unique built-in runtime registration");
        }
        registry
    }

    pub fn register(
        &mut self,
        harness: HarnessId,
        executable: &str,
        priority: i32,
        driver: impl Driver<Target = (), Error = RuntimeError, Launch = RuntimeCommand> + 'static,
    ) -> Result<(), RuntimeError> {
        if executable.is_empty()
            || executable.contains('/')
            || executable.chars().any(char::is_control)
            || self
                .registrations
                .iter()
                .any(|entry| entry.harness == harness)
        {
            return Err(RuntimeError::InvalidRegistration);
        }
        self.registrations.push(Registration {
            harness,
            executable: executable.into(),
            priority,
            driver: Box::new(driver),
        });
        self.registrations.sort_by(|a, b| {
            b.priority
                .cmp(&a.priority)
                .then_with(|| a.harness.as_str().cmp(b.harness.as_str()))
        });
        Ok(())
    }

    pub fn claim(&self, executable: &OsStr) -> Option<HarnessId> {
        let executable = executable.to_str()?;
        self.registrations.iter().find_map(|entry| {
            entry
                .driver
                .claims(executable)
                .filter(|claim| claim == &entry.harness)
        })
    }

    /// Relaunch resolves the registered bare executable through PATH, without
    /// replaying any previous arguments or executable path.
    pub fn relaunch(&self, harness: &HarnessId) -> Option<RuntimeCommand> {
        self.registrations
            .iter()
            .find(|entry| &entry.harness == harness)
            .map(|entry| RuntimeCommand {
                executable: entry.executable.clone(),
                args: Vec::new(),
            })
    }

    pub fn resume(
        &mut self,
        session: &RememberedSession,
    ) -> ActionResult<RuntimeCommand, RuntimeError> {
        let Some(entry) = self
            .registrations
            .iter_mut()
            .find(|entry| entry.harness == session.harness)
        else {
            return ActionResult::Unsupported;
        };
        entry.driver.resume(HarnessResume {
            start: HarnessStart {
                harness: &session.harness,
                context: None,
            },
            session: &session.provider_session,
            mode: &session.mode,
        })
    }
}

#[derive(Debug, Clone, Copy)]
enum FirstParty {
    Claude,
    Codex,
}

impl FirstParty {
    fn name(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }
}

impl Driver for FirstParty {
    type Target = ();
    type Error = RuntimeError;
    type Launch = RuntimeCommand;

    fn claims(&self, command: &str) -> Option<HarnessId> {
        (Path::new(command).file_name() == Some(OsStr::new(self.name())))
            .then(|| HarnessId::new(self.name()).expect("built-in harness ID"))
    }

    fn resume(&mut self, resume: HarnessResume<'_>) -> ActionResult<Self::Launch, Self::Error> {
        if resume.start.harness.as_str() != self.name() || resume.start.context.is_some() {
            return ActionResult::Unsupported;
        }
        let mode = resume.mode.as_str();
        if !matches!(
            (*self, mode),
            (Self::Claude, CLAUDE_MODE_DEFAULT)
                | (Self::Codex, CODEX_MODE_SHARED | CODEX_MODE_EMBEDDED)
        ) {
            return ActionResult::Unsupported;
        }
        // The first-party versions verified in #321 expose UUID session IDs.
        // Community drivers retain their own opaque-ID contract.
        if uuid::Uuid::parse_str(resume.session.as_str()).is_err() {
            return ActionResult::Failed(RuntimeError::InvalidSession);
        }
        let mut args = match self {
            Self::Claude => vec![
                OsString::from("--resume"),
                OsString::from(resume.session.as_str()),
            ],
            Self::Codex => vec![
                OsString::from("resume"),
                OsString::from(resume.session.as_str()),
            ],
        };
        if mode == CODEX_MODE_EMBEDDED {
            args.push("--no-daemon".into());
        }
        ActionResult::Completed(RuntimeCommand {
            executable: self.name().into(),
            args,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStringExt;
    use tmt_core::binding::session::{ProviderSessionId, RuntimeMode};

    fn id(name: &str) -> HarnessId {
        HarnessId::new(name).unwrap()
    }

    #[test]
    fn launch_preserves_every_argument_byte_and_claim_never_changes_it() {
        let command = vec![
            OsString::from("/custom/bin/claude"),
            "--session-id".into(),
            "user-session".into(),
            "--secret=do-not-store".into(),
            "".into(),
            "two words\nnext line".into(),
            OsString::from_vec(vec![0xff, b'a']),
        ];
        let before = command.clone();
        let registry = RuntimeRegistry::first_party();
        assert_eq!(registry.claim(&command[0]), Some(id("claude")));
        let plan = RuntimeCommand::verbatim(&command).unwrap();
        assert_eq!(plan.executable, before[0]);
        assert_eq!(plan.args, before[1..]);
        assert_eq!(command, before);
        assert_eq!(
            registry.relaunch(&id("claude")).unwrap(),
            RuntimeCommand {
                executable: "claude".into(),
                args: vec![]
            }
        );
        assert_eq!(registry.claim(OsStr::new("/bin/sh")), None);
    }

    struct Community {
        id: HarnessId,
    }
    impl Driver for Community {
        type Target = ();
        type Error = RuntimeError;
        type Launch = RuntimeCommand;
        fn claims(&self, command: &str) -> Option<HarnessId> {
            (command == "shared-command").then(|| self.id.clone())
        }
    }

    #[test]
    fn community_registration_uses_deterministic_priority_and_stable_ties() {
        for names in [["beta", "alpha"], ["alpha", "beta"]] {
            let mut registry = RuntimeRegistry::first_party();
            for name in names {
                registry
                    .register(id(name), "shared-command", 5, Community { id: id(name) })
                    .unwrap();
            }
            assert_eq!(
                registry.claim(OsStr::new("shared-command")),
                Some(id("alpha"))
            );
            registry
                .register(
                    id("preferred"),
                    "shared-command",
                    10,
                    Community {
                        id: id("preferred"),
                    },
                )
                .unwrap();
            assert_eq!(
                registry.claim(OsStr::new("shared-command")),
                Some(id("preferred"))
            );
            assert!(
                registry
                    .register(
                        id("alpha"),
                        "shared-command",
                        20,
                        Community { id: id("alpha") }
                    )
                    .is_err()
            );
        }
    }

    #[test]
    fn exact_resume_is_runtime_owned_and_preserves_mode_without_last_selection() {
        let mut registry = RuntimeRegistry::first_party();
        for (harness, mode, expected) in [
            (
                "claude",
                CLAUDE_MODE_DEFAULT,
                vec!["--resume", "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"],
            ),
            (
                "codex",
                CODEX_MODE_SHARED,
                vec!["resume", "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"],
            ),
            (
                "codex",
                CODEX_MODE_EMBEDDED,
                vec![
                    "resume",
                    "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
                    "--no-daemon",
                ],
            ),
        ] {
            let session = RememberedSession {
                harness: id(harness),
                mode: RuntimeMode::new(mode).unwrap(),
                provider_session: ProviderSessionId::new("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa")
                    .unwrap(),
            };
            assert_eq!(
                registry.resume(&session),
                ActionResult::Completed(RuntimeCommand {
                    executable: harness.into(),
                    args: expected.into_iter().map(OsString::from).collect()
                })
            );
        }
        let mut session = RememberedSession {
            harness: id("codex"),
            mode: RuntimeMode::new("remote").unwrap(),
            provider_session: ProviderSessionId::new("--last").unwrap(),
        };
        assert_eq!(registry.resume(&session), ActionResult::Unsupported);
        session.mode = RuntimeMode::new("shared").unwrap();
        assert_eq!(
            registry.resume(&session),
            ActionResult::Failed(RuntimeError::InvalidSession)
        );
    }
}

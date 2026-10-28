//! Narrow, read-only launch evidence, not a copy of Codex's config resolver.
//! Unknown policy retains the existing writable-root grant.

use crate::{bounded_file, skill_installation::ProviderEnvironment};
use std::{ffi::OsString, io::ErrorKind, path::Path};
use toml_edit::DocumentMut;

#[derive(Clone, Copy)]
enum Mode {
    ReadOnly,
    WorkspaceWrite,
    FullAccess,
    Unknown,
}

impl Mode {
    fn parse(value: &str) -> Self {
        match value {
            "read-only" => Self::ReadOnly,
            "workspace-write" => Self::WorkspaceWrite,
            "danger-full-access" => Self::FullAccess,
            _ => Self::Unknown,
        }
    }

    fn needs_grant(self) -> bool {
        matches!(self, Self::WorkspaceWrite | Self::Unknown)
    }
}

pub(super) fn takes_value(flag: &str) -> bool {
    matches!(
        flag,
        "-c" | "--config"
            | "--enable"
            | "--disable"
            | "-m"
            | "--model"
            | "-s"
            | "--sandbox"
            | "-a"
            | "--ask-for-approval"
            | "-C"
            | "--cd"
            | "-p"
            | "--profile"
            | "--local-provider"
            | "--remote-auth-token-env"
            | "-i"
            | "--image"
            | "--add-dir"
    )
}

pub(super) fn split_option(word: &str) -> (&str, Option<&str>) {
    if word.len() > 2
        && matches!(
            &word.as_bytes()[..2],
            b"-s" | b"-p" | b"-c" | b"-m" | b"-a" | b"-C" | b"-i"
        )
    {
        return (&word[..2], Some(&word[2..]));
    }
    word.split_once('=')
        .map_or((word, None), |(key, value)| (key, Some(value)))
}

pub(super) fn keep_state_root(
    args: &[OsString],
    environment: &ProviderEnvironment,
    launch_environment: &[(OsString, OsString)],
) -> bool {
    // An exact resume may restore a provider home absent from this caller.
    // Read the home the child will actually use, without changing process env.
    let home = launch_environment
        .iter()
        .rev()
        .find(|(key, _)| key == "CODEX_HOME")
        .map(|(_, value)| {
            if value.is_empty() {
                environment.home().join(".codex")
            } else {
                environment.resolve(Path::new(value))
            }
        })
        .unwrap_or_else(|| super::codex_home(environment));
    decide(args, |profile| inspect(&home, profile))
}

fn decide(args: &[OsString], config: impl FnOnce(Option<&str>) -> Option<Mode>) -> bool {
    let mut words = args.iter();
    let mut explicit = None;
    let mut profile = None;
    let mut ambiguous = false;
    while let Some(word) = words.next() {
        let Some(word) = word.to_str() else {
            return true;
        };
        if word == "--" {
            break;
        }
        if !word.starts_with('-') {
            if matches!(word, "resume" | "fork") {
                continue;
            }
            break;
        }
        let (flag, inline) = split_option(word);
        let value = if takes_value(flag) {
            inline.or_else(|| words.next().and_then(|v| v.to_str()))
        } else {
            inline
        };
        match flag {
            "-s" | "--sandbox" => explicit = Some(value.map_or(Mode::Unknown, Mode::parse)),
            "-p" | "--profile" => {
                let Some(value) = value else { return true };
                profile = Some(value);
            }
            "-c" | "--config" => {
                let Some((key, _)) = value.and_then(|v| v.split_once('=')) else {
                    return true;
                };
                let key = key.trim().trim_matches(['\'', '"']);
                ambiguous |= key.starts_with("sandbox")
                    || key.starts_with("permissions")
                    || key == "profile";
            }
            "--dangerously-bypass-approvals-and-sandbox" => explicit = Some(Mode::FullAccess),
            "--approve-for-me" => explicit = Some(Mode::WorkspaceWrite),
            _ if takes_value(flag) => {
                if value.is_none() {
                    return true;
                }
            }
            "--no-daemon" | "--search" | "--no-alt-screen" | "--oss" | "--worktree"
            | "--strict-config" => {}
            _ => ambiguous = true,
        }
    }
    if ambiguous {
        return true;
    }
    explicit
        .or_else(|| config(profile))
        .unwrap_or(Mode::Unknown)
        .needs_grant()
}

fn read(path: &Path) -> Option<DocumentMut> {
    let bytes = match bounded_file::read_no_follow(path, 1024 * 1024) {
        Ok(bytes) => bytes,
        Err(bounded_file::FileReadError::Io(error))
            if error.kind() == ErrorKind::NotFound && std::fs::symlink_metadata(path).is_err() =>
        {
            Vec::new()
        }
        _ => return None,
    };
    std::str::from_utf8(&bytes).ok()?.parse().ok()
}

fn mode(document: &DocumentMut) -> Option<Mode> {
    document
        .get("sandbox_mode")
        .map(|value| value.as_str().map_or(Mode::Unknown, Mode::parse))
}

fn inspect(home: &Path, selected: Option<&str>) -> Option<Mode> {
    let document = read(&home.join("config.toml"))?;
    // New permission profiles or managed requirements are not legacy sandbox_mode.
    if document.get("permissions").is_some() || document.get("requirements").is_some() {
        return None;
    }
    let selected = match selected {
        Some(selected) => Some(selected),
        None => match document.get("profile") {
            Some(value) => Some(value.as_str()?),
            None => None,
        },
    };
    if let Some(selected) = selected {
        // Recognize the old embedded profile and the provider's current sibling
        // profile file. Any unsupported/uncertain profile keeps the grant.
        if selected.is_empty() || selected.contains(['/', '\\']) || matches!(selected, "." | "..") {
            return None;
        }
        let profile_path = home.join(format!("{selected}.config.toml"));
        if std::fs::symlink_metadata(&profile_path).is_ok() {
            let profile = read(&profile_path)?;
            if profile.get("permissions").is_some() {
                return None;
            }
            return mode(&profile).or_else(|| mode(&document));
        }
        let profiles = document.get("profiles")?.as_table_like()?;
        let profile = profiles.get(selected)?.as_table_like()?;
        return match profile.get("sandbox_mode") {
            Some(value) => Some(value.as_str().map_or(Mode::Unknown, Mode::parse)),
            None => mode(&document),
        };
    }
    // A fresh/readable config without any selected sandbox policy is the
    // confirmed default that rejects --add-dir. No provider settings are written.
    Some(mode(&document).unwrap_or(Mode::ReadOnly))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decision(args: &[&str], configured: Option<Mode>) -> bool {
        decide(&args.iter().map(OsString::from).collect::<Vec<_>>(), |_| {
            configured
        })
    }

    #[test]
    fn inherited_workspace_write_and_ambiguous_policy_keep_the_grant() {
        assert!(decision(&[], Some(Mode::WorkspaceWrite)));
        assert!(decision(&[], None));
        assert!(!decision(&[], Some(Mode::ReadOnly)));
        assert!(!decision(&[], Some(Mode::FullAccess)));
        for args in [
            vec!["-p", "custom"],
            vec!["-c", "sandbox_mode=\"read-only\""],
            vec!["-s", "read-only", "-c", "permissions=custom"],
        ] {
            assert!(decision(&args, None));
        }
    }

    #[test]
    fn explicit_modes_override_local_config_without_rewriting_user_arguments() {
        for (args, expected) in [
            (vec!["-s", "read-only"], false),
            (vec!["--sandbox=workspace-write"], true),
            (vec!["-sread-only"], false),
            (vec!["--sandbox=danger-full-access"], false),
            (vec!["-s", "read-only", "-s", "workspace-write"], true),
            (vec!["--approve-for-me"], true),
            (vec!["--dangerously-bypass-approvals-and-sandbox"], false),
        ] {
            assert_eq!(decision(&args, Some(Mode::WorkspaceWrite)), expected);
        }
    }

    #[test]
    fn prompt_and_option_values_never_select_permissions() {
        for args in [
            vec!["-m", "--sandbox=read-only"],
            vec!["--", "-s", "read-only"],
            vec!["a prompt", "-s", "read-only"],
            vec!["--add-dir", "--sandbox=read-only"],
        ] {
            assert!(decision(&args, Some(Mode::WorkspaceWrite)));
        }
        assert!(!decision(
            &["resume", "-s", "read-only", "session"],
            Some(Mode::WorkspaceWrite)
        ));
    }
    #[test]
    fn private_config_modes_profiles_and_failed_reads_never_change_settings() {
        use crate::test_support::TestDirectory;
        use std::fs;
        let fixture = TestDirectory::new();
        let home = fixture.path.join("selected");
        fs::create_dir(&home).unwrap();
        let env = ProviderEnvironment::from_parts(
            &fixture.path,
            &fixture.path,
            Vec::new(),
            [("CODEX_HOME", Path::new("selected").to_owned())],
        );
        assert!(!keep_state_root(&[], &env, &[]));
        assert!(!home.join("config.toml").exists());
        let path = home.join("config.toml");
        for (text, expected) in [
            (
                "sandbox_mode = \"workspace-write\"\napproval_policy = \"on-request\"\n",
                true,
            ),
            ("sandbox_mode = \"read-only\"\n", false),
            ("sandbox_mode = \"future-mode\"\n", true),
            ("sandbox_mode = 42\n", true),
            ("not valid TOML {", true),
            ("[permissions]\ndefault = \"custom\"\n", true),
        ] {
            fs::write(&path, text).unwrap();
            let before = fs::read(&path).unwrap();
            assert_eq!(keep_state_root(&[], &env, &[]), expected);
            assert_eq!(fs::read(&path).unwrap(), before);
        }
        fs::write(
            &path,
            "sandbox_mode=\"workspace-write\"\n[profiles.reader]\nsandbox_mode=\"read-only\"\n",
        )
        .unwrap();
        let args = ["-p", "reader"].map(OsString::from);
        assert!(!keep_state_root(&args, &env, &[]));
        assert!(keep_state_root(
            &["-p", "missing"].map(OsString::from),
            &env,
            &[]
        ));
        fs::write(
            home.join("reader.config.toml"),
            "sandbox_mode=\"workspace-write\"\n",
        )
        .unwrap();
        assert!(keep_state_root(&args, &env, &[]));
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(keep_state_root(&[], &env, &[]));
    }
    #[test]
    fn a_resume_reads_its_replayed_provider_home_not_the_callers_default() {
        use crate::test_support::TestDirectory;
        let fixture = TestDirectory::new();
        let selected = fixture.path.join("replayed");
        std::fs::create_dir(&selected).unwrap();
        std::fs::write(
            selected.join("config.toml"),
            "sandbox_mode=\"workspace-write\"\n",
        )
        .unwrap();
        let env = ProviderEnvironment::from_parts(&fixture.path, &fixture.path, Vec::new(), []);
        assert!(!keep_state_root(&[], &env, &[]));
        assert!(keep_state_root(
            &[],
            &env,
            &[("CODEX_HOME".into(), selected.into_os_string())]
        ));
    }
}

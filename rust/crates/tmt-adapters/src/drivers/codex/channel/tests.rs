use super::*;

struct VersionOnly<'a>(&'a str);
impl CommandRunner for VersionOnly<'_> {
    fn execute(
        &self,
        request: CommandRequest<'_>,
    ) -> Result<crate::process::CommandOutput, crate::process::CommandError> {
        assert_eq!(request.program, "fixture-codex");
        assert_eq!(request.args, [std::ffi::OsString::from("--version")]);
        assert!(request.input.is_empty());
        Ok(crate::process::CommandOutput {
            stdout: self.0.as_bytes().to_vec(),
            stderr: Vec::new(),
        })
    }
}

#[test]
fn trust_note_is_local_read_only_and_uses_codex_cd_semantics() {
    use crate::{
        runtime::RuntimeCommand, skill_installation::ProviderEnvironment,
        test_support::TestDirectory,
    };
    use std::fs;

    let fixture = TestDirectory::new();
    let selected = fixture.path.join("selected");
    let home = fixture.path.join("alternate");
    fs::create_dir(&selected).unwrap();
    fs::create_dir(&home).unwrap();
    let environment = ProviderEnvironment::from_parts(
        &fixture.path,
        &fixture.path,
        Vec::new(),
        [("CODEX_HOME", home.clone())],
    );
    // Managed/profile sources are deliberately outside this local-only advisory.
    let mut config = toml_edit::DocumentMut::new();
    config["profiles"]["managed"]["model"] = toml_edit::value("unused");
    config["projects"][fixture.path.to_str().unwrap()]["trust_level"] = toml_edit::value("trusted");
    let path = home.join("config.toml");
    fs::write(&path, config.to_string()).unwrap();
    fs::write(home.join("auth.json"), "never read or change credentials").unwrap();
    let command = RuntimeCommand {
        executable: "fixture-codex".into(),
        args: vec!["-C".into(), "selected".into()],
    };
    let check = |version| {
        check_provider(
            &VersionOnly(version),
            &command,
            Some(&fixture.path),
            &fixture.path.join("channel"),
            Instant::now() + std::time::Duration::from_secs(5),
            Some(&environment),
        )
    };
    let before = fs::read(&path).unwrap();
    assert_eq!(
        check_provider(
            &VersionOnly("codex-cli 0.160.0"),
            &command,
            None,
            &fixture.path.join("channel"),
            Instant::now() + std::time::Duration::from_secs(5),
            Some(&environment)
        ),
        Ok(None)
    );
    assert_eq!(fs::read(&path).unwrap(), before);
    for version in [
        "codex-cli 0.159.2",
        "codex-cli 0.159.3",
        "codex-cli 0.160.0",
    ] {
        assert_eq!(
            check(version).unwrap().as_deref(),
            Some("Codex may ask you to trust this folder in its own window.")
        );
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(!fixture.path.join("channel").exists());
    }
    config["projects"][selected.to_str().unwrap()]["trust_level"] = toml_edit::value("trusted");
    fs::write(&path, config.to_string()).unwrap();
    assert_eq!(check("codex-cli 0.160.0"), Ok(None));
    fs::write(&path, "malformed = [").unwrap();
    assert_eq!(check("codex-cli 0.160.0"), Ok(None));
    assert_eq!(fs::read_to_string(&path).unwrap(), "malformed = [");
    assert_eq!(
        fs::read_to_string(home.join("auth.json")).unwrap(),
        "never read or change credentials"
    );
}

#[test]
fn codex_channel_requires_explicit_opt_in() {
    assert!(!CodexChannel.enabled_by_default());
}

#[test]
fn preflight_accepts_only_qualified_builds_and_classifies_later_0159_as_unavailable() {
    for version in [
        b"codex-cli 0.159.2".as_slice(),
        b"codex-cli 0.159.3\n",
        b"codex-cli 0.160.0\n",
    ] {
        assert!(matches!(version_advisory(version), Ok(None)));
    }
    for version in ["codex-cli 0.159.4", "codex-cli 0.159.999"] {
        let Err(ChannelError::ProviderUnqualified { reason }) =
            version_advisory(version.as_bytes())
        else {
            panic!("unqualified build must be unavailable");
        };
        assert!(reason.contains(version));
        assert!(reason.contains("has not been qualified"));
    }
    for version in [
        b"".as_slice(),
        b"0.159.3",
        b"codex-cli unknown",
        b"codex-cli 0.159",
        b"codex-cli 0.159.3.1",
        b"codex-cli 0.159.+3",
        b"codex-cli 0.159.3-beta",
        b"codex-cli 0.159.1",
        b"codex-cli 0.158.99",
        b"codex-cli 0.160.1",
        b"codex-cli 0.160.999",
        b"codex-cli 0.161.0",
        b"codex-cli 0.160.0-beta",
        b"codex-cli 0.160.+0",
        b"codex-cli 1.159.3",
        b"codex-cli 0.159.18446744073709551616",
        b"codex-cli 0.159.\xff",
    ] {
        assert!(
            matches!(
                version_advisory(version),
                Err(ChannelError::ProviderVersion { .. })
            ),
            "unexpected acceptance: {version:?}"
        );
    }
}

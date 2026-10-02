use super::*;

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

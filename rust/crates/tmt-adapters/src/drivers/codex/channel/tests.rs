use super::*;

#[test]
fn preflight_requires_the_supported_line_and_minimum_before_advising() {
    for version in [b"codex-cli 0.159.2".as_slice(), b"codex-cli 0.159.3\n"] {
        assert!(matches!(version_advisory(version), Ok(None)));
    }
    for version in ["codex-cli 0.159.4", "codex-cli 0.159.999"] {
        let note = version_advisory(version.as_bytes()).unwrap().unwrap();
        assert!(note.contains(version));
        assert!(note.contains("owned endpoint handshake must pass"));
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
        b"codex-cli 0.160.0",
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

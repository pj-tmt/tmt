use super::*;

#[test]
fn exact_extension_install_accepts_release_proof_arguments() {
    let result = parse(&args(&[
        "extension",
        "install",
        "digest",
        "--yes",
        "--json",
        "--channel",
        "alpha",
        "--to",
        "0.1.0-alpha.1",
        "--prefix",
        "/private-prefix",
    ]));
    let result = result.unwrap();
    assert!(result.mode.json);
    assert!(matches!(result.invocation,
        Invocation::ExtensionInstall(crate::invocation::ExtensionInstallRequest::Install {
            to: Some(version), channel: Some(tmt_core::native_install::Channel::Alpha),
            yes: true, repair: false, archive: None, manifest: None, ..
        }) if version == "0.1.0-alpha.1"));
    for tail in [
        vec!["--repair"],
        vec!["--archive", "archive", "--manifest", "manifest"],
        vec!["--unpin"],
    ] {
        let mut argv = vec!["extension", "install", "digest", "--to", "0.1.0-alpha.1"];
        argv.extend(tail);
        assert!(parse(&args(&argv)).is_err(), "{argv:?}");
    }
}

#[test]
fn extension_repair_accepts_paired_local_inputs_but_cannot_change_channel() {
    let result = parsed(&[
        "extension",
        "install",
        "squad",
        "--repair",
        "--yes",
        "--prefix",
        "/prefix",
    ]);
    assert!(matches!(
        result.invocation,
        Invocation::ExtensionInstall(crate::invocation::ExtensionInstallRequest::Install {
            repair: true,
            yes: true,
            channel: None,
            archive: None,
            manifest: None,
            ..
        })
    ));
    let local = parsed(&[
        "extension",
        "install",
        "squad",
        "--repair",
        "--archive",
        "archive",
        "--manifest",
        "manifest",
    ]);
    assert!(matches!(
        local.invocation,
        Invocation::ExtensionInstall(crate::invocation::ExtensionInstallRequest::Install {
            repair: true,
            archive: Some(_),
            manifest: Some(_),
            ..
        })
    ));
    for extra in [
        vec!["--channel", "stable"],
        vec!["--archive", "archive"],
        vec!["--manifest", "manifest"],
    ] {
        let mut args = vec!["extension", "install", "squad", "--repair"];
        args.extend(extra);
        assert_eq!(parse_error(&args).code, "USAGE_ERROR");
    }
    assert_eq!(
        parse_error(&["extension", "upgrade", "squad", "--repair"]).code,
        "USAGE_ERROR"
    );
}

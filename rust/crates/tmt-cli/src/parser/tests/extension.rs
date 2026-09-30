use super::*;

#[test]
fn extension_preflight_requires_both_named_files_and_rejects_execution_flags() {
    use crate::invocation::OfficeOperation;
    assert_eq!(
        parsed(&[
            "office",
            "extension",
            "validate",
            "--file",
            "definition.json",
            "--instance",
            "instance.json"
        ])
        .invocation,
        Invocation::Office {
            prefix: None,
            operation: OfficeOperation::ExtensionValidate {
                file: "definition.json".into(),
                instance: "instance.json".into()
            }
        }
    );
    for invalid in [
        vec![
            "office",
            "extension",
            "validate",
            "--file",
            "definition.json",
        ],
        vec![
            "office",
            "extension",
            "validate",
            "--instance",
            "instance.json",
        ],
        vec![
            "office",
            "extension",
            "validate",
            "--file",
            "definition.json",
            "--instance",
            "instance.json",
            "--execute",
        ],
        vec![
            "office",
            "extension",
            "install",
            "--file",
            "definition.json",
        ],
    ] {
        assert!(parse(&args(&invalid)).is_err(), "{invalid:?}");
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

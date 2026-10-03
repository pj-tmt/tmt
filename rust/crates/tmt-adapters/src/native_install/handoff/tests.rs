use super::*;
use crate::native_install::{Product, inspect, test_support::published_layout};
use crate::process::{CommandError, CommandOutput};
use std::cell::RefCell;

fn downloaded(extra: bool) -> DownloadedRelease {
    let (_, mut manifest, mut archive, archive_name) =
        super::super::release::valid_fixture("1.2.4", "aarch64-apple-darwin", 42);
    if extra {
        let artifact =
            artifact::acquire_candidate(&manifest, &archive_name, &archive, "aarch64-apple-darwin")
                .unwrap();
        let mut files = artifact.files;
        files.insert(
            "future-companion".into(),
            b"future companion bytes".to_vec(),
        );
        let root = archive_name.strip_suffix(".tar.gz").unwrap();
        let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        let mut builder = tar::Builder::new(encoder);
        for (name, bytes) in files {
            let mut header = tar::Header::new_gnu();
            header.set_path(format!("{root}/{name}")).unwrap();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder.append(&header, bytes.as_slice()).unwrap();
        }
        archive = builder.into_inner().unwrap().finish().unwrap();
        let mut value: serde_json::Value = serde_json::from_slice(&manifest).unwrap();
        value["artifacts"][&archive_name]["checksums"]["sha256"] =
            artifact::digest(&archive).into();
        value["artifacts"][&archive_name]["assets"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"path":"future-companion"}));
        manifest = serde_json::to_vec(&value).unwrap();
    }
    DownloadedRelease {
        version: "1.2.4".parse().unwrap(),
        provenance: GitHubProvenance {
            release_id: 42,
            manifest_sha256: artifact::digest(&manifest),
        },
        manifest,
        archive,
        archive_name,
    }
}

struct Runner<F> {
    run: F,
    stages: RefCell<Vec<PathBuf>>,
}
impl<F: Fn(CommandRequest<'_>) -> Result<CommandOutput, CommandError>> CommandRunner for Runner<F> {
    fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
        let candidate = PathBuf::from(request.program);
        self.stages
            .borrow_mut()
            .push(candidate.parent().unwrap().parent().unwrap().to_owned());
        assert!(candidate.is_file());
        (self.run)(request)
    }
}
fn output(value: serde_json::Value) -> CommandOutput {
    CommandOutput {
        stdout: serde_json::to_vec(&value).unwrap(),
        stderr: Vec::new(),
    }
}
fn response(report: Option<InstallReport>, error: Option<&str>) -> CommandOutput {
    output(
        serde_json::to_value(Response {
            protocol: VERSION,
            installation: report,
            error: error.map(str::to_owned),
        })
        .unwrap(),
    )
}
fn assert_clean<F>(runner: &Runner<F>) {
    for stage in runner.stages.borrow().iter() {
        assert!(!stage.exists(), "stage survives: {}", stage.display());
    }
}

#[test]
fn new_companion_is_judged_by_candidate_and_old_reader_never_reopens_new_inventory() {
    let (_directory, layout, old) = published_layout();
    let current = inspect(&layout.prefix.join("bin/tmt")).unwrap();
    let downloaded = downloaded(true);
    // Regression sensitivity: the pre-delegation installer policy rejects it.
    assert!(
        artifact::acquire_bytes(
            Product::Cli,
            &downloaded.manifest,
            &downloaded.archive_name,
            &downloaded.archive,
            &current.target
        )
        .unwrap_err()
        .to_string()
        .contains("inventory")
    );
    let calls = RefCell::new(0);
    let runner = Runner {
        stages: RefCell::new(Vec::new()),
        run: |request: CommandRequest<'_>| {
            *calls.borrow_mut() += 1;
            if request.input.is_empty() {
                return Ok(output(probe()));
            }
            let request: Request = serde_json::from_slice(request.input).unwrap();
            assert_eq!(request.protocol, VERSION);
            assert_eq!(request.expected_current, old.id.to_string());
            let manifest = fs::read(&request.manifest).unwrap();
            let archive = fs::read(&request.archive).unwrap();
            // A future candidate stand-in applies its own policy, not Product::Cli
            // from the old reader. Publication remains the real production owner.
            let artifact = artifact::acquire_candidate(
                &manifest,
                &downloaded.archive_name,
                &archive,
                &current.target,
            )
            .unwrap();
            assert_eq!(artifact.files.len(), Product::Cli.files().len() + 1);
            assert_eq!(
                artifact.files["future-companion"],
                b"future companion bytes"
            );
            let artifact = artifact::Artifact {
                name: downloaded.archive_name.clone(),
                version: artifact.version,
                target: current.target.clone(),
                sha256: artifact.sha256,
                files: artifact.files,
                executable_files: artifact.executable_files,
            };
            let report = super::super::activate(
                super::super::ActivationRequest {
                    product: Product::Cli,
                    prefix: &request.prefix,
                    channel: Channel::Stable,
                    pin: PinAction::Preserve,
                    expected: Some(old.id),
                    provenance: Some(downloaded.provenance.clone()),
                    verifier: None,
                },
                &artifact,
                || Ok(()),
            )
            .unwrap();
            // This stand-in represents a candidate with a new product row.
            // The current publisher cannot know that row: supply its companion
            // bytes before returning the candidate's completed report.
            let companion = report
                .active_executable
                .parent()
                .unwrap()
                .join("future-companion");
            fs::write(&companion, &artifact.files["future-companion"]).unwrap();
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&companion, fs::Permissions::from_mode(0o755)).unwrap();
            Ok(response(Some(report), None))
        },
    };
    let report = upgrade(
        &current,
        &downloaded,
        Channel::Stable,
        PinAction::Preserve,
        || Ok(()),
        &runner,
    )
    .unwrap();
    assert_eq!(*calls.borrow(), 2);
    assert!(report.changed);
    assert_eq!(
        fs::read(
            report
                .active_executable
                .parent()
                .unwrap()
                .join("future-companion")
        )
        .unwrap(),
        b"future companion bytes"
    );
    let receipt: serde_json::Value = serde_json::from_slice(
        &fs::read(
            report
                .active_executable
                .parent()
                .unwrap()
                .join("receipt.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(receipt["file_sha256"]["future-companion"].is_string());
    assert_eq!(receipt["source"]["release_id"], 42);
    assert_eq!(
        fs::read(&current.active_executable).unwrap(),
        b"synthetic tmt payload\n"
    );
    assert!(
        inspect(&report.active_executable).is_err(),
        "old policy must still reject new inventory when explicitly asked"
    );
    assert_clean(&runner);
}

#[test]
fn unsupported_or_malformed_probe_never_calls_install_or_changes_current() {
    for bytes in [
        b"old installer".as_slice(),
        br#"{"protocol":2}"#,
        br#"{"protocol":1,"extra":true}"#,
    ] {
        let (_directory, layout, old) = published_layout();
        let current = inspect(&layout.prefix.join("bin/tmt")).unwrap();
        let calls = RefCell::new(0);
        let runner = Runner {
            stages: RefCell::new(Vec::new()),
            run: |request: CommandRequest<'_>| {
                *calls.borrow_mut() += 1;
                assert!(request.input.is_empty());
                Ok(CommandOutput {
                    stdout: bytes.to_vec(),
                    stderr: Vec::new(),
                })
            },
        };
        let error = upgrade(
            &current,
            &downloaded(false),
            Channel::Stable,
            PinAction::Preserve,
            || Ok(()),
            &runner,
        )
        .unwrap_err();
        assert!(error.get_ref().unwrap().is::<Unsupported>());
        assert_eq!(error.to_string(), UNSUPPORTED);
        assert_eq!(*calls.borrow(), 1);
        assert_eq!(layout.current().unwrap().unwrap().id, old.id);
        assert_clean(&runner);
    }
}

#[test]
fn checksum_failure_precedes_candidate_execution_and_publication() {
    let (_directory, layout, old) = published_layout();
    let current = inspect(&layout.prefix.join("bin/tmt")).unwrap();
    let mut downloaded = downloaded(false);
    downloaded.archive[0] ^= 1;
    let runner = Runner {
        stages: RefCell::new(Vec::new()),
        run: |_: CommandRequest<'_>| -> Result<CommandOutput, CommandError> {
            panic!("unverified candidate executed")
        },
    };
    let error = upgrade(
        &current,
        &downloaded,
        Channel::Stable,
        PinAction::Preserve,
        || Ok(()),
        &runner,
    )
    .unwrap_err();
    assert!(error.to_string().contains("checksum"));
    assert!(runner.stages.borrow().is_empty());
    assert_eq!(layout.current().unwrap().unwrap().id, old.id);
}

#[test]
fn candidate_failure_preserves_active_receipt_and_old_bytes_and_cleans_stage() {
    let (_directory, layout, old) = published_layout();
    let current = inspect(&layout.prefix.join("bin/tmt")).unwrap();
    let before = fs::read(&current.active_executable).unwrap();
    let runner = Runner {
        stages: RefCell::new(Vec::new()),
        run: |request: CommandRequest<'_>| {
            if request.input.is_empty() {
                return Ok(output(probe()));
            }
            let mut failure = CommandError::new(CommandFailure::Exit {
                code: Some(1),
                signal: None,
            });
            failure.output = Some(response(None, Some("candidate inventory rejected")));
            Err(failure)
        },
    };
    let error = upgrade(
        &current,
        &downloaded(false),
        Channel::Stable,
        PinAction::Preserve,
        || Ok(()),
        &runner,
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "candidate inventory rejected");
    assert_eq!(layout.current().unwrap().unwrap().id, old.id);
    assert_eq!(fs::read(&current.active_executable).unwrap(), before);
    assert_clean(&runner);
}

fn append_unsafe(downloaded: &mut DownloadedRelease, name: &str, kind: tar::EntryType) {
    use std::io::Read;
    let mut decoded = Vec::new();
    flate2::read::GzDecoder::new(downloaded.archive.as_slice())
        .read_to_end(&mut decoded)
        .unwrap();
    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut builder = tar::Builder::new(encoder);
    for entry in tar::Archive::new(decoded.as_slice()).entries().unwrap() {
        let mut entry = entry.unwrap();
        let header = entry.header().clone();
        builder.append(&header, &mut entry).unwrap();
    }
    let mut header = tar::Header::new_gnu();
    let root = downloaded.archive_name.strip_suffix(".tar.gz").unwrap();
    let path = if name.starts_with('/') {
        name.to_owned()
    } else {
        format!("{root}/{name}")
    };
    // Construct the invalid path without tar's writer normalizing it.
    header.as_mut_bytes()[..path.len()].copy_from_slice(path.as_bytes());
    header.set_entry_type(kind);
    header.set_mode(0o755);
    let bytes = if kind.is_file() {
        b"bad".as_slice()
    } else {
        b"".as_slice()
    };
    header.set_size(bytes.len() as u64);
    if kind.is_symlink() || kind.is_hard_link() {
        header.set_link_name("../../outside").unwrap();
    }
    header.set_cksum();
    builder.append(&header, bytes).unwrap();
    downloaded.archive = builder.into_inner().unwrap().finish().unwrap();
    let mut manifest: serde_json::Value = serde_json::from_slice(&downloaded.manifest).unwrap();
    manifest["artifacts"][&downloaded.archive_name]["checksums"]["sha256"] =
        artifact::digest(&downloaded.archive).into();
    downloaded.manifest = serde_json::to_vec(&manifest).unwrap();
    downloaded.provenance.manifest_sha256 = artifact::digest(&downloaded.manifest);
}

#[test]
fn unsafe_archive_entries_fail_before_probe_even_with_matching_digests() {
    for (path, kind) in [
        ("../escape", tar::EntryType::Regular),
        ("/absolute", tar::EntryType::Regular),
        ("sub/../../escape", tar::EntryType::Regular),
        ("./alias", tar::EntryType::Regular),
        ("tmt", tar::EntryType::Regular),
        ("tmt/child", tar::EntryType::Regular),
        ("link", tar::EntryType::Symlink),
        ("hardlink", tar::EntryType::Link),
        ("pipe", tar::EntryType::Fifo),
    ] {
        let (_directory, layout, old) = published_layout();
        let current = inspect(&layout.prefix.join("bin/tmt")).unwrap();
        let mut downloaded = downloaded(false);
        append_unsafe(&mut downloaded, path, kind);
        let runner = Runner {
            stages: RefCell::new(Vec::new()),
            run: |_: CommandRequest<'_>| -> Result<CommandOutput, CommandError> {
                panic!("unsafe archive executed")
            },
        };
        assert!(
            upgrade(
                &current,
                &downloaded,
                Channel::Stable,
                PinAction::Preserve,
                || Ok(()),
                &runner
            )
            .is_err(),
            "accepted {path}"
        );
        assert!(runner.stages.borrow().is_empty());
        assert_eq!(layout.current().unwrap().unwrap().id, old.id);
    }
}

#[test]
fn cancellation_between_probe_and_install_preserves_old_install_and_removes_staging() {
    let (_directory, layout, old) = published_layout();
    let current = inspect(&layout.prefix.join("bin/tmt")).unwrap();
    let checkpoints = RefCell::new(0);
    let calls = RefCell::new(0);
    let runner = Runner {
        stages: RefCell::new(Vec::new()),
        run: |request: CommandRequest<'_>| {
            *calls.borrow_mut() += 1;
            assert!(request.input.is_empty());
            Ok(output(probe()))
        },
    };
    let error = upgrade(
        &current,
        &downloaded(false),
        Channel::Stable,
        PinAction::Preserve,
        || {
            *checkpoints.borrow_mut() += 1;
            if *checkpoints.borrow() == 2 {
                Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"))
            } else {
                Ok(())
            }
        },
        &runner,
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert_eq!(*calls.borrow(), 1);
    assert_eq!(layout.current().unwrap().unwrap().id, old.id);
    assert_clean(&runner);
}

#[test]
fn failed_probe_is_actionable_without_installation() {
    for kind in [
        CommandFailure::Spawn,
        CommandFailure::Timeout,
        CommandFailure::OutputLimit,
        CommandFailure::Exit {
            code: Some(1),
            signal: None,
        },
    ] {
        let (_directory, layout, old) = published_layout();
        let current = inspect(&layout.prefix.join("bin/tmt")).unwrap();
        let runner = Runner {
            stages: RefCell::new(Vec::new()),
            run: |request: CommandRequest<'_>| {
                assert!(request.input.is_empty());
                Err(CommandError::new(kind))
            },
        };
        let error = upgrade(
            &current,
            &downloaded(false),
            Channel::Stable,
            PinAction::Preserve,
            || Ok(()),
            &runner,
        )
        .unwrap_err();
        assert!(super::super::UpgradeFailure::from(error).needs_new_installer());
        assert_eq!(layout.current().unwrap().unwrap().id, old.id);
        assert_clean(&runner);
    }
}

#[test]
fn malformed_install_result_or_timeout_never_reports_success() {
    for kind in [
        None,
        Some(CommandFailure::Timeout),
        Some(CommandFailure::OutputLimit),
    ] {
        let (_directory, layout, old) = published_layout();
        let current = inspect(&layout.prefix.join("bin/tmt")).unwrap();
        let runner = Runner {
            stages: RefCell::new(Vec::new()),
            run: |request: CommandRequest<'_>| {
                if request.input.is_empty() {
                    return Ok(output(probe()));
                }
                match kind {
                    None => Ok(output(
                        serde_json::json!({"protocol":1,"installation":null,"error":null}),
                    )),
                    Some(kind) => Err(CommandError::new(kind)),
                }
            },
        };
        assert!(
            upgrade(
                &current,
                &downloaded(false),
                Channel::Stable,
                PinAction::Preserve,
                || Ok(()),
                &runner
            )
            .is_err()
        );
        assert_eq!(layout.current().unwrap().unwrap().id, old.id);
        assert_clean(&runner);
    }
}

#[test]
fn reported_post_activation_failure_retains_the_activated_installation() {
    let (_directory, layout, old) = published_layout();
    let current = inspect(&layout.prefix.join("bin/tmt")).unwrap();
    let downloaded = downloaded(false);
    let runner = Runner {
        stages: RefCell::new(Vec::new()),
        run: |request: CommandRequest<'_>| {
            if request.input.is_empty() {
                return Ok(output(probe()));
            }
            let report = super::super::test_support::install_downloaded(
                &current,
                &downloaded,
                Channel::Stable,
                PinAction::Preserve,
                &mut || Ok(()),
            )
            .unwrap();
            let mut error = CommandError::new(CommandFailure::Exit {
                code: Some(1),
                signal: None,
            });
            error.output = Some(response(
                Some(report),
                Some("Release activated; finalization failed"),
            ));
            Err(error)
        },
    };
    let error = upgrade(
        &current,
        &downloaded,
        Channel::Stable,
        PinAction::Preserve,
        || Ok(()),
        &runner,
    )
    .unwrap_err();
    let activated = error
        .get_ref()
        .unwrap()
        .downcast_ref::<ActivatedInstallation>()
        .unwrap();
    assert!(activated.report.changed);
    assert_eq!(activated.report.version, "1.2.4");
    assert_ne!(layout.current().unwrap().unwrap().id, old.id);
    assert_eq!(
        fs::read(&current.active_executable).unwrap(),
        b"synthetic tmt payload\n"
    );
    assert_clean(&runner);
}

#[test]
fn installer_response_requires_all_fields_and_rejects_unknown_fields() {
    assert!(parse_response(br#"{"protocol":1,"installation":null,"error":"failed"}"#).is_ok());
    for bytes in [
        br#"{"protocol":1,"installation":null}"#.as_slice(),
        br#"{"protocol":1,"error":null}"#,
        br#"{"protocol":1,"installation":null,"error":null,"extra":true}"#,
        br#"{"protocol":1,"installation":null,"extra":true}"#,
    ] {
        assert!(parse_response(bytes).is_err());
    }
}

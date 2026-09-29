//! Hostile protocol fixtures exercise the managed execution boundary, not release artifacts.

use crate::office_companion::{invoke_office_board, probe_office_companion};
use crate::test_support::{install_office, office_fixture, office_fixture_with_payload};
use std::fs;
use tmt_adapters::native_install::{self, Product};

#[test]
fn verified_installed_companion_capability_and_board_dispatch_are_one_shot() {
    let fixture=office_fixture_with_payload(br#"#!/bin/sh
control="${0%/lib/tmt-office/releases/*}/board-calls"
printf '%s\n' "$3" >> "$control"
case "$3" in
probe) printf 'TMT-OFFICE/1\n1.2.3\n' ;;
capabilities) printf 'TMT-OFFICE-CAPABILITIES/1\noffice_board_v1\n' ;;
board-post) cat >/dev/null; printf '{"entryId":"11111111-1111-4111-8111-111111111111","threadId":"11111111-1111-4111-8111-111111111111","revision":1,"created":true,"operationId":"22222222-2222-4222-8222-222222222222"}' ;;
*) exit 1 ;;
esac
"#);
    let prefix = fixture.directory.path.join("prefix");
    let report = install_office(&fixture, &prefix).unwrap();
    let calls = prefix.join("board-calls");
    fs::remove_file(&calls).unwrap();
    let value = invoke_office_board(
        &report.executable,
        tmt_office_model::office_protocol::OfficeInvocation::BoardPost,
        b"{}",
        std::time::Instant::now() + std::time::Duration::from_secs(3),
    )
    .unwrap()
    .unwrap();
    assert_eq!(value["created"], true);
    assert_eq!(value["revision"], 1);
    assert_eq!(
        fs::read_to_string(calls).unwrap(),
        "capabilities\nboard-post\n"
    );
}

#[test]
fn incompatible_capability_process_never_dispatches_board_operation() {
    for capability in ["", "unknown_v1", "office_board_v1\\noffice_board_v1"] {
        let payload = format!(
            r#"#!/bin/sh
control="${{0%/lib/tmt-office/releases/*}}/board-calls"
case "$3" in
probe) printf 'TMT-OFFICE/1\n1.2.3\n' ;;
capabilities) printf 'TMT-OFFICE-CAPABILITIES/1\n{capability}\n' ;;
board-post) printf called > "$control" ;;
*) exit 1 ;;
esac
"#
        );
        let fixture = office_fixture_with_payload(payload.as_bytes());
        let prefix = fixture.directory.path.join("prefix");
        let report = install_office(&fixture, &prefix).unwrap();
        let marker = prefix.join("board-calls");
        let error = invoke_office_board(
            &report.executable,
            tmt_office_model::office_protocol::OfficeInvocation::BoardPost,
            b"{}",
            std::time::Instant::now() + std::time::Duration::from_secs(3),
        )
        .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::Unsupported);
        assert!(!marker.exists());
    }
}

#[test]
fn uninstall_between_capability_and_board_launch_never_dispatches_mutation() {
    let fixture = office_fixture_with_payload(
        br#"#!/bin/sh
control="${0%/lib/tmt-office/releases/*}/board-race"
case "$3" in
probe) printf 'TMT-OFFICE/1\n1.2.3\n' ;;
capabilities)
  mkdir -p "$control"
  printf ready > "$control/ready"
  while [ ! -f "$control/release" ]; do sleep 0.01; done
  printf 'TMT-OFFICE-CAPABILITIES/1\noffice_board_v1\n'
  ;;
board-post) printf called > "$control/dispatched" ;;
*) exit 1 ;;
esac
"#,
    );
    let prefix = fixture.directory.path.join("prefix");
    let report = install_office(&fixture, &prefix).unwrap();
    let control = prefix.join("board-race");
    std::thread::scope(|scope| {
        let invocation = scope.spawn(|| {
            invoke_office_board(
                &report.executable,
                tmt_office_model::office_protocol::OfficeInvocation::BoardPost,
                b"{}",
                std::time::Instant::now() + std::time::Duration::from_secs(5),
            )
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while fs::read(control.join("ready")).ok().as_deref() != Some(b"ready") {
            assert!(!invocation.is_finished(), "capability process exited early");
            assert!(
                std::time::Instant::now() < deadline,
                "capability did not start"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(native_install::uninstall_extension(&prefix, Product::Office).unwrap());
        fs::write(control.join("release"), b"").unwrap();
        assert_eq!(
            invocation.join().unwrap().unwrap_err().kind(),
            std::io::ErrorKind::Unsupported
        );
    });
    assert!(!control.join("dispatched").exists());
}

#[test]
fn active_release_switch_after_capability_never_dispatches_on_either_release() {
    let old = office_fixture_with_payload(
        br#"#!/bin/sh
control="${0%/lib/tmt-office/releases/*}/board-race"
case "$3" in
probe) printf 'TMT-OFFICE/1\n1.2.3\n' ;;
capabilities)
  mkdir -p "$control"
  printf ready > "$control/ready"
  while [ ! -f "$control/release" ]; do sleep 0.01; done
  printf 'TMT-OFFICE-CAPABILITIES/1\noffice_board_v1\n'
  ;;
board-post) printf old > "$control/dispatched" ;;
*) exit 1 ;;
esac
"#,
    );
    let prefix = old.directory.path.join("prefix");
    let old_report = install_office(&old, &prefix).unwrap();
    let control = prefix.join("board-race");
    std::thread::scope(|scope| {
        let invocation = scope.spawn(|| {
            invoke_office_board(
                &old_report.executable,
                tmt_office_model::office_protocol::OfficeInvocation::BoardPost,
                b"{}",
                std::time::Instant::now() + std::time::Duration::from_secs(5),
            )
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while fs::read(control.join("ready")).ok().as_deref() != Some(b"ready") {
            assert!(!invocation.is_finished(), "capability process exited early");
            assert!(
                std::time::Instant::now() < deadline,
                "capability did not start"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let replacement = office_fixture_with_payload(
            b"#!/bin/sh\ncase \"$3\" in probe) printf 'TMT-OFFICE/1\\n1.2.4\\n' ;; board-post) printf called > \"${0%/lib/tmt-office/releases/*}/board-race/dispatched\" ;; *) exit 1 ;; esac\n",
        );
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&replacement.manifest).unwrap()).unwrap();
        manifest["releases"][0]["app_version"] = serde_json::json!("1.2.4");
        fs::write(
            &replacement.manifest,
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        install_office(&replacement, &prefix).unwrap();
        fs::write(control.join("release"), b"").unwrap();
        let error = invocation.join().unwrap().unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::Unsupported);
    });
    assert!(!control.join("dispatched").exists());
}

#[test]
fn verified_launch_releases_install_lock_before_waiting_for_the_companion() {
    let fixture = office_fixture_with_payload(
        br#"#!/bin/sh
control="${0%/lib/tmt-office/releases/*}/probe-control"
if [ -f "$control/block" ]; then
  printf ready > "$control/ready"
  while [ ! -f "$control/release" ]; do sleep 0.01; done
fi
printf 'TMT-OFFICE/1\n1.2.3\n'
"#,
    );
    let prefix = fixture.directory.path.join("prefix");
    let report = install_office(&fixture, &prefix).unwrap();
    let active = &report.active_executable;
    let control = prefix.join("probe-control");
    fs::create_dir(&control).unwrap();
    let marker = |suffix: &str| control.join(suffix);
    let before = fs::read(active).unwrap();
    fs::write(marker("block"), b"").unwrap();
    std::thread::scope(|scope| {
        let probe = scope.spawn(|| probe_office_companion(&report.executable));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while fs::read(marker("ready")).ok().as_deref() != Some(b"ready") {
            if probe.is_finished() {
                panic!(
                    "probe exited before its controlled gate: {:?}",
                    probe.join().unwrap()
                );
            }
            assert!(
                std::time::Instant::now() < deadline,
                "companion did not start"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        // A ready child proves launch happened. Deactivation must acquire the
        // real installer lock while that same child is still awaiting release.
        let removed = native_install::uninstall_extension(&prefix, Product::Office);
        fs::write(marker("release"), b"").unwrap();
        assert!(removed.unwrap());
        assert_eq!(probe.join().unwrap().unwrap(), "1.2.3");
    });
    assert!(!report.executable.exists());
    assert!(!prefix.join("lib/tmt-office/current").exists());
    assert_eq!(fs::read(active).unwrap(), before);
}

#[test]
fn verified_probe_uses_the_installed_executable_and_preserves_its_receipt() {
    let fixture = office_fixture_with_payload(b"#!/bin/sh\nprintf 'TMT-OFFICE/1\\n1.2.3\\n'\n");
    let prefix = fixture.directory.path.join("prefix");
    let report = install_office(&fixture, &prefix).unwrap();
    let receipt = report
        .active_executable
        .parent()
        .unwrap()
        .join("receipt.json");
    let before = fs::read(&receipt).unwrap();
    assert_eq!(probe_office_companion(&report.executable).unwrap(), "1.2.3");
    assert_eq!(fs::read(receipt).unwrap(), before);
}

#[test]
fn incompatible_noisy_and_nonzero_companions_are_not_successful_handshakes() {
    for (payload, expected) in [
        (
            b"#!/bin/sh\nprintf 'TMT-OFFICE/2\\n1.2.3\\n'\n".as_slice(),
            "Incompatible Office handshake.",
        ),
        (
            b"#!/bin/sh\nprintf 'TMT-OFFICE/1\\n9.0.0\\n'\n",
            "Office executable and installation versions disagree.",
        ),
        (
            b"#!/bin/sh\nprintf 'TMT-OFFICE/1\\n1.2.3\\n'; printf 'warning' >&2\n",
            "Office handshake produced unexpected diagnostics.",
        ),
        (
            b"#!/bin/sh\nprintf 'TMT-OFFICE/1\\n1.2.3\\n'; exit 1\n",
            "External command failed: Exit",
        ),
    ] {
        let fixture = office_fixture_with_payload(payload);
        let prefix = fixture.directory.path.join("prefix");
        let error = install_office(&fixture, &prefix).unwrap_err();
        assert!(error.to_string().starts_with(expected), "{error}");
        assert!(!prefix.join("lib/tmt-office/current").exists());
        assert_eq!(
            fs::read_dir(prefix.join("lib/tmt-office/releases"))
                .unwrap()
                .count(),
            0
        );
    }
}

#[test]
fn changed_payload_is_rejected_before_executing_it() {
    let fixture = office_fixture_with_payload(b"#!/bin/sh\nprintf 'TMT-OFFICE/1\\n1.2.3\\n'\n");
    let report = install_office(&fixture, &fixture.directory.path.join("prefix")).unwrap();
    // If executed, this replacement would produce a distinct protocol failure.
    fs::write(&report.active_executable, b"#!/bin/sh\nprintf 'tampered'\n").unwrap();
    let error = probe_office_companion(&report.executable).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Installed release file has changed; refusing replacement."
    );
}

#[test]
fn incompatible_upgrade_preserves_the_previous_working_release() {
    let good = office_fixture();
    let prefix = good.directory.path.join("prefix");
    let previous = install_office(&good, &prefix).unwrap();
    let before = fs::read(&previous.active_executable).unwrap();
    let pointer = fs::read_link(prefix.join("lib/tmt-office/current")).unwrap();
    let bad = office_fixture_with_payload(b"#!/bin/sh\nprintf 'TMT-OFFICE/2\\n1.2.4\\n'\n");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&bad.manifest).unwrap()).unwrap();
    manifest["releases"][0]["app_version"] = serde_json::json!("1.2.4");
    fs::write(&bad.manifest, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let error = install_office(&bad, &prefix).unwrap_err();
    assert_eq!(error.to_string(), "Incompatible Office handshake.");
    assert_eq!(
        fs::read_link(prefix.join("lib/tmt-office/current")).unwrap(),
        pointer
    );
    assert_eq!(fs::read(&previous.active_executable).unwrap(), before);
    assert_eq!(
        probe_office_companion(&previous.executable).unwrap(),
        "1.2.3"
    );
    assert_eq!(
        fs::read_dir(prefix.join("lib/tmt-office/releases"))
            .unwrap()
            .count(),
        1
    );
}

/// A rejected candidate leaves no current release and no retained release.
fn assert_nothing_published(prefix: &std::path::Path) {
    assert!(!prefix.join("lib/tmt-office/current").exists());
    assert!(!prefix.join("bin/tmt-office").exists());
    assert_eq!(
        fs::read_dir(prefix.join("lib/tmt-office/releases"))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn release_verifier_kills_a_hung_candidate_at_the_handshake_deadline() {
    let fixture = office_fixture_with_payload(b"#!/bin/sh\nexec sleep 30\n");
    let prefix = fixture.directory.path.join("prefix");
    let started = std::time::Instant::now();
    let error = install_office(&fixture, &prefix).unwrap_err();
    let elapsed = started.elapsed();
    assert_eq!(error.to_string(), "External command failed: Timeout");
    assert!(
        elapsed >= std::time::Duration::from_secs(5)
            && elapsed < std::time::Duration::from_secs(15),
        "{elapsed:?}"
    );
    assert_nothing_published(&prefix);
}

#[test]
fn release_verifier_rejects_a_handshake_over_the_output_bound() {
    let limit = tmt_office_model::office_protocol::OFFICE_PROTOCOL_OUTPUT_LIMIT;
    // A valid handshake padded one byte past the protocol's output bound.
    let payload = format!(
        "#!/bin/sh\nprintf 'TMT-OFFICE/1\\n1.2.3\\n'\nhead -c {} /dev/zero | tr '\\0' ' '\n",
        limit + 1 - "TMT-OFFICE/1\n1.2.3\n".len()
    );
    let fixture = office_fixture_with_payload(payload.as_bytes());
    let prefix = fixture.directory.path.join("prefix");
    let error = install_office(&fixture, &prefix).unwrap_err();
    assert_eq!(error.to_string(), "External command failed: OutputLimit");
    assert_nothing_published(&prefix);

    // Exactly at the bound, the output is read in full and judged by the codec.
    let payload = format!(
        "#!/bin/sh\nprintf 'TMT-OFFICE/1\\n1.2.3\\n'\nhead -c {} /dev/zero | tr '\\0' ' '\n",
        limit - "TMT-OFFICE/1\n1.2.3\n".len()
    );
    let fixture = office_fixture_with_payload(payload.as_bytes());
    let prefix = fixture.directory.path.join("prefix");
    let error = install_office(&fixture, &prefix).unwrap_err();
    assert_eq!(error.to_string(), "Incompatible Office handshake.");
    assert_nothing_published(&prefix);
}

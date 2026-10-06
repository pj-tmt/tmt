use super::*;
use crate::{
    native_install::{
        ActivationRequest, activate_with_local_schema, artifact, inspect, pr_resolver,
        publication::Layout, receipt::Provenance,
    },
    test_support::TestDirectory,
};
use std::fs;
use tmt_core::native_install::{PinAction, SchemaError};

fn local(version: u32) -> Vec<DatabaseSchema> {
    vec![DatabaseSchema {
        domain: "tmt-core-db".into(),
        version,
    }]
}
fn fixture() -> (artifact::Artifact, Provenance) {
    let downloaded = pr_resolver::tests::downloaded();
    let artifact = artifact::acquire_bytes(
        Product::Cli,
        &downloaded.manifest,
        &downloaded.archive_name,
        &downloaded.archive,
        pr_resolver::tests::TARGET,
    )
    .unwrap();
    (artifact, downloaded.provenance)
}
fn publish(
    prefix: &std::path::Path,
    artifact: &artifact::Artifact,
    provenance: Provenance,
    expected: Option<uuid::Uuid>,
    explicit_channel: bool,
    local_version: u32,
) -> io::Result<crate::native_install::InstallReport> {
    activate_with_local_schema(
        ActivationRequest {
            product: Product::Cli,
            prefix,
            channel: Channel::parse("pr234").unwrap(),
            pin: PinAction::Preserve,
            expected,
            provenance: Some(provenance),
            verifier: None,
            explicit_channel,
            schema: None,
        },
        artifact,
        || Ok(()),
        || Ok(local(local_version)),
    )
}
fn updated(proof: &Provenance, version: &str, run: u64) -> Provenance {
    let Provenance::Pr(proof) = proof else {
        panic!("PR evidence missing");
    };
    let mut proof = proof.clone();
    proof.producer.run_id = run;
    proof.candidate.payload_artifact.name = format!(
        "tmt-pr-rc-payload-v2-pr234-cli-{}-{run}-a1",
        proof.candidate.target
    );
    proof.candidate.version = version.into();
    Provenance::Pr(proof)
}

#[test]
fn managed_receipt_persists_pr_identity_epoch_and_admission_after_remote_expiry() {
    let directory = TestDirectory::new();
    let prefix = directory.path.join("prefix");
    let (artifact, source) = fixture();
    let report = publish(&prefix, &artifact, source.clone(), None, true, 48).unwrap();
    let installation = inspect(&report.active_executable).unwrap();
    assert_eq!(installation.state.channel, Channel::parse("pr234").unwrap());
    assert_eq!(installation.provenance, Some(source.clone()));
    let identity = installation
        .provenance
        .as_ref()
        .unwrap()
        .pr_identity()
        .unwrap()
        .unwrap();
    assert_eq!(identity.run_id(), 9001);
    let receipt_path = report
        .active_executable
        .parent()
        .unwrap()
        .join("receipt.json");
    let bytes = fs::read(&receipt_path).unwrap();
    assert!(bytes.len() > 16 * 1024 && bytes.len() < 64 * 1024);
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        json["source"]["evidence"]["eligibility"]["enabled_event_id"],
        6201
    );
    assert_eq!(
        json["source"]["evidence"]["admission"]["local_schemas"][0]["version"],
        48
    );
    // Reading this historical snapshot does not ask GitHub or judge today's clock.
    assert_eq!(
        Layout::existing(&prefix)
            .unwrap()
            .current()
            .unwrap()
            .unwrap()
            .provenance,
        Some(source)
    );
    for pointer in [
        "/source/evidence/head_sha",
        "/source/evidence/catalog_sha256",
        "/source/evidence/admission/timeline_sha256",
        "/source/evidence/eligibility/label_id",
        "/source/evidence/candidate/application_schema/databases/0/domain",
    ] {
        let mut invalid = json.clone();
        *invalid.pointer_mut(pointer).unwrap() = serde_json::Value::Null;
        fs::write(&receipt_path, serde_json::to_vec(&invalid).unwrap()).unwrap();
        assert!(inspect(&report.active_executable).is_err(), "{pointer}");
    }
    fs::write(receipt_path, bytes).unwrap();
    assert!(inspect(&report.active_executable).is_ok());
}

#[test]
fn equal_archived_version_new_run_refuses_and_changed_receipt_fences_publication() {
    let directory = TestDirectory::new();
    let prefix = directory.path.join("prefix");
    let (mut artifact, source) = fixture();
    let first = publish(&prefix, &artifact, source.clone(), None, true, 48).unwrap();
    let installation = inspect(&first.active_executable).unwrap();
    let next = updated(&source, "5.0.0-alpha.92", 9002);
    assert!(publish(&prefix, &artifact, next, Some(installation.id), false, 48).is_err());
    assert_eq!(
        inspect(&first.active_executable).unwrap().id,
        installation.id
    );
    artifact.version = "5.0.0-alpha.93".parse().unwrap();
    let next = updated(&source, "5.0.0-alpha.93", 9002);
    let second = publish(
        &prefix,
        &artifact,
        next.clone(),
        Some(installation.id),
        false,
        48,
    )
    .unwrap();
    assert!(second.changed);
    let active = inspect(&second.active_executable).unwrap();
    assert_ne!(active.id, installation.id);
    let old = fs::read(&first.active_executable).unwrap();
    assert!(publish(&prefix, &artifact, next, Some(installation.id), false, 48).is_err());
    assert_eq!(inspect(&second.active_executable).unwrap().id, active.id);
    assert_eq!(fs::read(&first.active_executable).unwrap(), old);
}

#[test]
fn local_schema_advancing_after_admission_refuses_before_publish_and_preserves_all_bytes() {
    let directory = TestDirectory::new();
    let prefix = directory.path.join("prefix");
    let (artifact, source) = fixture();
    let first = publish(&prefix, &artifact, source.clone(), None, true, 48).unwrap();
    let installation = inspect(&first.active_executable).unwrap();
    let before = fs::read_dir(prefix.join("lib/tmux-team/releases"))
        .unwrap()
        .count();
    let error = publish(&prefix, &artifact, source, Some(installation.id), false, 49).unwrap_err();
    assert_eq!(
        error.get_ref().unwrap().downcast_ref::<SchemaError>(),
        Some(&SchemaError::DataDowngrade)
    );
    assert_eq!(
        fs::read_dir(prefix.join("lib/tmux-team/releases"))
            .unwrap()
            .count(),
        before
    );
    assert_eq!(
        inspect(&first.active_executable).unwrap().id,
        installation.id
    );
}

#[test]
fn explicit_alpha_return_requires_known_schema_and_never_downgrades_data() {
    let directory = TestDirectory::new();
    let prefix = directory.path.join("prefix");
    let (artifact, source) = fixture();
    let first = publish(&prefix, &artifact, source.clone(), None, true, 48).unwrap();
    let installed = inspect(&first.active_executable).unwrap();
    let Provenance::Pr(pr) = source else {
        panic!("PR proof missing");
    };
    let schema = pr.candidate.application_schema;
    for (explicit, schema, local_version) in [
        (false, Some(schema.clone()), 48),
        (true, None, 48),
        (true, Some(schema.clone()), 49),
    ] {
        assert!(
            activate_with_local_schema(
                ActivationRequest {
                    product: Product::Cli,
                    prefix: &prefix,
                    channel: Channel::Alpha,
                    pin: PinAction::Preserve,
                    expected: Some(installed.id),
                    provenance: None,
                    verifier: None,
                    explicit_channel: explicit,
                    schema,
                },
                &artifact,
                || Ok(()),
                || Ok(local(local_version))
            )
            .is_err()
        );
        assert_eq!(inspect(&first.active_executable).unwrap().id, installed.id);
    }
    let returned = activate_with_local_schema(
        ActivationRequest {
            product: Product::Cli,
            prefix: &prefix,
            channel: Channel::Alpha,
            pin: PinAction::Preserve,
            expected: Some(installed.id),
            provenance: None,
            verifier: None,
            explicit_channel: true,
            schema: Some(schema),
        },
        &artifact,
        || Ok(()),
        || Ok(local(48)),
    )
    .unwrap();
    assert_eq!(
        inspect(&returned.active_executable).unwrap().state.channel,
        Channel::Alpha
    );
}

#[test]
fn schema_consent_does_not_cover_missing_domains_or_data_downgrades() {
    let (_, source) = fixture();
    let Provenance::Pr(proof) = source else {
        panic!("PR proof missing");
    };
    let mut candidate = proof.candidate.application_schema;
    let mut alpha = candidate.clone();
    alpha.databases[0].version = 47;
    let error = admit_databases(Product::Cli, &candidate, &alpha, &local(48), false).unwrap_err();
    assert_eq!(
        error.get_ref().unwrap().downcast_ref::<SchemaError>(),
        Some(&SchemaError::AheadOfAlpha)
    );
    admit_databases(Product::Cli, &candidate, &alpha, &local(48), true).unwrap();
    assert!(admit_databases(Product::Cli, &candidate, &alpha, &local(49), true).is_err());
    candidate.databases.clear();
    assert!(admit_databases(Product::Cli, &candidate, &alpha, &[], true).is_err());
    assert!(admit_databases(Product::Remote, &alpha, &alpha, &local(48), true).is_err());
}

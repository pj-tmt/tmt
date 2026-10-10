//! Remote-authored fixtures prove the offline entry. Real Colab bytes are read-only
//! inputs from Colab's vector home and are never substituted with fixture bytes.
#[allow(dead_code)]
#[path = "support/deploy_fixture.rs"]
mod deploy_fixture;
#[path = "support/deploy_vector.rs"]
mod deploy_vector;
use deploy_fixture::Root;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};
use tmt_remote::limits;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/deploy_vectors")
}
fn mirror(directory: &Path, stem: &str) -> Result<(), String> {
    let bytes = deploy_vector::read_envelope(&directory.join(format!("{stem}.json")))
        .map_err(str::to_owned)?;
    let composed = deploy_vector::compose(&bytes).map_err(str::to_owned)?;
    for (name, actual) in deploy_vector::OUTPUTS.iter().zip(&composed.files) {
        let path = directory.join(format!("{stem}.{name}"));
        let expected =
            fs::read(&path).map_err(|_| format!("missing golden: {}", path.display()))?;
        if expected != actual.as_bytes() {
            return Err(format!("golden drift: {name}"));
        }
    }
    let plan: Value = serde_json::from_str(&composed.files[2]).unwrap();
    let envelope: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(plan["declarationDigest"], envelope["declarationDigest"]);
    assert_eq!(
        plan["artifactDigest"],
        deploy_vector::digest(envelope["artifact"].as_str().unwrap().as_bytes())
    );
    let summary: Value = serde_json::from_str(&composed.summary).unwrap();
    assert_eq!(summary["planDigest"], plan["planDigest"]);
    Ok(())
}
#[test]
fn remote_fixture_matches_offline_composed_goldens_and_input_identity() {
    mirror(&fixture(), "remote-envelope").unwrap();
    let bytes = deploy_vector::read_envelope(&fixture().join("remote-envelope.json")).unwrap();
    let first = deploy_vector::compose(&bytes).unwrap();
    let second = deploy_vector::compose(&bytes).unwrap();
    assert_eq!(first.files, second.files);
    assert_eq!(first.summary, second.summary);
}
#[test]
fn colab_vector_matches_its_complete_composed_golden_trio_when_delivered() {
    let vectors =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../tmt-colab/contracts/vectors");
    let vector = vectors.join("deploy-declaration-v1.json");
    match vector.try_exists() {
        Ok(false) => println!(
            "PENDING #2397: real Colab declaration vector not delivered; Remote fixture proof only"
        ),
        Ok(true) => mirror(&vectors, "deploy-declaration-v1").unwrap(),
        Err(error) => panic!("cannot inspect Colab vector: {error}"),
    }
}
#[test]
fn a_present_vector_requires_every_golden_and_changed_bytes_require_regeneration() {
    let root = Root::new();
    for name in [
        "remote-envelope.json",
        "remote-envelope.firestore.rules",
        "remote-envelope.firestore.indexes.json",
        "remote-envelope.plan.json",
    ] {
        fs::copy(fixture().join(name), root.0.join(name)).unwrap();
    }
    for name in deploy_vector::OUTPUTS {
        let path = root.0.join(format!("remote-envelope.{name}"));
        let bytes = fs::read(&path).unwrap();
        fs::remove_file(&path).unwrap();
        assert!(
            mirror(&root.0, "remote-envelope")
                .unwrap_err()
                .starts_with("missing golden:")
        );
        fs::write(&path, bytes).unwrap();
    }
    let path = root.0.join("remote-envelope.json");
    let mut value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["declaration"] = json!(format!("{} ", value["declaration"].as_str().unwrap()));
    assert!(deploy_vector::compose(&serde_json::to_vec(&value).unwrap()).is_err());
    value["declarationDigest"] = json!(deploy_vector::digest(
        value["declaration"].as_str().unwrap().as_bytes()
    ));
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert_eq!(
        mirror(&root.0, "remote-envelope").unwrap_err(),
        "golden drift: plan.json"
    );
}
#[test]
fn malformed_capped_and_reserved_inputs_fail_through_the_production_owners() {
    let original: Value =
        serde_json::from_slice(&fs::read(fixture().join("remote-envelope.json")).unwrap()).unwrap();
    for change in [
        "digest",
        "artifact",
        "reserved",
        "index",
        "resources",
        "segments",
        "declaration-cap",
        "artifact-cap",
        "unknown",
    ] {
        let mut envelope = original.clone();
        let mut declaration: Value =
            serde_json::from_str(envelope["declaration"].as_str().unwrap()).unwrap();
        match change {
            "digest" => envelope["declarationDigest"] = json!("0".repeat(64)),
            "artifact" => {
                envelope["artifact"] = json!(format!("{} ", envelope["artifact"].as_str().unwrap()))
            }
            "reserved" => declaration["resources"][0]["path"] = json!("append"),
            "index" => declaration["resources"][0]["indexes"][0]["direction"] = json!("sideways"),
            "resources" => {
                declaration["resources"] = json!(vec![
                    declaration["resources"][0].clone();
                    limits::DECLARATION_RESOURCES + 1
                ])
            }
            "segments" => declaration["resources"][0]["path"] = json!("a/b/c/d/e/f/g/h/i"),
            "declaration-cap" => {
                envelope["declaration"] = json!(" ".repeat(limits::DECLARATION_BYTES + 1))
            }
            "artifact-cap" => {
                envelope["artifact"] = json!(" ".repeat(limits::DECLARATION_ARTIFACT_BYTES + 1))
            }
            "unknown" => envelope["unknown"] = json!(true),
            _ => unreachable!(),
        }
        if change == "artifact-cap" {
            declaration["admission"]["digest"] = json!(deploy_vector::digest(
                envelope["artifact"].as_str().unwrap().as_bytes()
            ));
        }
        if ["reserved", "index", "resources", "segments", "artifact-cap"].contains(&change) {
            envelope["declaration"] = json!(serde_json::to_string(&declaration).unwrap());
            envelope["declarationDigest"] = json!(deploy_vector::digest(
                envelope["declaration"].as_str().unwrap().as_bytes()
            ));
        }
        if change == "declaration-cap" {
            envelope["declarationDigest"] = json!(deploy_vector::digest(
                envelope["declaration"].as_str().unwrap().as_bytes()
            ));
        }
        assert!(
            deploy_vector::compose(&serde_json::to_vec(&envelope).unwrap()).is_err(),
            "{change}"
        );
    }
    let oversized = vec![b' '; limits::DEPLOY_DECLARATION_REPLY_BYTES + 1];
    assert!(deploy_vector::compose(&oversized).is_err());
    let root = Root::new();
    let path = root.0.join("oversized.json");
    fs::write(&path, oversized).unwrap();
    assert_eq!(
        deploy_vector::read_envelope(&path).unwrap_err(),
        "invalid-envelope"
    );
    let duplicate = serde_json::to_string(&original)
        .unwrap()
        .replacen("{", "{\"version\":1,", 1);
    assert!(deploy_vector::compose(duplicate.as_bytes()).is_err());
}

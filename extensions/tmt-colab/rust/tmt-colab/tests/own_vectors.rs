//! The same raw own update-v1 and projections are consumed by browser Worker tests.
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use nix::{errno::Errno, sys::signal::kill, unistd::Pid};
use serde_json::Value;
use tmt_colab::decoder::{Decoder, Namespace, Role, UpdateBatch};

#[test]
fn shared_own_vectors_preserve_raw_maps_deletions_and_checkpoint_dependencies() {
    let v: Value =
        serde_json::from_str(include_str!("../../../contracts/vectors/own-v1.json")).unwrap();
    let bytes = |value: &Value| URL_SAFE_NO_PAD.decode(value.as_str().unwrap()).unwrap();
    let checkpoint = bytes(&v["checkpoint"]);
    let tail = bytes(&v["tail"]);
    let originals = v["updates"]
        .as_array()
        .unwrap()
        .iter()
        .map(bytes)
        .collect::<Vec<_>>();
    let mut decoder = Decoder::new(env!("CARGO_BIN_EXE_tmt-colab").into()).unwrap();
    for (updates, expected) in [
        (vec![checkpoint.as_slice()], &v["expectedPrefix"]),
        (vec![checkpoint.as_slice(), tail.as_slice()], &v["expected"]),
        (
            originals
                .iter()
                .map(Vec::as_slice)
                .chain([tail.as_slice()])
                .collect(),
            &v["expected"],
        ),
    ] {
        let result = decoder
            .decode(
                UpdateBatch {
                    namespace: Namespace::Own,
                    baseline: &[],
                    updates: &updates,
                },
                Role::Commenter,
                None,
            )
            .unwrap();
        assert_eq!(&result.projection, expected);
        assert_eq!(
            kill(Pid::from_raw(result.child_pid as i32), None),
            Err(Errno::ESRCH)
        );
    }
    let boundary = bytes(&v["bodyBoundary"]);
    let result = decoder
        .decode(
            UpdateBatch {
                namespace: Namespace::Own,
                baseline: &[],
                updates: &[&boundary],
            },
            Role::Bridge,
            None,
        )
        .unwrap();
    assert_eq!(
        result.projection["messages"]["same"]["body"]
            .as_str()
            .unwrap()
            .len(),
        16 * 1024
    );
    assert_eq!(
        kill(Pid::from_raw(result.child_pid as i32), None),
        Err(Errno::ESRCH)
    );
    for negative in v["negative"].as_array().unwrap() {
        let update = bytes(&negative["update"]);
        assert!(
            decoder
                .decode(
                    UpdateBatch {
                        namespace: Namespace::Own,
                        baseline: &[],
                        updates: &[&update]
                    },
                    Role::Commenter,
                    None
                )
                .is_err(),
            "{}",
            negative["name"]
        );
    }
    assert!(
        decoder
            .decode(
                UpdateBatch {
                    namespace: Namespace::Content,
                    baseline: &[],
                    updates: &[&checkpoint]
                },
                Role::Editor,
                None
            )
            .is_err()
    );
    assert!(
        decoder
            .decode(
                UpdateBatch {
                    namespace: Namespace::Own,
                    baseline: &[],
                    updates: &[&checkpoint]
                },
                Role::Viewer,
                None
            )
            .is_err()
    );
}

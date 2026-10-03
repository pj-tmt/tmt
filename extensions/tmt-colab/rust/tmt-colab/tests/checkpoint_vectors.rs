//! Shared browser/native raw checkpoint bytes; foreign decoding remains in the child.
mod support;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use nix::{errno::Errno, sys::signal::kill, unistd::Pid};
use tmt_colab::decoder::{Decoder, Namespace, Role, UpdateBatch};

#[test]
fn shared_checkpoint_preserves_dependencies_and_delete_sets() {
    let v: serde_json::Value = serde_json::from_str(include_str!(
        "../../../contracts/vectors/checkpoint-v1.json"
    ))
    .unwrap();
    let bytes = |name: &str| URL_SAFE_NO_PAD.decode(v[name].as_str().unwrap()).unwrap();
    let checkpoint = bytes("checkpoint");
    let tail = bytes("tail");
    let mut decoder = Decoder::with_config(support::decoder_config(
        env!("CARGO_BIN_EXE_tmt-colab").into(),
    ))
    .unwrap();
    let prefix = decoder
        .decode(
            UpdateBatch {
                namespace: Namespace::Content,
                baseline: &[],
                updates: &[&checkpoint],
            },
            Role::Editor,
            None,
        )
        .unwrap();
    assert_eq!(prefix.projection["html"], v["sourceAtPrefix"]);
    assert_eq!(prefix.projection["meta"]["title"], v["title"]);
    let complete = decoder
        .decode(
            UpdateBatch {
                namespace: Namespace::Content,
                baseline: &[],
                updates: &[&checkpoint, &tail],
            },
            Role::Editor,
            None,
        )
        .unwrap();
    assert_eq!(complete.projection["html"], v["sourceAfterTail"]);
    for pid in [prefix.child_pid, complete.child_pid] {
        assert_eq!(kill(Pid::from_raw(pid as i32), None), Err(Errno::ESRCH));
    }
    assert!(
        decoder
            .decode(
                UpdateBatch {
                    namespace: Namespace::Content,
                    baseline: &[],
                    updates: &[&tail]
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
                Role::Editor,
                None
            )
            .is_err()
    );
    for negative in v["negativeBodies"].as_array().unwrap() {
        let bad = URL_SAFE_NO_PAD
            .decode(negative["bytes"].as_str().unwrap())
            .unwrap();
        assert!(
            decoder
                .decode(
                    UpdateBatch {
                        namespace: Namespace::Content,
                        baseline: &[],
                        updates: &[&bad]
                    },
                    Role::Editor,
                    None
                )
                .is_err()
        );
    }
}

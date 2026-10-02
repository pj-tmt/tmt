use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use nix::{errno::Errno, sys::signal::kill, unistd::Pid};
use std::{
    path::PathBuf,
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};
use tmt_colab::decoder::{
    BaselineInput, DecodeFault, Decoder, MemoryLimit, Namespace, Role, STREAM_BYTES, UpdateBatch,
};
use tmt_invoke::{Cleanup, EnvironmentPolicy, FailureKind, LaunchOptions, Request};
use yrs::{
    Array, Doc, GetString, Map, ReadTxn, StateVector, Text, Transact, Update,
    updates::decoder::Decode,
};
/// DEVELOPMENT ETXTBSY rule, case 2: something else execs the stand-in by path,
/// so a short-lived `sh` writes it and no test thread holds its descriptor.
fn write_executable(path: &std::path::Path, script: &str) {
    use std::io::Write;
    let mut writer = std::process::Command::new("/bin/sh")
        .args(["-c", "cat > \"$1\" && chmod 700 \"$1\"", "sh"])
        .arg(path)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .expect("start sh to write the executable");
    writer
        .stdin
        .take()
        .expect("sh stdin")
        .write_all(script.as_bytes())
        .expect("send the script to sh");
    let status = writer.wait().expect("wait for sh");
    assert!(status.success(), "sh could not write {}", path.display());
}
fn program() -> PathBuf {
    env!("CARGO_BIN_EXE_tmt-colab").into()
}
fn owner() -> Decoder {
    Decoder::new(program()).unwrap()
}
fn gone(pid: u32) {
    assert_eq!(
        kill(Pid::from_raw(pid as i32), None),
        Err(Errno::ESRCH),
        "decoder leaked"
    );
}
fn source() -> (Vec<u8>, Vec<Vec<u8>>) {
    let doc = Doc::with_client_id(123);
    let html = doc.get_or_insert_text("html");
    let meta = doc.get_or_insert_map("meta");
    html.insert(&mut doc.transact_mut(), 0, "hello");
    meta.insert(&mut doc.transact_mut(), "title", "test");
    let baseline = doc
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    let mut updates = Vec::new();
    for text in [" world", "!"] {
        let mut txn = doc.transact_mut();
        html.insert(&mut txn, 5, text);
        updates.push(txn.encode_update_v1());
    }
    let mut txn = doc.transact_mut();
    html.remove_range(&mut txn, 0, 1);
    updates.push(txn.encode_update_v1());
    (baseline, updates)
}
#[test]
fn child_materializes_and_merges_only_author_updates_preserving_dependencies_and_deletes() {
    let (baseline, updates) = source();
    let refs = updates.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let decoded = owner()
        .decode(
            UpdateBatch {
                namespace: Namespace::Content,
                baseline: &baseline,
                updates: &refs,
            },
            Role::Editor,
            None,
        )
        .unwrap();
    assert_eq!(decoded.projection["html"], "ello! world");
    assert_eq!(decoded.projection["meta"]["title"], "test");
    assert_eq!(
        decoded.memory_limit,
        if cfg!(target_os = "linux") {
            MemoryLimit::Enforced
        } else {
            MemoryLimit::Unavailable
        }
    );
    gone(decoded.child_pid);
    let doc = Doc::new();
    let html = doc.get_or_insert_text("html");
    doc.get_or_insert_map("meta");
    doc.transact_mut()
        .apply_update(Update::decode_v1(&baseline).unwrap())
        .unwrap();
    doc.transact_mut()
        .apply_update(Update::decode_v1(&decoded.merged).unwrap())
        .unwrap();
    assert_eq!(html.get_string(&doc.transact()), "ello! world");
    let empty = Doc::new();
    empty.get_or_insert_text("html");
    empty.get_or_insert_map("meta");
    empty
        .transact_mut()
        .apply_update(Update::decode_v1(&decoded.merged).unwrap())
        .unwrap();
    assert!(
        empty.transact().store().pending_update().is_some(),
        "baseline was folded into author's merge"
    );
}
#[test]
fn mixed_roots_wrong_types_and_incomplete_dependencies_apply_nothing() {
    for root in ["threads", "html", "meta"] {
        let doc = Doc::new();
        if root == "meta" {
            doc.get_or_insert_text(root)
                .insert(&mut doc.transact_mut(), 0, "wrong");
        } else {
            doc.get_or_insert_map(root)
                .insert(&mut doc.transact_mut(), "bad", true);
        }
        let input = doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default());
        assert!(matches!(
            owner().decode(
                UpdateBatch {
                    namespace: Namespace::Content,
                    baseline: &[],
                    updates: &[&input]
                },
                Role::Editor,
                None
            ),
            Err(DecodeFault::Rejected)
        ));
    }
    let (_, updates) = source();
    assert!(
        owner()
            .decode(
                UpdateBatch {
                    namespace: Namespace::Content,
                    baseline: &[],
                    updates: &[&updates[1]]
                },
                Role::Editor,
                None
            )
            .is_err()
    );
    let stop = AtomicBool::new(true);
    assert!(
        matches!(owner().decode(UpdateBatch { namespace: Namespace::Content, baseline: &[], updates: &[] }, Role::Editor, Some(&stop)), Err(DecodeFault::Invoke(e)) if e.kind == FailureKind::Interrupted)
    );
}
#[test]
fn archived_hostile_corpus_is_contained_with_confirmed_cleanup_twice() {
    let valid = include_bytes!("fixtures/hostile/valid.bin").to_vec();
    let mut seed = 0x830c01ab_u64;
    let mut corpus = vec![
        vec![],
        vec![255; 32],
        vec![1, 255, 255, 255, 255, 15],
        vec![0, 255, 255, 255, 255, 15],
        valid.clone(),
    ];
    for n in 0..256 {
        let mut bytes = if n % 2 == 0 {
            valid.clone()
        } else {
            Vec::new()
        };
        for _ in 0..(n % 256 + 1) {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            if n % 2 == 0 {
                let i = seed as usize % bytes.len();
                bytes[i] ^= (seed >> 32) as u8;
            } else {
                bytes.push(seed as u8);
            }
        }
        if n % 7 == 0 {
            bytes.truncate(n % bytes.len().max(1));
        }
        corpus.push(bytes);
    }
    for (index, bytes) in [
        (26, include_bytes!("fixtures/hostile/26.bin").as_slice()),
        (60, include_bytes!("fixtures/hostile/60.bin")),
        (106, include_bytes!("fixtures/hostile/106.bin")),
        (147, include_bytes!("fixtures/hostile/147.bin")),
        (157, include_bytes!("fixtures/hostile/157.bin")),
        (192, include_bytes!("fixtures/hostile/192.bin")),
    ] {
        assert_eq!(corpus[index], bytes, "generator drift at {index}");
    }
    // Record the exec-preserved PID so every contained outcome proves reaping.
    let path = program().to_string_lossy().replace('\'', "'\\''");
    let fixture = FixtureProgram::new(&format!("exec '{path}' \"$@\""));
    for run in 0..2 {
        let start = Instant::now();
        let mut timeout = 0;
        let mut panic = 0;
        for (index, bytes) in corpus.iter().enumerate() {
            assert!(
                start.elapsed() < Duration::from_secs(45),
                "corpus suite exceeded budget"
            );
            let input=serde_json::to_vec(&serde_json::json!({"version":1,"namespace":"content","baseline":"","updates":[URL_SAFE_NO_PAD.encode(bytes)]})).unwrap();
            let program = &fixture.script;
            if fixture.directory.join("pid").exists() {
                std::fs::remove_file(fixture.directory.join("pid")).unwrap();
            }
            let args = ["__decoder".into()];
            let outcome = match tmt_invoke::invoke(
                Request {
                    program,
                    args: &args,
                    input: &input,
                    deadline: Instant::now() + Duration::from_secs(1),
                    max_stream_bytes: STREAM_BYTES,
                    launch: LaunchOptions {
                        environment: EnvironmentPolicy::ClearAllowlist(&[]),
                        ..Default::default()
                    },
                },
                None,
            ) {
                Ok(output) => {
                    if !cfg!(target_os = "linux") {
                        assert_ne!(index, 26, "saved timeout did not hit its deadline");
                    }
                    assert!(
                        output.stdout.is_empty(),
                        "rejected fixture returned result bytes: {index}"
                    );
                    assert!(
                        !output.status.success(),
                        "fixture schema cannot be admitted: {index}"
                    );
                    let diagnostic = String::from_utf8_lossy(&output.stderr);
                    let panicked = diagnostic.contains("decoder panic");
                    if cfg!(target_os = "linux") && [26, 60, 106, 147, 157, 192].contains(&index) {
                        assert!(
                            panicked
                                || diagnostic.contains("decoder rejected")
                                || diagnostic.contains("decoder memory limit failed")
                                || (diagnostic.contains("memory allocation of")
                                    && diagnostic.contains("failed")),
                            "Linux saved dump exited without a containment diagnostic: {diagnostic}"
                        );
                    }
                    if !cfg!(target_os = "linux") && [60, 106, 147, 157, 192].contains(&index) {
                        assert!(panicked, "saved panic was not reproduced: {index}");
                    }
                    if panicked {
                        panic += 1;
                    }
                    if !cfg!(target_os = "linux") {
                        assert!(diagnostic.contains("memory limit unavailable"));
                    }
                    if panicked {
                        "panic"
                    } else if diagnostic.contains("decoder rejected") {
                        "rejected"
                    } else {
                        "memory-limit"
                    }
                }
                Err(e) => {
                    assert!(
                        cfg!(target_os = "linux") || ![60, 106, 147, 157, 192].contains(&index),
                        "saved panic timed out instead: {index}"
                    );
                    assert!(
                        matches!(e.cleanup, Cleanup::Confirmed),
                        "cleanup failed at {index}: {e}"
                    );
                    assert_eq!(e.kind, FailureKind::Deadline);
                    timeout += 1;
                    "deadline"
                }
            };
            match fixture.recorded_pid() {
                Some(pid) => gone(pid),
                None => assert_eq!(outcome, "deadline", "no pid without a deadline at {index}"),
            }
            if [26, 60, 106, 147, 157, 192].contains(&index) {
                eprintln!("hostile saved run {run}: dump {index}, {outcome}, reaped, no result");
            }
        }
        assert!(timeout + panic >= 1, "hostile evidence unexpectedly lost");
        eprintln!(
            "hostile run {run}: 261 cases, {panic} panics, {timeout} deadlines; confirmed cleanup; {:?}",
            start.elapsed()
        );
    }
    let mut decoder = Decoder::new(fixture.script.clone()).unwrap();
    for index in [26, 60, 106, 147, 157, 192] {
        match decoder.decode(
            UpdateBatch {
                namespace: Namespace::Content,
                baseline: &[],
                updates: &[&corpus[index]],
            },
            Role::Editor,
            None,
        ) {
            Err(DecodeFault::Invoke(e)) => {
                assert!(cfg!(target_os = "linux") || index == 26);
                assert_eq!(e.kind, FailureKind::Deadline);
                assert!(matches!(e.cleanup, Cleanup::Confirmed));
            }
            // invoke has waited/reaped a non-success exit; the corpus assertion above
            // checks its diagnostic. The public runner deliberately hides child stderr.
            // Dump 26 may finish inside the runner's two-second deadline on a fast host,
            // so a clean rejection is contained here on every platform.
            Err(DecodeFault::Rejected) => {}
            Ok(_) => panic!("saved dump {index} unexpectedly succeeded"),
            Err(other) => panic!("saved dump {index} was not contained: {other:?}"),
        }
        gone(fixture.pid());
        let reply = decoder
            .decode(
                UpdateBatch {
                    namespace: Namespace::Content,
                    baseline: &[],
                    updates: &[],
                },
                Role::Editor,
                None,
            )
            .unwrap();
        gone(reply.child_pid);
    }
}

struct FixtureProgram {
    directory: PathBuf,
    script: PathBuf,
}
impl FixtureProgram {
    fn new(body: &str) -> Self {
        use std::{
            os::unix::fs::PermissionsExt,
            sync::atomic::{AtomicU64, Ordering},
        };
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "tmt-decoder-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&directory).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        let script = directory.join("child");
        let pid_path = directory
            .join("pid")
            .to_string_lossy()
            .replace('\'', "'\\''");
        write_executable(
            &script,
            &format!("#!/bin/sh\nprintf '%s\\n' \"$$\" > '{pid_path}'\n{body}\n"),
        );
        Self { directory, script }
    }
    fn pid(&self) -> u32 {
        self.recorded_pid().expect("fixture child recorded its pid")
    }
    /// None when a deadline killed the wrapper's process group before it wrote
    /// its pid; Confirmed cleanup then already proves termination.
    fn recorded_pid(&self) -> Option<u32> {
        match std::fs::read_to_string(self.directory.join("pid")) {
            Ok(text) => Some(text.trim().parse().unwrap()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => panic!("read fixture pid: {e}"),
        }
    }
    fn actual(&self) {
        let path = program().to_string_lossy().replace('\'', "'\\''");
        write_executable(&self.script, &format!("#!/bin/sh\nexec '{path}' \"$@\"\n"));
    }
}
impl Drop for FixtureProgram {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.directory).unwrap();
    }
}
#[test]
fn environment_is_cleared_and_successful_invalid_output_is_rejected() {
    assert!(
        std::env::var_os("HOME").is_some(),
        "inherited control needs HOME"
    );
    let fixture = FixtureProgram::new("if [ -n \"${HOME+x}\" ]; then exit 7; fi\nprintf '{}'");
    assert_eq!(
        std::process::Command::new(&fixture.script)
            .status()
            .unwrap()
            .code(),
        Some(7)
    );
    assert!(matches!(
        Decoder::new(fixture.script.clone()).unwrap().decode(
            UpdateBatch {
                namespace: Namespace::Content,
                baseline: &[],
                updates: &[]
            },
            Role::Editor,
            None
        ),
        Err(DecodeFault::InvalidOutput)
    ));
}
#[test]
fn deadline_and_output_backpressure_confirm_cleanup_before_owner_reuse() {
    for body in [
        "exec /bin/sleep 60",
        "exec /usr/bin/head -c 4194305 /dev/zero",
    ] {
        let fixture = FixtureProgram::new(body);
        let mut decoder = Decoder::new(fixture.script.clone()).unwrap();
        // A child which never reads stdin also exercises bounded input backpressure.
        let baseline = vec![0; tmt_colab::decoder::BASELINE_BYTES];
        let error = decoder
            .decode(
                UpdateBatch {
                    namespace: Namespace::Content,
                    baseline: &baseline,
                    updates: &[],
                },
                Role::Editor,
                None,
            )
            .err()
            .unwrap();
        assert!(
            matches!(error,DecodeFault::Invoke(ref e) if matches!(e.cleanup,Cleanup::Confirmed) && matches!(e.kind,FailureKind::Deadline|FailureKind::OutputLimit(_)))
        );
        match fixture.recorded_pid() {
            Some(pid) => gone(pid),
            None => assert!(
                matches!(error, DecodeFault::Invoke(ref e) if e.kind == FailureKind::Deadline),
                "no pid without a deadline"
            ),
        }
        fixture.actual();
        let reply = decoder
            .decode(
                UpdateBatch {
                    namespace: Namespace::Content,
                    baseline: &[],
                    updates: &[],
                },
                Role::Editor,
                None,
            )
            .unwrap();
        gone(reply.child_pid);
    }
}

#[test]
fn own_maps_have_a_positive_control_and_array_substitution_rejects() {
    let doc = Doc::new();
    for name in ["threads", "messages", "intents", "replies"] {
        doc.get_or_insert_map(name);
    }
    doc.get_or_insert_map("threads")
        .insert(&mut doc.transact_mut(), "id", "thread");
    let input = doc
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    let reply = owner()
        .decode(
            UpdateBatch {
                namespace: Namespace::Own,
                baseline: &[],
                updates: &[&input],
            },
            Role::Commenter,
            None,
        )
        .unwrap();
    assert_eq!(reply.projection["threads"]["id"], "thread");
    gone(reply.child_pid);
    let wrong = Doc::new();
    wrong
        .get_or_insert_array("html")
        .insert(&mut wrong.transact_mut(), 0, "wrong");
    let input = wrong
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    assert!(matches!(
        owner().decode(
            UpdateBatch {
                namespace: Namespace::Content,
                baseline: &[],
                updates: &[&input]
            },
            Role::Editor,
            None
        ),
        Err(DecodeFault::Rejected)
    ));
}

fn view<'a>(source: &'a [u8], title: &'a str) -> BaselineInput<'a> {
    use sha2::{Digest, Sha256};
    BaselineInput {
        source,
        title,
        source_digest: Sha256::digest(source).into(),
    }
}
fn apply_baseline(doc: &Doc, update: &[u8]) {
    doc.get_or_insert_text("html");
    doc.get_or_insert_map("meta");
    doc.transact_mut()
        .apply_update(Update::decode_v1(update).unwrap())
        .unwrap();
}
#[test]
fn baseline_exact_vectors_materialize_and_concurrent_clients_converge() {
    let vectors: serde_json::Value =
        serde_json::from_str(include_str!("../../../contracts/vectors/baseline-v1.json")).unwrap();
    for vector in vectors.as_array().unwrap() {
        let source = vector["source"].as_str().unwrap();
        let title = vector["title"].as_str().unwrap();
        let update = URL_SAFE_NO_PAD
            .decode(vector["update"].as_str().unwrap())
            .unwrap();
        let commitment: [u8; 32] = URL_SAFE_NO_PAD
            .decode(vector["commitment"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        let mut decoder = owner();
        let verified = decoder
            .verify_baseline(view(source.as_bytes(), title), &update, commitment, None)
            .unwrap();
        assert_eq!(verified.update, update);
        gone(verified.child_pid);
        let produced = decoder
            .produce_baseline(view(source.as_bytes(), title), None)
            .unwrap();
        gone(produced.child_pid);
        // Both clients start from the identical owner-produced struct identity.
        let a = Doc::with_client_id(41);
        let b = Doc::with_client_id(42);
        apply_baseline(&a, &produced.update);
        apply_baseline(&b, &produced.update);
        for doc in [&a, &b] {
            let html = doc.get_or_insert_text("html");
            let meta = doc.get_or_insert_map("meta");
            let txn = doc.transact();
            assert_eq!(html.get_string(&txn), source);
            assert_eq!(meta.get(&txn, "title").unwrap().to_string(&txn), title);
        }
        let change = |doc: &Doc, text: &str| {
            let html = doc.get_or_insert_text("html");
            let mut txn = doc.transact_mut();
            html.insert(&mut txn, 0, text);
            txn.encode_update_v1()
        };
        let first = change(&a, "A");
        let second = change(&b, "B");
        apply_baseline(&a, &second);
        apply_baseline(&b, &first);
        // Replay the baseline never independently inserts or duplicates source.
        apply_baseline(&a, &produced.update);
        apply_baseline(&b, &produced.update);
        assert_eq!(a.transact().state_vector(), b.transact().state_vector());
        let actual = a.get_or_insert_text("html").get_string(&a.transact());
        assert_eq!(
            actual,
            b.get_or_insert_text("html").get_string(&b.transact())
        );
        assert_eq!(actual, format!("AB{source}"));
    }
}
#[test]
fn baseline_digest_commitment_and_materialization_mismatches_return_no_result() {
    use sha2::{Digest, Sha256};
    let mut decoder = owner();
    let baseline = decoder
        .produce_baseline(view(b"exact", "title"), None)
        .unwrap();
    gone(baseline.child_pid);
    let wrong_digest = BaselineInput {
        source: b"exact",
        title: "title",
        source_digest: [0; 32],
    };
    assert!(matches!(
        decoder.produce_baseline(wrong_digest, None),
        Err(DecodeFault::InvalidInput)
    ));
    assert!(matches!(
        decoder.verify_baseline(view(b"exact", "title"), &baseline.update, [0; 32], None),
        Err(DecodeFault::Rejected)
    ));
    assert!(matches!(
        decoder.verify_baseline(
            view(b"exact", "different title"),
            &baseline.update,
            baseline.commitment,
            None
        ),
        Err(DecodeFault::Rejected)
    ));
    let changed_source = b"other";
    // Valid hashes for the changed claim cannot substitute materialized text.
    let framed = tmt_colab_model::framing::frame(&[
        b"tmt-colab-baseline-v1",
        b"1",
        changed_source,
        &baseline.update,
    ])
    .unwrap();
    assert!(matches!(
        decoder.verify_baseline(
            view(changed_source, "title"),
            &baseline.update,
            Sha256::digest(framed).into(),
            None
        ),
        Err(DecodeFault::Rejected)
    ));
    for update in [&[255u8; 32][..], &[0u8, 0][..]] {
        let framed =
            tmt_colab_model::framing::frame(&[b"tmt-colab-baseline-v1", b"1", b"exact", update])
                .unwrap();
        assert!(matches!(
            decoder.verify_baseline(
                view(b"exact", "title"),
                update,
                Sha256::digest(framed).into(),
                None
            ),
            Err(DecodeFault::Rejected)
        ));
    }
    // A normal child remains usable after every clean rejection.
    let checked = decoder
        .verify_baseline(
            view(b"exact", "title"),
            &baseline.update,
            baseline.commitment,
            None,
        )
        .unwrap();
    gone(checked.child_pid);
}
#[test]
fn baseline_bounds_and_cleanup_use_the_existing_runner() {
    use tmt_colab::decoder::{BASELINE_BYTES, BASELINE_TITLE_BYTES};
    let mut decoder = owner();
    let source = vec![b'x'; BASELINE_BYTES];
    let produced = decoder
        .produce_baseline(view(&source, "title"), None)
        .unwrap();
    gone(produced.child_pid);
    assert!(produced.update.len() > BASELINE_BYTES);
    assert!(matches!(
        decoder.produce_baseline(view(&vec![b'x'; BASELINE_BYTES + 1], "title"), None),
        Err(DecodeFault::InvalidInput)
    ));
    assert!(matches!(
        decoder.produce_baseline(view(&[255], "title"), None),
        Err(DecodeFault::InvalidInput)
    ));
    assert!(matches!(
        decoder.produce_baseline(view(b"", &"x".repeat(BASELINE_TITLE_BYTES + 1)), None),
        Err(DecodeFault::InvalidInput)
    ));
    let checked = decoder
        .verify_baseline(
            view(&source, "title"),
            &produced.update,
            produced.commitment,
            None,
        )
        .unwrap();
    assert_eq!(checked.update, produced.update);
    gone(checked.child_pid);
    for body in [
        "exec /bin/sleep 60",
        "exec /usr/bin/head -c 4194305 /dev/zero",
        "cat >/dev/null; printf '{}'",
    ] {
        let fixture = FixtureProgram::new(body);
        let mut decoder = Decoder::new(fixture.script.clone()).unwrap();
        let error = decoder
            .produce_baseline(view(&source, "title"), None)
            .err()
            .unwrap();
        if body == "cat >/dev/null; printf '{}'" {
            assert!(matches!(error, DecodeFault::InvalidOutput));
        } else {
            assert!(
                matches!(error, DecodeFault::Invoke(ref e) if matches!(e.cleanup, Cleanup::Confirmed) && matches!(e.kind, FailureKind::Deadline|FailureKind::OutputLimit(_)))
            );
        }
        match fixture.recorded_pid() {
            Some(pid) => gone(pid),
            None => assert!(
                matches!(error, DecodeFault::Invoke(ref e) if e.kind == FailureKind::Deadline),
                "no pid without a deadline"
            ),
        }
        fixture.actual();
        let produced = decoder
            .produce_baseline(view(b"reusable", "title"), None)
            .unwrap();
        gone(produced.child_pid);
    }
    let stop = AtomicBool::new(true);
    assert!(
        matches!(decoder.produce_baseline(view(b"", ""), Some(&stop)), Err(DecodeFault::Invoke(e)) if e.kind == FailureKind::Interrupted)
    );
}

#[test]
fn baseline_private_child_rejects_digest_and_strict_wire_mutations() {
    use sha2::{Digest, Sha256};
    let valid = serde_json::json!({"version":1,"source":URL_SAFE_NO_PAD.encode(b"exact"),
        "title":"title", "source_digest":URL_SAFE_NO_PAD.encode(Sha256::digest(b"exact")),
        "action":{"mode":"produce"}});
    let run = |input: &[u8]| {
        let program = program();
        let args = ["__decoder".into(), "baseline".into()];
        tmt_invoke::invoke(
            Request {
                program: &program,
                args: &args,
                input,
                deadline: Instant::now() + Duration::from_secs(2),
                max_stream_bytes: STREAM_BYTES,
                launch: LaunchOptions {
                    environment: EnvironmentPolicy::ClearAllowlist(&[]),
                    ..Default::default()
                },
            },
            None,
        )
        .unwrap()
    };
    let control = run(&serde_json::to_vec(&valid).unwrap());
    assert!(control.status.success());
    let reply: serde_json::Value = serde_json::from_slice(&control.stdout).unwrap();
    gone(reply["pid"].as_u64().unwrap() as u32);
    for (field, value) in [
        (
            "source_digest",
            serde_json::json!(URL_SAFE_NO_PAD.encode([0; 32])),
        ),
        ("version", serde_json::json!(2)),
        ("extra", serde_json::json!(true)),
        ("source", serde_json::json!("ZXhhY3Q=")),
        ("action", serde_json::json!({"mode":"produce","extra":true})),
    ] {
        let mut changed = valid.clone();
        changed[field] = value;
        let rejected = run(&serde_json::to_vec(&changed).unwrap());
        assert!(!rejected.status.success(), "accepted mutation {field}");
        assert!(rejected.stdout.is_empty(), "rejected input returned bytes");
    }
    let mut raw = serde_json::to_string(&valid).unwrap();
    raw.insert_str(1, "\"version\":1,");
    let rejected = run(raw.as_bytes());
    assert!(!rejected.status.success());
    assert!(rejected.stdout.is_empty());
}

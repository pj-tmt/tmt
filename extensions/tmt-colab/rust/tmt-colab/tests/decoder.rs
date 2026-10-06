mod support;
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
use tmt_test_support::write_executable;
use yrs::{
    Array, Doc, GetString, Map, ReadTxn, StateVector, Text, Transact, Update,
    updates::decoder::Decode,
};
fn program() -> PathBuf {
    env!("CARGO_BIN_EXE_tmt-colab").into()
}
fn owner() -> Decoder {
    Decoder::with_config(support::decoder_config(program())).unwrap()
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
        decoder.set_deadline(tmt_colab::decoder::DEADLINE).unwrap();
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
        decoder.set_deadline(support::DECODER_DEADLINE).unwrap();
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
    barrier: Option<(std::fs::File, std::fs::File)>,
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
            format!("#!/bin/sh\nprintf '%s\\n' \"$$\" > '{pid_path}'\n{body}\n").as_bytes(),
            0o700,
        )
        .unwrap();
        Self {
            directory,
            script,
            barrier: None,
        }
    }
    fn blocked() -> Self {
        use nix::{sys::stat::Mode, unistd::mkfifo};
        let mut fixture = Self::new("");
        let ready = fixture.directory.join("ready");
        let release = fixture.directory.join("release");
        for path in [&ready, &release] {
            mkfifo(path, Mode::S_IRUSR | Mode::S_IWUSR).unwrap();
        }
        let open = |path| {
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
                .unwrap()
        };
        fixture.barrier = Some((open(&ready), open(&release)));
        let directory = fixture.directory.to_string_lossy().replace('\'', "'\\''");
        let actual = program().to_string_lossy().replace('\'', "'\\''");
        write_executable(
            &fixture.script,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$$\" > '{directory}/pid'\nexec 3< '{directory}/release'\nprintf x > '{directory}/ready'\nread line <&3\nexec 3<&-\nexec '{actual}' \"$@\"\n"
            ).as_bytes(),
            0o700,
        )
        .unwrap();
        fixture
    }
    fn wait_ready(&self) {
        use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
        use std::{io::Read, os::fd::AsFd};
        let (ready, _) = self.barrier.as_ref().unwrap();
        let mut events = [PollFd::new(ready.as_fd(), PollFlags::POLLIN)];
        assert_eq!(
            poll(&mut events, PollTimeout::try_from(30_000).unwrap()).unwrap(),
            1,
            "blocked child never reached its FIFO"
        );
        let mut byte = [0];
        (&*ready).read_exact(&mut byte).unwrap();
        assert_eq!(byte, [b'x']);
    }
    fn release(&self) {
        use std::io::Write;
        let (_, release) = self.barrier.as_ref().unwrap();
        (&*release).write_all(b"continue\n").unwrap();
    }
    fn assert_blocked_child_was_reaped(&self) {
        self.wait_ready();
        gone(self.pid());
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
        write_executable(
            &self.script,
            format!("#!/bin/sh\nexec '{path}' \"$@\"\n").as_bytes(),
            0o700,
        )
        .unwrap();
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
        Decoder::with_config(support::decoder_config(fixture.script.clone()))
            .unwrap()
            .decode(
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
fn fifo_blocked_child_completes_when_explicitly_released() {
    let fixture = FixtureProgram::blocked();
    let mut decoder =
        Decoder::with_config(support::decoder_config(fixture.script.clone())).unwrap();
    std::thread::scope(|scope| {
        let work = scope.spawn(|| {
            decoder.decode(
                UpdateBatch {
                    namespace: Namespace::Content,
                    baseline: &[],
                    updates: &[],
                },
                Role::Editor,
                None,
            )
        });
        fixture.wait_ready();
        fixture.release();
        let reply = work.join().unwrap().unwrap();
        assert_eq!(reply.projection["html"], "");
        assert_eq!(reply.child_pid, fixture.pid());
        gone(reply.child_pid);
    });
}

#[test]
fn deadline_confirms_cleanup_before_owner_reuse() {
    let fixture = FixtureProgram::blocked();
    let mut decoder = Decoder::new(fixture.script.clone()).unwrap();
    // The FIFO cannot complete on its own. Keep the request minimal so this
    // measures the deadline rather than the time to serialize a large input.
    let baseline = [];
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
    assert!(matches!(error, DecodeFault::Invoke(ref e)
        if e.kind == FailureKind::Deadline && matches!(e.cleanup, Cleanup::Confirmed)));
    fixture.assert_blocked_child_was_reaped();
    fixture.actual();
    decoder.set_deadline(support::DECODER_DEADLINE).unwrap();
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

#[test]
fn interrupted_input_backpressure_confirms_cleanup_before_owner_reuse() {
    use std::sync::atomic::Ordering;
    let fixture = FixtureProgram::blocked();
    let mut decoder =
        Decoder::with_config(support::decoder_config(fixture.script.clone())).unwrap();
    let stop = AtomicBool::new(false);
    // Larger than a pipe buffer; the child signals readiness but never reads stdin.
    let baseline = vec![0; tmt_colab::decoder::BASELINE_BYTES];
    std::thread::scope(|scope| {
        let work = scope.spawn(|| {
            decoder.decode(
                UpdateBatch {
                    namespace: Namespace::Content,
                    baseline: &baseline,
                    updates: &[],
                },
                Role::Editor,
                Some(&stop),
            )
        });
        fixture.wait_ready();
        stop.store(true, Ordering::Relaxed);
        let error = work.join().unwrap().err().unwrap();
        assert!(matches!(error, DecodeFault::Invoke(ref e)
            if e.kind == FailureKind::Interrupted && matches!(e.cleanup, Cleanup::Confirmed)));
        gone(fixture.pid());
    });
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

#[test]
fn output_backpressure_confirms_exact_limit_and_cleanup_before_owner_reuse() {
    let fixture = FixtureProgram::new(&format!(
        "exec /usr/bin/head -c {} /dev/zero",
        tmt_colab::decoder::STREAM_BYTES + 1
    ));
    let mut decoder =
        Decoder::with_config(support::decoder_config(fixture.script.clone())).unwrap();
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
    assert!(matches!(error, DecodeFault::Invoke(ref e)
        if e.kind == FailureKind::OutputLimit(tmt_invoke::Stream::Stdout)
            && matches!(e.cleanup, Cleanup::Confirmed)));
    gone(fixture.pid());
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
        creation_recipient: None,
        source,
        title,
        publisher_agent: None,
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
        let recipient = vector.get("creationRecipient").map(|v| {
            serde_json::from_value::<tmt_colab::decoder::CreationRecipient>(v.clone()).unwrap()
        });
        let mut decoder = owner();
        let verified = decoder
            .verify_baseline(view(source.as_bytes(), title), &update, commitment, None)
            .unwrap();
        assert_eq!(verified.update, update);
        gone(verified.child_pid);
        if let Some(alternate) = vector["alternateUpdate"].as_str() {
            let mismatched = URL_SAFE_NO_PAD
                .decode(vector["alternateCommitment"].as_str().unwrap())
                .unwrap()
                .try_into()
                .unwrap();
            assert!(matches!(
                decoder.verify_baseline(view(source.as_bytes(), title), &update, mismatched, None),
                Err(DecodeFault::Rejected)
            ));
            let alternate = URL_SAFE_NO_PAD.decode(alternate).unwrap();
            let commitment = URL_SAFE_NO_PAD
                .decode(vector["alternateCommitment"].as_str().unwrap())
                .unwrap()
                .try_into()
                .unwrap();
            let verified = decoder
                .verify_baseline(view(source.as_bytes(), title), &alternate, commitment, None)
                .unwrap();
            assert_eq!(verified.update, alternate);
            gone(verified.child_pid);
        }
        let produced = decoder
            .produce_baseline(
                BaselineInput {
                    creation_recipient: recipient.as_ref(),
                    publisher_agent: vector["publisherAgent"].as_str(),
                    ..view(source.as_bytes(), title)
                },
                None,
            )
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
            assert_eq!(
                meta.get(&txn, "publisherAgent")
                    .map(|value| value.to_string(&txn)),
                vector["publisherAgent"].as_str().map(str::to_owned)
            );
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
        creation_recipient: None,
        source: b"exact",
        title: "title",
        source_digest: [0; 32],
        publisher_agent: None,
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
        &format!(
            "exec /usr/bin/head -c {} /dev/zero",
            tmt_colab::decoder::STREAM_BYTES + 1
        ),
        "cat >/dev/null; printf '{}'",
    ] {
        let fixture = FixtureProgram::new(body);
        let mut decoder =
            Decoder::with_config(support::decoder_config(fixture.script.clone())).unwrap();
        let error = decoder
            .produce_baseline(view(&source, "title"), None)
            .err()
            .unwrap();
        if body == "cat >/dev/null; printf '{}'" {
            assert!(matches!(error, DecodeFault::InvalidOutput));
        } else {
            assert!(matches!(error, DecodeFault::Invoke(ref e)
                if e.kind == FailureKind::OutputLimit(tmt_invoke::Stream::Stdout)
                    && matches!(e.cleanup, Cleanup::Confirmed)));
        }
        gone(fixture.pid());
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
fn baseline_deadline_confirms_cleanup_before_owner_reuse() {
    let fixture = FixtureProgram::blocked();
    let mut decoder = Decoder::new(fixture.script.clone()).unwrap();
    let error = decoder.produce_baseline(view(b"", ""), None).err().unwrap();
    assert!(matches!(error, DecodeFault::Invoke(ref e)
        if e.kind == FailureKind::Deadline && matches!(e.cleanup, Cleanup::Confirmed)));
    fixture.assert_blocked_child_was_reaped();
    fixture.actual();
    decoder.set_deadline(support::DECODER_DEADLINE).unwrap();
    let produced = decoder
        .produce_baseline(view(b"reusable", "title"), None)
        .unwrap();
    gone(produced.child_pid);
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
                deadline: Instant::now() + support::DECODER_DEADLINE,
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

#[test]
fn publisher_metadata_updates_and_unknown_cli_edits_clear_it() {
    use tmt_colab::decoder::ContentEdit;
    let mut decoder = owner();
    let baseline = decoder
        .produce_baseline(
            BaselineInput {
                creation_recipient: None,
                publisher_agent: Some("publisher"),
                ..view(b"before", "Title")
            },
            None,
        )
        .unwrap();
    let mut update = baseline.update;
    for (source, publisher) in [("after", Some("next-agent")), ("after", None)] {
        let edited = decoder
            .prepare(
                UpdateBatch {
                    namespace: Namespace::Content,
                    baseline: &update,
                    updates: &[],
                },
                ContentEdit {
                    source,
                    publisher_agent: publisher,
                },
                None,
            )
            .unwrap();
        let folded = decoder
            .decode(
                UpdateBatch {
                    namespace: Namespace::Content,
                    baseline: &update,
                    updates: &[&edited.merged],
                },
                Role::Editor,
                None,
            )
            .unwrap();
        assert_eq!(folded.projection["html"], source);
        assert_eq!(
            folded.projection["meta"]["publisherAgent"].as_str(),
            publisher
        );
        let doc = Doc::new();
        apply_baseline(&doc, &update);
        apply_baseline(&doc, &edited.merged);
        update = doc
            .transact()
            .encode_state_as_update_v1(&yrs::StateVector::default());
    }
    for invalid in ["".to_owned(), "x".repeat(129), "line\nbreak".to_owned()] {
        assert!(
            decoder
                .prepare(
                    UpdateBatch {
                        namespace: Namespace::Content,
                        baseline: &update,
                        updates: &[]
                    },
                    ContentEdit {
                        source: "after",
                        publisher_agent: Some(&invalid)
                    },
                    None
                )
                .is_err()
        );
    }
}

#[test]
fn a_page_source_larger_than_one_update_is_produced_as_ordered_updates_that_rebuild_it() {
    let mut decoder = Decoder::with_config(support::decoder_config(program())).unwrap();
    // Multi-byte text across a chunk boundary: chunks must never split a character.
    let source = "héllo wörld 🌍 <p>text</p>\n".repeat(60_000);
    assert!(source.len() > 1_500_000);
    let made = decoder
        .produce_page(view(source.as_bytes(), "T"), None)
        .unwrap();
    assert!(made.chunks.len() > 1);
    let doc = Doc::new();
    doc.get_or_insert_text("html");
    doc.get_or_insert_map("meta");
    for chunk in &made.chunks {
        assert!(
            chunk.len() <= tmt_colab::decoder::UPDATE_BYTES,
            "{} bytes",
            chunk.len()
        );
        doc.transact_mut()
            .apply_update(Update::decode_v1(chunk).unwrap())
            .unwrap();
    }
    let rebuilt = doc.get_or_insert_text("html").get_string(&doc.transact());
    assert!(rebuilt == source, "the chunks rebuild a different source");
    // The merged update is the same page in one piece, for the baseline commitment.
    let merged = Doc::new();
    apply_baseline(&merged, &made.update);
    assert!(
        merged
            .get_or_insert_text("html")
            .get_string(&merged.transact())
            == source
    );
    gone(made.child_pid);
}

#[test]
fn merging_one_devices_updates_keeps_structs_that_depend_on_another_device() {
    let mut decoder = Decoder::with_config(support::decoder_config(program())).unwrap();
    // Device one writes the text; device two edits after it, so its updates alone are not a page.
    let one = Doc::with_client_id(1);
    one.get_or_insert_map("meta");
    one.get_or_insert_text("html")
        .insert(&mut one.transact_mut(), 0, "base");
    let base = one
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    let two = Doc::with_client_id(2);
    two.get_or_insert_map("meta");
    two.get_or_insert_text("html");
    two.transact_mut()
        .apply_update(Update::decode_v1(&base).unwrap())
        .unwrap();
    let mut mine = Vec::new();
    for piece in [" edit", " more"] {
        let mut txn = two.transact_mut();
        let html = txn.get_text("html").unwrap();
        let end = html.len(&txn);
        html.insert(&mut txn, end, piece);
        mine.push(txn.encode_update_v1());
    }
    let refs = mine.iter().map(Vec::as_slice).collect::<Vec<_>>();
    // Read as a page on its own, it is incomplete and rejected...
    assert!(
        decoder
            .decode(
                UpdateBatch {
                    namespace: Namespace::Content,
                    baseline: &[],
                    updates: &refs,
                },
                Role::Editor,
                None,
            )
            .is_err()
    );
    // ...but merging keeps every struct, and with the other device's update it is the full page.
    let merged = decoder.merge(Namespace::Content, &refs, None).unwrap();
    let page = Doc::new();
    page.get_or_insert_map("meta");
    page.get_or_insert_text("html");
    for update in [base.as_slice(), merged.as_slice()] {
        page.transact_mut()
            .apply_update(Update::decode_v1(update).unwrap())
            .unwrap();
    }
    assert_eq!(
        page.get_or_insert_text("html").get_string(&page.transact()),
        "base edit more"
    );
    // A malformed update is refused, not merged.
    assert!(
        decoder
            .merge(Namespace::Content, &[b"not an update"], None)
            .is_err()
    );
}

#[test]
fn creation_recipient_survives_chunked_baselines_and_known_or_unknown_source_edits() {
    use tmt_colab::decoder::{ContentEdit, CreationRecipient};
    let recipient = CreationRecipient {
        machine_id: "40000000-0000-4000-8000-000000000001".into(),
        agent_id: "50000000-0000-1000-8000-000000000001".into(),
    };
    let source = "<p>Unicode 🐈</p>".repeat(20_000);
    let mut decoder = owner();
    for hint in [Some(&recipient), None] {
        let baseline = decoder
            .produce_page(
                BaselineInput {
                    creation_recipient: hint,
                    ..view(source.as_bytes(), "Title")
                },
                None,
            )
            .unwrap();
        gone(baseline.child_pid);
        assert!(baseline.update.len() > 256 * 1024);
        assert!(!baseline.chunks.is_empty());
        let doc = Doc::new();
        for chunk in &baseline.chunks {
            apply_baseline(&doc, chunk);
        }
        let merged = doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default());
        let expected = hint.map(|v| serde_json::to_value(v).unwrap());
        let folded = decoder
            .decode(
                UpdateBatch {
                    namespace: Namespace::Content,
                    baseline: &merged,
                    updates: &[],
                },
                Role::Editor,
                None,
            )
            .unwrap();
        assert_eq!(
            folded.projection["meta"].get("creationRecipient"),
            expected.as_ref()
        );
        gone(folded.child_pid);
        for publisher in [Some("later-agent"), None] {
            let prepared = decoder
                .prepare(
                    UpdateBatch {
                        namespace: Namespace::Content,
                        baseline: &merged,
                        updates: &[],
                    },
                    ContentEdit {
                        source: "later",
                        publisher_agent: publisher,
                    },
                    None,
                )
                .unwrap();
            let folded = decoder
                .decode(
                    UpdateBatch {
                        namespace: Namespace::Content,
                        baseline: &merged,
                        updates: &[&prepared.merged],
                    },
                    Role::Editor,
                    None,
                )
                .unwrap();
            assert_eq!(
                folded.projection["meta"].get("creationRecipient"),
                expected.as_ref()
            );
            gone(prepared.child_pid);
            gone(folded.child_pid);
        }
    }
}

#[test]
fn creation_metadata_rejects_partial_unknown_nested_or_noncanonical_values() {
    use std::collections::HashMap;
    let pair = || {
        HashMap::from([
            (
                "machineId".into(),
                yrs::Any::String("40000000-0000-4000-8000-000000000001".into()),
            ),
            (
                "agentId".into(),
                yrs::Any::String("50000000-0000-1000-8000-000000000001".into()),
            ),
        ])
    };
    let mut partial = pair();
    partial.remove("agentId");
    let mut extra = pair();
    extra.insert("extra".into(), yrs::Any::Bool(true));
    let mut nested = pair();
    nested.insert("agentId".into(), yrs::Any::Map(pair().into()));
    let mut invalid = pair();
    invalid.insert(
        "machineId".into(),
        yrs::Any::String("40000000-0000-1000-8000-000000000001".into()),
    );
    for value in [
        yrs::Any::Null,
        yrs::Any::Map(partial.into()),
        yrs::Any::Map(extra.into()),
        yrs::Any::Map(nested.into()),
        yrs::Any::Map(invalid.into()),
    ] {
        let doc = Doc::new();
        doc.get_or_insert_text("html");
        let meta = doc.get_or_insert_map("meta");
        meta.insert(&mut doc.transact_mut(), "title", "Title");
        meta.insert(&mut doc.transact_mut(), "creationRecipient", value);
        let update = doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default());
        assert!(matches!(
            owner().decode(
                UpdateBatch {
                    namespace: Namespace::Content,
                    baseline: &update,
                    updates: &[]
                },
                Role::Editor,
                None
            ),
            Err(DecodeFault::Rejected)
        ));
    }
}

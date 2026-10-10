//! `tmt colab deploy-declaration --json` (#2397): the installed binary's compiled-in Firestore
//! declaration against the checked-in vector, the contract's caps and the declaration grammar.
//! Colab cannot depend on Remote, so the grammar below is re-implemented from
//! `remote-channel-v1.md` ("extension backend declarations"); Remote's `deploy_vectors` mirror
//! parses the same vector with Remote's own owners, so a drift in either grammar fails a build.
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

const BINARY: &str = env!("CARGO_BIN_EXE_tmt-colab");

fn vectors() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../contracts/vectors")
}
fn sources() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../firestore")
}
fn sha256(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// A scratch directory that must still be empty after the command ran in it.
struct Scratch(PathBuf);
impl Scratch {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("tmt-2397-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Runs the binary with a cleared environment in an empty directory: no HOME, no data root,
/// no TMT variables, nothing the command could read or write.
fn run(args: &[&str], scratch: &Scratch) -> Output {
    Command::new(BINARY)
        .args(args)
        .env_clear()
        .current_dir(&scratch.0)
        .output()
        .unwrap()
}

fn vector_text() -> String {
    fs::read_to_string(vectors().join("deploy-declaration-v1.json")).unwrap()
}
fn envelope() -> Value {
    serde_json::from_str(&vector_text()).unwrap()
}
fn declaration() -> Value {
    serde_json::from_str(envelope()["declaration"].as_str().unwrap()).unwrap()
}

#[test]
fn the_command_prints_the_vector_byte_for_byte_and_touches_nothing() {
    let scratch = Scratch::new("run");
    let output = run(&["deploy-declaration", "--json"], &scratch);
    assert!(output.status.success());
    assert!(output.stderr.is_empty(), "Remote reads nothing from stderr");
    assert_eq!(String::from_utf8(output.stdout).unwrap(), vector_text());
    assert_eq!(
        fs::read_dir(&scratch.0).unwrap().count(),
        0,
        "no state, files or sockets"
    );
}

#[test]
fn the_reply_has_exactly_the_six_fields_in_order_and_independent_digests() {
    let text = vector_text();
    let positions: Vec<usize> = [
        "version",
        "extension",
        "backend",
        "declaration",
        "declarationDigest",
        "artifact",
    ]
    .iter()
    .map(|field| text.find(&format!("\"{field}\":")).unwrap())
    .collect();
    assert!(
        positions.windows(2).all(|pair| pair[0] < pair[1]),
        "field order"
    );
    let reply = envelope();
    assert_eq!(reply.as_object().unwrap().len(), 6);
    assert_eq!(reply["version"], 1);
    assert_eq!(reply["extension"], "colab");
    assert_eq!(reply["backend"], "firestore");
    // Independent of the command: the embedded source files themselves.
    let declaration_source = fs::read_to_string(sources().join("declaration.json")).unwrap();
    let artifact_source = fs::read_to_string(sources().join("admission.rules")).unwrap();
    assert_eq!(reply["declaration"], declaration_source);
    assert_eq!(reply["artifact"], artifact_source);
    assert_eq!(reply["declarationDigest"], sha256(&declaration_source));
    assert_eq!(
        declaration()["admission"]["digest"],
        sha256(&artifact_source)
    );
}

#[test]
fn the_caps_hold_with_room_for_worst_case_escaping() {
    let text = vector_text();
    let reply = envelope();
    let declaration_text = reply["declaration"].as_str().unwrap();
    let artifact = reply["artifact"].as_str().unwrap();
    assert!(declaration_text.len() <= 64 * 1024 && artifact.len() <= 64 * 1024);
    assert!(text.len() <= 12 * 64 * 1024 + 4096);
    assert!(declaration()["resources"].as_array().unwrap().len() <= 64);
    // Remote's tokenizer bound (16,384 tokens) is checked by its mirror test on this vector.
}

#[test]
fn a_real_failure_is_never_the_no_declaration_code_and_a_valid_call_cannot_reach_input_invalid() {
    let scratch = Scratch::new("usage");
    // The only path to COLAB_INPUT_INVALID is a malformed invocation, which Remote never makes.
    let bare = run(&["deploy-declaration"], &scratch);
    assert!(!bare.status.success());
    let valid = run(&["deploy-declaration", "--json"], &scratch);
    assert!(
        !String::from_utf8(valid.stdout)
            .unwrap()
            .contains("COLAB_INPUT_INVALID")
    );
    let extra = run(&["deploy-declaration", "--json", "extra"], &scratch);
    assert!(!extra.status.success());
    assert!(
        String::from_utf8(extra.stdout)
            .unwrap()
            .contains("COLAB_INPUT_INVALID")
    );
}

#[test]
fn the_command_is_hidden_from_help() {
    let scratch = Scratch::new("help");
    let output = run(&["--help"], &scratch);
    assert!(
        !String::from_utf8(output.stdout)
            .unwrap()
            .contains("deploy-declaration")
    );
}

fn lowercase_identifier(value: &str, extra: &[char]) -> bool {
    (1..=64).contains(&value.len())
        && value
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || extra.contains(&c))
}

/// The declaration grammar of `remote-channel-v1.md`, restated for Firestore.
#[test]
fn the_declaration_follows_the_contract_grammar() {
    let declaration = declaration();
    let top = declaration.as_object().unwrap();
    let keys: BTreeSet<&str> = top.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        BTreeSet::from(["version", "extension", "backend", "resources", "admission"])
    );
    assert_eq!(
        (
            &declaration["version"],
            &declaration["extension"],
            &declaration["backend"]
        ),
        (
            &Value::from(1),
            &Value::from("colab"),
            &Value::from("firestore")
        )
    );

    let mut names = BTreeSet::new();
    let mut paths: Vec<Vec<&str>> = Vec::new();
    for resource in declaration["resources"].as_array().unwrap() {
        let resource = resource.as_object().unwrap();
        let keys: BTreeSet<&str> = resource.keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            BTreeSet::from(["name", "kind", "path", "limits", "ttlField", "indexes"])
        );
        let name = resource["name"].as_str().unwrap();
        assert!(
            lowercase_identifier(name, &['-']) && names.insert(name),
            "{name}"
        );
        assert!(
            ["log", "checkpoint", "blob", "awareness"]
                .contains(&resource["kind"].as_str().unwrap())
        );
        assert_ne!(
            resource["kind"], "blob",
            "attachments over Firebase are out of the first declaration (#2165)"
        );

        let path = resource["path"].as_str().unwrap();
        let segments: Vec<&str> = path.split('/').collect();
        assert!(
            segments.len() <= 8
                && segments
                    .iter()
                    .all(|s| lowercase_identifier(s, &['-', '_'])),
            "{path}"
        );
        assert!(
            !["append", "subscribe", "ack", "pair", "x"].contains(&segments[0]),
            "reserved route {path}"
        );
        assert!(
            segments.len() % 2 == 1 && segments.len() <= 7,
            "a Firestore path names a collection: {path}"
        );
        paths.push(segments);

        let limits = resource["limits"].as_object().unwrap();
        assert_eq!(limits.len(), 3);
        let bound = |key: &str| limits[key].as_u64().unwrap();
        assert!(bound("maxObjectBytes") > 0 && bound("maxEntries") > 0);
        assert!(
            bound("maxObjectBytes") <= bound("maxNamespaceBytes")
                && bound("maxNamespaceBytes") < 1 << 53
        );
        // Remote's current backend ceilings (limits.rs): the plan refuses anything above them.
        assert!(
            bound("maxObjectBytes") <= 12 << 20
                && bound("maxNamespaceBytes") <= 64 << 20
                && bound("maxEntries") <= 1024
        );

        if let Some(field) = resource["ttlField"].as_str() {
            assert!(
                field.starts_with(|c: char| c.is_ascii_alphabetic())
                    && field.chars().all(|c| c.is_ascii_alphanumeric())
                    && field.len() <= 64
            );
        } else {
            assert!(resource["ttlField"].is_null());
        }
        let indexes = resource["indexes"].as_array().unwrap();
        assert!(indexes.len() <= 64);
        let mut distinct = BTreeSet::new();
        for index in indexes {
            assert!(["asc", "desc"].contains(&index["direction"].as_str().unwrap()));
            assert!(distinct.insert((
                index["field"].as_str().unwrap(),
                index["direction"].as_str().unwrap()
            )));
        }
    }
    assert!(paths.len() <= 64);
    for (i, a) in paths.iter().enumerate() {
        for b in &paths[i + 1..] {
            let shorter = a.len().min(b.len());
            assert_ne!(
                a[..shorter],
                b[..shorter],
                "equal or nested paths: {a:?} {b:?}"
            );
        }
    }

    let admission = declaration["admission"].as_object().unwrap();
    let keys: BTreeSet<&str> = admission.keys().map(String::as_str).collect();
    assert_eq!(keys, BTreeSet::from(["artifact", "digest", "entryPoint"]));
    let artifact = admission["artifact"].as_str().unwrap();
    assert!(artifact.split('/').all(|s| {
        s.starts_with(|c: char| c.is_ascii_alphanumeric())
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
    }));
    let digest = admission["digest"].as_str().unwrap();
    assert!(
        digest.len() == 64
            && digest
                .bytes()
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    );
    let entry = admission["entryPoint"].as_str().unwrap();
    assert!(
        entry.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
            && entry.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    );
}

/// Remote's composer does not cross-check declared collections against the fragment, so Colab
/// does: every top-level `match` and every `ext.*` read names a declared collection, and the
/// declaration declares nothing the Rules never mention.
#[test]
fn the_rules_touch_exactly_the_declared_collections() {
    let artifact = fs::read_to_string(sources().join("admission.rules")).unwrap();
    let declared: BTreeSet<String> = declaration()["resources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|resource| resource["path"].as_str().unwrap().to_owned())
        .collect();
    let matched: BTreeSet<String> = artifact
        .lines()
        .filter_map(|line| line.strip_prefix("match /"))
        .filter_map(|rest| rest.split('/').next().map(str::to_owned))
        .collect();
    assert_eq!(declared, matched);
    let mut reads = BTreeSet::new();
    for call in [
        "ext.get(/",
        "ext.exists(/",
        "ext.getAfter(/",
        "ext.existsAfter(/",
    ] {
        for part in artifact.split(call).skip(1) {
            reads.insert(part.split('/').next().unwrap().to_owned());
        }
    }
    assert!(
        reads.is_subset(&declared),
        "undeclared reads: {:?}",
        reads.difference(&declared).collect::<Vec<_>>()
    );
}

#[test]
fn the_vector_ships_its_complete_composed_golden_trio() {
    for name in ["firestore.rules", "firestore.indexes.json", "plan.json"] {
        let path: &Path = &vectors().join(format!("deploy-declaration-v1.{name}"));
        assert!(path.is_file(), "{}", path.display());
    }
}

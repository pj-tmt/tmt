//! Backend declarations and the deploy plan (#2161). Fixtures are real files; the golden
//! plan bytes and digests come from `fixtures/declarations/plan-reference.py`, not from
//! the code under test. Each refusal changes one condition of a valid control.
//! The Colab-named fixtures are illustrative stand-ins, not Colab's declaration: their
//! limits and paths are not Colab's numbers.
use serde_json::{Value, json};
use std::{fs, path::PathBuf};
use tmt_remote::{
    declaration::{self, Reason},
    deploy_plan::{CloudBackend, Enabled, Plan, PlanError, PlanReason, Supplied, Target, compose},
    limits, routes,
};

fn path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/declarations")
        .join(name)
}
fn fixture(name: &str) -> Vec<u8> {
    fs::read(path(name)).unwrap_or_else(|error| panic!("{name}: {error}"))
}
fn json_fixture(name: &str) -> Value {
    serde_json::from_slice(&fixture(name)).unwrap()
}
fn bytes(value: &Value) -> Vec<u8> {
    serde_json::to_vec(value).unwrap()
}
fn edit(name: &str, change: impl FnOnce(&mut Value)) -> Vec<u8> {
    let mut value = json_fixture(name);
    change(&mut value);
    bytes(&value)
}
const FIRESTORE: Target = Target {
    backend: CloudBackend::Firestore,
    physical_ttl: false,
};
fn colab_artifact() -> Vec<u8> {
    fixture("rules/colab-admission.rules")
}
fn notes_artifact() -> Vec<u8> {
    fixture("rules/notes-admission.rules")
}
fn colab<'a>(declaration: &'a [u8], artifact: &'a [u8]) -> Enabled<'a> {
    Enabled {
        name: "colab",
        supplied: Some(Supplied {
            declaration,
            artifact,
        }),
    }
}
fn notes<'a>(declaration: &'a [u8], artifact: &'a [u8]) -> Enabled<'a> {
    Enabled {
        name: "notes",
        supplied: Some(Supplied {
            declaration,
            artifact,
        }),
    }
}
fn refused(target: Target, enabled: &[Enabled<'_>]) -> PlanError {
    compose(target, enabled).expect_err("the plan must be refused")
}
fn golden(name: &str) -> (Vec<u8>, String) {
    (
        fixture(&format!("plan-firestore-{name}.json")),
        String::from_utf8(fixture(&format!("plan-firestore-{name}.sha256"))).unwrap(),
    )
}
fn both(target: Target) -> Plan {
    let (colab_bytes, notes_bytes) = (
        fixture("colab-firestore.json"),
        fixture("notes-firestore.json"),
    );
    let (colab_artifact, notes_artifact) = (colab_artifact(), notes_artifact());
    compose(
        target,
        &[
            colab(&colab_bytes, &colab_artifact),
            notes(&notes_bytes, &notes_artifact),
        ],
    )
    .unwrap()
}

#[test]
fn colab_fixture_composes_to_the_independent_golden_plan() {
    for (name, physical_ttl) in [("no-ttl", false), ("ttl", true)] {
        let plan = both(Target {
            backend: CloudBackend::Firestore,
            physical_ttl,
        });
        let (expected_bytes, expected_digest) = golden(name);
        assert_eq!(
            String::from_utf8_lossy(plan.bytes()),
            String::from_utf8_lossy(&expected_bytes),
            "{name}"
        );
        assert_eq!(plan.digest(), expected_digest, "{name}");
    }
}

#[test]
fn a_target_without_physical_ttl_records_a_no_op_and_keeps_the_plan() {
    let plan = both(FIRESTORE);
    let log = &plan.view().extensions[0].resources[2];
    assert_eq!(
        (log.name.as_str(), log.ttl_field.as_deref()),
        ("page-log", Some("expiresAt"))
    );
    assert_eq!(log.ttl, "not-provisioned");
    // The same declaration with physical TTL differs only in what is provisioned.
    assert_eq!(
        both(Target {
            physical_ttl: true,
            ..FIRESTORE
        })
        .view()
        .extensions[0]
            .resources[2]
            .ttl,
        "provisioned"
    );
    // Resources are listed by name: page-blob first, without a ttlField.
    assert_eq!(plan.view().extensions[0].resources[0].ttl, "none");
}

#[test]
fn cloudflare_is_a_target_value_only() {
    let declaration = edit("colab-firestore.json", |v| {
        v["backend"] = json!("cloudflare")
    });
    let artifact = colab_artifact();
    let target = Target {
        backend: CloudBackend::Cloudflare,
        physical_ttl: false,
    };
    let plan = compose(target, &[colab(&declaration, &artifact)]).unwrap();
    assert_eq!(plan.view().backend, "cloudflare");
    // A Firestore declaration does not compose for Cloudflare, nor the reverse.
    let firestore = fixture("colab-firestore.json");
    assert_eq!(
        refused(target, &[colab(&firestore, &artifact)]).reason,
        PlanReason::BackendMismatch
    );
    assert_eq!(
        refused(FIRESTORE, &[colab(&declaration, &artifact)]).reason,
        PlanReason::BackendMismatch
    );
}

#[test]
fn composition_is_deterministic_and_digest_addressed() {
    let (colab_bytes, notes_bytes) = (
        fixture("colab-firestore.json"),
        fixture("notes-firestore.json"),
    );
    let (colab_artifact, notes_artifact) = (colab_artifact(), notes_artifact());
    let forward = compose(
        FIRESTORE,
        &[
            colab(&colab_bytes, &colab_artifact),
            notes(&notes_bytes, &notes_artifact),
        ],
    )
    .unwrap();
    let reversed = compose(
        FIRESTORE,
        &[
            notes(&notes_bytes, &notes_artifact),
            colab(&colab_bytes, &colab_artifact),
        ],
    )
    .unwrap();
    assert_eq!(forward.bytes(), reversed.bytes());
    assert_eq!(forward.digest(), reversed.digest());
    // Resource and index order in the input does not change the plan view, only the
    // digest of the declaration bytes the owner approved.
    let shuffled = edit("colab-firestore.json", |v| {
        v["resources"].as_array_mut().unwrap().reverse();
        v["resources"][3]["indexes"]
            .as_array_mut()
            .unwrap()
            .reverse();
    });
    let other = compose(
        FIRESTORE,
        &[
            colab(&shuffled, &colab_artifact),
            notes(&notes_bytes, &notes_artifact),
        ],
    )
    .unwrap();
    assert_eq!(other.view().extensions[0].resources.len(), 4);
    assert_ne!(other.digest(), forward.digest());
    let mut view = serde_json::to_value(other.view()).unwrap();
    view["extensions"][0]["declarationDigest"] =
        serde_json::to_value(&forward.view().extensions[0].declaration_digest).unwrap();
    assert_eq!(view, serde_json::to_value(forward.view()).unwrap());
    // One changed byte of a declaration, or of the target, is a different digest.
    let spaced = [colab_bytes.as_slice(), b" "].concat();
    let changed = compose(
        FIRESTORE,
        &[
            colab(&spaced, &colab_artifact),
            notes(&notes_bytes, &notes_artifact),
        ],
    )
    .unwrap();
    assert_ne!(changed.digest(), forward.digest());
    assert_ne!(
        both(Target {
            physical_ttl: true,
            ..FIRESTORE
        })
        .digest(),
        forward.digest()
    );
}

#[test]
fn an_extension_without_a_declaration_is_unavailable_and_the_rest_composes() {
    let (colab_bytes, artifact) = (fixture("colab-firestore.json"), colab_artifact());
    let plan = compose(
        FIRESTORE,
        &[
            Enabled {
                name: "notes",
                supplied: None,
            },
            colab(&colab_bytes, &artifact),
        ],
    )
    .unwrap();
    assert_eq!(plan.view().extensions.len(), 1);
    assert_eq!(
        serde_json::to_value(&plan.view().unavailable).unwrap(),
        json!([{"name": "notes", "reason": "no-declaration"}])
    );
}

/// A labelled change to the valid control and the reason it must be refused for.
type Refusal = (&'static str, Box<dyn Fn(&mut Value)>, Reason);

fn declaration_refusals() -> Vec<Refusal> {
    fn case(label: &'static str, reason: Reason, change: impl Fn(&mut Value) + 'static) -> Refusal {
        (label, Box::new(change), reason)
    }
    let mut cases = vec![
        case("unknown top member (roles)", Reason::Shape, |v| {
            v["roles"] = json!(["roles/owner"])
        }),
        case("unknown top member (iam)", Reason::Shape, |v| {
            v["iam"] = json!({})
        }),
        case("missing top member", Reason::Shape, |v| {
            v.as_object_mut().unwrap().remove("admission");
        }),
        case("unknown resource member", Reason::Shape, |v| {
            v["resources"][0]["role"] = json!("owner")
        }),
        case("version 2", Reason::Version, |v| v["version"] = json!(2)),
        case("version as string", Reason::Version, |v| {
            v["version"] = json!("1")
        }),
        case("version as fraction", Reason::Version, |v| {
            v["version"] = json!(1.5)
        }),
        case("extension name grammar", Reason::ExtensionName, |v| {
            v["extension"] = json!("Colab")
        }),
        case("unknown backend", Reason::Backend, |v| {
            v["backend"] = json!("s3")
        }),
        case("65 resources", Reason::TooManyResources, |v| {
            let one = v["resources"][3].clone();
            v["resources"] = Value::Array(vec![one; 65]);
        }),
        case("resource name grammar", Reason::ResourceName, |v| {
            v["resources"][0]["name"] = json!("Page_Log")
        }),
        case(
            "duplicate resource name",
            Reason::DuplicateResourceName,
            |v| v["resources"][1]["name"] = json!("page-log"),
        ),
        case("unknown kind", Reason::Kind, |v| {
            v["resources"][0]["kind"] = json!("queue")
        }),
        case("absolute path", Reason::Path, |v| {
            v["resources"][0]["path"] = json!("/pages/log")
        }),
        case("empty segment", Reason::Path, |v| {
            v["resources"][0]["path"] = json!("pages//log")
        }),
        case("trailing slash", Reason::Path, |v| {
            v["resources"][0]["path"] = json!("pages/log/")
        }),
        case("empty path", Reason::Path, |v| {
            v["resources"][0]["path"] = json!("")
        }),
        case("dot segment", Reason::Path, |v| {
            v["resources"][0]["path"] = json!("pages/./log")
        }),
        case("escape segment", Reason::Path, |v| {
            v["resources"][0]["path"] = json!("../colab/pages")
        }),
        case("wildcard", Reason::Path, |v| {
            v["resources"][0]["path"] = json!("pages/{page}")
        }),
        case("star", Reason::Path, |v| {
            v["resources"][0]["path"] = json!("pages/*")
        }),
        case("backslash", Reason::Path, |v| {
            v["resources"][0]["path"] = json!("pages\\log")
        }),
        case("percent escape", Reason::Path, |v| {
            v["resources"][0]["path"] = json!("pages/%2e%2e")
        }),
        case("uppercase", Reason::Path, |v| {
            v["resources"][0]["path"] = json!("Pages/log")
        }),
        case("9 segments", Reason::Path, |v| {
            v["resources"][0]["path"] = json!("a/b/c/d/e/f/g/h/i")
        }),
        case("long segment", Reason::Path, |v| {
            v["resources"][0]["path"] = json!("a".repeat(65))
        }),
        case("zero limit", Reason::Limit, |v| {
            v["resources"][0]["limits"]["maxEntries"] = json!(0)
        }),
        case("negative limit", Reason::Limit, |v| {
            v["resources"][0]["limits"]["maxEntries"] = json!(-1)
        }),
        case("fractional limit", Reason::Limit, |v| {
            v["resources"][0]["limits"]["maxEntries"] = json!(1.5)
        }),
        case("exponent limit", Reason::Limit, |v| {
            v["resources"][0]["limits"]["maxEntries"] = serde_json::from_str("1e3").unwrap();
        }),
        case("string limit", Reason::Limit, |v| {
            v["resources"][0]["limits"]["maxEntries"] = json!("5")
        }),
        case("limit beyond 2^53", Reason::Limit, |v| {
            v["resources"][0]["limits"]["maxNamespaceBytes"] = json!(1u64 << 53)
        }),
        case("object larger than namespace", Reason::Limit, |v| {
            v["resources"][0]["limits"]["maxObjectBytes"] = json!(16777217)
        }),
        case("missing limit", Reason::Shape, |v| {
            v["resources"][0]["limits"]
                .as_object_mut()
                .unwrap()
                .remove("maxEntries");
        }),
        case("ttl field grammar", Reason::TtlField, |v| {
            v["resources"][0]["ttlField"] = json!("expires_at")
        }),
        case("ttl field not text", Reason::TtlField, |v| {
            v["resources"][0]["ttlField"] = json!(5)
        }),
        case("awareness with ttl", Reason::AwarenessStores, |v| {
            v["resources"][3]["ttlField"] = json!("expiresAt")
        }),
        case("awareness with indexes", Reason::AwarenessStores, |v| {
            v["resources"][3]["indexes"] = json!([{"field": "sequence", "direction": "asc"}]);
        }),
        case("65 indexes", Reason::TooManyIndexes, |v| {
            let entries: Vec<Value> = (0..65)
                .map(|n| json!({"field": format!("f{n}"), "direction": "asc"}))
                .collect();
            v["resources"][0]["indexes"] = Value::Array(entries);
        }),
        case("index direction", Reason::IndexDirection, |v| {
            v["resources"][0]["indexes"][0]["direction"] = json!("up")
        }),
        case("index field grammar", Reason::IndexField, |v| {
            v["resources"][0]["indexes"][0]["field"] = json!("a.b")
        }),
        case("duplicate index", Reason::DuplicateIndex, |v| {
            v["resources"][0]["indexes"][1] = json!({"field": "sequence", "direction": "asc"})
        }),
        case("artifact absolute", Reason::ArtifactPath, |v| {
            v["admission"]["artifact"] = json!("/etc/passwd")
        }),
        case("artifact escape", Reason::ArtifactPath, |v| {
            v["admission"]["artifact"] = json!("../rules/x.rules")
        }),
        case("artifact dotfile", Reason::ArtifactPath, |v| {
            v["admission"]["artifact"] = json!("rules/.hidden")
        }),
        case("digest uppercase", Reason::Digest, |v| {
            let upper = v["admission"]["digest"].as_str().unwrap().to_uppercase();
            v["admission"]["digest"] = json!(upper);
        }),
        case("digest short", Reason::Digest, |v| {
            v["admission"]["digest"] = json!("abc")
        }),
        case("entry point grammar", Reason::EntryPoint, |v| {
            v["admission"]["entryPoint"] = json!("colab admitted")
        }),
        case("admission command member", Reason::Shape, |v| {
            v["admission"]["command"] = json!("sh")
        }),
    ];
    // Operation routes and the mount segment may not start a namespace.
    for route in routes::ROUTES {
        let segment = route.trim_start_matches('/').to_owned();
        cases.push(case(
            "reserved operation route",
            Reason::ReservedPath,
            move |v| {
                v["resources"][0]["path"] = json!(format!("{segment}/log"));
            },
        ));
    }
    cases.push(case("reserved mount segment", Reason::ReservedPath, |v| {
        v["resources"][0]["path"] = json!("x/colab")
    }));
    cases
}

#[test]
fn every_declaration_refusal_changes_one_condition_of_a_valid_control() {
    let control = fixture("colab-firestore.json");
    declaration::parse(&control).expect("the control parses");
    for (label, change, reason) in declaration_refusals() {
        let mutated = edit("colab-firestore.json", |v| change(v));
        let error = declaration::parse(&mutated).expect_err(label);
        assert_eq!(error.reason, reason, "{label}");
    }
}

#[test]
fn size_and_json_strictness_are_checked_before_meaning() {
    let control = fixture("colab-firestore.json");
    let mut padded = control.clone();
    padded.resize(limits::DECLARATION_BYTES, b' ');
    declaration::parse(&padded).expect("exactly 64 KiB is allowed");
    padded.push(b' ');
    assert_eq!(
        declaration::parse(&padded).unwrap_err().reason,
        Reason::TooLarge
    );
    // A duplicate member is refused even though the second value would be valid.
    let text = String::from_utf8(control).unwrap();
    let duplicated = text.replacen("\"version\": 1,", "\"version\": 2, \"version\": 1,", 1);
    assert_ne!(duplicated, text);
    assert_eq!(
        declaration::parse(duplicated.as_bytes())
            .unwrap_err()
            .reason,
        Reason::NotStrictJson
    );
    assert_eq!(declaration::parse(b"[]").unwrap_err().reason, Reason::Shape);
    assert_eq!(
        declaration::parse(b"{").unwrap_err().reason,
        Reason::NotStrictJson
    );
}

#[test]
fn reserved_segments_follow_the_binding_routes() {
    for route in routes::ROUTES {
        assert!(
            declaration::reserved_segment(route.trim_start_matches('/')),
            "{route}"
        );
    }
    assert!(declaration::reserved_segment("x"));
    // The contract's route table lists exactly these four POST routes; a new route must
    // be added to `routes::ROUTES`, which this list then follows.
    assert_eq!(routes::ROUTES, ["/append", "/subscribe", "/ack", "/pair"]);
    assert!(!declaration::reserved_segment("pages"));
}

#[test]
fn plan_level_refusals_fail_the_whole_plan_and_name_the_extension() {
    let colab_bytes = fixture("colab-firestore.json");
    let (colab_artifact, notes_artifact) = (colab_artifact(), notes_artifact());
    let good_colab = colab(&colab_bytes, &colab_artifact);

    // A bad extension among good ones leaves no plan, whatever the input order.
    let broken = edit("notes-firestore.json", |v| {
        v["resources"][0]["path"] = json!("/abs")
    });
    for order in [
        [good_colab, notes(&broken, &notes_artifact)],
        [notes(&broken, &notes_artifact), good_colab],
    ] {
        let error = refused(FIRESTORE, &order);
        assert_eq!(error.extension.as_deref(), Some("notes"));
        assert_eq!(
            (error.reason, error.resource),
            (PlanReason::Declaration(Reason::Path), Some(0))
        );
    }

    let mismatch = refused(FIRESTORE, &[notes(&colab_bytes, &colab_artifact)]);
    assert_eq!(mismatch.reason, PlanReason::ExtensionMismatch);
    let local = edit("colab-firestore.json", |v| v["backend"] = json!("local"));
    assert_eq!(
        refused(FIRESTORE, &[colab(&local, &colab_artifact)]).reason,
        PlanReason::BackendMismatch
    );
    let twice = refused(FIRESTORE, &[good_colab, good_colab]);
    assert_eq!(
        (twice.reason, twice.extension.as_deref()),
        (PlanReason::DuplicateExtension, Some("colab"))
    );
    let bad_name = Enabled {
        name: "../colab",
        supplied: None,
    };
    assert_eq!(
        refused(FIRESTORE, &[bad_name]).reason,
        PlanReason::ExtensionName
    );
    let many = vec![
        Enabled {
            name: "colab",
            supplied: None
        };
        limits::PLAN_EXTENSIONS + 1
    ];
    assert_eq!(
        refused(FIRESTORE, &many).reason,
        PlanReason::TooManyExtensions
    );
}

#[test]
fn backend_limits_and_overlaps_are_unsupported_requirements() {
    let artifact = colab_artifact();
    for (label, change) in [
        (
            "object",
            Box::new(|v: &mut Value| {
                v["resources"][0]["limits"]["maxObjectBytes"] = json!(12582913)
            }) as Box<dyn Fn(&mut Value)>,
        ),
        (
            "namespace",
            Box::new(|v: &mut Value| {
                v["resources"][0]["limits"]["maxNamespaceBytes"] = json!(67108865)
            }),
        ),
        (
            "entries",
            Box::new(|v: &mut Value| v["resources"][0]["limits"]["maxEntries"] = json!(1025)),
        ),
    ] {
        let declaration = edit("colab-firestore.json", |v| change(v));
        let error = refused(FIRESTORE, &[colab(&declaration, &artifact)]);
        assert_eq!(
            (error.reason, error.resource),
            (PlanReason::LimitAboveBackend, Some(0)),
            "{label}"
        );
    }
    // Exactly the backend bounds are allowed.
    let at_bounds = edit("colab-firestore.json", |v| {
        v["resources"][0]["limits"] =
            json!({"maxObjectBytes": 12582912, "maxNamespaceBytes": 67108864, "maxEntries": 1024});
    });
    compose(FIRESTORE, &[colab(&at_bounds, &artifact)]).unwrap();

    for (label, first, second) in [
        ("equal", "log", "log"),
        ("nested", "pages", "pages/a/log"),
        ("nested the other way", "pages/a/log", "pages"),
    ] {
        let declaration = edit("colab-firestore.json", |v| {
            v["resources"][0]["path"] = json!(first);
            v["resources"][1]["path"] = json!(second);
        });
        let error = refused(FIRESTORE, &[colab(&declaration, &artifact)]);
        assert_eq!(error.reason, PlanReason::Overlap, "{label}");
        assert_eq!(error.extension.as_deref(), Some("colab"));
    }
    // A name prefix is not a path prefix, and two extensions never share a root.
    let siblings = edit("colab-firestore.json", |v| {
        v["resources"][0]["path"] = json!("log");
        v["resources"][1]["path"] = json!("logs");
    });
    compose(FIRESTORE, &[colab(&siblings, &artifact)]).unwrap();
    both(FIRESTORE);
}

#[test]
fn admission_artifact_must_be_the_approved_bytes() {
    let declaration = fixture("colab-firestore.json");
    let mut tampered = colab_artifact();
    tampered.push(b'\n');
    assert_eq!(
        refused(FIRESTORE, &[colab(&declaration, &tampered)]).reason,
        PlanReason::ArtifactDigest
    );
    // Another extension's artifact does not satisfy this declaration.
    assert_eq!(
        refused(FIRESTORE, &[colab(&declaration, &notes_artifact())]).reason,
        PlanReason::ArtifactDigest
    );
    let oversized = vec![b'a'; limits::DECLARATION_ARTIFACT_BYTES + 1];
    assert_eq!(
        refused(FIRESTORE, &[colab(&declaration, &oversized)]).reason,
        PlanReason::ArtifactTooLarge
    );
    // The plan records the digest it verified, never the artifact bytes.
    assert!(!String::from_utf8_lossy(both(FIRESTORE).bytes()).contains("colabAdmitted()"));
}

#[test]
fn on_firestore_a_resource_path_names_a_collection() {
    let artifact = colab_artifact();
    let with_path = |path: &str| {
        edit("colab-firestore.json", |v| {
            v["resources"][0]["path"] = json!(path)
        })
    };
    for (count, path) in [
        (1, "a"),
        (3, "a/b/c"),
        (5, "a/b/c/d/e"),
        (7, "a/b/c/d/e/f/g"),
    ] {
        let declaration = with_path(path);
        compose(FIRESTORE, &[colab(&declaration, &artifact)])
            .unwrap_or_else(|e| panic!("{count}: {e:?}"));
    }
    // Even counts name a document. The refusal is the plan's, for the Firestore target only:
    // the declaration grammar (1 to 8 segments) and other backends are unchanged.
    for path in ["a/b", "a/b/c/d", "a/b/c/d/e/f", "a/b/c/d/e/f/g/h"] {
        let declaration = with_path(path);
        let error = refused(FIRESTORE, &[colab(&declaration, &artifact)]);
        assert_eq!(
            (error.reason, error.resource),
            (PlanReason::PathNotCollection, Some(0)),
            "{path}"
        );
        declaration::parse(&declaration).expect("the grammar still admits it");
        let cloudflare = edit("colab-firestore.json", |v| {
            v["backend"] = json!("cloudflare");
            v["resources"][0]["path"] = json!(path);
        });
        let target = Target {
            backend: CloudBackend::Cloudflare,
            physical_ttl: false,
        };
        compose(target, &[colab(&cloudflare, &artifact)]).expect(path);
    }
}

#[test]
fn optional_hosting_manifest_is_covered_by_the_exact_declaration_digest() {
    // Remote-authored data, not Colab release bytes or hosted acceptance.
    let declaration = edit("colab-firestore.json", |value| {
        value["hosting"] = json!({"version":1,"files":[
            {"path":"/index.html","sha256":"a".repeat(64),"length":5,"contentType":"text/html"}
        ]});
    });
    declaration::parse(&declaration).expect("the optional versioned manifest is valid");
    let artifact = colab_artifact();
    let plan = compose(FIRESTORE, &[colab(&declaration, &artifact)]).unwrap();
    let old = fixture("colab-firestore.json");
    let old_plan = compose(FIRESTORE, &[colab(&old, &artifact)]).unwrap();
    assert_ne!(plan.digest(), old_plan.digest());
}

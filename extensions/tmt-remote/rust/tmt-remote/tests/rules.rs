//! Rules and index composition (#2163). Goldens are hand-built from the documented output
//! shape, not produced by the code under test. Every accepted construct has a near-miss
//! that must be refused; hostile fragments are real files shared with the emulator suite.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};
use tmt_remote::{
    deploy_plan::{self, CloudBackend, Enabled, Plan, Supplied, Target},
    rules::{self, Fragment, RulesError, RulesReason},
};

fn path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/rules")
        .join(name)
}
fn fixture(name: &str) -> Vec<u8> {
    fs::read(path(name)).unwrap_or_else(|error| panic!("{name}: {error}"))
}
fn text(name: &str) -> String {
    String::from_utf8(fixture(name)).unwrap()
}
fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn target(physical_ttl: bool) -> Target {
    Target {
        backend: CloudBackend::Firestore,
        physical_ttl,
    }
}
/// A declaration whose admission digest names `artifact`, so only the fragment varies.
fn declaration(name: &str, artifact: &[u8]) -> Vec<u8> {
    let mut value: Value = serde_json::from_slice(&fixture(&format!("{name}.json"))).unwrap();
    value["admission"]["digest"] = json!(sha(artifact));
    serde_json::to_vec(&value).unwrap()
}
fn plan_of(extensions: &[(&str, &[u8])], physical_ttl: bool) -> Plan {
    let declarations: Vec<_> = extensions.iter().map(|(n, a)| declaration(n, a)).collect();
    plan_from(extensions, &declarations, physical_ttl)
}
fn plan_from(extensions: &[(&str, &[u8])], declarations: &[Vec<u8>], physical_ttl: bool) -> Plan {
    let enabled: Vec<_> = extensions
        .iter()
        .zip(declarations)
        .map(|((name, artifact), declaration)| Enabled {
            name,
            supplied: Some(Supplied {
                declaration,
                artifact,
            }),
        })
        .collect();
    deploy_plan::compose(target(physical_ttl), &enabled).expect("the plan composes")
}
fn compose_one(source: &[u8]) -> Result<String, RulesError> {
    let plan = plan_of(&[("colab", source)], false);
    rules::compose(
        &plan,
        &[Fragment {
            extension: "colab",
            source,
        }],
    )
    .map(|c| c.rules)
}
fn refusal(source: &str) -> RulesReason {
    compose_one(source.as_bytes()).expect_err(source).reason
}
/// A condition inside a fragment with an auth reference, so only `expr` is under test.
fn condition(expr: &str) -> String {
    format!("match /d/{{id}} {{ allow read: if request.auth != null && ({expr}); }}")
}
fn accepts(expr: &str) {
    let fragment = condition(expr);
    compose_one(fragment.as_bytes()).unwrap_or_else(|e| panic!("{expr}: {e:?}"));
}
fn refuses(expr: &str, reason: RulesReason) {
    assert_eq!(refusal(&condition(expr)), reason, "{expr}");
}

#[test]
fn composes_the_documented_rules_and_indexes() {
    let (colab, notes) = (fixture("colab.rules"), fixture("notes.rules"));
    for physical_ttl in [false, true] {
        let plan = plan_of(&[("colab", &colab), ("notes", &notes)], physical_ttl);
        let composed = rules::compose(
            &plan,
            &[
                Fragment {
                    extension: "notes",
                    source: &notes,
                },
                Fragment {
                    extension: "colab",
                    source: &colab,
                },
            ],
        )
        .unwrap();
        assert_eq!(composed.rules, text("composed.rules"));
        let golden = if physical_ttl {
            "composed-ttl.indexes.json"
        } else {
            "composed-no-ttl.indexes.json"
        };
        assert_eq!(
            composed.indexes,
            text(golden),
            "physical_ttl {physical_ttl}"
        );
    }
}

#[test]
fn an_extension_without_statements_is_denied_inside_its_wrapper() {
    assert_eq!(
        compose_one(b"// nothing to grant\n").unwrap(),
        text("wrapper-colab-empty.rules")
    );
}

#[test]
fn every_hostile_fragment_is_refused() {
    let expected = [
        ("absolute-get", RulesReason::Identifier),
        ("absolute-exists", RulesReason::Identifier),
        ("dynamic-first-segment", RulesReason::Identifier),
        ("path-literal", RulesReason::Identifier),
        ("path-concatenation", RulesReason::Identifier),
        ("shadowing-function", RulesReason::Statement),
        ("unconditional-allow", RulesReason::BareTrue),
        ("time-only", RulesReason::Unconditional),
        ("brace-escape", RulesReason::Statement),
    ];
    let on_disk = fs::read_dir(path("hostile")).unwrap().count();
    assert_eq!(
        on_disk,
        expected.len(),
        "every hostile file has an expectation"
    );
    for (name, reason) in expected {
        let error = compose_one(&fixture(&format!("hostile/{name}.rules"))).expect_err(name);
        assert_eq!(error.reason, reason, "{name}");
        assert_eq!(error.extension.as_deref(), Some("colab"), "{name}");
    }
}

#[test]
fn the_wrapper_is_never_taken_from_the_fragment() {
    // A fragment carrying its own header or service refuses; the output header is a constant.
    for fragment in [
        "rules_version = '2';",
        "service cloud.firestore { }",
        "function f() { return true; }",
        "let a = 1;",
        "match /databases/{database}/documents { }",
    ] {
        let reason = refusal(fragment);
        assert!(
            matches!(
                reason,
                RulesReason::Statement | RulesReason::Syntax | RulesReason::WildcardName
            ),
            "{fragment}: {reason:?}"
        );
    }
    assert!(
        text("composed.rules").starts_with("rules_version = '2';\nservice cloud.firestore {\n")
    );
}

#[test]
fn allow_listed_references_and_their_near_misses() {
    use RulesReason::*;
    accepts("request.time > resource.data.t");
    refuses("request.timestamp > resource.data.t", Identifier);
    accepts("request.method == 'get'");
    refuses("request.methods == 'get'", Identifier);
    accepts("request.auth.token.email_verified == true");
    refuses("request.query.limit == 1", Identifier);
    accepts("request.resource.data.a == 1");
    refuses("request.resource.id == 1", Identifier);
    refuses("request.resource == 1", Syntax);
    accepts("resource.id == 'a'");
    refuses("resource.name == 'a'", Identifier);
    accepts("id == 'a'");
    refuses("other == 'a'", Identifier);
    accepts("resource.data.a[0] == 1 && resource.data['b'] == 2");
    refuses("database == 'a'", Identifier);
    refuses("path('/a') == 1", Identifier);
    refuses("get('a') == 1", Identifier);
    refuses("exists('a')", Identifier);
    refuses("getAfter('a')", Identifier);
    refuses("existsAfter('a')", Identifier);
    refuses("$(id) == 1", Syntax);
}

#[test]
fn allow_listed_operators_literals_and_types() {
    use RulesReason::*;
    accepts("resource.data.a in ['x', 'y'] || resource.data.m in {'k': 1}");
    refuses("resource.data.a inn ['x']", Syntax);
    for kind in [
        "string",
        "int",
        "float",
        "bool",
        "bytes",
        "list",
        "map",
        "timestamp",
        "duration",
        "null",
    ] {
        accepts(&format!("resource.data.a is {kind}"));
    }
    refuses("resource.data.a is bytez", Identifier);
    refuses("resource.data.a is", Syntax);
    for op in ["+", "-", "*", "/", "%"] {
        accepts(&format!("resource.data.a {op} 2 == 1"));
    }
    for op in ["<", "<=", ">", ">=", "==", "!=", "&&", "||"] {
        accepts(&format!("resource.data.a {op} resource.data.b"));
    }
    accepts("!resource.data.a && -resource.data.b < 0");
    accepts("(resource.data.a == 1 ? 2 : 3) == 2");
    refuses("resource.data.a & resource.data.b", Character);
    refuses("resource.data.a | resource.data.b", Character);
    refuses("resource.data.a ^ 1", Character);
    refuses("resource.data.a === 1", Syntax);
    accepts(
        "resource.data.a == 1.5 && resource.data.s == \"dq\" && resource.data.n == null && resource.data.f == false",
    );
    refuses("resource.data.s == 'open", Unterminated);
    refuses("resource.data.a == 1 ? 2", Syntax);
    refuses("{'a': 1, b: 2}", Syntax);
    refuses("(resource.data.a", Syntax);
}

#[test]
fn allow_listed_methods_and_their_near_misses() {
    use RulesReason::*;
    for call in ["size()", "keys()", "values()", "toMillis()"] {
        accepts(&format!("resource.data.a.{call} == 1"));
    }
    for call in [
        "hasAll(['a'])",
        "hasAny(['a'])",
        "hasOnly(['a'])",
        "diff(resource.data)",
    ] {
        accepts(&format!("resource.data.a.{call}"));
    }
    accepts("request.resource.data.diff(resource.data).affectedKeys().hasOnly(['a'])");
    accepts("resource.data.a.matches('^a.*')");
    accepts("resource.data.get('a', 1) == 1 && resource.data.get('b') == 2");
    refuses("resource.data.a.size(1) == 1", Call);
    refuses("resource.data.a.keys(1)", Call);
    refuses("resource.data.a.hasAll()", Call);
    refuses("resource.data.a.hasOnly(['a'], ['b'])", Call);
    refuses("resource.data.a.diff()", Call);
    refuses("resource.data.a.matches(resource.data.b)", Call);
    refuses("resource.data.a.matches()", Call);
    refuses("resource.data.get() == 1", Call);
    refuses("resource.data.get('a', 1, 2) == 1", Call);
    refuses("resource.data.a.sizes() == 1", Method);
    refuses("resource.data.a.hasNone(['a'])", Method);
    refuses("resource.data.a.affectedKey()", Method);
    refuses("resource.data.a.toMillis2() == 1", Method);
    accepts("request.time < request.time + duration.value(30, 'd')");
    refuses("request.time < request.time + duration.value(30)", Call);
    refuses(
        "request.time < request.time + duration.values(30, 'd')",
        Call,
    );
    accepts("int(resource.data.a) == 1 && string(resource.data.a) == 'x'");
    refuses("float(resource.data.a) == 1", Identifier);
    refuses("int() == 1", Call);
    refuses("int(1, 2) == 1", Call);
}

#[test]
fn true_is_only_an_operand_of_equality() {
    use RulesReason::*;
    accepts("resource.data.a == true");
    accepts("true != resource.data.a");
    accepts("resource.data.a == true && resource.data.b");
    accepts("resource.data.b || true == resource.data.a");
    refuses("resource.data.a || true", BareTrue);
    refuses("true || resource.data.a", BareTrue);
    refuses("!true", BareTrue);
    refuses("(true)", BareTrue);
    refuses("[true]", BareTrue);
    refuses("resource.data.a.get(true)", BareTrue);
    refuses("resource.data.a == true + 1", BareTrue);
    refuses("resource.data.a < true == resource.data.b", BareTrue);
    refuses("resource.data.a ? true : false", BareTrue);
    refuses("{'k': true}", BareTrue);
    assert_eq!(refusal("match /d/{id} { allow read: if true; }"), BareTrue);
}

#[test]
fn a_condition_must_refer_to_auth_or_document_data() {
    use RulesReason::*;
    for fragment in [
        "match /d/{id} { allow read: if request.time < request.time + duration.value(1, 'd'); }",
        "match /d/{id} { allow read: if id == 'a'; }",
        "match /d/{id} { allow read: if 1 == 1; }",
        "match /d/{id} { allow read: if false; }",
        "match /d/{id} { allow read: if request.method == 'get'; }",
    ] {
        assert_eq!(refusal(fragment), Unconditional, "{fragment}");
    }
    for fragment in [
        "match /d/{id} { allow read: if request.auth != null; }",
        "match /d/{id} { allow read: if resource.data.public == true; }",
        "match /d/{id} { allow create: if request.resource.data.a == 1; }",
        "match /d/{id} { allow read: if ext.exists(/m/$(id)); }",
    ] {
        compose_one(fragment.as_bytes()).unwrap_or_else(|e| panic!("{fragment}: {e:?}"));
    }
}

#[test]
fn macros_expand_to_a_fixed_prefix_and_refuse_anything_else() {
    use RulesReason::*;
    let rules = compose_one(b"match /d/{id} { allow read: if ext.getAfter(/a/b-1/$(id)/$(request.auth.uid)).data.seq == 1; }").unwrap();
    assert!(rules.contains("getAfter(/databases/$(database)/documents/x/colab/a/b-1/$(id)/$(request.auth.uid)).data.seq == 1"));
    for name in ["get", "exists", "getAfter", "existsAfter"] {
        accepts(&format!("ext.{name}(/a/$(id))"));
    }
    accepts("ext.exists(/$(id))");
    refuses("ext.list(/a)", Macro);
    refuses("ext.get()", Macro);
    refuses("ext.get(a)", Syntax);
    refuses("ext.get(/a.b)", Syntax);
    refuses("ext.get(/a/$(id)x)", Syntax);
    refuses("ext.get(/a/$(true))", BareTrue);
    refuses("ext.get(/a/$(get(/b)))", Identifier);
    refuses("ext.get(/a/$(path('/b')))", Identifier);
    refuses("ext", Syntax);
}

#[test]
fn statements_wildcards_and_limits() {
    use RulesReason::*;
    for method in ["get", "list", "read", "create", "update", "delete", "write"] {
        compose_one(
            format!("match /d/{{id}} {{ allow {method}: if resource.data.a == 1; }}").as_bytes(),
        )
        .unwrap();
    }
    compose_one(b"allow get, list: if request.auth != null;").unwrap();
    assert_eq!(
        refusal("match /d/{id} { allow reads: if request.auth != null; }"),
        Method
    );
    assert_eq!(refusal("match /d/{id} { allow read; }"), Syntax);
    assert_eq!(
        refusal("match /d/{id} { allow read: request.auth != null; }"),
        Syntax
    );
    assert_eq!(
        refusal("match /d/{id} { allow read: if request.auth != null }"),
        Syntax
    );
    for name in [
        "request", "resource", "database", "ext", "path", "get", "exists", "duration",
    ] {
        assert_eq!(
            refusal(&format!(
                "match /{{{name}}} {{ allow read: if request.auth != null; }}"
            )),
            WildcardName,
            "{name}"
        );
    }
    assert_eq!(
        refusal("match /{a}/{a} { allow read: if request.auth != null; }"),
        WildcardName
    );
    assert_eq!(
        refusal("match /{a=**}/b { allow read: if request.auth != null; }"),
        Syntax
    );
    assert_eq!(
        refusal("match /a.b { allow read: if request.auth != null; }"),
        Syntax
    );
    let nested = |levels: usize| {
        let mut source = String::new();
        for _ in 0..levels {
            source.push_str("match /a { ");
        }
        source.push_str("allow read: if request.auth != null;");
        source.push_str(&" }".repeat(levels));
        source
    };
    compose_one(nested(8).as_bytes()).unwrap();
    assert_eq!(refusal(&nested(9)), TooDeep);
    let parens = |levels: usize| {
        format!(
            "match /d/{{id}} {{ allow read: if {}request.auth != null{}; }}",
            "(".repeat(levels),
            ")".repeat(levels)
        )
    };
    compose_one(parens(31).as_bytes()).unwrap();
    assert_eq!(refusal(&parens(33)), TooDeep);
    assert_eq!(refusal("/* never closed"), Unterminated);
    assert_eq!(
        refusal("match /d/{id} { allow read: if request.auth != 'caf\u{e9}'; }"),
        Character
    );
    assert_eq!(
        refusal("match /d/{id} { allow read: if request.auth != `a`; }"),
        Character
    );
    compose_one(
        b"// comment\r\n\tmatch /d/{id} { /* inline */ allow read: if request.auth != null; }\r\n",
    )
    .unwrap();
}

#[test]
fn the_plan_binds_the_fragments_it_verified() {
    let (colab, notes) = (fixture("colab.rules"), fixture("notes.rules"));
    let plan = plan_of(&[("colab", &colab), ("notes", &notes)], false);
    let fragment = |extension, source| Fragment { extension, source };
    let reason = |fragments: &[Fragment<'_>]| rules::compose(&plan, fragments).unwrap_err();
    let missing = reason(&[fragment("colab", &colab)]);
    assert_eq!(
        (missing.reason, missing.extension.as_deref()),
        (RulesReason::MissingFragment, Some("notes"))
    );
    assert_eq!(
        reason(&[
            fragment("colab", &colab),
            fragment("notes", &notes),
            fragment("other", &notes)
        ])
        .reason,
        RulesReason::ExtraFragment
    );
    assert_eq!(
        reason(&[
            fragment("colab", &colab),
            fragment("colab", &colab),
            fragment("notes", &notes)
        ])
        .reason,
        RulesReason::ExtraFragment
    );
    let mut tampered = notes.clone();
    tampered.extend_from_slice(b"\nmatch /z/{id} { allow read: if request.auth != null; }\n");
    let error = reason(&[fragment("colab", &colab), fragment("notes", &tampered)]);
    assert_eq!(
        (error.reason, error.extension.as_deref()),
        (RulesReason::FragmentDigest, Some("notes"))
    );
    // A fragment that is not in the plan (an unavailable extension) is not composed.
    let only_colab = plan_of(&[("colab", &colab)], false);
    assert_eq!(
        rules::compose(
            &only_colab,
            &[fragment("colab", &colab), fragment("notes", &notes)]
        )
        .unwrap_err()
        .reason,
        RulesReason::ExtraFragment
    );
}

/// One log resource per entry: its collection, the fields indexed ascending (each one a
/// single-field index config) and an optional TTL field.
type Collections = Vec<(&'static str, Vec<String>, Option<&'static str>)>;
fn declared_configs(name: &str, artifact: &[u8], collections: &Collections) -> Vec<u8> {
    let resources: Vec<_> = collections
        .iter()
        .enumerate()
        .map(|(n, (collection, fields, ttl))| {
            json!({
                "name": format!("res-{n}"),
                "kind": "log",
                "path": collection,
                "limits": { "maxObjectBytes": 4096, "maxNamespaceBytes": 65536, "maxEntries": 16 },
                "ttlField": ttl,
                "indexes": fields.iter().map(|f| json!({"field": f, "direction": "asc"})).collect::<Vec<_>>(),
            })
        })
        .collect();
    serde_json::to_vec(&json!({
        "version": 1, "extension": name, "backend": "firestore", "resources": resources,
        "admission": { "artifact": "rules/x.rules", "digest": sha(artifact), "entryPoint": name },
    }))
    .unwrap()
}
fn fields(prefix: &str, count: usize) -> Vec<String> {
    (0..count).map(|n| format!("{prefix}{n}")).collect()
}
fn compose_declared(
    extensions: &[(&str, Collections)],
    physical_ttl: bool,
) -> Result<rules::Composed, RulesError> {
    let artifact = fixture("notes.rules");
    let declarations: Vec<_> = extensions
        .iter()
        .map(|(name, collections)| declared_configs(name, &artifact, collections))
        .collect();
    let named: Vec<_> = extensions
        .iter()
        .map(|(n, _)| (*n, artifact.as_slice()))
        .collect();
    let plan = plan_from(&named, &declarations, physical_ttl);
    let fragments: Vec<_> = named
        .iter()
        .map(|(extension, source)| Fragment { extension, source })
        .collect();
    rules::compose(&plan, &fragments)
}

#[test]
fn the_free_plan_allows_200_single_field_index_configs_and_no_more() {
    let at = |configs: usize| {
        // Four collections of at most 64 fields each, so the declaration bounds hold.
        let per = configs.div_ceil(4);
        let mut collections = Vec::new();
        for (n, id) in ["alpha", "beta", "gamma", "delta"].into_iter().enumerate() {
            let have = configs.saturating_sub(n * per).min(per);
            collections.push((id, fields("f", have), None));
        }
        compose_declared(&[("notes", collections)], false)
    };
    assert!(at(200).is_ok());
    let error = at(201).unwrap_err();
    assert_eq!(
        (error.reason, error.extension, error.offset),
        (RulesReason::TooManyIndexConfigs, None, None)
    );
    assert_eq!(error.reason.code(), "too-many-index-configs");
}

#[test]
fn index_configs_are_counted_after_extensions_share_a_collection_and_field() {
    // 300 declared index entries but 200 distinct (collection, field) configs.
    let one: Collections = vec![
        ("shared", fields("f", 50), None),
        ("other", fields("g", 50), None),
        ("third", fields("h", 50), None),
    ];
    let two: Collections = vec![
        ("shared", fields("f", 50), None),
        ("other", fields("g", 50), None),
        ("fourth", fields("i", 50), None),
    ];
    assert!(compose_declared(&[("notes", one.clone()), ("colab", two.clone())], false).is_ok());
    // One more distinct field in the second extension makes 201.
    let mut over = two;
    over[2].1.push("extra".to_owned());
    let error = compose_declared(&[("notes", one), ("colab", over)], false).unwrap_err();
    assert_eq!(error.reason, RulesReason::TooManyIndexConfigs);
}

#[test]
fn a_ttl_only_field_is_an_index_config_only_where_the_target_provisions_ttl() {
    // 199 configs plus two TTL-only fields: with physical TTL each TTL field is a config
    // (201); on Spark no TTL override is written, so they do not count (199).
    let plan = |physical_ttl| {
        let collections: Collections = vec![
            ("alpha", fields("f", 64), Some("expiresA")),
            ("beta", fields("f", 64), Some("expiresB")),
            ("gamma", fields("f", 64), None),
            ("delta", fields("f", 7), None),
        ];
        compose_declared(&[("notes", collections)], physical_ttl)
    };
    assert_eq!(
        plan(true).unwrap_err().reason,
        RulesReason::TooManyIndexConfigs
    );
    assert!(plan(false).is_ok());
}

#[test]
fn colab_sign_in_provider_string_ids_and_nested_wildcards_are_composable() {
    accepts("request.auth.token.firebase.sign_in_provider == 'anonymous'");
    accepts("request.auth.token.firebase.sign_in_provider == 'google.com'");
    let source = b"match /spaces/{spaceId} { match /log/{docId} { allow create: if request.auth != null && docId == spaceId + '_' + request.resource.data.page + '_' + string(request.resource.data.seq) && request.resource.data.space == spaceId; } }";
    let composed = compose_one(source).unwrap();
    assert!(composed.contains("match /x/colab"));
    assert!(composed.contains("docId == spaceId + '_' + request.resource.data.page + '_' + string(request.resource.data.seq)"));
    assert!(composed.contains("request.resource.data.space == spaceId"));
    assert!(compose_one(b"match /spaces/{spaceId} { match /log/{docId} { allow create: if request.auth != null && docId == missingId; } }").is_err());
    assert!(
        compose_one(b"match /spaces/{request} { allow read: if request.auth != null; }").is_err()
    );
    refuses("request.resource.id == 'id'", RulesReason::Identifier);
    refuses("request.query.limit == 1", RulesReason::Identifier);
    refuses(
        "resource.data.id.matches(resource.data.pattern)",
        RulesReason::Call,
    );
}

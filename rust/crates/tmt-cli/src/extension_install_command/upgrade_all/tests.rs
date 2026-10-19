use super::*;
use std::cell::Cell;

#[test]
fn bulk_protocol_retains_one_notice_on_success_and_partial_failure() {
    let success = upgraded(
        Product::Remote,
        json!({"changed":true,"version":"0.1.0-alpha.2","restartHint":"one restart notice"}),
    );
    let failed = failed(
        Product::Remote,
        Failure::new("EXTENSION_SKILLS_FAILED", "release active", 1)
            .suggestion("one restart notice".into()),
    );
    for row in [success, failed] {
        let bytes = serde_json::to_vec(&json!({"products":[row]})).unwrap();
        assert_eq!(
            String::from_utf8(bytes.clone())
                .unwrap()
                .matches("one restart notice")
                .count(),
            1
        );
        assert_eq!(parse_results(&bytes).unwrap(), vec![row]);
    }
}

fn pending(product: Product) -> (Product, String, String) {
    (product, "1.0.0".into(), "1.1.0".into())
}

#[test]
fn one_consent_lists_all_versions_and_failure_does_not_stop_other_products() {
    let asked = Cell::new(0);
    let mut applied = Vec::new();
    let rows = settle(
        vec![],
        vec![
            pending(Product::Ops),
            pending(Product::Remote),
            pending(Product::Colab),
            pending(Product::Digest),
        ],
        |question| {
            asked.set(asked.get() + 1);
            assert!(!question.contains("office"));
            assert!(question.contains("ops 1.0.0 -> 1.1.0"));
            assert!(question.contains("remote 1.0.0 -> 1.1.0"));
            assert!(question.contains("colab 1.0.0 -> 1.1.0"));
            assert!(question.contains("digest 1.0.0 -> 1.1.0"));
            Ok(true)
        },
        |product, selected| {
            applied.push(product);
            assert_eq!(selected, "1.1.0");
            if product == Product::Ops {
                return Err(Failure::new("EXTENSION_UPGRADE_FAILED", "refused", 1));
            }
            Ok(Some((
                json!({"changed":true,"version":selected}),
                super::super::Human::plain(String::new()),
            )))
        },
    );
    assert_eq!(asked.get(), 1);
    assert_eq!(
        applied,
        vec![
            Product::Ops,
            Product::Remote,
            Product::Colab,
            Product::Digest
        ]
    );
    assert_eq!(rows[0]["status"], "failed");
    assert_eq!(rows[0]["error"]["code"], "EXTENSION_UPGRADE_FAILED");
    assert_eq!(rows[1]["status"], "changed");
    assert_eq!(rows[2]["status"], "changed");
    assert_eq!(rows[3]["status"], "changed");
}

#[test]
fn missing_consent_changes_no_extension_and_returns_exact_rerun_hint() {
    let rows = settle(
        vec![],
        vec![pending(Product::Ops)],
        |_| Ok(false),
        |_, _| panic!("must not mutate"),
    );
    assert_eq!(rows[0]["status"], "consentRequired");
    assert_eq!(rows[0]["hint"], "tmt upgrade --yes");
}

#[test]
fn nothing_installed_neither_prompts_nor_installs() {
    let rows = settle(
        vec![],
        vec![],
        |_| panic!("no prompt"),
        |_, _| panic!("no installation"),
    );
    assert!(rows.is_empty());
}

#[test]
fn child_protocol_rejects_duplicate_products_unknown_fields_and_unbounded_reports() {
    let valid = Plan {
        products: Vec::new(),
        pending: vec![
            pending(Product::Ops),
            pending(Product::Remote),
            pending(Product::Colab),
            pending(Product::Digest),
        ],
    }
    .document();
    assert!(Plan::parse(&serde_json::to_vec(&valid).unwrap()).is_some());
    let mut duplicate = valid.clone();
    duplicate["pending"]
        .as_array_mut()
        .unwrap()
        .push(valid["pending"][0].clone());
    assert!(Plan::parse(&serde_json::to_vec(&duplicate).unwrap()).is_none());
    let mut unexpected = valid.clone();
    unexpected["unknown"] = true.into();
    assert!(Plan::parse(&serde_json::to_vec(&unexpected).unwrap()).is_none());
    let mut wrong_product = valid;
    wrong_product["pending"][0]["product"] = "cli".into();
    assert!(Plan::parse(&serde_json::to_vec(&wrong_product).unwrap()).is_none());
    assert!(Plan::parse(&vec![b' '; LIMIT + 1]).is_none());
    assert!(Plan::parse(br#"{"products":[{"product":"ops","status":"changed","version":"1.1.0"}],"pending":[]}"#).is_none());
    assert!(
        parse_results(br#"{"products":[{"product":"ops","status":"failed","error":{}}]}"#)
            .is_none()
    );
}

#[test]
fn a_changed_extension_reports_its_version_without_a_message() {
    let row = upgraded(Product::Ops, json!({"changed":true,"version":"1.1.0"}));
    assert_eq!(row["status"], "changed");
    assert_eq!(row["version"], "1.1.0");
    assert!(row.get("message").is_none());
    assert!(parse_results(&serde_json::to_vec(&json!({"products":[row]})).unwrap()).is_some());
}

#[test]
fn ordinary_extension_list_is_local_observation_without_update_discovery() {
    let root = std::env::temp_dir().join(format!(
        "tmt-public-list-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    let result = crate::extension_install_command::listing(
        &root,
        false,
        root.join("empty-path").as_os_str(),
    );
    std::fs::remove_dir_all(&root).unwrap();
    let (document, _) = result.unwrap();
    assert!(
        document["extensions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row.get("update").is_none()),
        "extension list without --check is local-only"
    );
}

/// Exercise the real installation reader and planner; only published discovery is injected.
#[test]
fn stale_and_equal_pointers_leave_installed_extensions_out_of_the_apply_plan() {
    use std::{
        fs,
        os::unix::fs::{PermissionsExt, symlink},
    };
    use tmt_core::{content_digest::sha256, native_install::Channel};

    struct Fixture(std::path::PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    let fixture = Fixture(
        std::env::temp_dir().join(format!("tmt-extension-pointer-plan-{}", std::process::id())),
    );
    fs::create_dir(&fixture.0).unwrap();
    let prefix = fixture.0.canonicalize().unwrap();
    let product = Product::Ops;
    let id = "12345678-1234-4234-8234-123456789abc";
    let root = prefix.join(product.namespace());
    let release = root.join("releases").join(id);
    fs::create_dir_all(&release).unwrap();
    fs::create_dir(prefix.join("bin")).unwrap();
    let bytes = b"managed planner fixture\n";
    let mut hashes = serde_json::Map::new();
    for name in product.files() {
        fs::write(release.join(name), bytes).unwrap();
        if name == product.executable() {
            fs::set_permissions(release.join(name), fs::Permissions::from_mode(0o755)).unwrap();
        }
        hashes.insert(name.into(), json!(sha256(bytes)));
    }
    fs::write(
        release.join("receipt.json"),
        serde_json::to_vec(&json!({
            "schema_version": 1, "release_id": id, "prefix": prefix,
            "version": "0.1.0-alpha.2", "channel": "alpha", "pinned_version": null,
            "archive": "tmt-ops-0.1.0-alpha.2-aarch64-apple-darwin.tar.gz",
            "archive_sha256": sha256(b"archive"), "target": "aarch64-apple-darwin",
            "file_sha256": hashes, "source": "local-archive"
        }))
        .unwrap(),
    )
    .unwrap();
    symlink(format!("releases/{id}"), root.join("current")).unwrap();
    for name in product.links() {
        symlink(product.link_target(), prefix.join("bin").join(name)).unwrap();
    }
    let receipt_before = fs::read(release.join("receipt.json")).unwrap();
    for candidate in ["0.1.0-alpha.1", "0.1.0-alpha.2", "0.1.0-alpha.3"] {
        let calls = Cell::new(0);
        let plan = plan_with(&prefix, |requested, channel| {
            calls.set(calls.get() + 1);
            assert_eq!(requested, product);
            assert_eq!(channel, Channel::Alpha);
            Ok(candidate.into())
        });
        assert_eq!(calls.get(), 1);
        if candidate == "0.1.0-alpha.3" {
            assert!(plan.products.is_empty());
            assert_eq!(
                plan.pending,
                vec![(product, "0.1.0-alpha.2".into(), candidate.into())]
            );
        } else {
            assert!(plan.pending.is_empty());
            assert_eq!(plan.products, vec![unchanged(product, "0.1.0-alpha.2")]);
            assert_eq!(
                without_consent(plan, Ok(true)),
                vec![unchanged(product, "0.1.0-alpha.2")]
            );
        }
        assert_eq!(
            fs::read(release.join("receipt.json")).unwrap(),
            receipt_before
        );
        assert_eq!(fs::read(release.join(product.executable())).unwrap(), bytes);
        assert_eq!(
            fs::read_link(root.join("current")).unwrap(),
            Path::new(&format!("releases/{id}"))
        );
    }
}

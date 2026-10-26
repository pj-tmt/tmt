use super::*;
use crate::setup_command::guided::tests::Directory;

fn plan() -> Plan {
    Plan {installed:super::super::Plan {products:vec![],pending:vec![]},missing:setup::PRODUCTS.iter().map(|product|
        json!({"product":product.as_str(),"action":"install","channel":"alpha","version":"0.1.0-alpha.2","status":"pending","selected":true,
            "hint":format!("tmt extension install {} --yes",product.as_str())})).collect(),
        providers:vec![json!({"provider":"claude","status":"pending","selected":true,"skills":[],"hooks":[],"hint":"tmt setup claude"})]}
}

#[test]
fn yes_applies_extension_defaults_but_never_new_provider_scopes_and_machine_without_yes_only_reports()
 {
    let mut approved = plan();
    approved.defaults(true);
    assert!(approved.missing.iter().all(|row| row["selected"] == true));
    assert!(
        approved
            .providers
            .iter()
            .all(|row| row["selected"] == false)
    );
    let parsed = Plan::parse(&serde_json::to_vec(&approved.document()).unwrap()).unwrap();
    assert!(approved.same_scope(&parsed));
    approved.defaults(false);
    assert!(
        approved
            .missing
            .iter()
            .chain(&approved.providers)
            .all(|row| row["selected"] == false)
    );
    let (products, providers) = approved.without_consent();
    assert!(
        products
            .iter()
            .all(|row| row["status"] == "consentRequired" && row["hint"].is_string())
    );
    assert_eq!(providers[0]["hint"], "tmt setup claude");
    assert_eq!(providers[0]["status"], "consentRequired");
}

#[test]
fn protocol_refuses_unknown_fields_duplicates_products_and_oversize_before_apply() {
    let original = plan().document();
    assert!(Plan::parse(&serde_json::to_vec(&original).unwrap()).is_some());
    for defect in ["extra", "duplicate", "provider", "version", "unknown field"] {
        let mut document = original.clone();
        match defect {
            "extra" => document["extra"] = true.into(),
            "duplicate" => {
                let duplicate = document["missing"][0].clone();
                document["missing"].as_array_mut().unwrap().push(duplicate);
            }
            "provider" => document["providers"][0]["provider"] = "arbitrary".into(),
            "version" => document["missing"][0]["version"] = "../../payload".into(),
            _ => document["missing"][0]["archive"] = "untrusted".into(),
        }
        assert!(
            Plan::parse(&serde_json::to_vec(&document).unwrap()).is_none(),
            "{defect}"
        );
    }
    assert!(Plan::parse(&vec![b' '; LIMIT + 1]).is_none());
}

#[test]
fn apply_without_selected_items_never_calls_an_installer_or_creates_a_prefix() {
    let root = Directory::new();
    let prefix = root.path.join("custom prefix");
    let mut plan = plan();
    plan.defaults(false);
    let result = apply_with(&prefix, plan, false, &|| false, |_, _, _, _| {
        panic!("no consent")
    });
    assert_eq!(result["providers"][0]["status"], "consentRequired");
    assert!(
        result["products"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["status"] == "consentRequired")
    );
    assert!(!prefix.exists());
}

#[test]
fn a_remote_failure_skips_colab_and_continues_independent_installation() {
    let root = Directory::new();
    let prefix = root.path.join("custom prefix");
    let mut plan = plan();
    plan.defaults(true);
    let mut called = Vec::new();
    let result = apply_with(
        &prefix,
        plan,
        false,
        &|| false,
        |product, actual, version, _| {
            assert_eq!(actual, prefix);
            assert_eq!(version, Some("0.1.0-alpha.2"));
            called.push(product);
            if product == Product::Remote {
                return Err(Failure::new(
                    "EXTENSION_INSTALL_FAILED",
                    "fixture verification refused",
                    1,
                ));
            }
            Ok(json!({"changed":true,"version":version,"skills":{"status":"current"}}))
        },
    );
    assert_eq!(called, [Product::Ops, Product::Remote, Product::Digest]);
    let rows = result["products"].as_array().unwrap();
    assert_eq!(
        rows.iter()
            .map(|r| r["status"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["changed", "failed", "failed", "changed"]
    );
    assert!(
        rows[2]["message"]
            .as_str()
            .unwrap()
            .contains("needs Remote")
    );
    assert_eq!(result["providers"][0]["status"], "consentRequired");
}

#[test]
fn an_in_process_installer_interrupt_keeps_completed_work_and_stops_later_items() {
    let root = Directory::new();
    let prefix = root.path.join("prefix");
    let mut plan = plan();
    plan.defaults(true);
    let mut called = Vec::new();
    let result = apply_with(&prefix, plan, false, &|| false, |product, _, _, _| {
        called.push(product);
        if product == Product::Remote {
            return Err(Failure::new("EXTENSION_INSTALL_FAILED", "interrupted", 130));
        }
        Ok(json!({"changed":true,"skills":{"status":"current"}}))
    });
    assert_eq!(called, [Product::Ops, Product::Remote]);
    assert_eq!(result["interrupted"], true);
    assert_eq!(result["products"][0]["status"], "changed");
    assert_eq!(result["products"][3]["status"], "failed");
}

#[test]
fn a_foreign_entry_that_appears_after_approval_is_preserved_without_an_installer_call() {
    let root = Directory::new();
    let prefix = root.path.join("prefix");
    std::fs::create_dir_all(prefix.join("bin")).unwrap();
    std::fs::write(prefix.join("bin/tmt-ops"), b"user command").unwrap();
    let mut plan = plan();
    plan.defaults(true);
    plan.missing.retain(|row| row["product"] == "ops");
    let result = apply_with(&prefix, plan, false, &|| false, |_, _, _, _| {
        panic!("ownership changed")
    });
    assert_eq!(result["products"][0]["status"], "failed");
    assert_eq!(
        std::fs::read(prefix.join("bin/tmt-ops")).unwrap(),
        b"user command"
    );
}

#[test]
fn missing_plan_uses_shared_inventory_and_a_foreign_command_never_becomes_a_new_install() {
    let root = Directory::new();
    let prefix = root.path.join("custom prefix");
    std::fs::create_dir_all(prefix.join("bin")).unwrap();
    std::fs::write(prefix.join("bin/tmt-ops"), b"user command").unwrap();
    let mut discovered = Vec::new();
    let planned = Plan::read_with(&prefix, |product| {
        discovered.push(product);
        Ok("0.1.0-alpha.2".into())
    });
    assert_eq!(
        discovered,
        [Product::Remote, Product::Colab, Product::Digest]
    );
    assert!(!planned.missing.iter().any(|row| row["product"] == "ops"));
    assert!(
        planned
            .installed
            .products
            .iter()
            .any(|row| row["product"] == "ops" && row["status"] == "failed")
    );
    assert_eq!(
        std::fs::read(prefix.join("bin/tmt-ops")).unwrap(),
        b"user command"
    );
    assert!(!prefix.join("lib").exists());
}

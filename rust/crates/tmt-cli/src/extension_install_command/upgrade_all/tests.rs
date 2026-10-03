use super::*;
use std::cell::Cell;

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
            pending(Product::Office),
            pending(Product::Squad),
            pending(Product::Remote),
            pending(Product::Colab),
        ],
        |question| {
            asked.set(asked.get() + 1);
            assert!(question.contains("office 1.0.0 -> 1.1.0"));
            assert!(question.contains("squad 1.0.0 -> 1.1.0"));
            assert!(question.contains("remote 1.0.0 -> 1.1.0"));
            assert!(question.contains("colab 1.0.0 -> 1.1.0"));
            Ok(true)
        },
        |product, selected| {
            applied.push(product);
            assert_eq!(selected, "1.1.0");
            if product == Product::Office {
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
            Product::Office,
            Product::Squad,
            Product::Remote,
            Product::Colab
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
        vec![pending(Product::Squad)],
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
            pending(Product::Squad),
            pending(Product::Remote),
            pending(Product::Colab),
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
    assert!(Plan::parse(br#"{"products":[{"product":"squad","status":"changed","version":"1.1.0"}],"pending":[]}"#).is_none());
    assert!(
        parse_results(br#"{"products":[{"product":"squad","status":"failed","error":{}}]}"#)
            .is_none()
    );
}

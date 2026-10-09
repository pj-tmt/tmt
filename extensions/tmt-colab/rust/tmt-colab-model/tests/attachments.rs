use serde_json::Value;
use tmt_colab_model::{
    attachment::{AttachmentPublication, AttachmentSelector, Descriptor, Manifest},
    values,
};

#[test]
fn shared_attachment_corpus_matches_canonical_bytes_and_verifies_exact_assets() {
    let corpus: Value = serde_json::from_str(include_str!(
        "../../../contracts/vectors/attachment-v1.json"
    ))
    .unwrap();
    let bytes = |value: &Value| values::binary(value.as_str().unwrap(), 1024 * 1024).unwrap();
    let hex = |value: &[u8]| value.iter().map(|b| format!("{b:02x}")).collect::<String>();
    for case in corpus["cases"].as_array().unwrap() {
        let raw = case["input"].as_str().unwrap().as_bytes();
        let accepted = match case["operation"].as_str().unwrap() {
            "descriptor" => Descriptor::from_json(raw)
                .map(|d| {
                    if case["admit"] == true {
                        assert_eq!(
                            d.to_json().unwrap(),
                            case["canonical"].as_str().unwrap().as_bytes()
                        );
                        assert_eq!(d.input().unwrap(), bytes(&case["inputBytes"]));
                        assert_eq!(hex(&d.hash().unwrap()), case["hash"].as_str().unwrap());
                    }
                })
                .is_ok(),
            "publication" => AttachmentPublication::from_json(raw)
                .map(|p| {
                    if case["admit"] == true {
                        assert_eq!(
                            serde_json::to_vec(&p).unwrap(),
                            case["canonical"].as_str().unwrap().as_bytes()
                        );
                    }
                })
                .is_ok(),
            "publication-binding" => AttachmentPublication::from_json(raw)
                .and_then(|p| {
                    p.matches_descriptor(&Descriptor::from_json(
                        case["descriptor"].as_str().unwrap().as_bytes(),
                    )?)
                })
                .is_ok(),
            "selector" => AttachmentSelector::from_json(raw)
                .map(|selector| {
                    if case["admit"] == true {
                        assert_eq!(
                            serde_json::to_vec(&selector).unwrap(),
                            case["canonical"].as_str().unwrap().as_bytes()
                        );
                    }
                })
                .is_ok(),
            "manifest" => Manifest::from_json(raw)
                .map(|m| {
                    if case.get("inputBytes").is_some() {
                        assert_eq!(m.input().unwrap(), bytes(&case["inputBytes"]));
                        assert_eq!(hex(&m.hash().unwrap()), case["hash"].as_str().unwrap());
                    }
                })
                .is_ok(),
            "open" => Descriptor::from_json(raw)
                .and_then(|d| {
                    let mut context = d.context();
                    if let Some(changes) = case["context"].as_object() {
                        for (key, value) in changes {
                            let value = value.as_str().unwrap().to_owned();
                            match key.as_str() {
                                "space" => context.space = value,
                                "page" => context.page = value,
                                "epoch" => context.epoch = value,
                                "authorDevice" => context.author_device = value,
                                "membershipRevision" => context.membership_revision = value,
                                _ => panic!("unknown fixture context field"),
                            }
                        }
                    }
                    let secret = bytes(case.get("secret").unwrap_or(&corpus["secret"]))
                        .try_into()
                        .unwrap();
                    let key = bytes(case.get("publicKey").unwrap_or(&corpus["publicKey"]))
                        .try_into()
                        .unwrap();
                    let plain = d.open(&bytes(&case["payload"]), &context, &secret, &key)?;
                    assert_eq!(plain, bytes(&corpus["plaintext"]));
                    Ok(())
                })
                .is_ok(),
            "document" | "comment" => continue,
            other => panic!("unknown fixture operation {other}"),
        };
        assert_eq!(
            accepted,
            case["admit"].as_bool().unwrap(),
            "{}",
            case["name"]
        );
    }
}

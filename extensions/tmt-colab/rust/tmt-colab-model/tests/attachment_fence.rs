use serde_json::Value;
use tmt_colab_model::attachment::message_fence;

fn field<'a>(case: &'a Value, name: &str) -> &'a str {
    case[name].as_str().unwrap()
}
fn hash(case: &Value) -> [u8; 32] {
    let text = field(case, "headHash");
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[2 * i..2 * i + 2], 16).unwrap();
    }
    out
}
fn compute(case: &Value) -> tmt_colab_model::Result<String> {
    message_fence(
        field(case, "space"),
        field(case, "page"),
        field(case, "epoch"),
        field(case, "revision"),
        &hash(case),
        field(case, "author"),
    )
}

#[test]
fn shared_fence_vectors_match_the_independent_oracle() {
    let corpus: Value = serde_json::from_str(include_str!(
        "../../../contracts/vectors/attachment-fence-v1.json"
    ))
    .unwrap();
    let cases = corpus["cases"].as_array().unwrap();
    assert!(cases.len() >= 7);
    for case in cases {
        assert_eq!(compute(case).unwrap(), field(case, "fence"), "{case}");
    }
}

#[test]
fn a_fence_refuses_malformed_inputs_instead_of_hashing_them() {
    let corpus: Value = serde_json::from_str(include_str!(
        "../../../contracts/vectors/attachment-fence-v1.json"
    ))
    .unwrap();
    let base = corpus["cases"][0].clone();
    for (name, value) in [
        ("space", "short"),
        ("page", "not-an-id"),
        ("epoch", "0"),
        ("revision", "01"),
        ("author", "x"),
    ] {
        let mut case = base.clone();
        case[name] = Value::from(value);
        assert!(compute(&case).is_err(), "{name}");
    }
}

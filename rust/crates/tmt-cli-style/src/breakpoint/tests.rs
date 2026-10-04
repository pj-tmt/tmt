use super::*;

#[test]
fn table_equals_the_design_tokens_file() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../design/tokens/tokens.json");
    let tokens: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let file = tokens["breakpoint"].as_object().unwrap();
    // Compare by name, so the file's key order is free; the table stays ascending.
    assert_eq!(file.len(), ALL.len());
    for step in ALL {
        assert_eq!(
            file[step.name]["cells"].as_u64(),
            Some(u64::from(step.cells)),
            "{}",
            step.name
        );
    }
    assert!(ALL.windows(2).all(|pair| pair[0].cells < pair[1].cells));
    assert_eq!([SM, MD, LG], ALL);
}

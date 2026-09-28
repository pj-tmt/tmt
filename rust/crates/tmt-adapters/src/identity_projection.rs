//! Identity JSON projections shared by CLI commands and the local API.

use serde_json::{Value, json};
use std::collections::BTreeMap;
use tmt_core::identity::Identity;

pub fn identity_value(identity: &Identity) -> Value {
    json!({"id": identity.id, "name": identity.name,
        "canonicalName": identity.canonical_name, "lifetime": identity.lifetime.as_str()})
}

/// Keys keep the storage owner's binary order.
pub fn metadata_value(metadata: &BTreeMap<String, String>) -> Value {
    json!(metadata)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tmt_core::identity::Lifetime;

    // Literal bytes of the projections the CLI emitted before these encoders
    // moved here. Key order is public output under serde_json preserve_order.
    #[test]
    fn projections_keep_the_published_cli_bytes() {
        let identity = Identity {
            id: "11111111-1111-4111-8111-111111111111".into(),
            name: "Ada \"Q\"".into(),
            canonical_name: "ada \"q\"".into(),
            lifetime: Lifetime::Temporary,
            created_at: "excluded".into(),
            updated_at: "excluded".into(),
        };
        assert_eq!(
            serde_json::to_string(&identity_value(&identity)).unwrap(),
            r#"{"id":"11111111-1111-4111-8111-111111111111","name":"Ada \"Q\"","canonicalName":"ada \"q\"","lifetime":"temporary"}"#
        );
        let metadata = BTreeMap::from([
            ("team".to_owned(), "core".to_owned()),
            ("squad.a.state".to_owned(), "é\t".to_owned()),
        ]);
        assert_eq!(
            serde_json::to_string(&metadata_value(&metadata)).unwrap(),
            r#"{"squad.a.state":"é\t","team":"core"}"#
        );
        assert_eq!(
            serde_json::to_string(&metadata_value(&BTreeMap::new())).unwrap(),
            "{}"
        );
    }
}

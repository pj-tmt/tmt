//! Remote's native carrier namespace. Not an extension fragment or deploy-plan activation.
/// Emulator qualification artifact. Its cap is generated from the same named document bound.
pub fn artifact() -> String {
    include_str!("../../assets/cloud-inbox.rules").replace(
        "CLOUD_CARRIER_BYTES",
        &crate::limits::CLOUD_CARRIER_BYTES.to_string(),
    )
}
#[cfg(test)]
mod tests {
    #[test]
    fn artifact_has_the_same_carrier_bound_and_owner_only_deletion() {
        let rules = super::artifact();
        assert!(!rules.contains("CLOUD_CARRIER_BYTES"));
        assert!(rules.contains(&format!("<= {}", crate::limits::CLOUD_CARRIER_BYTES)));
        assert_eq!(
            rules
                .matches("allow delete: if machineOwner(machine);")
                .count(),
            2
        );
        assert!(!rules.contains("allow update"));
        assert!(rules.contains("match /m/{machine}/inbox/{id}"));
        assert!(rules.contains("peerCreate(machine)"));
        assert!(rules.contains("peerRead(machine)"));
        assert_eq!(
            rules.matches("allow list:").count(),
            1,
            "responses cannot list"
        );
        assert!(!rules.contains("{tail=**}"));
    }
}

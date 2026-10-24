//! Remote-owned native policy. Extension fragments cannot supply or replace this namespace.
/// Qualification uses the ordinary deployment composer, including its final default deny.
pub fn artifact() -> String {
    let plan = crate::deploy_plan::compose(
        crate::deploy_plan::Target {
            backend: crate::deploy_plan::CloudBackend::Firestore,
            physical_ttl: false,
        },
        &[],
    )
    .expect("fixed native Rules plan");
    crate::rules::compose(&plan, &[])
        .expect("fixed native Rules composition")
        .rules
}

/// Trusted built-in body only; untrusted extension bytes still use the confined parser.
pub(crate) fn namespace() -> String {
    include_str!("../../assets/cloud-inbox.rules")
        .replace(
            "CLOUD_CARRIER_BYTES",
            &crate::limits::CLOUD_CARRIER_BYTES.to_string(),
        )
        .replace(
            "CLOUD_INBOX_PAGE_DOCUMENTS",
            &crate::limits::CLOUD_INBOX_PAGE_DOCUMENTS.to_string(),
        )
        .replace(
            "CLOUD_DESCRIPTOR_BYTES",
            &crate::limits::CLOUD_DESCRIPTOR_BYTES.to_string(),
        )
        .replace(
            "CLOUD_ENROLLMENT_BYTES",
            &crate::limits::CLOUD_ENROLLMENT_BYTES.to_string(),
        )
}
#[cfg(test)]
mod tests {
    #[test]
    fn artifact_has_the_same_carrier_bound_and_owner_only_deletion() {
        let rules = super::artifact();
        assert!(!rules.contains("CLOUD_CARRIER_BYTES"));
        assert!(!rules.contains("CLOUD_INBOX_PAGE_DOCUMENTS"));
        assert!(rules.contains("request.query.limit >= 1 && request.query.limit <= 4"));
        assert_eq!(
            rules.matches("request.query.limit == 1").count(),
            1,
            "enrollment retains its one-document cap"
        );
        assert!(rules.contains(&format!("<= {}", crate::limits::CLOUD_CARRIER_BYTES)));
        assert_eq!(
            rules
                .matches("allow delete: if machineOwner(machine);")
                .count(),
            4
        );
        assert!(rules.contains(
            "request.resource.data.authenticationKey == resource.data.authenticationKey"
        ));
        assert!(rules.contains("match /m/{machine}/inbox/{id}"));
        assert!(rules.contains("peerCreate(machine)"));
        assert!(rules.contains("peerRead(machine)"));
        assert_eq!(
            rules.matches("allow list:").count(),
            2,
            "only machine inbox and enrollment can list"
        );
        assert!(!rules.contains("{tail=**}"));
    }
}

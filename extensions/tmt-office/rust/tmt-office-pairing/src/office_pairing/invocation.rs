//! One-shot companion composition. Only allowlisted public data leaves this owner.

use super::{
    AgentCredential, Approval, OfficeError, OfficeInstallation, PairingCore, PairingIdentity,
    PairingRecord, Proof, ProtectedEntry, RetirementFence, claim_pairing, fence::write_authority,
};
use crate::office_deployment::{DeploymentMode, OfficeDeployment, WorldTarget};
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tmt_adapters::config::ConfigPaths;
use tmt_office_model::office_protocol::OfficeInvocation;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Request {
    world: String,
    identity_id: String,
    emulator: bool,
    read_only: bool,
    #[serde(default)]
    block_id: Option<String>,
    #[serde(default)]
    layout: Option<tmt_office_model::codec::office_block::LayoutInput>,
    #[serde(default)]
    expected_revision: Option<u64>,
}

/// Runs one pairing operation. Retirement delivery (`sync`) belongs to the
/// companion's consumer, which settles each scope through [`super::settle_scope`].
pub fn execute(
    operation: OfficeInvocation,
    bytes: &[u8],
    core: &dyn PairingCore,
    fence: &dyn RetirementFence,
) -> Vec<u8> {
    let result = run(operation, bytes, core, fence);
    let value = result.unwrap_or_else(|error| json!({"error":error.code()}));
    serde_json::to_vec(&value).expect("public Office result is JSON data")
}

fn run(
    operation: OfficeInvocation,
    bytes: &[u8],
    core: &dyn PairingCore,
    fence: &dyn RetirementFence,
) -> Result<Value, OfficeError> {
    if bytes.len() > 4096 || matches!(operation, OfficeInvocation::Probe | OfficeInvocation::Sync) {
        return Err(OfficeError::CredentialsInvalid);
    }
    let mut request: Request =
        serde_json::from_slice(bytes).map_err(|_| OfficeError::CredentialsInvalid)?;
    let block = matches!(
        operation,
        OfficeInvocation::BlockShow | OfficeInvocation::BlockApply
    );
    let raw: Value = serde_json::from_slice(bytes).map_err(|_| OfficeError::CredentialsInvalid)?;
    let fields = raw.as_object().ok_or(OfficeError::CredentialsInvalid)?;
    let expected_fields = if operation == OfficeInvocation::BlockApply {
        7
    } else if block {
        5
    } else {
        4
    };
    if fields.len() != expected_fields
        || (block && (!fields.contains_key("blockId") || request.read_only))
        || (!block
            && ["blockId", "layout", "expectedRevision"]
                .iter()
                .any(|key| fields.contains_key(*key)))
    {
        return Err(OfficeError::CredentialsInvalid);
    }
    let edit = match operation {
        OfficeInvocation::BlockApply => {
            let revision = request
                .expected_revision
                .filter(|value| *value < tmt_office_model::office_block::MAX_REVISION)
                .ok_or(OfficeError::LayoutInvalid)?;
            let layout = request
                .layout
                .take()
                .ok_or(OfficeError::LayoutInvalid)?
                .validate()?;
            Some((layout, revision))
        }
        _ if request.layout.is_some() || request.expected_revision.is_some() => {
            return Err(OfficeError::CredentialsInvalid);
        }
        _ => None,
    };
    let mode = if request.emulator {
        DeploymentMode::Emulator
    } else {
        DeploymentMode::Cloud
    };
    let target =
        WorldTarget::parse(&request.world, mode).map_err(|_| OfficeError::DeploymentInvalid)?;
    let paths = ConfigPaths::discover().map_err(|_| OfficeError::CredentialsUnavailable)?;
    let identity = active_identity(core, &request.identity_id)?;
    let create = operation == OfficeInvocation::PairBegin;
    let Some(installation) = OfficeInstallation::open(&paths, create)? else {
        return if operation == OfficeInvocation::PairStatus {
            Ok(json!({"state":"unpaired"}))
        } else {
            Err(OfficeError::NotPaired)
        };
    };
    let deadline = Instant::now() + Duration::from_secs(25);
    let operate = |key: &str| {
        let entry = ProtectedEntry::open(key)?;
        let existing = entry
            .read()?
            .map(|bytes| PairingRecord::decode(&bytes, &target, installation.id(), &identity.id))
            .transpose()?;
        if existing.is_some() && operation != OfficeInvocation::PairStatus {
            core.register_retirement_hook(&identity.id, key)?;
            active_identity(core, &identity.id)?;
        }
        match operation {
            OfficeInvocation::Unpair => {
                let mut record = existing.ok_or(OfficeError::NotPaired)?;
                if record.has_credentials()
                    && record.refresh_if_needed(&target, now_ms()?, deadline)?
                {
                    entry.write(&record.encode()?)?;
                }
                match record.revoke(&target, deadline) {
                    Err(OfficeError::RemoteDenied) if record.is_pending() => {
                        // Unknown or denied pending approval is not confirmed cleanup.
                        return Ok(
                            json!({"state":"pending","approvalUrl":record.approval().approval_url(&target)?}),
                        );
                    }
                    result => result?,
                }
                entry.write(&record.encode()?)?;
                Ok(json!({"state":"revoked"}))
            }
            OfficeInvocation::Inspect
            | OfficeInvocation::BlockShow
            | OfficeInvocation::BlockApply => {
                let mut record = existing.ok_or(OfficeError::NotPaired)?;
                if record.refresh_if_needed(&target, now_ms()?, deadline)? {
                    active_identity(core, &identity.id)?;
                    write_authority(fence, &identity.id, || entry.write(&record.encode()?))?;
                }
                active_identity(core, &identity.id)?;
                if record.renew_if_needed(&target, now_ms()?, deadline)? {
                    active_identity(core, &identity.id)?;
                    write_authority(fence, &identity.id, || entry.write(&record.encode()?))?;
                }
                active_identity(core, &identity.id)?;
                if block {
                    let snapshot = record.block(
                        &target,
                        now_ms()?,
                        request.block_id.as_deref(),
                        edit.as_ref().map(|(layout, revision)| (layout, *revision)),
                        deadline,
                    )?;
                    Ok(snapshot.wire_value())
                } else {
                    let exists = record.inspect(&target, now_ms()?, deadline)?;
                    Ok(json!({"blockExists":exists}))
                }
            }
            OfficeInvocation::PairStatus => {
                let now = now_ms()?;
                Ok(
                    json!({"state":existing.as_ref().map_or("unpaired", |record| record.local_state(now))}),
                )
            }
            OfficeInvocation::PairBegin => {
                let record = match existing {
                    Some(record) if !record.is_revoked() => {
                        if record.approval().read_only() != request.read_only {
                            return Err(OfficeError::CredentialsInvalid);
                        }
                        record
                    }
                    _ => {
                        let deployment = OfficeDeployment::discover(target.clone(), deadline)
                            .map_err(|error| {
                                if error.kind() == std::io::ErrorKind::InvalidData {
                                    OfficeError::DeploymentInvalid
                                } else {
                                    OfficeError::RemoteUncertain
                                }
                            })?;
                        let proof = Proof::generate()?;
                        let approval = Approval::new(
                            &target,
                            installation.id(),
                            &identity.id,
                            &identity.name,
                            request.read_only,
                            &proof,
                        )?;
                        let record =
                            PairingRecord::pending(&deployment, approval, proof, now_ms()?)?;
                        active_identity(core, &identity.id)?;
                        write_authority(fence, &identity.id, || entry.write(&record.encode()?))?;
                        core.register_retirement_hook(&identity.id, key)?;
                        active_identity(core, &identity.id)?;
                        record
                    }
                };
                match record.local_state(now_ms()?) {
                    "pending" => Ok(
                        json!({"state":"pending","approvalUrl":record.approval().approval_url(&target)?}),
                    ),
                    "credential" => Ok(json!({"state":"credential"})),
                    _ => Err(OfficeError::PairingExpired),
                }
            }
            OfficeInvocation::PairPoll => {
                let mut record = existing.ok_or(OfficeError::NotPaired)?;
                if record.local_state(now_ms()?) == "credential" {
                    return Ok(json!({"state":"credential"}));
                }
                let proof = record.reserve_claim(now_ms()?)?;
                write_authority(fence, &identity.id, || entry.write(&record.encode()?))?;
                active_identity(core, &identity.id)?;
                let deployment = record.deployment(&target)?;
                let claim =
                    claim_pairing(&deployment, &proof, record.approval(), now_ms()?, deadline)?;
                let credential = AgentCredential::exchange(
                    &deployment,
                    record.approval(),
                    &claim,
                    now_ms()?,
                    deadline,
                )?;
                active_identity(core, &identity.id)?;
                record.complete(claim, credential, &target)?;
                write_authority(fence, &identity.id, || entry.write(&record.encode()?))?;
                active_identity(core, &identity.id)?;
                Ok(json!({"state":"credential"}))
            }
            OfficeInvocation::Probe | OfficeInvocation::Capabilities | OfficeInvocation::Sync => {
                Err(OfficeError::CredentialsInvalid)
            }
            OfficeInvocation::LocalWorldShow
            | OfficeInvocation::LocalWorldApply
            | OfficeInvocation::LocalProfileShow
            | OfficeInvocation::LocalProfileApply
            | OfficeInvocation::LocalPropValidate
            | OfficeInvocation::LocalPropInstall
            | OfficeInvocation::LocalPropRemove
            | OfficeInvocation::LocalPropList
            | OfficeInvocation::LocalPropShow
            | OfficeInvocation::LocalAvatarValidate
            | OfficeInvocation::LocalAvatarInstall
            | OfficeInvocation::LocalAvatarRemove
            | OfficeInvocation::LocalAvatarList
            | OfficeInvocation::LocalAvatarShow
            | OfficeInvocation::LocalExtensionValidate
            | OfficeInvocation::BoardPost
            | OfficeInvocation::BoardList
            | OfficeInvocation::BoardShow
            | OfficeInvocation::BoardReply
            | OfficeInvocation::BoardEdit
            | OfficeInvocation::BoardDelete
            | OfficeInvocation::BoardCategories
            | OfficeInvocation::WhiteboardSnapshotShow
            | OfficeInvocation::WhiteboardSnapshotImage
            | OfficeInvocation::StoragePlan
            | OfficeInvocation::StorageMigrate => Err(OfficeError::CredentialsInvalid),
        }
    };
    if operation == OfficeInvocation::PairStatus {
        operate(&installation.scope_key(&target, &identity.id)?)
    } else {
        installation.with_scope(&target, &identity.id, operate)
    }
}

fn active_identity(core: &dyn PairingCore, id: &str) -> Result<PairingIdentity, OfficeError> {
    core.active_identity(id)?.ok_or(OfficeError::NotPaired)
}

pub(super) fn now_ms() -> Result<u64, OfficeError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| OfficeError::CredentialsUnavailable)?
        .as_millis()
        .try_into()
        .map_err(|_| OfficeError::CredentialsUnavailable)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scoped() -> Value {
        // Invalid deployment is an intentional downstream sentinel: valid input
        // reaches deployment validation without reading any host identity/vault.
        json!({"world":"invalid", "identityId":"identity", "emulator":false, "readOnly":false})
    }

    /// Input validation fails before any core or fence query.
    struct Unused;

    impl PairingCore for Unused {
        fn active_identity(&self, _: &str) -> Result<Option<PairingIdentity>, OfficeError> {
            unreachable!("validation precedes core reads")
        }
        fn register_retirement_hook(&self, _: &str, _: &str) -> Result<(), OfficeError> {
            unreachable!("validation precedes core writes")
        }
    }

    impl RetirementFence for Unused {
        fn is_retired(&self, _: &str) -> Result<bool, OfficeError> {
            unreachable!("validation precedes the fence")
        }
        fn mark_retired(&self, _: &str) -> Result<(), OfficeError> {
            unreachable!("validation precedes the fence")
        }
    }

    fn error(operation: OfficeInvocation, value: &Value) -> OfficeError {
        run(
            operation,
            &serde_json::to_vec(value).unwrap(),
            &Unused,
            &Unused,
        )
        .unwrap_err()
    }

    /// Retirement delivery moved to the companion's consumer; pairing never
    /// opens core storage, so the port is its only route to core.
    #[test]
    fn pairing_never_opens_core_storage_and_leaves_sync_to_the_companion() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = vec![root.join("office_pairing.rs")];
        let mut directories = vec![root.join("office_pairing")];
        while let Some(directory) = directories.pop() {
            for entry in std::fs::read_dir(directory).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    directories.push(path);
                } else if path.extension().is_some_and(|extension| extension == "rs")
                    && !path.to_string_lossy().ends_with("_tests.rs")
                {
                    files.push(path);
                }
            }
        }
        assert!(files.len() > 8, "{files:?}");
        for file in files {
            let source = std::fs::read_to_string(&file).unwrap();
            let production = source.split("#[cfg(test)]").next().unwrap();
            for forbidden in ["storage::", "Storage::", "Storage,", "paths.database"] {
                assert!(
                    !production.contains(forbidden),
                    "{} uses {forbidden}",
                    file.display()
                );
            }
        }
        assert_eq!(
            run(OfficeInvocation::Sync, b"{}", &Unused, &Unused),
            Err(OfficeError::CredentialsInvalid)
        );
    }

    /// Every grant or extension of pairing authority goes through the fence;
    /// only unpair writes the record without it, because revocation reduces
    /// authority. The protected store is the real OS keychain with no test
    /// double, so this pins each gated site in the source.
    #[test]
    fn every_authority_write_is_fenced_and_only_unpair_is_exempt() {
        let source = include_str!("invocation.rs");
        let production = source.split("#[cfg(test)]").next().unwrap();
        let unpair_start = production.find("OfficeInvocation::Unpair =>").unwrap();
        let unpair_end = production.find("OfficeInvocation::Inspect").unwrap();
        let mut gated = Vec::new();
        let mut exempt = 0;
        for (offset, _) in production.match_indices("entry.write(") {
            let line_start = production[..offset].rfind('\n').unwrap() + 1;
            let line = &production[line_start..offset];
            if line
                .trim_start()
                .starts_with("write_authority(fence, &identity.id, || ")
            {
                let arm = [
                    "refresh_if_needed",
                    "renew_if_needed",
                    "PairingRecord::pending",
                    "reserve_claim",
                    "record.complete",
                ]
                .into_iter()
                .filter(|marker| {
                    production[..offset]
                        .rfind(marker)
                        .is_some_and(|at| offset - at < 400)
                })
                .max_by_key(|marker| production[..offset].rfind(marker))
                .unwrap_or("unknown");
                gated.push(arm);
            } else if (unpair_start..unpair_end).contains(&offset) {
                exempt += 1;
            } else {
                panic!("ungated pairing write: {line}entry.write(");
            }
        }
        gated.sort_unstable();
        assert_eq!(
            gated,
            [
                "PairingRecord::pending",
                "record.complete",
                "refresh_if_needed",
                "renew_if_needed",
                "reserve_claim"
            ]
        );
        assert_eq!(exempt, 2);
    }

    #[test]
    fn block_input_is_typed_and_does_not_expand_pairing_inputs() {
        let mut show = scoped();
        assert_eq!(
            error(OfficeInvocation::Inspect, &show),
            OfficeError::DeploymentInvalid
        );
        show["blockId"] = Value::Null;
        assert_eq!(
            error(OfficeInvocation::BlockShow, &show),
            OfficeError::DeploymentInvalid
        );
        assert_eq!(
            error(OfficeInvocation::Inspect, &show),
            OfficeError::CredentialsInvalid
        );
        show["layout"] = json!({"objects":[]});
        show["expectedRevision"] = json!(0);
        assert_eq!(
            error(OfficeInvocation::BlockApply, &show),
            OfficeError::DeploymentInvalid
        );
        assert_eq!(
            error(OfficeInvocation::BlockShow, &show),
            OfficeError::CredentialsInvalid
        );
        show["expectedRevision"] = json!(tmt_office_model::office_block::MAX_REVISION);
        assert_eq!(
            error(OfficeInvocation::BlockApply, &show),
            OfficeError::LayoutInvalid
        );
        show["expectedRevision"] = json!(0);
        show["layout"] = json!({"objects":[{"asset":"rug","rotation":0,"x":31,"y":31}]});
        assert_eq!(
            error(OfficeInvocation::BlockApply, &show),
            OfficeError::LayoutInvalid
        );
    }

    #[test]
    fn unknown_duplicate_and_oversized_payloads_fail_before_scope_access() {
        let mut value = scoped();
        value["blockId"] = Value::Null;
        value["unknown"] = json!(true);
        assert_eq!(
            error(OfficeInvocation::BlockShow, &value),
            OfficeError::CredentialsInvalid
        );
        let duplicate = br#"{"world":"invalid","identityId":"identity","emulator":false,"readOnly":false,"blockId":null,"layout":{"objects":[],"objects":[]},"expectedRevision":0}"#;
        assert_eq!(
            run(OfficeInvocation::BlockApply, duplicate, &Unused, &Unused),
            Err(OfficeError::CredentialsInvalid)
        );
        assert_eq!(
            run(
                OfficeInvocation::BlockShow,
                &vec![b' '; 4097],
                &Unused,
                &Unused
            ),
            Err(OfficeError::CredentialsInvalid)
        );
    }
}

//! Versioned internal companion handshake, not terminal output recognition.

use semver::Version;

pub const OFFICE_PROTOCOL_VERSION: &str = "1";
pub const OFFICE_PROTOCOL_OUTPUT_LIMIT: usize = 1024;
pub const OFFICE_HOOK_BATCH_LIMIT: usize = 16;
pub const OFFICE_BOARD_CAPABILITY: &str = "office_board_v1";

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OfficeSyncReport {
    pub completed: u64,
    pub failed: u64,
    pub pending: u64,
    pub failure: Option<OfficeError>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfficeError {
    DeploymentInvalid,
    CredentialsUnavailable,
    CredentialsInvalid,
    NotPaired,
    PairingPending,
    PairingExpired,
    RemoteDenied,
    RemoteUncertain,
    LayoutInvalid,
    ProfileInvalid,
    PropInvalid,
    PropCorrupt,
    PropNotFound,
    PropLimit,
    PropBuiltin,
    AvatarInvalid,
    AvatarCorrupt,
    AvatarNotFound,
    AvatarLimit,
    AvatarCatalogRevisionConflict,
    AvatarCatalogCursorInvalid,
    AvatarCatalogCursorStale,
    CatalogRevisionConflict,
    CatalogCursorInvalid,
    CatalogCursorStale,
    RevisionConflict,
    IdentityInactive,
    WhiteboardInvalid,
    WhiteboardNotFound,
    StorageUnavailable,
    Busy,
}

impl OfficeError {
    pub fn code(self) -> &'static str {
        match self {
            Self::DeploymentInvalid => "OFFICE_DEPLOYMENT_INVALID",
            Self::CredentialsUnavailable => "OFFICE_CREDENTIALS_UNAVAILABLE",
            Self::CredentialsInvalid => "OFFICE_CREDENTIALS_INVALID",
            Self::NotPaired => "OFFICE_NOT_PAIRED",
            Self::PairingPending => "OFFICE_PAIRING_PENDING",
            Self::PairingExpired => "OFFICE_PAIRING_EXPIRED",
            Self::RemoteDenied => "OFFICE_REMOTE_DENIED",
            Self::RemoteUncertain => "OFFICE_REMOTE_UNCERTAIN",
            Self::LayoutInvalid => "OFFICE_LAYOUT_INVALID",
            Self::ProfileInvalid => "OFFICE_PROFILE_INVALID",
            Self::PropInvalid => "OFFICE_PROP_INVALID",
            Self::PropCorrupt => "OFFICE_PROP_CORRUPT",
            Self::PropNotFound => "OFFICE_PROP_NOT_FOUND",
            Self::PropLimit => "OFFICE_PROP_LIMIT",
            Self::PropBuiltin => "OFFICE_PROP_BUILTIN",
            Self::AvatarInvalid => "OFFICE_AVATAR_INVALID",
            Self::AvatarCorrupt => "OFFICE_AVATAR_CORRUPT",
            Self::AvatarNotFound => "OFFICE_AVATAR_NOT_FOUND",
            Self::AvatarLimit => "OFFICE_AVATAR_LIMIT",
            Self::AvatarCatalogRevisionConflict => "OFFICE_AVATAR_CATALOG_REVISION_CONFLICT",
            Self::AvatarCatalogCursorInvalid => "OFFICE_AVATAR_CATALOG_CURSOR_INVALID",
            Self::AvatarCatalogCursorStale => "OFFICE_AVATAR_CATALOG_CURSOR_STALE",
            Self::CatalogRevisionConflict => "OFFICE_CATALOG_REVISION_CONFLICT",
            Self::CatalogCursorInvalid => "OFFICE_CATALOG_CURSOR_INVALID",
            Self::CatalogCursorStale => "OFFICE_CATALOG_CURSOR_STALE",
            Self::RevisionConflict => "OFFICE_REVISION_CONFLICT",
            Self::IdentityInactive => "OFFICE_IDENTITY_INACTIVE",
            Self::WhiteboardInvalid => "WHITEBOARD_INVALID",
            Self::WhiteboardNotFound => "WHITEBOARD_NOT_FOUND",
            Self::StorageUnavailable => "STORAGE_UNAVAILABLE",
            Self::Busy => "OFFICE_BUSY",
        }
    }

    pub fn parse(code: &str) -> Option<Self> {
        [
            Self::DeploymentInvalid,
            Self::CredentialsUnavailable,
            Self::CredentialsInvalid,
            Self::NotPaired,
            Self::PairingPending,
            Self::PairingExpired,
            Self::RemoteDenied,
            Self::RemoteUncertain,
            Self::LayoutInvalid,
            Self::ProfileInvalid,
            Self::PropInvalid,
            Self::PropCorrupt,
            Self::PropNotFound,
            Self::PropLimit,
            Self::PropBuiltin,
            Self::AvatarInvalid,
            Self::AvatarCorrupt,
            Self::AvatarNotFound,
            Self::AvatarLimit,
            Self::AvatarCatalogRevisionConflict,
            Self::AvatarCatalogCursorInvalid,
            Self::AvatarCatalogCursorStale,
            Self::CatalogRevisionConflict,
            Self::CatalogCursorInvalid,
            Self::CatalogCursorStale,
            Self::RevisionConflict,
            Self::IdentityInactive,
            Self::WhiteboardInvalid,
            Self::WhiteboardNotFound,
            Self::StorageUnavailable,
            Self::Busy,
        ]
        .into_iter()
        .find(|value| value.code() == code)
    }
}

impl std::fmt::Display for OfficeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}
impl std::error::Error for OfficeError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfficeInvocation {
    Probe,
    Capabilities,
    PairBegin,
    PairPoll,
    PairStatus,
    Unpair,
    Inspect,
    Sync,
    BlockShow,
    BlockApply,
    LocalWorldShow,
    LocalWorldApply,
    LocalProfileShow,
    LocalProfileApply,
    LocalPropValidate,
    LocalPropInstall,
    LocalPropRemove,
    LocalPropList,
    LocalPropShow,
    LocalAvatarValidate,
    LocalAvatarInstall,
    LocalAvatarRemove,
    LocalAvatarList,
    LocalAvatarShow,
    LocalExtensionValidate,
    BoardPost,
    BoardList,
    BoardShow,
    BoardReply,
    BoardEdit,
    BoardDelete,
    BoardCategories,
    WhiteboardSnapshotShow,
    WhiteboardSnapshotImage,
}

impl OfficeInvocation {
    pub fn arguments(self) -> [&'static str; 3] {
        match self {
            Self::Probe => ["__tmt-office", OFFICE_PROTOCOL_VERSION, "probe"],
            Self::Capabilities => ["__tmt-office", OFFICE_PROTOCOL_VERSION, "capabilities"],
            Self::PairBegin => ["__tmt-office", OFFICE_PROTOCOL_VERSION, "pair-begin"],
            Self::PairPoll => ["__tmt-office", OFFICE_PROTOCOL_VERSION, "pair-poll"],
            Self::PairStatus => ["__tmt-office", OFFICE_PROTOCOL_VERSION, "pair-status"],
            Self::Unpair => ["__tmt-office", OFFICE_PROTOCOL_VERSION, "unpair"],
            Self::Inspect => ["__tmt-office", OFFICE_PROTOCOL_VERSION, "inspect"],
            Self::Sync => ["__tmt-office", OFFICE_PROTOCOL_VERSION, "sync"],
            Self::BlockShow => ["__tmt-office", OFFICE_PROTOCOL_VERSION, "block-show"],
            Self::BlockApply => ["__tmt-office", OFFICE_PROTOCOL_VERSION, "block-apply"],
            Self::LocalWorldShow => ["__tmt-office", OFFICE_PROTOCOL_VERSION, "local-world-show"],
            Self::LocalWorldApply => ["__tmt-office", OFFICE_PROTOCOL_VERSION, "local-world-apply"],
            Self::LocalProfileShow => [
                "__tmt-office",
                OFFICE_PROTOCOL_VERSION,
                "local-profile-show",
            ],
            Self::LocalProfileApply => [
                "__tmt-office",
                OFFICE_PROTOCOL_VERSION,
                "local-profile-apply",
            ],
            Self::LocalPropValidate => [
                "__tmt-office",
                OFFICE_PROTOCOL_VERSION,
                "local-prop-validate",
            ],
            Self::LocalPropInstall => [
                "__tmt-office",
                OFFICE_PROTOCOL_VERSION,
                "local-prop-install",
            ],
            Self::LocalPropRemove => ["__tmt-office", OFFICE_PROTOCOL_VERSION, "local-prop-remove"],
            Self::LocalPropList => ["__tmt-office", OFFICE_PROTOCOL_VERSION, "local-prop-list"],
            Self::LocalPropShow => ["__tmt-office", OFFICE_PROTOCOL_VERSION, "local-prop-show"],
            Self::LocalAvatarValidate => [
                "__tmt-office",
                OFFICE_PROTOCOL_VERSION,
                "local-avatar-validate",
            ],
            Self::LocalAvatarInstall => [
                "__tmt-office",
                OFFICE_PROTOCOL_VERSION,
                "local-avatar-install",
            ],
            Self::LocalAvatarRemove => [
                "__tmt-office",
                OFFICE_PROTOCOL_VERSION,
                "local-avatar-remove",
            ],
            Self::LocalAvatarList => ["__tmt-office", OFFICE_PROTOCOL_VERSION, "local-avatar-list"],
            Self::LocalAvatarShow => ["__tmt-office", OFFICE_PROTOCOL_VERSION, "local-avatar-show"],
            Self::LocalExtensionValidate => [
                "__tmt-office",
                OFFICE_PROTOCOL_VERSION,
                "local-extension-validate",
            ],
            Self::BoardPost => ["__tmt-office", OFFICE_PROTOCOL_VERSION, "board-post"],
            Self::BoardList => ["__tmt-office", OFFICE_PROTOCOL_VERSION, "board-list"],
            Self::BoardShow => ["__tmt-office", OFFICE_PROTOCOL_VERSION, "board-show"],
            Self::BoardReply => ["__tmt-office", OFFICE_PROTOCOL_VERSION, "board-reply"],
            Self::BoardEdit => ["__tmt-office", OFFICE_PROTOCOL_VERSION, "board-edit"],
            Self::BoardDelete => ["__tmt-office", OFFICE_PROTOCOL_VERSION, "board-delete"],
            Self::BoardCategories => ["__tmt-office", OFFICE_PROTOCOL_VERSION, "board-categories"],
            Self::WhiteboardSnapshotShow => [
                "__tmt-office",
                OFFICE_PROTOCOL_VERSION,
                "whiteboard-snapshot-show",
            ],
            Self::WhiteboardSnapshotImage => [
                "__tmt-office",
                OFFICE_PROTOCOL_VERSION,
                "whiteboard-snapshot-image",
            ],
        }
    }

    pub fn parse(arguments: &[&str]) -> Result<Self, &'static str> {
        [
            Self::Probe,
            Self::Capabilities,
            Self::PairBegin,
            Self::PairPoll,
            Self::PairStatus,
            Self::Unpair,
            Self::Inspect,
            Self::Sync,
            Self::BlockShow,
            Self::BlockApply,
            Self::LocalWorldShow,
            Self::LocalWorldApply,
            Self::LocalProfileShow,
            Self::LocalProfileApply,
            Self::LocalPropValidate,
            Self::LocalPropInstall,
            Self::LocalPropRemove,
            Self::LocalPropList,
            Self::LocalPropShow,
            Self::LocalAvatarValidate,
            Self::LocalAvatarInstall,
            Self::LocalAvatarRemove,
            Self::LocalAvatarList,
            Self::LocalAvatarShow,
            Self::LocalExtensionValidate,
            Self::BoardPost,
            Self::BoardList,
            Self::BoardShow,
            Self::BoardReply,
            Self::BoardEdit,
            Self::BoardDelete,
            Self::BoardCategories,
            Self::WhiteboardSnapshotShow,
            Self::WhiteboardSnapshotImage,
        ]
        .into_iter()
        .find(|operation| arguments == operation.arguments())
        .ok_or("Unsupported Office invocation or protocol version.")
    }
}

pub fn encode_office_capabilities() -> String {
    format!("TMT-OFFICE-CAPABILITIES/{OFFICE_PROTOCOL_VERSION}\n{OFFICE_BOARD_CAPABILITY}\n")
}

pub fn decode_office_capabilities(bytes: &[u8]) -> Result<(), &'static str> {
    if bytes.len() > OFFICE_PROTOCOL_OUTPUT_LIMIT {
        return Err("Office capabilities exceed their bound.");
    }
    if bytes == encode_office_capabilities().as_bytes() {
        Ok(())
    } else {
        Err("Invalid or unsupported Office capabilities.")
    }
}

pub fn encode_office_probe(version: &Version) -> String {
    format!("TMT-OFFICE/{OFFICE_PROTOCOL_VERSION}\n{version}\n")
}

pub fn decode_office_probe(bytes: &[u8]) -> Result<Version, &'static str> {
    if bytes.len() > OFFICE_PROTOCOL_OUTPUT_LIMIT {
        return Err("Office handshake exceeds its bound.");
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "Invalid Office handshake encoding.")?;
    let prefix = format!("TMT-OFFICE/{OFFICE_PROTOCOL_VERSION}\n");
    let version = text
        .strip_prefix(&prefix)
        .and_then(|text| text.strip_suffix('\n'))
        .ok_or("Incompatible Office handshake.")?;
    let parsed: Version = version
        .parse()
        .map_err(|_| "Invalid Office companion version.")?;
    if encode_office_probe(&parsed).as_bytes() != bytes {
        return Err("Noncanonical Office handshake.");
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_invocation_and_independent_package_version_round_trip() {
        assert_eq!(
            OfficeInvocation::parse(&["__tmt-office", "1", "probe"]),
            Ok(OfficeInvocation::Probe)
        );
        let version = Version::parse("0.1.0-alpha.1").unwrap();
        assert_eq!(
            encode_office_probe(&version),
            "TMT-OFFICE/1\n0.1.0-alpha.1\n"
        );
        assert_eq!(
            decode_office_probe(b"TMT-OFFICE/1\n0.1.0-alpha.1\n"),
            Ok(version)
        );
    }

    #[test]
    fn world_operations_replace_local_block_protocol_without_aliases() {
        for operation in [
            OfficeInvocation::LocalWorldShow,
            OfficeInvocation::LocalWorldApply,
        ] {
            assert_eq!(
                OfficeInvocation::parse(&operation.arguments()),
                Ok(operation)
            );
        }
        for removed in ["local-block-show", "local-block-apply"] {
            assert!(OfficeInvocation::parse(&["__tmt-office", "1", removed]).is_err());
        }
    }

    #[test]
    fn capabilities_are_exact_bounded_and_fail_closed() {
        assert_eq!(
            decode_office_capabilities(encode_office_capabilities().as_bytes()),
            Ok(())
        );
        for bytes in [
            b"TMT-OFFICE-CAPABILITIES/1\n".as_slice(),
            b"TMT-OFFICE-CAPABILITIES/1\noffice_board_v1\noffice_board_v1\n",
            b"TMT-OFFICE-CAPABILITIES/1\nunknown\n",
            b"TMT-OFFICE-CAPABILITIES/1\noffice_board_v1\nunknown\n",
        ] {
            assert!(decode_office_capabilities(bytes).is_err());
        }
        assert!(decode_office_capabilities(&vec![b'x'; OFFICE_PROTOCOL_OUTPUT_LIMIT + 1]).is_err());
    }

    #[test]
    fn pairing_operations_have_exact_versioned_arguments() {
        for (name, operation) in [
            ("pair-begin", OfficeInvocation::PairBegin),
            ("pair-poll", OfficeInvocation::PairPoll),
            ("pair-status", OfficeInvocation::PairStatus),
            ("unpair", OfficeInvocation::Unpair),
            ("inspect", OfficeInvocation::Inspect),
            ("sync", OfficeInvocation::Sync),
            ("block-show", OfficeInvocation::BlockShow),
            ("block-apply", OfficeInvocation::BlockApply),
        ] {
            assert_eq!(operation.arguments(), ["__tmt-office", "1", name]);
            assert_eq!(
                OfficeInvocation::parse(&["__tmt-office", "1", name]),
                Ok(operation)
            );
            assert!(OfficeInvocation::parse(&["__tmt-office", "2", name]).is_err());
            assert!(OfficeInvocation::parse(&["__tmt-office", "1", name, "extra"]).is_err());
        }
    }

    #[test]
    fn board_operations_have_exact_versioned_arguments() {
        for (name, operation) in [
            ("board-post", OfficeInvocation::BoardPost),
            ("board-list", OfficeInvocation::BoardList),
            ("board-show", OfficeInvocation::BoardShow),
            ("board-reply", OfficeInvocation::BoardReply),
            ("board-edit", OfficeInvocation::BoardEdit),
            ("board-delete", OfficeInvocation::BoardDelete),
            ("board-categories", OfficeInvocation::BoardCategories),
        ] {
            assert_eq!(operation.arguments(), ["__tmt-office", "1", name]);
            assert_eq!(
                OfficeInvocation::parse(&operation.arguments()),
                Ok(operation)
            );
        }
    }

    #[test]
    fn local_profile_operations_have_exact_versioned_arguments() {
        for (name, operation) in [
            ("local-profile-show", OfficeInvocation::LocalProfileShow),
            ("local-profile-apply", OfficeInvocation::LocalProfileApply),
        ] {
            assert_eq!(operation.arguments(), ["__tmt-office", "1", name]);
            assert_eq!(
                OfficeInvocation::parse(&operation.arguments()),
                Ok(operation)
            );
            assert!(OfficeInvocation::parse(&["__tmt-office", "2", name]).is_err());
        }
    }

    #[test]
    fn local_avatar_operations_have_exact_versioned_arguments() {
        for (name, operation) in [
            (
                "local-avatar-validate",
                OfficeInvocation::LocalAvatarValidate,
            ),
            ("local-avatar-install", OfficeInvocation::LocalAvatarInstall),
            ("local-avatar-remove", OfficeInvocation::LocalAvatarRemove),
            ("local-avatar-list", OfficeInvocation::LocalAvatarList),
            ("local-avatar-show", OfficeInvocation::LocalAvatarShow),
        ] {
            assert_eq!(operation.arguments(), ["__tmt-office", "1", name]);
            assert_eq!(
                OfficeInvocation::parse(&operation.arguments()),
                Ok(operation)
            );
            assert!(OfficeInvocation::parse(&["__tmt-office", "2", name]).is_err());
        }
    }

    #[test]
    fn unknown_versions_extra_arguments_and_noisy_responses_are_rejected() {
        for args in [
            vec![],
            vec!["__tmt-office", "2", "probe"],
            vec!["__tmt-office", "1", "probe", "extra"],
            vec!["__tmt-office", "1", "pair"],
        ] {
            assert!(OfficeInvocation::parse(&args).is_err());
        }
        for bytes in [
            b"TMT-OFFICE/2\n0.1.0\n".as_slice(),
            b"TMT-OFFICE/1\n0.1.0\nextra\n",
            b"noise\nTMT-OFFICE/1\n0.1.0\n",
            b"TMT-OFFICE/1\n0.1.0",
            b"TMT-OFFICE/1\nv0.1.0\n",
            b"\xff",
        ] {
            assert!(decode_office_probe(bytes).is_err());
        }
        assert!(decode_office_probe(&vec![b'x'; OFFICE_PROTOCOL_OUTPUT_LIMIT + 1]).is_err());
    }
}

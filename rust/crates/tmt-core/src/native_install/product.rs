//! Fixed artifact identities for the CLI and its official extensions (Ops,
//! Remote, Colab and Digest). The table is reviewed code; archive data never adds a product.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Product {
    Cli,
    Ops,
    Remote,
    Colab,
    Digest,
}

/// Optional product-owned step after a replaced release is active. The adapter
/// captures prior evidence and invokes the product, which owns lifecycle admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostUpgradeCheck {
    RemoteDoor,
}

/// Read-only installation identity before a product rename or a move to a new
/// namespace. This is never a selectable product, a dispatch alias or a
/// completion candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormerProduct {
    pub name: &'static str,
    pub executable: &'static str,
    pub namespace: &'static str,
    pub tag_prefix: &'static str,
    pub links: &'static [&'static str],
    pub renamed_skills: &'static [(&'static str, &'static str)],
    /// Whether the former installation could be on the PR channel, so its
    /// receipts may carry PR provenance. A same-product move keeps that.
    pub pr_channel: bool,
}

impl FormerProduct {
    pub const fn files(self) -> [&'static str; 4] {
        [
            self.executable,
            "LICENSE",
            "NATIVE-INSTALL.md",
            "THIRD-PARTY-NOTICES.txt",
        ]
    }
}

impl Product {
    pub const fn post_upgrade_check(self) -> Option<PostUpgradeCheck> {
        match self {
            Self::Remote => Some(PostUpgradeCheck::RemoteDoor),
            _ => None,
        }
    }

    pub const fn former(self) -> Option<&'static FormerProduct> {
        match self {
            Self::Ops => Some(&FormerProduct {
                name: "squad",
                executable: "tmt-squad",
                namespace: "lib/tmt-squad",
                tag_prefix: "tmt-squad-v",
                links: &["tmt-squad", "tmt-sq"],
                renamed_skills: &[("tmt-squad", "tmt-ops")],
                pr_channel: false,
            }),
            // The CLI keeps its identity and command names; only the install
            // prefix moved. The old layout stays readable for rollback.
            Self::Cli => Some(&FormerProduct {
                name: "cli",
                executable: "tmt",
                namespace: "lib/tmux-team",
                tag_prefix: "v",
                links: &["tmt", "tmux-team"],
                renamed_skills: &[],
                pr_channel: true,
            }),
            _ => None,
        }
    }
    pub const fn tag_prefix(self) -> &'static str {
        match self {
            Self::Cli => "v",
            Self::Ops => "tmt-ops-v",
            Self::Remote => "tmt-remote-v",
            Self::Colab => "tmt-colab-v",
            Self::Digest => "tmt-digest-v",
        }
    }

    /// Whether a release of `version` may carry GitHub's `prerelease` flag
    /// `flagged`, following `typescript/scripts/native-release-policy.mjs`.
    /// The CLI release is a normal release so it can be the repository's
    /// latest; earlier CLI alphas were flagged, so a pre-release version may
    /// carry either flag, but a stable version never a set one. Extensions
    /// are flagged exactly when their version is a pre-release.
    pub fn accepts_prerelease_flag(self, version: &semver::Version, flagged: bool) -> bool {
        let pre_release = !version.pre.is_empty();
        match self {
            Self::Cli => pre_release || !flagged,
            Self::Ops | Self::Remote | Self::Colab | Self::Digest => flagged == pre_release,
        }
    }

    pub const ALL: [Self; 5] = [
        Self::Cli,
        Self::Ops,
        Self::Remote,
        Self::Colab,
        Self::Digest,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Cli => "cli",
            Self::Ops => "ops",
            Self::Remote => "remote",
            Self::Colab => "colab",
            Self::Digest => "digest",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|product| product.as_str() == value)
    }

    pub const fn executable(self) -> &'static str {
        match self {
            Self::Cli => "tmt",
            Self::Ops => "tmt-ops",
            Self::Remote => "tmt-remote",
            Self::Colab => "tmt-colab",
            Self::Digest => "tmt-digest",
        }
    }

    pub const fn package(self) -> &'static str {
        match self {
            Self::Cli => "tmt-cli",
            Self::Ops => "tmt-ops",
            Self::Remote => "tmt-remote",
            Self::Colab => "tmt-colab",
            Self::Digest => "tmt-digest",
        }
    }

    pub const fn namespace(self) -> &'static str {
        match self {
            Self::Cli => "lib/tmt",
            Self::Ops => "lib/tmt-ops",
            Self::Remote => "lib/tmt-remote",
            Self::Colab => "lib/tmt-colab",
            Self::Digest => "lib/tmt-digest",
        }
    }

    pub const fn links(self) -> &'static [&'static str] {
        match self {
            Self::Cli => &["tmt", "tmux-team"],
            Self::Ops => &["tmt-ops"],
            Self::Remote => &["tmt-remote"],
            Self::Colab => &["tmt-colab"],
            Self::Digest => &["tmt-digest"],
        }
    }

    pub fn link_target(self) -> String {
        format!("../{}/current/{}", self.namespace(), self.executable())
    }

    /// Where the former installation pointed its command link `name`, when this
    /// product keeps that command across the move.
    pub fn former_link_target(self, name: &str) -> Option<String> {
        let former = self.former()?;
        former
            .links
            .contains(&name)
            .then(|| format!("../{}/current/{}", former.namespace, former.executable))
    }

    /// Executables a release may carry beside the product's own, such as a
    /// first-party host driver in the CLI release (#479). Each is optional:
    /// the receipt records one exactly when the release carries it, so a
    /// release from before a companion existed still verifies.
    pub const fn companions(self) -> &'static [&'static str] {
        match self {
            Self::Cli => &["tmt-driver-herdr"],
            Self::Ops | Self::Remote | Self::Colab | Self::Digest => &[],
        }
    }

    /// Plain, non-executable files an extension release may carry beside the
    /// required ones: `TMT-USES.json` declares optional uses of other
    /// extensions. Like a companion, each is optional and recorded in the
    /// receipt exactly when the release carries it.
    pub const fn optional_files(self) -> &'static [&'static str] {
        match self {
            Self::Cli => &[],
            Self::Ops | Self::Remote | Self::Colab | Self::Digest => &["TMT-USES.json"],
        }
    }

    pub const fn files(self) -> [&'static str; 4] {
        [
            self.executable(),
            "LICENSE",
            "NATIVE-INSTALL.md",
            "THIRD-PARTY-NOTICES.txt",
        ]
    }
}

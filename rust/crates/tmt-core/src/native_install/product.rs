//! Fixed artifact identities for the CLI and its official extensions (Office,
//! Squad and Remote). The table is reviewed code; archive data never adds a product.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Product {
    Cli,
    Office,
    Squad,
    Remote,
}

impl Product {
    pub const fn tag_prefix(self) -> &'static str {
        match self {
            Self::Cli => "v",
            Self::Office => "tmt-office-v",
            Self::Squad => "tmt-squad-v",
            Self::Remote => "tmt-remote-v",
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
            Self::Office | Self::Squad | Self::Remote => flagged == pre_release,
        }
    }

    pub const ALL: [Self; 4] = [Self::Cli, Self::Office, Self::Squad, Self::Remote];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Cli => "cli",
            Self::Office => "office",
            Self::Squad => "squad",
            Self::Remote => "remote",
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
            Self::Office => "tmt-office",
            Self::Squad => "tmt-squad",
            Self::Remote => "tmt-remote",
        }
    }

    pub const fn package(self) -> &'static str {
        match self {
            Self::Cli => "tmt-cli",
            Self::Office => "tmt-office",
            Self::Squad => "tmt-squad",
            Self::Remote => "tmt-remote",
        }
    }

    pub const fn namespace(self) -> &'static str {
        match self {
            Self::Cli => "lib/tmux-team",
            Self::Office => "lib/tmt-office",
            Self::Squad => "lib/tmt-squad",
            Self::Remote => "lib/tmt-remote",
        }
    }

    pub const fn links(self) -> &'static [&'static str] {
        match self {
            Self::Cli => &["tmt", "tmux-team"],
            Self::Office => &["tmt-office"],
            Self::Squad => &["tmt-squad", "tmt-sq"],
            Self::Remote => &["tmt-remote"],
        }
    }

    pub fn link_target(self) -> String {
        format!("../{}/current/{}", self.namespace(), self.executable())
    }

    /// Releases whose executable must pass the owner's post-write check before
    /// publication. Native installation refuses such a product without one.
    pub const fn requires_release_verifier(self) -> bool {
        match self {
            Self::Cli | Self::Squad | Self::Remote => false,
            Self::Office => true,
        }
    }

    /// Executables a release may carry beside the product's own, such as a
    /// first-party host driver in the CLI release (#479). Each is optional:
    /// the receipt records one exactly when the release carries it, so a
    /// release from before a companion existed still verifies.
    pub const fn companions(self) -> &'static [&'static str] {
        match self {
            Self::Cli => &["tmt-driver-herdr"],
            Self::Office | Self::Squad | Self::Remote => &[],
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

//! Fixed artifact identities for the CLI and its official extensions (Office
//! and Squad). The table is reviewed code; archive data never adds a product.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Product {
    Cli,
    Office,
    Squad,
}

impl Product {
    pub const fn tag_prefix(self) -> &'static str {
        match self {
            Self::Cli => "v",
            Self::Office => "tmt-office-v",
            Self::Squad => "tmt-squad-v",
        }
    }
    pub const ALL: [Self; 3] = [Self::Cli, Self::Office, Self::Squad];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Cli => "cli",
            Self::Office => "office",
            Self::Squad => "squad",
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
        }
    }

    pub const fn package(self) -> &'static str {
        match self {
            Self::Cli => "tmt-cli",
            Self::Office => "tmt-office",
            Self::Squad => "tmt-squad",
        }
    }

    pub const fn namespace(self) -> &'static str {
        match self {
            Self::Cli => "lib/tmux-team",
            Self::Office => "lib/tmt-office",
            Self::Squad => "lib/tmt-squad",
        }
    }

    pub const fn links(self) -> &'static [&'static str] {
        match self {
            Self::Cli => &["tmt", "tmux-team"],
            Self::Office => &["tmt-office"],
            Self::Squad => &["tmt-squad", "tmt-sq"],
        }
    }

    pub fn link_target(self) -> String {
        format!("../{}/current/{}", self.namespace(), self.executable())
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

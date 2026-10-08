//! Pure release-channel and version policy; no filesystem or transport effects.

mod product;
pub use product::{FormerProduct, Product};
mod pr;
pub use pr::{PrCandidateIdentity, PrNumber, SchemaAdmission, SchemaError, admit_schema};

pub fn native_target(os: &str, architecture: &str) -> Option<&'static str> {
    match (os, architecture) {
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        ("macos", "x86_64") => Some("x86_64-apple-darwin"),
        ("linux", "aarch64") => Some("aarch64-unknown-linux-musl"),
        ("linux", "x86_64") => Some("x86_64-unknown-linux-musl"),
        _ => None,
    }
}

use semver::Version;
use std::{borrow::Cow, cmp::Ordering, error::Error, fmt};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Stable,
    Alpha,
    Pr(PrNumber),
}

impl Channel {
    /// Published channel families; parameterized PR channels use `parse`.
    pub const ALL: [Self; 2] = [Self::Stable, Self::Alpha];

    pub fn as_str(self) -> Cow<'static, str> {
        match self {
            Self::Stable => Cow::Borrowed("stable"),
            Self::Alpha => Cow::Borrowed("alpha"),
            Self::Pr(number) => Cow::Owned(format!("pr{number}")),
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|channel| channel.as_str().eq_ignore_ascii_case(value))
            .or_else(|| {
                value
                    .strip_prefix("pr")
                    .and_then(PrNumber::parse)
                    .map(Self::Pr)
            })
    }

    pub fn accepts(self, version: &Version) -> bool {
        match self {
            Self::Stable => version.pre.is_empty(),
            Self::Alpha => version.pre.as_str().split('.').next() == Some("alpha"),
            // PR admission verifies the archived version and source independently;
            // the channel must never manufacture a replacement SemVer.
            Self::Pr(_) => true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledVersion {
    pub version: Version,
    pub channel: Channel,
    pub pinned_version: Option<Version>,
}

impl InstalledVersion {
    pub fn validate(&self) -> Result<(), VersionError> {
        if !self.channel.accepts(&self.version)
            || self
                .pinned_version
                .as_ref()
                .is_some_and(|pinned| pinned != &self.version)
        {
            return Err(VersionError::InvalidCurrentState);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinAction {
    Preserve,
    PinCandidate,
    Clear,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionPlan {
    pub state: InstalledVersion,
    pub changed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionError {
    InvalidSelection,
    InvalidCurrentState,
    WrongChannel,
    Pinned,
    Downgrade,
    EqualPrecedenceChange,
    StalePrCandidate,
    MissingPrEvidence,
    ExplicitChannelRequired,
    SameVersionPrCandidate,
}

impl fmt::Display for VersionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidSelection => "Select a valid exact version or unpin, not both.",
            Self::InvalidCurrentState => "Installed version, channel and pin disagree.",
            Self::WrongChannel => "The selected version does not belong to the selected channel.",
            Self::Pinned => "The installation is pinned; explicitly select a version or unpin it.",
            Self::Downgrade => "Native installation does not permit a version downgrade.",
            Self::EqualPrecedenceChange => {
                "Build metadata alone cannot select a different installed release."
            }
            Self::StalePrCandidate => "The PR candidate must be newer than the installed run.",
            Self::MissingPrEvidence => "PR channel policy requires matching PR, head and run evidence.",
            Self::ExplicitChannelRequired => "Explicitly select a different channel to switch a PR installation.",
            Self::SameVersionPrCandidate => "A different PR candidate at the same archived version requires an explicitly selected different channel.",
        })
    }
}
impl Error for VersionError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpgradeSelection {
    Pinned,
    Fetch {
        channel: Channel,
        exact: Option<Version>,
        pin: PinAction,
    },
}

/// Resolve explicit user intent before any release discovery or network effect.
pub fn select_upgrade(
    current: &InstalledVersion,
    channel: Option<Channel>,
    exact: Option<&str>,
    unpin: bool,
) -> Result<UpgradeSelection, VersionError> {
    current.validate()?;
    if unpin && exact.is_some() {
        return Err(VersionError::InvalidSelection);
    }
    let channel = channel.unwrap_or(current.channel);
    if current.pinned_version.is_some() && exact.is_none() && !unpin {
        if channel != current.channel {
            return Err(VersionError::Pinned);
        }
        return Ok(UpgradeSelection::Pinned);
    }
    let exact = exact
        .map(|version| {
            version
                .parse::<Version>()
                .map_err(|_| VersionError::InvalidSelection)
        })
        .transpose()?;
    let pin = if exact.is_some() {
        PinAction::PinCandidate
    } else if unpin {
        PinAction::Clear
    } else {
        PinAction::Preserve
    };
    if let Some(version) = &exact {
        // Discovery has not supplied candidate identity yet. Preserve the existing
        // exact-version/pin checks here; PR evidence is required before publication.
        plan_numeric_version(Some(current), version, channel, pin)?;
    }
    Ok(UpgradeSelection::Fetch {
        channel,
        exact,
        pin,
    })
}

pub fn latest_in_channel(
    versions: &[Version],
    channel: Channel,
) -> Result<Option<&Version>, VersionError> {
    if matches!(channel, Channel::Pr(_)) {
        // PR discovery requires exact-source/run evidence, not release tag precedence.
        return Err(VersionError::InvalidSelection);
    }
    let selected = versions
        .iter()
        .filter(|version| channel.accepts(version))
        .max_by(|a, b| a.cmp_precedence(b));
    if let Some(selected) = selected
        && versions.iter().any(|candidate| {
            channel.accepts(candidate)
                && candidate != selected
                && candidate.cmp_precedence(selected) == Ordering::Equal
        })
    {
        return Err(VersionError::EqualPrecedenceChange);
    }
    Ok(selected)
}

/// Filesystem ownership and equal-version artifact integrity are separate
/// adapter checks. A version no-op never authorizes ignoring those checks.
pub fn plan_version(
    current: Option<&InstalledVersion>,
    candidate: &Version,
    channel: Channel,
    pin: PinAction,
) -> Result<VersionPlan, VersionError> {
    plan_candidate_version(
        current,
        candidate,
        channel,
        pin,
        PrVersionContext::default(),
    )
}

/// Receipt-derived identities and explicit channel intent for the same version owner.
#[derive(Debug, Clone, Copy, Default)]
pub struct PrVersionContext<'a> {
    pub current: Option<&'a PrCandidateIdentity>,
    pub candidate: Option<&'a PrCandidateIdentity>,
    pub explicit_channel: bool,
}

pub fn plan_candidate_version(
    current: Option<&InstalledVersion>,
    candidate: &Version,
    channel: Channel,
    pin: PinAction,
    evidence: PrVersionContext<'_>,
) -> Result<VersionPlan, VersionError> {
    let matches_channel =
        |channel, identity: Option<&PrCandidateIdentity>| match (channel, identity) {
            (Channel::Pr(number), Some(identity)) => identity.pr() == number,
            (Channel::Pr(_), None) | (_, Some(_)) => false,
            (_, None) => true,
        };
    if !matches_channel(channel, evidence.candidate)
        || !current.is_none_or(|state| matches_channel(state.channel, evidence.current))
        || (current.is_none() && evidence.current.is_some())
    {
        return Err(VersionError::MissingPrEvidence);
    }
    if current.is_none() && matches!(channel, Channel::Pr(_)) && !evidence.explicit_channel {
        return Err(VersionError::ExplicitChannelRequired);
    }
    if let Some(current) = current {
        if current.channel != channel
            && (matches!(current.channel, Channel::Pr(_)) || matches!(channel, Channel::Pr(_)))
            && !evidence.explicit_channel
        {
            return Err(VersionError::ExplicitChannelRequired);
        }
        if current.channel == channel
            && let (Some(previous), Some(next)) = (evidence.current, evidence.candidate)
        {
            let changed = previous.successor_changed(next)?;
            if changed && candidate.cmp_precedence(&current.version) == Ordering::Equal {
                return Err(VersionError::SameVersionPrCandidate);
            }
            if !changed && candidate != &current.version {
                return Err(VersionError::InvalidSelection);
            }
        }
    }
    plan_numeric_version(current, candidate, channel, pin)
}

fn plan_numeric_version(
    current: Option<&InstalledVersion>,
    candidate: &Version,
    channel: Channel,
    pin: PinAction,
) -> Result<VersionPlan, VersionError> {
    if let Some(current) = current {
        current.validate()?;
    }
    if !channel.accepts(candidate) {
        return Err(VersionError::WrongChannel);
    }
    let pinned_version = match pin {
        PinAction::PinCandidate => Some(candidate.clone()),
        PinAction::Clear => None,
        PinAction::Preserve => current.and_then(|state| state.pinned_version.clone()),
    };
    if pinned_version
        .as_ref()
        .is_some_and(|pinned| pinned != candidate)
    {
        return Err(VersionError::Pinned);
    }
    if let Some(current) = current {
        match candidate.cmp_precedence(&current.version) {
            Ordering::Less => return Err(VersionError::Downgrade),
            Ordering::Equal if candidate != &current.version => {
                return Err(VersionError::EqualPrecedenceChange);
            }
            Ordering::Equal | Ordering::Greater => {}
        }
    }
    let state = InstalledVersion {
        version: candidate.clone(),
        channel,
        pinned_version,
    };
    Ok(VersionPlan {
        changed: current != Some(&state),
        state,
    })
}

#[cfg(test)]
mod tests;

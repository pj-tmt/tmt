//! PR candidate identity and application-schema admission, without acquisition effects.

use std::{error::Error, fmt, num::NonZeroU32};

use super::VersionError;

/// A positive PR number bounded to signed 32 bits, in canonical decimal form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrNumber(NonZeroU32);

impl PrNumber {
    pub fn parse(value: &str) -> Option<Self> {
        if value.is_empty()
            || value.len() > 10
            || value.starts_with('0')
            || !value.bytes().all(|byte| byte.is_ascii_digit())
        {
            return None;
        }
        let number = value.parse::<u32>().ok()?;
        (number <= i32::MAX as u32).then_some(Self(NonZeroU32::new(number)?))
    }

    pub const fn get(self) -> u32 {
        self.0.get()
    }
}

impl fmt::Display for PrNumber {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.get().fmt(formatter)
    }
}

/// Identity of archived bytes; the executable's archived SemVer is kept separately.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrCandidateIdentity {
    pr: PrNumber,
    head_sha: String,
    run_id: u64,
}

impl PrCandidateIdentity {
    pub fn new(pr: PrNumber, head_sha: String, run_id: u64) -> Option<Self> {
        if run_id == 0
            || head_sha.len() != 40
            || !head_sha
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return None;
        }
        Some(Self {
            pr,
            head_sha,
            run_id,
        })
    }

    pub const fn pr(&self) -> PrNumber {
        self.pr
    }

    pub fn head_sha(&self) -> &str {
        &self.head_sha
    }

    pub const fn run_id(&self) -> u64 {
        self.run_id
    }

    /// An exact repeat is unchanged. Every replacement, including a rebase, needs a newer run.
    pub fn successor_changed(&self, next: &Self) -> Result<bool, VersionError> {
        if self.pr != next.pr {
            return Err(VersionError::WrongChannel);
        }
        if self == next {
            return Ok(false);
        }
        if next.run_id <= self.run_id {
            return Err(VersionError::StalePrCandidate);
        }
        Ok(true)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchemaAdmission {
    pub candidate: u32,
    pub latest_alpha: u32,
    pub local: u32,
    pub opted_in: bool,
}

impl SchemaAdmission {
    pub const fn ahead_of_alpha(self) -> bool {
        self.candidate > self.latest_alpha
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaError {
    Unknown,
    AheadOfAlpha,
    DataDowngrade,
}

impl fmt::Display for SchemaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Unknown => "Candidate, latest alpha and local application schemas must all be known.",
            Self::AheadOfAlpha => {
                "The candidate schema is ahead of the latest alpha; explicit schema-ahead consent is required."
            }
            Self::DataDowngrade => {
                "The selected application schema is older than local data; wait for a compatible release."
            }
        })
    }
}

impl Error for SchemaError {}

/// Unknown evidence and data downgrades always refuse, including with an opt-in.
pub fn admit_schema(
    candidate: Option<u32>,
    latest_alpha: Option<u32>,
    local: Option<u32>,
    opted_in: bool,
) -> Result<SchemaAdmission, SchemaError> {
    let (Some(candidate), Some(latest_alpha), Some(local)) = (candidate, latest_alpha, local)
    else {
        return Err(SchemaError::Unknown);
    };
    if candidate < local {
        return Err(SchemaError::DataDowngrade);
    }
    if candidate > latest_alpha && !opted_in {
        return Err(SchemaError::AheadOfAlpha);
    }
    Ok(SchemaAdmission {
        candidate,
        latest_alpha,
        local,
        opted_in,
    })
}

#[cfg(test)]
#[path = "pr_tests.rs"]
mod tests;

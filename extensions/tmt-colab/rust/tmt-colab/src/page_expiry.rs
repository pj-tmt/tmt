//! How long a page may sit without activity before it is archived (#2600). The space has one
//! default (`tmt colab settings page-expiry`), a page's own signed `retention.set` overrides it,
//! and [`effective_days`] is the one place that decides which applies; every reader of the policy
//! (the management views, the scheduled archive, the cloud lifetime) goes through it.

/// The days without activity after which a page is archived, when neither the space nor the page
/// says otherwise.
pub const DEFAULT_DAYS: u64 = 180;
/// Larger settings are not a limit; `off` is how a space never archives.
pub const MAX_DAYS: u64 = 3650;

/// The space's setting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SpaceExpiry {
    /// The file does not say: [`DEFAULT_DAYS`].
    #[default]
    Default,
    Days(u64),
    /// Pages are never archived for inactivity.
    Off,
}
impl SpaceExpiry {
    /// The days without activity, or `None` when pages are never archived.
    pub fn days(self) -> Option<u64> {
        match self {
            Self::Default => Some(DEFAULT_DAYS),
            Self::Days(days) => Some(days),
            Self::Off => None,
        }
    }
}

/// What a page's signed policy says about its retention.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageRetention {
    /// No `retention.set` was ever signed: the space decides.
    Inherit,
    /// Signed `forever`.
    Forever,
    Days(u64),
}

/// The days without activity after which the page is archived; `None` means never.
pub fn effective_days(page: PageRetention, space: SpaceExpiry) -> Option<u64> {
    match page {
        PageRetention::Inherit => space.days(),
        PageRetention::Forever => None,
        PageRetention::Days(days) => Some(days),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_statement_wins_and_otherwise_the_space_decides() {
        for (page, space, days) in [
            (PageRetention::Inherit, SpaceExpiry::Default, Some(180)),
            (PageRetention::Inherit, SpaceExpiry::Days(90), Some(90)),
            (PageRetention::Inherit, SpaceExpiry::Off, None),
            (PageRetention::Forever, SpaceExpiry::Days(90), None),
            (PageRetention::Days(7), SpaceExpiry::Off, Some(7)),
            (PageRetention::Days(7), SpaceExpiry::Days(90), Some(7)),
        ] {
            assert_eq!(effective_days(page, space), days, "{page:?} {space:?}");
        }
    }
}

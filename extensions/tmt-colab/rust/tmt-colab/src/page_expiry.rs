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

/// Reads the space's setting each time it is used, so a changed setting applies at once. `None`:
/// the setting could not be read (a damaged `settings.json`), which is not the same as the default:
/// whoever would act on the setting waits rather than guess, since `off` would silently become 180.
pub type Reader = std::sync::Arc<dyn Fn() -> Option<SpaceExpiry> + Send + Sync>;

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

/// Milliseconds in a day, the unit of a page's expiry.
pub const DAY_MS: u64 = 86_400_000;
/// The largest time a Colab record carries (a JavaScript safe integer).
const MAX_TIME_MS: u64 = 9_007_199_254_740_991;

/// When a page last active at `active` is due to be archived after `days` without activity. `None`
/// when it never is (`days` is `None`), when its activity is unknown, or when the time would not
/// fit a record: an unknown activity is never an old one.
pub fn archive_at_ms(active: Option<u64>, days: Option<u64>) -> Option<u64> {
    let due = active?.checked_add(days?.checked_mul(DAY_MS)?)?;
    (due <= MAX_TIME_MS).then_some(due)
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

    #[test]
    fn the_archive_time_is_the_activity_plus_the_days_and_otherwise_none() {
        assert_eq!(
            archive_at_ms(Some(1_000), Some(2)),
            Some(1_000 + 2 * DAY_MS)
        );
        assert_eq!(archive_at_ms(Some(1_000), None), None, "never archived");
        assert_eq!(archive_at_ms(None, Some(180)), None, "unknown is not old");
        // A day count or a time too large for a record is never a due time.
        assert_eq!(archive_at_ms(Some(1), Some(u64::MAX)), None);
        assert_eq!(archive_at_ms(Some(MAX_TIME_MS), Some(1)), None);
        assert_eq!(
            archive_at_ms(Some(MAX_TIME_MS - DAY_MS), Some(1)),
            Some(MAX_TIME_MS)
        );
    }
}

//! Per-send feedback over the board's existing typed provider observation.
//! This does not correlate a reply, acquire data or change delivery outcomes.
use crate::board::activity::{ObservedActivity, Phase};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Progress {
    #[default]
    Unknown,
    Earlier {
        since_ms: u64,
    },
    Working {
        since_ms: u64,
    },
    Finished {
        at_ms: u64,
    },
}

impl Progress {
    pub(super) fn observe(
        &mut self,
        observation: Option<ObservedActivity>,
        sent_ms: u64,
        now_ms: u64,
    ) {
        // A later unrelated turn cannot restart feedback for a finished send.
        if matches!(self, Self::Finished { .. }) {
            return;
        }
        let previous = *self;
        let admitted = observation.and_then(|activity| {
            let since = activity.since_ms?;
            (sent_ms <= now_ms && since <= now_ms).then_some((activity.phase, since))
        });
        *self = match admitted {
            Some((_, since))
                if matches!(previous, Self::Working { since_ms } | Self::Earlier { since_ms }
                    if since < since_ms) =>
            {
                Self::Unknown
            }
            Some((Phase::Working, since_ms)) if since_ms >= sent_ms => Self::Working { since_ms },
            Some((Phase::Working, since_ms)) => Self::Earlier { since_ms },
            Some((Phase::Idle, at_ms)) if matches!(previous, Self::Working { since_ms } if at_ms >= since_ms) => {
                Self::Finished { at_ms }
            }
            _ => Self::Unknown,
        };
    }

    pub(super) fn text(self, name: &str, now_ms: u64, working_glyph: &str) -> String {
        let name = crate::board::notes::single_line(name);
        let unknown = || format!("✓ sent · {name}'s activity is not reported");
        match self {
            Self::Working { since_ms } if since_ms <= now_ms => {
                let seconds = (now_ms - since_ms) / 1000;
                format!(
                    "{working_glyph} {name} is working · {}:{:02}",
                    seconds / 60,
                    seconds % 60
                )
            }
            Self::Earlier { since_ms } if since_ms <= now_ms => format!(
                "◌ {name} is finishing an earlier turn · {}",
                crate::requests::age(now_ms, since_ms)
            ),
            Self::Finished { at_ms } if at_ms <= now_ms => format!(
                "✓ {name} finished without replying · {}",
                super::super::exchange::age_stamp(&crate::requests::age(now_ms, at_ms))
            ),
            _ => unknown(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SENT: u64 = 1_000_000;

    fn observed(phase: Phase, since_ms: u64) -> Option<ObservedActivity> {
        Some(ObservedActivity {
            phase,
            since_ms: Some(since_ms),
            last_ms: Some(since_ms),
        })
    }

    #[test]
    fn phase_start_at_the_send_boundary_is_current_work_and_earlier_is_queued() {
        for (since, expected) in [
            (
                SENT - 180_000,
                "◌ night-2 is finishing an earlier turn · 3m",
            ),
            (SENT, "⠹ night-2 is working · 0:42"),
            (SENT + 1000, "⠹ night-2 is working · 0:41"),
        ] {
            let mut progress = Progress::default();
            progress.observe(observed(Phase::Working, since), SENT, SENT + 42_000);
            let now = if since < SENT { SENT } else { SENT + 42_000 };
            assert_eq!(progress.text("night-2", now, "⠹"), expected);
        }
        let mut progress = Progress::default();
        progress.observe(observed(Phase::Working, SENT - 1), SENT, SENT);
        assert!(matches!(progress, Progress::Earlier { .. }));
    }

    #[test]
    fn only_observed_current_work_can_become_finished_without_a_reply() {
        let mut progress = Progress::default();
        progress.observe(observed(Phase::Idle, SENT + 60_000), SENT, SENT + 180_000);
        assert_eq!(progress, Progress::Unknown);
        progress.observe(observed(Phase::Working, SENT - 1), SENT, SENT);
        progress.observe(observed(Phase::Idle, SENT + 60_000), SENT, SENT + 180_000);
        assert_eq!(
            progress,
            Progress::Unknown,
            "an earlier turn is not this send"
        );
        progress.observe(observed(Phase::Working, SENT), SENT, SENT + 1);
        progress.observe(observed(Phase::Idle, SENT + 60_000), SENT, SENT + 180_000);
        assert_eq!(
            progress.text("night-2", SENT + 180_000, "⠹"),
            "✓ night-2 finished without replying · 2m ago"
        );
        progress.observe(
            observed(Phase::Working, SENT + 180_000),
            SENT,
            SENT + 180_001,
        );
        assert_eq!(
            progress,
            Progress::Finished {
                at_ms: SENT + 60_000
            }
        );
    }

    #[test]
    fn unknown_missing_stale_and_future_evidence_never_keep_working_or_finish_it() {
        for unavailable in [
            None,
            Some(ObservedActivity::default()),
            Some(ObservedActivity {
                phase: Phase::Working,
                ..Default::default()
            }),
            observed(Phase::Ended, SENT),
            observed(Phase::Working, SENT + 100),
        ] {
            let mut progress = Progress::default();
            progress.observe(observed(Phase::Working, SENT), SENT, SENT + 1);
            progress.observe(unavailable, SENT, SENT + 2);
            assert_eq!(progress, Progress::Unknown);
            assert_eq!(
                progress.text("night-2", SENT + 2, "⠋"),
                "✓ sent · night-2's activity is not reported"
            );
            progress.observe(observed(Phase::Idle, SENT + 3), SENT, SENT + 4);
            assert_eq!(progress, Progress::Unknown, "a gap cannot prove completion");
        }
    }

    #[test]
    fn regressing_phases_and_clock_rollback_cannot_manufacture_a_completion() {
        for next in [
            observed(Phase::Idle, SENT - 1),
            observed(Phase::Working, SENT - 1),
        ] {
            let mut progress = Progress::default();
            progress.observe(observed(Phase::Working, SENT), SENT, SENT + 1);
            progress.observe(next, SENT, SENT + 2);
            assert_eq!(progress, Progress::Unknown);
        }
        let mut progress = Progress::default();
        progress.observe(observed(Phase::Working, SENT), SENT, SENT - 1);
        assert_eq!(progress, Progress::Unknown);
        assert_eq!(
            Progress::Working { since_ms: SENT }.text("night-2", SENT - 1, "⠹"),
            "✓ sent · night-2's activity is not reported"
        );
    }

    #[test]
    fn long_work_keeps_elapsed_time_and_names_remain_inert_single_lines() {
        assert_eq!(
            Progress::Working { since_ms: SENT }.text("night-2", SENT + 2_830_000, "⠹"),
            "⠹ night-2 is working · 47:10"
        );
        let text = Progress::Unknown.text("agent\n\u{1b}[2J", SENT, "⠹");
        assert!(!text.contains('\n') && !text.contains('\u{1b}'));
    }
}

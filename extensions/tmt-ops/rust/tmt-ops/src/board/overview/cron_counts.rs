//! Counts from one acquired cron snapshot, never a clock or service read.
use std::collections::BTreeSet;

pub struct Job<'a> {
    pub room: &'a str,
    pub id: &'a str,
    pub owner: Option<&'a str>,
    pub on: bool,
    pub owner_valid: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Count {
    pub active: u32,
    pub total: u32,
}

pub enum Scope<'a> {
    Owner { id: &'a str, room: Option<&'a str> },
    Room(&'a str),
}

/// None is an unavailable/incomplete read, not zero. Job IDs are room-scoped;
/// the same observation in two squad projections must never double the count.
pub fn count(jobs: Option<&[Job<'_>]>, scope: Scope<'_>) -> Option<Count> {
    let jobs = jobs?;
    let mut seen = BTreeSet::new();
    let mut result = Count {
        active: 0,
        total: 0,
    };
    for job in jobs {
        let included = match scope {
            Scope::Owner { id, room } => {
                job.owner == Some(id) && room.is_none_or(|room| job.room == room)
            }
            Scope::Room(room) => job.room == room,
        };
        if included && seen.insert((job.room, job.id)) {
            result.total += 1;
            result.active += u32::from(job.on && job.owner_valid);
        }
    }
    Some(result)
}

pub fn label(count: Option<Count>, ascii: bool) -> String {
    let prefix = if ascii { "CRON" } else { "◷" };
    match count {
        Some(count) => format!("{prefix} {}/{}", count.active, count.total),
        None => format!("{prefix} unknown"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn room_identity_deduplicates_but_orphans_remain_in_room_totals() {
        let job = |room, id, owner, on, owner_valid| Job {
            room,
            id,
            owner,
            on,
            owner_valid,
        };
        let jobs = [
            job("r1", "c1", Some("a"), true, true),
            job("r1", "c1", Some("a"), true, true),
            job("r2", "c1", Some("a"), false, true),
            job("r1", "c2", Some("gone"), true, false),
            job("r1", "c3", None, true, false),
        ];
        assert_eq!(
            count(
                Some(&jobs),
                Scope::Owner {
                    id: "a",
                    room: None
                }
            ),
            Some(Count {
                active: 1,
                total: 2
            })
        );
        assert_eq!(
            count(
                Some(&jobs),
                Scope::Owner {
                    id: "a",
                    room: Some("r2")
                }
            ),
            Some(Count {
                active: 0,
                total: 1
            })
        );
        assert_eq!(
            count(Some(&jobs), Scope::Room("r1")),
            Some(Count {
                active: 1,
                total: 3
            })
        );
    }

    #[test]
    fn missing_zero_and_clock_independence_are_visible() {
        assert_eq!(label(count(None, Scope::Room("r")), false), "◷ unknown");
        assert_eq!(label(count(Some(&[]), Scope::Room("r")), true), "CRON 0/0");
        assert_eq!(
            label(
                Some(Count {
                    active: 2,
                    total: 3
                }),
                false
            ),
            "◷ 2/3"
        );
        // There is intentionally no clock input: paused clock is not paused job.
    }
}

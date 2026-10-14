//! Counts from one acquired cron snapshot, never a clock or service read.
use std::collections::{BTreeMap, BTreeSet};

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

/// One immutable count projection per acquired job snapshot. Missing is not zero;
/// the same room/id in two projections contributes only once.
#[derive(Default)]
pub struct Summary {
    known: bool,
    rooms: BTreeMap<String, Count>,
    owners: BTreeMap<(String, Option<String>), Count>,
}

impl Summary {
    pub fn of(jobs: Option<&[Job<'_>]>) -> Self {
        let Some(jobs) = jobs else {
            return Self::default();
        };
        let mut result = Self {
            known: true,
            ..Self::default()
        };
        let mut seen = BTreeSet::new();
        for job in jobs {
            if !seen.insert((job.room, job.id)) {
                continue;
            }
            let add = |count: &mut Count| {
                count.total = count.total.saturating_add(1);
                count.active = count
                    .active
                    .saturating_add(u32::from(job.on && job.owner_valid && job.owner.is_some()));
            };
            add(result.rooms.entry(job.room.to_owned()).or_insert(Count {
                active: 0,
                total: 0,
            }));
            if let Some(owner) = job.owner {
                for room in [None, Some(job.room.to_owned())] {
                    add(result
                        .owners
                        .entry((owner.to_owned(), room))
                        .or_insert(Count {
                            active: 0,
                            total: 0,
                        }));
                }
            }
        }
        result
    }

    pub fn count(&self, scope: Scope<'_>) -> Option<Count> {
        if !self.known {
            return None;
        }
        let count = match scope {
            Scope::Room(room) => self.rooms.get(room),
            Scope::Owner { id, room } => self.owners.get(&(id.to_owned(), room.map(str::to_owned))),
        };
        Some(count.copied().unwrap_or(Count {
            active: 0,
            total: 0,
        }))
    }
}

#[cfg(test)]
fn count(jobs: Option<&[Job<'_>]>, scope: Scope<'_>) -> Option<Count> {
    Summary::of(jobs).count(scope)
}

/// Renderer-supplied glyph capability; independent of color depth and Config.
#[derive(Clone, Copy, Default)]
pub enum GlyphSet {
    #[default]
    Unicode,
    Ascii,
}
impl GlyphSet {
    pub fn from_ascii(ascii: bool) -> Self {
        if ascii { Self::Ascii } else { Self::Unicode }
    }
    pub fn label(self, count: Option<Count>) -> String {
        label(count, matches!(self, Self::Ascii))
    }
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

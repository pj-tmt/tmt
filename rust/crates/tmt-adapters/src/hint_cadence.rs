//! When an optional hint about lasting state was last shown, so it repeats at
//! most once a day for each place it is about. Best effort throughout:
//! unreadable state shows the hint, and a failed write only shows it again.

use std::{collections::BTreeMap, path::Path};

/// The record of shown hints, in TMT's global directory.
pub const HINTS_FILE: &str = "hints.json";

/// At most this many places are remembered for one hint on one day; past it
/// the hint shows without being recorded.
const MAX_PLACES: usize = 256;
const READ_LIMIT: usize = 64 * 1024;

/// `hint` → place → the UTC day it was last shown there.
type Shown = BTreeMap<String, BTreeMap<String, u64>>;

/// Whether `hint` is due at `place` on UTC day `day` (days since the Unix
/// epoch), recording it as shown when it is. Only today's records are kept.
pub fn due_today(global_dir: &Path, hint: &str, place: &str, day: u64) -> bool {
    let path = global_dir.join(HINTS_FILE);
    let mut shown: Shown = crate::bounded_file::read_no_follow(&path, READ_LIMIT)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    for places in shown.values_mut() {
        places.retain(|_, shown_on| *shown_on == day);
    }
    shown.retain(|_, places| !places.is_empty());
    let places = shown.entry(hint.to_owned()).or_default();
    if places.contains_key(place) {
        return false;
    }
    if places.len() < MAX_PLACES {
        places.insert(place.to_owned(), day);
        if let Ok(bytes) = serde_json::to_vec(&shown) {
            let _ = crate::private_file::replace(&path, &bytes);
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDirectory;
    use std::fs;

    #[test]
    fn a_hint_shows_once_a_day_for_each_place() {
        let directory = TestDirectory::new();
        let global = directory.path.join("global");
        assert!(due_today(&global, "driver", "/s w1:p1", 100));
        assert!(!due_today(&global, "driver", "/s w1:p1", 100));
        // Another place, another hint: each is its own.
        assert!(due_today(&global, "driver", "/s w1:p2", 100));
        assert!(due_today(&global, "other", "/s w1:p1", 100));
        // The next day it shows again, and yesterday's records are dropped.
        assert!(due_today(&global, "driver", "/s w1:p1", 101));
        let kept: Shown =
            serde_json::from_slice(&fs::read(global.join(HINTS_FILE)).unwrap()).unwrap();
        assert_eq!(
            kept,
            Shown::from([("driver".into(), BTreeMap::from([("/s w1:p1".into(), 101)]))])
        );
    }

    #[test]
    fn unreadable_or_full_state_still_shows_the_hint() {
        let directory = TestDirectory::new();
        let global = directory.path.join("global");
        fs::create_dir_all(&global).unwrap();
        fs::write(global.join(HINTS_FILE), b"not json").unwrap();
        assert!(due_today(&global, "driver", "a", 7));
        assert!(!due_today(&global, "driver", "a", 7));
        for place in 0..MAX_PLACES {
            due_today(&global, "driver", &format!("p{place}"), 7);
        }
        // Past the bound the hint shows each time, unrecorded.
        assert!(due_today(&global, "driver", "late", 7));
        assert!(due_today(&global, "driver", "late", 7));
        // No directory to write in: shown, nothing recorded.
        let missing = directory.path.join("file").join("global");
        fs::write(directory.path.join("file"), b"").unwrap();
        assert!(due_today(&missing, "driver", "a", 7));
        assert!(due_today(&missing, "driver", "a", 7));
    }
}

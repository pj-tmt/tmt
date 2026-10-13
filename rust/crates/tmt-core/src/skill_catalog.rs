//! The bundled skills TMT installs, as data: the one place their names are
//! spelled. Parsing reads the names here; the adapters' catalog
//! (`tmt-adapters/src/skill_installation/catalog.rs`) pairs each with its
//! embedded bytes and joins owned skills from owner records.

/// Who publishes a bundled skill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    /// Core's own skills; their names are reserved from every owner.
    Core,
}

#[derive(Debug, PartialEq, Eq)]
pub struct BundledName {
    pub name: &'static str,
    pub group: Group,
}

/// Core's main skill: the one a driver's skill target names.
pub const MAIN: &str = "tmt";
/// Core's focused inbox skill, published beside the main one.
pub const INBOX: &str = "tmt-inbox";

/// The current core-only bundle in digest order.
pub static BUNDLED: [BundledName; 2] = [
    BundledName {
        name: MAIN,
        group: Group::Core,
    },
    BundledName {
        name: INBOX,
        group: Group::Core,
    },
];

/// Fixed historical bundle inventory, used only to verify older source generations.
/// Retired names are not bundled, selectable, publishable or adoption candidates.
pub const HISTORICAL_BUNDLE_NAMES: [&str; 5] = [
    MAIN,
    INBOX,
    "tmt-office",
    "tmt-prop-create",
    "tmt-avatar-create",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_and_core_leads() {
        let mut names: Vec<&str> = BUNDLED.iter().map(|skill| skill.name).collect();
        assert_eq!((names[0], names[1]), (MAIN, INBOX));
        assert!(BUNDLED[..2].iter().all(|skill| skill.group == Group::Core));
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), BUNDLED.len());
    }
}

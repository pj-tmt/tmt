//! The bundled skills TMT installs, as data: the one place their names are
//! spelled. Parsing reads the names here; the adapters' catalog
//! (`tmt-adapters/src/skill_installation/catalog.rs`) pairs each with its
//! embedded bytes and joins owned skills from owner records.

/// Who publishes a bundled skill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    /// Core's own skills; their names are reserved from every owner.
    Core,
    /// Office's skills, bundled before owners existed; owner `office`
    /// adopts them without force.
    Office,
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

/// The current bundle in digest order: the bundle digest frames the skills
/// in this order, so a new skill is appended, never inserted.
pub static BUNDLED: [BundledName; 5] = [
    BundledName {
        name: MAIN,
        group: Group::Core,
    },
    BundledName {
        name: INBOX,
        group: Group::Core,
    },
    BundledName {
        name: "tmt-office",
        group: Group::Office,
    },
    BundledName {
        name: "tmt-prop-create",
        group: Group::Office,
    },
    BundledName {
        name: "tmt-avatar-create",
        group: Group::Office,
    },
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

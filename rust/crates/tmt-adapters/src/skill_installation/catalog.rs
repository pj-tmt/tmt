//! The skill catalog: every skill TMT installs. Bundled skills are the core
//! names (`tmt_core::skill_catalog`) with their embedded bytes; owned skills
//! come from owner records and join through [`Catalog::new`]. Other modules
//! derive names, sources and inventories from here.

use std::collections::BTreeSet;
use tmt_core::skill_catalog::{self as names, BundledName};
pub use tmt_core::skill_catalog::{Group, INBOX, MAIN};

pub struct BundledSkill {
    pub name: &'static str,
    pub group: Group,
    pub bytes: &'static [u8],
}

const fn skill(entry: &'static BundledName, bytes: &'static [u8]) -> BundledSkill {
    BundledSkill {
        name: entry.name,
        group: entry.group,
        bytes,
    }
}

/// Each bundled name (`tmt_core::skill_catalog::BUNDLED`, same order) with its
/// embedded authored bytes.
pub(super) static BUNDLED: [BundledSkill; 5] = [
    skill(
        &names::BUNDLED[0],
        include_bytes!("../../../../../skills/tmux-team/SKILL.md"),
    ),
    skill(
        &names::BUNDLED[1],
        include_bytes!("../../../../../skills/tmt-inbox/SKILL.md"),
    ),
    skill(
        &names::BUNDLED[2],
        include_bytes!("../../../../../extensions/tmt-office/skills/tmt-office/SKILL.md"),
    ),
    skill(
        &names::BUNDLED[3],
        include_bytes!("../../../../../extensions/tmt-office/skills/tmt-prop-create/SKILL.md"),
    ),
    skill(
        &names::BUNDLED[4],
        include_bytes!("../../../../../extensions/tmt-office/skills/tmt-avatar-create/SKILL.md"),
    ),
];

/// Earlier bundle layouts, newest first, still verified so an upgrade can
/// recognize the sources it published. Each is a prefix of [`BUNDLED`] in
/// digest order. The oldest layout (only [`MAIN`], digested unframed) is
/// handled on its own.
pub(super) const PRIOR_LAYOUTS: [usize; 3] = [4, 3, 2];

pub(super) fn bundled(name: &str) -> Option<&'static BundledSkill> {
    BUNDLED.iter().find(|skill| skill.name == name)
}

/// Where a catalog entry comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    Bundled(Group),
    /// An installed extension, by owner name.
    Owner(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogEntry {
    pub name: String,
    pub origin: Origin,
}

/// The skills TMT knows about, from its sources. A name has one entry: a
/// bundled core skill is never shadowed by an owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Catalog {
    entries: Vec<CatalogEntry>,
}

impl Catalog {
    /// `owned` is `(name, owner)` pairs from owner records.
    pub fn new(
        bundled: &[BundledSkill],
        owned: impl IntoIterator<Item = (String, String)>,
    ) -> Self {
        let mut entries: Vec<CatalogEntry> = bundled
            .iter()
            .map(|skill| CatalogEntry {
                name: skill.name.to_owned(),
                origin: Origin::Bundled(skill.group),
            })
            .collect();
        for (name, owner) in owned {
            match entries.iter_mut().find(|entry| entry.name == name) {
                Some(entry) if entry.origin == Origin::Bundled(Group::Core) => {}
                Some(entry) => entry.origin = Origin::Owner(owner),
                None => entries.push(CatalogEntry {
                    name,
                    origin: Origin::Owner(owner),
                }),
            }
        }
        Self { entries }
    }

    /// The bundled skills alone.
    pub fn bundled() -> Self {
        Self::new(&BUNDLED, [])
    }

    pub fn entries(&self) -> &[CatalogEntry] {
        &self.entries
    }

    /// Names still published from the given bundle group.
    pub fn names(&self, group: Group) -> BTreeSet<&str> {
        self.entries
            .iter()
            .filter(|entry| entry.origin == Origin::Bundled(group))
            .map(|entry| entry.name.as_str())
            .collect()
    }

    pub fn is_core(&self, name: &str) -> bool {
        self.names(Group::Core).contains(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_names_are_unique_and_core_leads() {
        let names: BTreeSet<&str> = BUNDLED.iter().map(|skill| skill.name).collect();
        assert_eq!(names.len(), BUNDLED.len());
        assert_eq!((BUNDLED[0].name, BUNDLED[1].name), (MAIN, INBOX));
        assert!(
            BUNDLED
                .iter()
                .all(|skill| !skill.bytes.is_empty() && skill.bytes.len() <= 1_048_576)
        );
        // Each embedded file is the authored tree of the same name.
        for skill in &BUNDLED {
            let text = std::str::from_utf8(skill.bytes).unwrap();
            assert!(
                text.contains(&format!("\nname: {}\n", skill.name)),
                "{}",
                skill.name
            );
        }
        assert!(
            PRIOR_LAYOUTS
                .windows(2)
                .all(|pair| pair[0] > pair[1] && pair[0] < BUNDLED.len())
        );
    }

    #[test]
    fn owner_records_join_without_shadowing_core() {
        let catalog = Catalog::new(
            &BUNDLED,
            [
                (MAIN.to_owned(), "squad".to_owned()),
                (BUNDLED[2].name.to_owned(), "office".to_owned()),
                ("squad-playbook".to_owned(), "squad".to_owned()),
            ],
        );
        assert!(catalog.is_core(MAIN));
        assert!(!catalog.names(Group::Office).contains(BUNDLED[2].name));
        assert_eq!(
            catalog.entries().last(),
            Some(&CatalogEntry {
                name: "squad-playbook".into(),
                origin: Origin::Owner("squad".into())
            })
        );
        assert_eq!(
            Catalog::bundled().names(Group::Core),
            BTreeSet::from([MAIN, INBOX])
        );
    }
}

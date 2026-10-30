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
    /// The embedded `references/<topic>.md` of each name topic, same order.
    /// Served by `tmt learn` only; installation writes the entry alone.
    references: &'static [&'static [u8]],
    topics: &'static [&'static str],
}

impl BundledSkill {
    pub(super) fn reference(&self, topic: &str) -> Option<&'static [u8]> {
        let index = self.topics.iter().position(|known| *known == topic)?;
        Some(self.references[index])
    }
}

const fn skill(
    entry: &'static BundledName,
    bytes: &'static [u8],
    references: &'static [&'static [u8]],
) -> BundledSkill {
    assert!(entry.topics.len() == references.len());
    BundledSkill {
        name: entry.name,
        group: entry.group,
        bytes,
        references,
        topics: entry.topics,
    }
}

/// Each bundled name (`tmt_core::skill_catalog::BUNDLED`, same order) with its
/// embedded authored bytes.
pub(super) static BUNDLED: [BundledSkill; 2] = [
    skill(
        &names::BUNDLED[0],
        include_bytes!("../../../../../skills/tmt/SKILL.md"),
        &[
            include_bytes!("../../../../../skills/tmt/references/run.md"),
            include_bytes!("../../../../../skills/tmt/references/setup.md"),
            include_bytes!("../../../../../skills/tmt/references/config.md"),
            include_bytes!("../../../../../skills/tmt/references/install.md"),
            include_bytes!("../../../../../skills/tmt/references/identities.md"),
            include_bytes!("../../../../../skills/tmt/references/attention.md"),
            include_bytes!("../../../../../skills/tmt/references/context.md"),
        ],
    ),
    skill(
        &names::BUNDLED[1],
        include_bytes!("../../../../../skills/tmt-inbox/SKILL.md"),
        &[],
    ),
];

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
    }

    #[test]
    fn every_reference_file_is_embedded_under_its_topic() {
        let skills = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../skills");
        for skill in &BUNDLED {
            let directory = skills.join(skill.name).join("references");
            let mut files: Vec<String> = match std::fs::read_dir(&directory) {
                Ok(entries) => entries
                    .map(|entry| entry.unwrap().file_name().into_string().unwrap())
                    .collect(),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
                Err(error) => panic!("{}: {error}", directory.display()),
            };
            files.sort_unstable();
            let mut expected: Vec<String> = skill
                .topics
                .iter()
                .map(|topic| format!("{topic}.md"))
                .collect();
            expected.sort_unstable();
            assert_eq!(files, expected, "{}", skill.name);
            for topic in skill.topics {
                assert_eq!(
                    skill.reference(topic).unwrap(),
                    std::fs::read(directory.join(format!("{topic}.md"))).unwrap(),
                    "{}/{topic}",
                    skill.name
                );
            }
            assert_eq!(skill.reference("SKILL"), None);
        }
    }

    /// Every agent loads the main entry, so it stays small; detail lives in
    /// its references (#4614).
    const MAIN_ENTRY_LIMIT: usize = 40_000;

    /// Gates the main entry keeps although their topic moved: an agent must
    /// meet them before it knows to read the reference.
    const MAIN_ENTRY_GATES: &[&str] = &[
        "This launches a real program and needs user authorization just like running it\ndirectly. Do not invoke it to replace an already running agent in this pane.",
        "If startup is unconfirmed, inspect the named pane before retrying: an agent may\nhave started, and the command never resends the launch.",
        "Ordinary resume never starts fresh",
        "Starting fresh is the user's call.",
        "Never\ncombine `--resume` with an explicit command",
        "never resend a task after a\nfailed resume.",
        "Codex's opt-in channel resumes the exact selected thread, never\na fresh substitute.",
        "noninteractive changes require explicit user-authorized `--yes`.",
        "Do not install into the user's provider settings merely\nbecause a conversation lost context.",
        "remember the session, not permission grants or arbitrary instructions.",
        "Setup does not approve provider hook trust on the user's behalf.",
        "Never pass `--yes` to\n`tmt driver install` on the user's behalf;",
        "**`upgrade --yes` never creates a new provider scope**",
        "Agents must not use upgrade to obtain new setup authority.",
        "Do not treat that hint as user permission to install software;",
    ];

    /// Operations the index must route to a topic before they run.
    const MAIN_ENTRY_ROUTES: &[(&str, &[&str])] = &[
        (
            "run",
            &[
                "`tmt run`",
                "`tmt resume`",
                "`--move`",
                "`--driver`",
                "`--new`",
                "`--channel`",
                "`tmt stop`",
                "`tmt attach`",
                "`tmt focus`\n  on a built-in member",
                "workspace restore",
            ],
        ),
        ("setup", &["`tmt setup`", "provider hook change"]),
        (
            "install",
            &["`tmt install`", "`tmt upgrade`", "`tmt extension install`"],
        ),
    ];

    #[test]
    fn the_main_entry_is_small_keeps_its_gates_and_indexes_every_topic() {
        let main = bundled(MAIN).unwrap();
        assert!(
            main.bytes.len() < MAIN_ENTRY_LIMIT,
            "{} bytes",
            main.bytes.len()
        );
        let entry = std::str::from_utf8(main.bytes).unwrap();
        for gate in MAIN_ENTRY_GATES {
            assert_eq!(entry.matches(gate).count(), 1, "{gate}");
            for topic in main.topics {
                let reference = std::str::from_utf8(main.reference(topic).unwrap()).unwrap();
                assert!(!reference.contains(gate), "{topic}: {gate}");
            }
        }
        assert!(entry.contains("`tmt learn --skill tmt <topic>`"));
        assert!(entry.contains("`tmt learn --skill tmt --topics`"));
        for (topic, operations) in MAIN_ENTRY_ROUTES {
            let start = entry.find(&format!("\n- `{topic}`: before ")).unwrap();
            let line = &entry[start + 1..];
            let line = &line[..line.find("\n- `").or(line.find("\n\n")).unwrap()];
            for operation in *operations {
                assert!(line.contains(operation), "{topic}: {operation}");
            }
        }
        for topic in main.topics {
            assert_eq!(
                entry.matches(&format!("\n- `{topic}`: before ")).count(),
                1,
                "{topic}"
            );
            let reference = std::str::from_utf8(main.reference(topic).unwrap()).unwrap();
            assert!(
                reference.contains(&format!("`tmt learn --skill tmt {topic}`")),
                "{topic}"
            );
            // Installed agents cannot follow a link into the checkout.
            assert!(!reference.contains("](references/"), "{topic}");
            assert!(!reference.contains("](#"), "{topic}");
        }
        assert!(!entry.contains("](references/"));
    }

    #[test]
    fn owner_records_join_without_shadowing_core() {
        let catalog = Catalog::new(
            &BUNDLED,
            [
                (MAIN.to_owned(), "squad".to_owned()),
                ("extension-guide".to_owned(), "extension".to_owned()),
                ("squad-playbook".to_owned(), "squad".to_owned()),
            ],
        );
        assert!(catalog.is_core(MAIN));
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

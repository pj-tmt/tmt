//! The session state a board carries across an in-place reload. The old
//! process writes it just before `exec`; the new one has the same pid, finds
//! it by that pid and deletes it. It is a hint: a missing, old, unreadable or
//! foreign file means a normal start, and nothing here is evidence about Core.

use super::{app::RowTarget, home::Target as HomeTarget, row_detail::Target as DetailTarget};
use crate::{cache, config::Pane};
use serde_json::{Value, json};
use std::{
    fs, io,
    io::Read,
    path::{Path, PathBuf},
};

const VERSION: u64 = 1;
/// A snapshot older than this is a leftover, not a handoff.
const FRESH_MS: u64 = 60_000;
/// A real snapshot is a few hundred bytes; the cap bounds a foreign file.
const MAX_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Resume {
    pub tab: String,
    pub selected: Option<RowTarget>,
    pub focus: usize,
    pub follow: bool,
    pub scrolls: Vec<(Pane, usize)>,
    pub expanded: Vec<DetailTarget>,
    /// Pane fold overrides of the shown tab; `true` is folded.
    pub folds: Vec<(Pane, bool)>,
    /// The tabs this board admits; `None` while every tab is.
    pub picks: Option<Vec<String>>,
    pub search: String,
}

/// `$XDG_CACHE_HOME/tmt-ops/reload`: the snapshots and the reload request.
pub(super) fn directory() -> Option<PathBuf> {
    cache::directory("reload")
}

fn path(directory: &Path, pid: u32) -> PathBuf {
    directory.join(format!("resume-{pid}.json"))
}

impl Resume {
    /// Publishes the snapshot for the process that will have `pid` after the exec.
    pub fn write(&self, directory: &Path, pid: u32, now_ms: u64) -> io::Result<()> {
        let document = json!({
            "version": VERSION,
            "pid": pid,
            "writtenAtMs": now_ms,
            "tab": self.tab,
            "selected": self.selected.as_ref().map(row_json),
            "focus": self.focus,
            "follow": self.follow,
            "scrolls": self.scrolls.iter()
                .map(|(pane, offset)| json!([pane.title(), offset]))
                .collect::<Vec<_>>(),
            "expanded": self.expanded.iter().map(detail_json).collect::<Vec<_>>(),
            "folds": self.folds.iter()
                .map(|(pane, folded)| json!([pane.title(), folded]))
                .collect::<Vec<_>>(),
            "picks": self.picks,
            "search": self.search,
        });
        cache::replace(&path(directory, pid), document.to_string().as_bytes())
    }

    /// Reads and deletes this process's snapshot. Whatever is wrong with it,
    /// the file is gone afterwards and the board starts normally.
    pub fn take(directory: &Path, pid: u32, now_ms: u64) -> Option<Self> {
        let path = path(directory, pid);
        let read = read_regular(&path);
        let _ = fs::remove_file(&path);
        Self::decode(&serde_json::from_slice(&read?).ok()?, pid, now_ms)
    }

    /// Removes the snapshot of a restart that did not happen.
    pub fn discard(directory: &Path, pid: u32) {
        let _ = fs::remove_file(path(directory, pid));
    }

    fn decode(document: &Value, pid: u32, now_ms: u64) -> Option<Self> {
        if document["version"].as_u64()? != VERSION
            || document["pid"].as_u64()? != u64::from(pid)
            || now_ms.saturating_sub(document["writtenAtMs"].as_u64()?) > FRESH_MS
        {
            return None;
        }
        let pairs = |key: &str| -> Option<Vec<(Pane, Value)>> {
            document[key]
                .as_array()?
                .iter()
                .map(|pair| Some((Pane::parse(pair[0].as_str()?)?, pair[1].clone())))
                .collect()
        };
        Some(Self {
            tab: document["tab"].as_str()?.to_owned(),
            selected: match &document["selected"] {
                Value::Null => None,
                value => Some(row_from(value)?),
            },
            focus: usize::try_from(document["focus"].as_u64()?).ok()?,
            follow: document["follow"].as_bool()?,
            scrolls: pairs("scrolls")?
                .into_iter()
                .map(|(pane, offset)| Some((pane, usize::try_from(offset.as_u64()?).ok()?)))
                .collect::<Option<_>>()?,
            expanded: document["expanded"]
                .as_array()?
                .iter()
                .map(detail_from)
                .collect::<Option<_>>()?,
            folds: pairs("folds")?
                .into_iter()
                .map(|(pane, folded)| Some((pane, folded.as_bool()?)))
                .collect::<Option<_>>()?,
            picks: match &document["picks"] {
                Value::Null => None,
                value => Some(
                    value
                        .as_array()?
                        .iter()
                        .map(|key| key.as_str().map(str::to_owned))
                        .collect::<Option<_>>()?,
                ),
            },
            search: document["search"].as_str()?.to_owned(),
        })
    }
}

/// A regular file of bounded size, never a link: the directory is the user's,
/// but nothing the board reads here may redirect it elsewhere.
fn read_regular(path: &Path) -> Option<Vec<u8>> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_BYTES {
        return None;
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .ok()?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() as u64 <= MAX_BYTES).then_some(bytes)
}

fn row_json(target: &RowTarget) -> Value {
    match target {
        RowTarget::Home(target) => json!({
            "kind": "home",
            "section": target.section,
            "squad": target.squad,
            "member": target.member,
        }),
        RowTarget::Member {
            tab,
            section,
            squad,
            id,
        } => json!({"kind": "member", "tab": tab, "section": section, "squad": squad, "id": id}),
        RowTarget::Lead { tab, squad, id } => {
            json!({"kind": "lead", "tab": tab, "squad": squad, "id": id})
        }
    }
}

fn row_from(value: &Value) -> Option<RowTarget> {
    let text = |key: &str| value[key].as_str().map(str::to_owned);
    Some(match value["kind"].as_str()? {
        "home" => RowTarget::Home(HomeTarget {
            section: text("section")?,
            squad: text("squad")?,
            member: match &value["member"] {
                Value::Null => None,
                member => Some(member.as_str()?.to_owned()),
            },
        }),
        "member" => RowTarget::Member {
            tab: text("tab")?,
            section: usize::try_from(value["section"].as_u64()?).ok()?,
            squad: text("squad")?,
            id: text("id")?,
        },
        "lead" => RowTarget::Lead {
            tab: text("tab")?,
            squad: text("squad")?,
            id: text("id")?,
        },
        _ => return None,
    })
}

fn detail_json(target: &DetailTarget) -> Value {
    match target {
        DetailTarget::Row(target) => json!({"kind": "row", "target": row_json(target)}),
        DetailTarget::Job(id) => json!({"kind": "job", "id": id}),
    }
}

fn detail_from(value: &Value) -> Option<DetailTarget> {
    Some(match value["kind"].as_str()? {
        "row" => DetailTarget::Row(row_from(&value["target"])?),
        "job" => DetailTarget::Job(value["id"].as_str()?.to_owned()),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn sample() -> Resume {
        Resume {
            tab: "product".into(),
            selected: Some(RowTarget::Member {
                tab: "product".into(),
                section: 1,
                squad: "product".into(),
                id: "9d1f".into(),
            }),
            focus: 1,
            follow: false,
            scrolls: vec![(Pane::Rows, 7), (Pane::Notes, 2)],
            expanded: vec![
                DetailTarget::Row(RowTarget::Lead {
                    tab: "product".into(),
                    squad: "product".into(),
                    id: "a1".into(),
                }),
                DetailTarget::Job("c1".into()),
                DetailTarget::Row(RowTarget::Home(HomeTarget {
                    section: "needs-you".into(),
                    squad: "infra".into(),
                    member: None,
                })),
            ],
            folds: vec![(Pane::Detail, true), (Pane::Replies, false)],
            picks: Some(vec!["product".into(), "@all".into()]),
            search: "auth".into(),
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("ops-resume-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        root
    }

    #[test]
    fn a_snapshot_round_trips_and_is_consumed_once() {
        let root = scratch("round-trip");
        for resume in [
            sample(),
            Resume {
                selected: Some(RowTarget::Home(HomeTarget {
                    section: "squads".into(),
                    squad: "infra".into(),
                    member: Some("m1".into()),
                })),
                picks: None,
                ..sample()
            },
            Resume {
                selected: None,
                scrolls: Vec::new(),
                expanded: Vec::new(),
                folds: Vec::new(),
                search: String::new(),
                ..sample()
            },
        ] {
            resume.write(&root, 41, 1_000).unwrap();
            let file = path(&root, 41);
            assert_eq!(
                fs::metadata(&file).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(Resume::take(&root, 41, 1_500).as_ref(), Some(&resume));
            assert!(!file.exists(), "reading consumes the snapshot");
            assert_eq!(Resume::take(&root, 41, 1_500), None);
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn an_old_foreign_or_unreadable_snapshot_means_a_normal_start_and_is_removed() {
        let root = scratch("rejected");
        let write = |document: Value| {
            fs::create_dir_all(&root).unwrap();
            fs::write(path(&root, 41), document.to_string()).unwrap();
        };
        let valid = |edit: &dyn Fn(&mut Value)| {
            sample().write(&root, 41, 1_000).unwrap();
            let mut document: Value =
                serde_json::from_slice(&fs::read(path(&root, 41)).unwrap()).unwrap();
            edit(&mut document);
            write(document);
        };
        let rejects = |reason: &str| {
            assert_eq!(Resume::take(&root, 41, 1_500), None, "{reason}");
            assert!(!path(&root, 41).exists(), "{reason}: removed");
        };
        valid(&|_| {});
        assert!(
            Resume::take(&root, 41, 1_500).is_some(),
            "the control is valid"
        );
        valid(&|document| document["version"] = json!(2));
        rejects("another version");
        valid(&|document| document["pid"] = json!(42));
        rejects("another process");
        valid(&|_| {});
        assert_eq!(
            Resume::take(&root, 41, 1_000 + FRESH_MS + 1),
            None,
            "older than a minute"
        );
        valid(&|document| document["scrolls"] = json!([["nowhere", 1]]));
        rejects("an unknown pane");
        valid(&|document| document["selected"] = json!({"kind": "guess"}));
        rejects("an unknown target");
        valid(&|document| document["focus"] = json!(-1));
        rejects("a negative index");
        write(json!("not an object"));
        rejects("the wrong shape");
        fs::write(path(&root, 41), b"{").unwrap();
        rejects("invalid JSON");
        fs::write(path(&root, 41), vec![b' '; MAX_BYTES as usize + 1]).unwrap();
        rejects("an oversized file");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_link_is_never_followed() {
        let root = scratch("link");
        fs::create_dir_all(&root).unwrap();
        let target = root.join("elsewhere.json");
        sample().write(&root, 41, 1_000).unwrap();
        fs::rename(path(&root, 41), &target).unwrap();
        std::os::unix::fs::symlink(&target, path(&root, 41)).unwrap();
        assert_eq!(Resume::take(&root, 41, 1_500), None);
        assert!(target.exists(), "the link's target is untouched");
        assert!(
            fs::symlink_metadata(path(&root, 41)).is_err(),
            "only the link is removed"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn discard_removes_only_this_process_s_file() {
        let root = scratch("discard");
        sample().write(&root, 41, 1_000).unwrap();
        sample().write(&root, 42, 1_000).unwrap();
        Resume::discard(&root, 41);
        assert!(!path(&root, 41).exists() && path(&root, 42).exists());
        Resume::discard(&root, 41);
        let _ = fs::remove_dir_all(root);
    }
}

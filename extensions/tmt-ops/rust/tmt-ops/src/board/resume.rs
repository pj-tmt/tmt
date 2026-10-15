//! The session state a board carries across an in-place reload. The old
//! process writes it just before `exec`; the new one has the same pid, finds
//! it by that pid and deletes it. It is a hint: a missing, old, unreadable or
//! foreign file means a normal start, and nothing here is evidence about Core.

use super::{app::RowTarget, row_detail::Target as DetailTarget};
use crate::{cache, config::Pane};
use serde_json::{Value, json};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

const VERSION: u64 = 3;
/// A snapshot older than this is a leftover, not a handoff.
const FRESH_MS: u64 = 60_000;
/// A real snapshot is a few hundred bytes; the cap bounds a foreign file.
const MAX_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Resume {
    pub tab: String,
    pub layer: super::view_state::Layer,
    pub controls: Option<super::view_state::Controls>,
    pub shell: super::shell::OpsTab,
    pub squads_expanded: Vec<String>,
    pub pool_expanded: bool,
    pub visibility: super::workspace::OpsVisibility,
    pub selected: Option<RowTarget>,
    pub focus: usize,
    pub follow: bool,
    pub scrolls: Vec<(Pane, usize)>,
    pub expanded: Vec<DetailTarget>,
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
            "view": self.layer.name(),
            "controls": self.controls.as_ref().map(super::view_state::Controls::json),
            "shell": shell_json(self.shell),
            "squadsExpanded": self.squads_expanded,
            "poolExpanded": self.pool_expanded,
            "scope": self.visibility.scope,
            "revealHidden": self.visibility.reveal_hidden,
            "selected": self.selected.as_ref().map(row_json),
            "focus": self.focus,
            "follow": self.follow,
            "scrolls": self.scrolls.iter()
                .map(|(pane, offset)| json!([pane.title(), offset]))
                .collect::<Vec<_>>(),
            "expanded": self.expanded.iter().map(detail_json).collect::<Vec<_>>(),
            "search": self.search,
        });
        cache::replace(&path(directory, pid), document.to_string().as_bytes())
    }

    /// Reads and deletes this process's snapshot. Whatever is wrong with it,
    /// the file is gone afterwards and the board starts normally.
    pub fn take(directory: &Path, pid: u32, now_ms: u64) -> Option<Self> {
        let path = path(directory, pid);
        let read = cache::read_regular(&path, MAX_BYTES);
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
            layer: match document.get("view") {
                None => Default::default(),
                Some(value) => super::view_state::Layer::parse(value.as_str()?).ok()?,
            },
            controls: match document.get("controls") {
                None | Some(Value::Null) => None,
                Some(value) => Some(super::view_state::Controls::decode(value)?),
            },
            shell: shell_from(&document["shell"])?,
            squads_expanded: room_ids(&document["squadsExpanded"])?,
            pool_expanded: document["poolExpanded"].as_bool()?,
            visibility: super::workspace::OpsVisibility {
                scope: match document.get("scope") {
                    None => Vec::new(),
                    Some(value) => room_ids(value)?,
                },
                reveal_hidden: match document.get("revealHidden") {
                    None => false,
                    Some(value) => value.as_bool()?,
                },
            },
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
            search: document["search"].as_str()?.to_owned(),
        })
    }
}

pub(super) fn shell_json(tab: super::shell::OpsTab) -> &'static str {
    match tab {
        super::shell::OpsTab::Overview => "overview",
        super::shell::OpsTab::Agents => "agents",
        super::shell::OpsTab::Digest => "digest",
    }
}

pub(super) fn shell_from(value: &Value) -> Option<super::shell::OpsTab> {
    Some(match value.as_str()? {
        "overview" => super::shell::OpsTab::Overview,
        "agents" => super::shell::OpsTab::Agents,
        "digest" => super::shell::OpsTab::Digest,
        _ => return None,
    })
}

pub(super) fn room_ids(value: &Value) -> Option<Vec<String>> {
    value
        .as_array()?
        .iter()
        .map(|id| {
            id.as_str()
                .filter(|id| crate::config::uuid_like(id))
                .map(str::to_owned)
        })
        .collect()
}

pub(super) fn row_json(target: &RowTarget) -> Value {
    match target {
        RowTarget::Workspace(crate::board::workspace::OpsTarget::Member {
            room_id,
            identity_id,
        }) => json!({"kind":"workspace-member","roomId":room_id,"id":identity_id}),
        RowTarget::Workspace(crate::board::workspace::OpsTarget::Squad(id)) => {
            json!({"kind":"workspace-squad","id":id})
        }
        RowTarget::Workspace(crate::board::workspace::OpsTarget::Pool) => {
            json!({"kind":"workspace-pool"})
        }
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

pub(super) fn row_from(value: &Value) -> Option<RowTarget> {
    let text = |key: &str| value[key].as_str().map(str::to_owned);
    Some(match value["kind"].as_str()? {
        "workspace-squad" => RowTarget::Workspace(crate::board::workspace::OpsTarget::Squad(
            text("id").filter(|id| crate::config::uuid_like(id))?,
        )),
        "workspace-pool" => RowTarget::Workspace(crate::board::workspace::OpsTarget::Pool),
        "workspace-member" => RowTarget::Workspace(crate::board::workspace::OpsTarget::Member {
            room_id: match &value["roomId"] {
                Value::Null => None,
                value => Some(
                    value
                        .as_str()
                        .filter(|id| crate::config::uuid_like(id))?
                        .to_owned(),
                ),
            },
            identity_id: text("id").filter(|id| crate::config::uuid_like(id))?,
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
            layer: Default::default(),
            controls: None,
            visibility: Default::default(),
            shell: super::super::shell::OpsTab::Overview,
            squads_expanded: vec![],
            pool_expanded: false,
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
                DetailTarget::Row(RowTarget::Workspace(
                    crate::board::workspace::OpsTarget::Squad(
                        "44444444-4444-4444-8444-444444444444".into(),
                    ),
                )),
            ],
            search: "auth".into(),
        }
    }

    #[test]
    fn reload_visibility_round_trips_and_old_snapshots_default_to_unrestricted() {
        let root = scratch("visibility");
        let mut resume = sample();
        resume.visibility.scope = vec!["44444444-4444-4444-8444-444444444444".into()];
        resume.visibility.reveal_hidden = true;
        resume.write(&root, 73, 1_000).unwrap();
        assert_eq!(Resume::take(&root, 73, 1_001), Some(resume.clone()));
        resume.write(&root, 73, 1_000).unwrap();
        let mut document: Value =
            serde_json::from_slice(&std::fs::read(path(&root, 73)).unwrap()).unwrap();
        document.as_object_mut().unwrap().remove("scope");
        document.as_object_mut().unwrap().remove("revealHidden");
        assert_eq!(
            Resume::decode(&document, 73, 1_001).unwrap().visibility,
            Default::default()
        );
        document["scope"] = json!(["not-a-room-id"]);
        assert!(Resume::decode(&document, 73, 1_001).is_none());
        std::fs::remove_dir_all(root).unwrap();
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
                selected: Some(RowTarget::Workspace(
                    crate::board::workspace::OpsTarget::Member {
                        room_id: Some("44444444-4444-4444-8444-444444444444".into()),
                        identity_id: "55555555-5555-4555-8555-555555555555".into(),
                    },
                )),
                ..sample()
            },
            Resume {
                selected: None,
                scrolls: Vec::new(),
                expanded: Vec::new(),
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
            let stored: Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
            assert!(
                stored.get("picks").is_none() && stored.get("folds").is_none(),
                "reload cannot carry old named-tab admission or pane folds"
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
        valid(&|document| {
            document["selected"] =
                json!({"kind":"home","section":"blocked","squad":"infra","member":null})
        });
        rejects("a retired HOME cursor cannot confer a selection");
        valid(
            &|document| document["expanded"] = json!([{"kind":"row","target":{"kind":"home","section":"usage","squad":"","member":null}}]),
        );
        rejects("a retired HOME detail cannot reopen a surface");
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

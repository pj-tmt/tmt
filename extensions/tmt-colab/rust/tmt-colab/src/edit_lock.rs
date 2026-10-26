//! Versioned identity lock cells. Repeated edits replace one Y.Map value, so normal
//! own compaction collects old values and sealed updates rather than an action ledger.
use crate::{limits, threads::QuoteSelector};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use tmt_colab_model::{Invalid, Result, values};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Holder {
    pub id: String,
    pub name: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum Target {
    Whole(Whole),
    Quote(QuoteSelector),
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Whole {
    pub whole: bool,
}
impl Target {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Whole(value) => require(value.whole),
            Self::Quote(value) => value.validate(),
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Action {
    pub version: u8,
    pub kind: String,
    pub space_id: String,
    pub page_id: String,
    pub epoch: String,
    pub sender_device: String,
    pub revision: String,
    pub lock_id: String,
    pub holder: Holder,
    pub target: Target,
    pub released: bool,
}
impl Action {
    pub fn validate(&self) -> Result<()> {
        require(self.version == 1 && self.kind == "edit-lock")?;
        values::space_id(&self.space_id)?;
        values::generated_id(&self.page_id)?;
        values::decimal(&self.epoch, false)?;
        values::generated_id(&self.sender_device)?;
        values::generated_id(&self.lock_id)?;
        require(values::decimal(&self.revision, false)? <= limits::MAX_SEQUENCE)?;
        values::core_id(&self.holder.id)?;
        require(
            !self.holder.name.is_empty()
                && self.holder.name.len() <= 128
                && !self.holder.name.chars().any(char::is_control),
        )?;
        self.target.validate()
    }
}
fn require(valid: bool) -> Result<()> {
    if valid { Ok(()) } else { Err(Invalid) }
}
pub fn parse(root: &str, key: &str, value: &Value) -> Result<Action> {
    require(root == "messages")?;
    let action: Action = serde_json::from_value(value.clone()).map_err(|_| Invalid)?;
    action.validate()?;
    require(key == format!("{}:edit-lock", action.holder.id))?;
    Ok(action)
}
pub(crate) fn validate_record(root: &str, key: &str, value: &Value) -> Result<()> {
    if value["kind"] != "edit-lock" {
        return Ok(());
    }
    parse(root, key, value).map(|_| ())
}
/// Other own kinds remain immutable. Scope/holder cannot change within a lock cell,
/// and a stale same-key value cannot win merely because its Yjs client ID is greater.
pub(crate) fn advances(previous: &Value, next: &Value, root: &str, key: &str) -> bool {
    let (Ok(old), Ok(new)) = (parse(root, key, previous), parse(root, key, next)) else {
        return false;
    };
    old.space_id == new.space_id
        && old.page_id == new.page_id
        && old.epoch == new.epoch
        && old.sender_device == new.sender_device
        && old.holder.id == new.holder.id
        && new.revision.parse::<u64>().unwrap_or(0) > old.revision.parse::<u64>().unwrap_or(0)
}
/// The publishing owner increments the greatest revision it observed. Concurrent
/// revisions have a stable writer-ID tie; wall clocks and labels never order them.
pub fn fold(records: &[Action]) -> Vec<Action> {
    let mut winners: BTreeMap<String, Action> = BTreeMap::new();
    for record in records {
        if winners.get(&record.holder.id).is_none_or(|old| {
            record.revision.parse::<u64>().unwrap_or(0) > old.revision.parse::<u64>().unwrap_or(0)
                || (record.revision == old.revision && record.sender_device > old.sender_device)
        }) {
            winners.insert(record.holder.id.clone(), record.clone());
        }
    }
    winners.into_values().collect()
}
/// Colab lifecycle projection combining public presence and ended activity; no Core binding type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockHolderState {
    Active,
    Offline,
    Ended,
    Unknown,
}
impl LockHolderState {
    pub fn keeps_lock(self) -> bool {
        matches!(self, Self::Active | Self::Unknown)
    }
}
/// An absent local holder was retired from this machine. A foreign writer's absent
/// holder is only unobserved; the local directory cannot establish its lifecycle.
pub fn holder_state(
    lock: &Action,
    local_writer: &str,
    observed: &BTreeMap<String, LockHolderState>,
) -> LockHolderState {
    observed.get(&lock.holder.id).copied().unwrap_or_else(|| {
        if lock.sender_device == local_writer {
            LockHolderState::Ended
        } else {
            LockHolderState::Unknown
        }
    })
}
pub(crate) fn validate_limits(roots: &Value) -> Result<()> {
    let mut count = 0;
    if let Some(messages) = roots["messages"].as_object() {
        for (key, value) in messages {
            if value["kind"] == "edit-lock" {
                parse("messages", key, value)?;
                count += 1;
            }
        }
    }
    require(count <= limits::EDIT_LOCK_HOLDERS_PER_WRITER)
}
pub(crate) fn project(
    own: &BTreeMap<String, Value>,
    owners: &BTreeSet<String>,
    space: &str,
    page: &str,
    epoch: &str,
) -> Result<Vec<Action>> {
    let mut records = Vec::new();
    for (writer, roots) in own {
        if !owners.contains(writer) {
            continue;
        }
        if let Some(messages) = roots["messages"].as_object() {
            for (key, value) in messages {
                if value["kind"] != "edit-lock" {
                    continue;
                }
                let record = parse("messages", key, value)?;
                if record.sender_device == *writer
                    && record.space_id == space
                    && record.page_id == page
                    && record.epoch == epoch
                {
                    records.push(record);
                }
            }
        }
    }
    Ok(fold(&records))
}

/// A missing or ambiguous source quote is conservative: it covers the page until
/// the holder retargets it. The published text diff is the one overlap oracle.
pub fn overlaps(target: &Target, old: &str, new: &str) -> bool {
    let hunks = crate::edit_scale::content_hunks(old, new);
    if hunks.is_empty() {
        return false;
    }
    let Target::Quote(quote) = target else {
        return true;
    };
    let mut matches = old.match_indices(&quote.exact).filter(|(at, _)| {
        old[..*at].ends_with(&quote.prefix)
            && old[*at + quote.exact.len()..].starts_with(&quote.suffix)
    });
    let Some((start, _)) = matches.next() else {
        return true;
    };
    if matches.next().is_some() {
        return true;
    }
    let end = start + quote.exact.len();
    hunks.iter().any(|h| {
        if h.remove == 0 {
            h.start >= start && h.start <= end
        } else {
            h.start < end && h.start + h.remove > start
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Value {
        serde_json::from_str(include_str!("../../../contracts/vectors/edit-lock-v1.json")).unwrap()
    }
    #[test]
    fn literal_grammar_and_revision_admission_match_browser() {
        let f = fixture();
        let record = &f["record"];
        let key = format!("{}:edit-lock", record["holder"]["id"].as_str().unwrap());
        let first = parse("messages", &key, record).unwrap();
        assert!(parse("threads", &key, record).is_err());
        assert!(parse("messages", "substituted", record).is_err());
        assert_eq!(f["perWriter"], limits::EDIT_LOCKS_PER_WRITER);
        assert_eq!(f["holdersPerWriter"], limits::EDIT_LOCK_HOLDERS_PER_WRITER);
        for target in [
            serde_json::json!({"whole":false}),
            serde_json::json!({"whole":true,"exact":"x"}),
            serde_json::json!({"exact":"","prefix":"","suffix":""}),
            serde_json::json!({"exact":"x".repeat(16385),"prefix":"","suffix":""}),
        ] {
            let mut bad = record.clone();
            bad["target"] = target;
            assert!(parse("messages", &key, &bad).is_err());
        }
        let mut next = first.clone();
        next.revision = "2".into();
        next.released = true;
        let value = serde_json::to_value(&next).unwrap();
        assert!(advances(record, &value, "messages", &key));
        assert!(!advances(&value, record, "messages", &key));
        assert!(!advances(record, record, "messages", &key));
        let mut moved = value.clone();
        moved["pageId"] = serde_json::json!("10000000-0000-4000-8000-000000000002");
        assert!(!advances(record, &moved, "messages", &key));
    }
    #[test]
    fn retained_holder_cells_are_bounded_independently_of_revisions() {
        let mut messages = serde_json::Map::new();
        for n in 1..=limits::EDIT_LOCK_HOLDERS_PER_WRITER + 1 {
            let mut record = fixture()["record"].clone();
            let id = format!("50000000-0000-7000-8000-{n:012x}");
            record["holder"]["id"] = serde_json::json!(id);
            record["released"] = serde_json::json!(true);
            messages.insert(format!("{id}:edit-lock"), record);
        }
        assert!(validate_limits(&serde_json::json!({"messages":messages})).is_err());
        messages.remove("50000000-0000-7000-8000-000000000001:edit-lock");
        assert!(validate_limits(&serde_json::json!({"messages":messages})).is_ok());
    }
    #[test]
    fn absent_local_writer_holders_are_ended_but_foreign_absence_and_explicit_unknown_keep_locks() {
        let lock: Action = serde_json::from_value(fixture()["record"].clone()).unwrap();
        let empty = BTreeMap::new();
        assert_eq!(
            holder_state(&lock, &lock.sender_device, &empty),
            LockHolderState::Ended
        );
        assert_eq!(
            holder_state(&lock, "20000000-0000-4000-8000-000000000002", &empty),
            LockHolderState::Unknown
        );
        let observed = BTreeMap::from([(lock.holder.id.clone(), LockHolderState::Unknown)]);
        assert_eq!(
            holder_state(&lock, &lock.sender_device, &observed),
            LockHolderState::Unknown
        );
    }
    #[test]
    fn overlap_uses_precise_hunks_and_utf8_quote_boundaries() {
        let old = "左 Alpha 中 Beta 右";
        let target = Target::Quote(QuoteSelector {
            exact: "Beta".into(),
            prefix: " 中 ".into(),
            suffix: " 右".into(),
        });
        assert!(!overlaps(&target, old, old));
        assert!(!overlaps(&target, old, "左 Delta 中 Beta 右"));
        assert!(overlaps(&target, old, "左 Alpha 中 Gamma 右"));
        assert!(overlaps(&target, old, "左 Delta 中 Gamma 右"));
        assert!(overlaps(&target, "missing", "changed"));
        assert!(overlaps(
            &Target::Whole(Whole { whole: true }),
            old,
            "changed"
        ));
    }
    #[test]
    fn revision_winner_is_bounded_current_state_without_ancestry_or_timers() {
        let first: Action = serde_json::from_value(fixture()["record"].clone()).unwrap();
        let mut next = first.clone();
        next.sender_device = "20000000-0000-4000-8000-000000000002".into();
        next.revision = "2".into();
        next.lock_id = "40000000-0000-4000-8000-000000000002".into();
        for values in [
            vec![first.clone(), next.clone()],
            vec![next.clone(), first.clone()],
        ] {
            let folded = fold(&values);
            assert_eq!(folded.len(), 1);
            assert_eq!(folded[0].lock_id, next.lock_id);
        }
        assert!(LockHolderState::Unknown.keeps_lock());
        assert!(LockHolderState::Active.keeps_lock());
        assert!(!LockHolderState::Offline.keeps_lock());
        assert!(!LockHolderState::Ended.keeps_lock());
    }
}

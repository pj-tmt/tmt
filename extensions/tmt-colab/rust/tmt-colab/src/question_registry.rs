//! Editor-owned question registration and creator values. Verified answer records supply the
//! effective answered state separately; this value never grants a commenter content authority.
pub mod answers;

use crate::closed::{Invalid, Result, keys, plain, require, text};
use crate::summary_lines::validate_attribution;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use tmt_colab_model::values;

/// A registered identity is a stable Core ID plus its observed display name, never authority.
pub fn validate_agent(value: &Value) -> Result<()> {
    keys(value, &["id", "name"], &[])?;
    values::core_id(text(&value["id"])?).map_err(|_| Invalid)?;
    require(crate::decoder::valid_publisher_agent(text(&value["name"])?))
}

/// Creator attribution belongs to the admitted editor, not the selected identity.
pub fn validate_creator(value: &Value) -> Result<()> {
    keys(value, &["agent", "attribution"], &["machineId"])?;
    validate_agent(&value["agent"])?;
    validate_attribution(&value["attribution"])?;
    if let Some(machine) = value.get("machineId") {
        values::generated_id(text(machine)?).map_err(|_| Invalid)?;
    }
    Ok(())
}

/// Produces the selected routing preference with the actual editor's attribution.
pub fn set_creator(body: &Value, attribution: &Value) -> Result<Value> {
    validate_creator_selection(body)?;
    let mut creator = body.clone();
    creator["attribution"] = attribution.clone();
    validate_creator(&creator)?;
    Ok(creator)
}

pub fn validate_creator_selection(body: &Value) -> Result<()> {
    keys(body, &["agent"], &["machineId"])?;
    validate_agent(&body["agent"])?;
    if let Some(machine) = body.get("machineId") {
        values::generated_id(text(machine)?).map_err(|_| Invalid)?;
    }
    Ok(())
}

fn question_id(value: &Value) -> Result<&str> {
    let value = text(value)?;
    require(crate::page_check::valid_question_id(value))?;
    Ok(value)
}

fn title(value: &Value) -> Result<()> {
    let value = plain(value, crate::limits::QUESTION_TITLE_POINTS * 4)?;
    require(!value.is_empty() && value.chars().count() <= crate::limits::QUESTION_TITLE_POINTS)
}

/// Only open/handled are editor-owned. Answered is projected from a verified own record, so a
/// lost notification, deleted answer, or a commenter cannot leave stale content metadata behind.
pub fn validate(value: &Value) -> Result<()> {
    let entries = value.as_array().ok_or(Invalid)?;
    require(entries.len() <= crate::limits::QUESTION_PAGE)?;
    let mut seen = BTreeSet::new();
    for entry in entries {
        keys(
            entry,
            &["id", "state", "attribution"],
            &["agent", "title", "group", "task"],
        )?;
        require(seen.insert(question_id(&entry["id"])?))?;
        if let Some(agent) = entry.get("agent") {
            validate_agent(agent)?;
        }
        validate_attribution(&entry["attribution"])?;
        require(matches!(text(&entry["state"])?, "open" | "handled"))?;
        if let Some(value) = entry.get("title") {
            title(value)?;
        }
        if let Some(group) = entry.get("group") {
            question_id(group)?;
        }
        // A task-era entry may still name the task that handled it (#5825); it is read as plain
        // handled state and dropped by the entry's next change.
        if let Some(task) = entry.get("task") {
            require(entry["state"] == "handled")?;
            require(legacy_task_number(text(task)?))?;
        }
    }
    Ok(())
}

/// A task-era task ID, `T-` and a decimal number from 1 to 9,999,999 without a leading zero.
fn legacy_task_number(value: &str) -> bool {
    value.strip_prefix("T-").is_some_and(|digits| {
        (1..=7).contains(&digits.len())
            && !digits.starts_with('0')
            && digits.bytes().all(|b| b.is_ascii_digit())
    })
}

/// Pure trial action. The admitted write owner supplies attribution and resolves selected agents
/// before calling this function. An error leaves the input unchanged.
pub fn apply(base: &Value, action: &str, body: &Value, attribution: &Value) -> Result<Value> {
    validate(base)?;
    validate_attribution(attribution)?;
    let mut next = base.clone();
    let entries = next.as_array_mut().ok_or(Invalid)?;
    match action {
        "question.add" => {
            keys(body, &["id"], &["agent", "title", "group"])?;
            let id = question_id(&body["id"])?;
            if let Some(agent) = body.get("agent") {
                validate_agent(agent)?;
            }
            let existing = entries.iter_mut().find(|entry| entry["id"] == id);
            if let Some(entry) = existing {
                // An explicit editor update changes routing, never the handled lifecycle state.
                if let Some(agent) = body.get("agent")
                    && entry.get("agent") != Some(agent)
                {
                    entry["agent"] = agent.clone();
                    entry["attribution"] = attribution.clone();
                }
                if let Some(group) = body.get("group")
                    && entry.get("group") != Some(group)
                {
                    entry["group"] = group.clone();
                    entry["attribution"] = attribution.clone();
                }
                if let Some(value) = body.get("title") {
                    title(value)?;
                    if entry.get("title") != Some(value) {
                        entry["title"] = value.clone();
                        entry["attribution"] = attribution.clone();
                    }
                }
            } else {
                let mut entry = json!({"id":id,"state":"open","attribution":attribution});
                for key in ["agent", "title", "group"] {
                    if let Some(value) = body.get(key) {
                        entry[key] = value.clone();
                    }
                }
                entries.push(entry);
            }
        }
        "question.handle" | "question.reopen" => {
            keys(body, &["id"], &[])?;
            let id = question_id(&body["id"])?;
            let entry = entries
                .iter_mut()
                .find(|entry| entry["id"] == id)
                .ok_or(Invalid)?;
            let state = if action == "question.handle" {
                "handled"
            } else {
                "open"
            };
            if entry["state"] != state {
                entry["state"] = json!(state);
                entry["attribution"] = attribution.clone();
                entry.as_object_mut().ok_or(Invalid)?.remove("task");
            }
        }
        _ => return Err(Invalid),
    }
    validate(&next)?;
    Ok(next)
}

/// A page action on the page's question registry. Both the parent preflight and the isolated
/// decoder use this same bounded rule.
pub fn apply_to_page(
    meta: &Value,
    action: &str,
    body: &Value,
    attribution: &Value,
) -> Result<Value> {
    apply(
        meta.get("questions").unwrap_or(&json!([])),
        action,
        body,
        attribution,
    )
}

/// A best-effort public identity observation. Missing lookup information is unverified, never
/// proof of an unknown identity. `Some(None)` alone is an affirmative not-found result.
pub type Identities = BTreeMap<String, Option<String>>;

pub fn identity_ids(meta: &Value) -> Vec<String> {
    let mut ids = BTreeSet::new();
    if let Some(creator) = creator_agent(meta)
        && let Some(id) = creator["id"].as_str()
    {
        ids.insert(id.to_owned());
    }
    if let Some(entries) = meta["questions"].as_array() {
        for entry in entries {
            if let Some(id) = entry["agent"]["id"].as_str() {
                ids.insert(id.to_owned());
            }
        }
    }
    ids.into_iter().collect()
}

/// The current routing preference. Legacy migration is read-time only and requires the original
/// stable recipient pair; an original-author label alone can never supply an agent identity.
pub fn creator_agent(meta: &Value) -> Option<Value> {
    if let Some(creator) = meta.get("creator") {
        return Some(creator["agent"].clone());
    }
    let recipient = meta.get("creationRecipient")?;
    let id = recipient["agentId"].as_str()?;
    let name = meta["originalAuthor"].as_str().unwrap_or(id);
    Some(json!({"id":id,"name":name}))
}

/// An omitted assignment follows the creator; selecting a creator never backfills the entry.
pub fn question_agent<'a>(question: &'a Value, creator: Option<&'a Value>) -> Option<&'a Value> {
    question.get("agent").or(creator)
}

/// Registry findings over the markup checker's existing parsed tree, including ignored blocks.
/// `identities` is descriptive lookup evidence, not authorization.
pub fn findings(
    report: &crate::page_check::Report,
    meta: &Value,
    page: &str,
    identities: Option<&Identities>,
) -> Vec<crate::page_check::Finding> {
    findings_for_blocks(
        report,
        report.question_sources.iter(),
        meta,
        page,
        identities,
    )
}

/// Existing routing pairs remain admitted during an unrelated source edit. Consume occurrences,
/// not just IDs: duplicating a legacy block still introduces an unregistered question.
pub fn edit_findings(
    report: &crate::page_check::Report,
    base_source: &str,
    meta: &Value,
    page: &str,
    identities: Option<&Identities>,
) -> crate::Result<Vec<crate::page_check::Finding>> {
    if report.question_sources.is_empty() {
        return Ok(Vec::new());
    }
    let base = crate::page_check::check(base_source).map_err(|_| crate::page::Fault::Capacity)?;
    let mut remaining = BTreeMap::new();
    for block in &base.question_sources {
        *remaining
            .entry((block.id.as_str(), block.agent.trim()))
            .or_insert(0usize) += 1;
    }
    let changed = report.question_sources.iter().filter(|block| {
        if let Some(count) = remaining.get_mut(&(block.id.as_str(), block.agent.trim()))
            && *count > 0
        {
            *count -= 1;
            false
        } else {
            true
        }
    });
    Ok(findings_for_blocks(report, changed, meta, page, identities))
}

fn findings_for_blocks<'a>(
    report: &crate::page_check::Report,
    blocks: impl Iterator<Item = &'a crate::page_check::QuestionBlock>,
    meta: &Value,
    page: &str,
    identities: Option<&Identities>,
) -> Vec<crate::page_check::Finding> {
    use crate::page_check::{Finding, Severity};
    use crate::repair::{argument, option};
    let creator = creator_agent(meta);
    let entries = meta["questions"].as_array();
    let mut found = Vec::new();
    for block in blocks {
        if !crate::page_check::valid_question_id(&block.id) {
            continue; // The ordinary markup finding already gives the invalid-ID repair.
        }
        let entry =
            entries.and_then(|entries| entries.iter().find(|entry| entry["id"] == block.id));
        let explicit = block.agent.trim();
        let registration = || {
            let mut fix = format!(
                "tmt colab question add {} --id {}",
                argument(page),
                argument(&block.id)
            );
            if !explicit.is_empty() {
                let agent = if crate::decoder::valid_publisher_agent(explicit) {
                    explicit
                } else {
                    "AGENT_UUID"
                };
                fix.push_str(&format!(" {}", option("agent", agent)));
            }
            if let Some(question) = report
                .questions
                .iter()
                .find(|question| question.id == block.id)
            {
                fix.push_str(&format!(" {}", option("title", &question.title)));
                if let Some(group) = &question.group {
                    fix.push_str(&format!(" --group {}", argument(group)));
                }
            }
            fix
        };
        let (code, fix) = if let Some(entry) = entry {
            // An unassigned registration inherits the creator without rewriting the registry.
            // An assigned registration keeps its own target across creator changes.
            let target = question_agent(entry, creator.as_ref());
            let agent_id = target.and_then(|agent| agent["id"].as_str());
            if !explicit.is_empty()
                && agent_id.and_then(|id| identities.and_then(|known| known.get(id))) == Some(&None)
            {
                ("COLAB_QUESTION_AGENT_UNKNOWN", registration())
            } else if explicit.is_empty()
                || target.is_some_and(|agent| {
                    agent["name"].as_str() == Some(explicit)
                        || agent["id"].as_str() == Some(explicit)
                })
            {
                continue;
            } else {
                ("COLAB_QUESTION_AGENT_MISMATCH", registration())
            }
        } else {
            ("COLAB_QUESTION_UNREGISTERED", registration())
        };
        found.push(Finding {
            fix: Some(fix),
            element: "tmt-question".into(),
            ordinal: block.ordinal,
            id: Some(block.id.clone()),
            line: block.line,
            severity: Severity::Blocking,
            code,
            questions_skipped: None,
        });
        if found.len() == crate::limits::QUESTION_PAGE {
            break;
        }
    }
    found
}

/// A supported source write is refused before publication; the first code is stable, while every
/// affected block retains its line and executable repair in `findings`.
#[derive(Debug)]
pub struct Refused {
    pub findings: Vec<crate::page_check::Finding>,
}
/// Bounded wire details for source-write refusals, separate from the short Save message.
#[derive(Clone, Debug, serde::Serialize)]
pub struct WriteFinding {
    pub code: &'static str,
    pub line: u64,
    pub fix: String,
}
impl Refused {
    pub fn details(&self) -> Vec<WriteFinding> {
        // Leave space for the Save result's scope and short message in the existing 64 KiB
        // frame. The CLI can show all findings; a browser receives the first bounded repairs.
        let mut remaining = crate::limits::WS_FRAME_BYTES - 2048;
        self.findings
            .iter()
            .take(crate::limits::QUESTION_PAGE)
            .filter_map(|finding| {
                Some(WriteFinding {
                    code: finding.code,
                    line: finding.line,
                    fix: finding.fix.clone()?,
                })
            })
            .take_while(|finding| {
                let bytes = serde_json::to_vec(finding).map_or(usize::MAX, |bytes| bytes.len() + 1);
                if bytes > remaining {
                    false
                } else {
                    remaining -= bytes;
                    true
                }
            })
            .collect()
    }
    pub fn code(&self) -> &'static str {
        self.findings
            .first()
            .map_or("COLAB_INPUT_INVALID", |finding| finding.code)
    }
}
pub fn reason(code: &str) -> Option<&'static str> {
    Some(match code {
        "COLAB_QUESTION_UNREGISTERED" => "Register this question before placing it on the page.",
        "COLAB_QUESTION_AGENT_MISMATCH" => {
            "The question's data-agent differs from its registered agent."
        }
        "COLAB_QUESTION_AGENT_UNKNOWN" => {
            "The registered identity was positively reported as unknown; choose a known identity and use the same identity in data-agent."
        }
        _ => return None,
    })
}
impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (index, finding) in self.findings.iter().enumerate() {
            if index > 0 {
                f.write_str("\n")?;
            }
            write!(
                f,
                "{} at line {}. {} Nothing was written. Fix: {}",
                finding.code,
                finding.line,
                reason(finding.code).unwrap_or("Invalid question registration."),
                finding.fix.as_deref().unwrap_or_default()
            )?;
        }
        Ok(())
    }
}
impl std::error::Error for Refused {}

pub fn admit_source(
    source: &str,
    meta: &Value,
    page: &str,
    identities: Option<&Identities>,
) -> crate::Result<()> {
    let report = crate::page_check::check(source).map_err(|_| crate::page::Fault::Capacity)?;
    let findings = findings(&report, meta, page, identities);
    if !findings.is_empty() {
        return Err(Box::new(Refused { findings }));
    }
    Ok(())
}

pub fn admit_edit(source: &str, base_source: &str, meta: &Value) -> crate::Result<()> {
    let report = crate::page_check::check(source).map_err(|_| crate::page::Fault::Capacity)?;
    admit_checked(&report, base_source, meta)
}
/// [`admit_edit`] with the edited source's check already made by the caller.
pub(crate) fn admit_checked(
    report: &crate::page_check::Report,
    base_source: &str,
    meta: &Value,
) -> crate::Result<()> {
    let findings = edit_findings(report, base_source, meta, "PAGE", None)?;
    if !findings.is_empty() {
        return Err(Box::new(Refused { findings }));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const QUESTION: &str = "00000000-0000-0000-0000-000000000001";
    fn attribution(time: u64) -> Value {
        json!({"writerId":"10000000-0000-4000-8000-000000000001","timeMs":time})
    }
    fn registration() -> Value {
        json!({"id":QUESTION,"agent":{"id":"50000000-0000-1000-8000-000000000001","name":"agent"},"title":"Choose"})
    }
    #[test]
    fn shared_metadata_vectors_match_the_closed_native_codec() {
        let corpus: Value = serde_json::from_str(include_str!(
            "../../../contracts/vectors/question-metadata-v1.json"
        ))
        .unwrap();
        for case in corpus["cases"].as_array().unwrap() {
            let result = match case["kind"].as_str().unwrap() {
                "creator" => validate_creator(&case["value"]),
                "questions" => validate(&case["value"]),
                _ => panic!("unknown vector kind"),
            };
            assert_eq!(
                result.is_ok(),
                case["valid"].as_bool().unwrap(),
                "{}",
                case["name"]
            );
        }
    }
    #[test]
    fn registration_is_idempotent_and_title_updates_preserve_handled_state() {
        let first = apply(&json!([]), "question.add", &registration(), &attribution(1)).unwrap();
        assert_eq!(
            apply(&first, "question.add", &registration(), &attribution(2)).unwrap(),
            first
        );
        let handled = apply(
            &first,
            "question.handle",
            &json!({"id":QUESTION}),
            &attribution(3),
        )
        .unwrap();
        assert!(
            apply(
                &first,
                "question.handle",
                &json!({"id":QUESTION,"task":"T-1"}),
                &attribution(3)
            )
            .is_err()
        );
        let mut renamed = registration();
        renamed["title"] = json!("Updated question");
        let updated = apply(&handled, "question.add", &renamed, &attribution(4)).unwrap();
        assert_eq!(updated[0]["state"], "handled");
        assert_eq!(updated[0]["attribution"], attribution(4));
        let reopened = apply(
            &updated,
            "question.reopen",
            &json!({"id":QUESTION}),
            &attribution(5),
        )
        .unwrap();
        assert_eq!(reopened[0]["state"], "open");
        assert!(reopened[0].get("task").is_none());
    }
    #[test]
    fn invalid_or_forged_registry_values_are_refused_without_mutating_the_base() {
        let base = apply(&json!([]), "question.add", &registration(), &attribution(1)).unwrap();
        let mut other = registration();
        other["agent"]["id"] = json!("not-an-identity");
        assert!(apply(&base, "question.add", &other, &attribution(2)).is_err());
        assert_eq!(base[0]["agent"], registration()["agent"]);
        let mut forged = base.clone();
        forged[0]["state"] = json!("answered");
        assert!(validate(&forged).is_err());
        forged = base.clone();
        forged[0]["answer"] = json!("not a verified record");
        assert!(validate(&forged).is_err());
        assert!(validate(&json!([base[0], base[0]])).is_err());
        assert!(
            apply(
                &base,
                "question.handle",
                &json!({"id":QUESTION,"task":"T-0"}),
                &attribution(2)
            )
            .is_err()
        );
    }

    fn source(agent: &str) -> String {
        format!(
            "<p>intro</p>\n<tmt-question data-id=\"{QUESTION}\" data-agent=\"{agent}\" data-kind=\"short\"><p data-question>Choose</p></tmt-question>"
        )
    }

    #[test]
    fn review_edit_checks_new_occurrences_before_the_finding_cap() {
        let base = source("legacy").repeat(crate::limits::QUESTION_PAGE);
        let changed = format!("{base}{}", source("new agent"));
        let report = crate::page_check::check(&changed).unwrap();
        let findings = edit_findings(&report, &base, &json!({}), "page", None).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].ordinal, crate::limits::QUESTION_PAGE + 1);
        assert_eq!(findings[0].code, "COLAB_QUESTION_UNREGISTERED");
        assert_eq!(findings[0].severity.as_str(), "blocking");
        // Explicit checks and creation still compare the entire source, not the edit allowance.
        assert!(admit_source(&source("legacy"), &json!({}), "page", None).is_err());
    }

    #[test]
    fn source_mismatches_keep_lines_and_quoted_repairs_without_changing_metadata() {
        let registry = apply(&json!([]), "question.add", &registration(), &attribution(1)).unwrap();
        let meta = json!({"questions":registry});
        assert!(admit_source(&source("agent"), &meta, "page", None).is_ok());
        let report = crate::page_check::check(&source("other")).unwrap();
        let mismatch = findings(&report, &meta, "page", None);
        assert_eq!(mismatch[0].severity, crate::page_check::Severity::Blocking);
        assert_eq!(mismatch.len(), 1);
        assert_eq!(mismatch[0].line, 2);
        assert_eq!(mismatch[0].code, "COLAB_QUESTION_AGENT_MISMATCH");
        assert!(mismatch[0].fix.as_ref().unwrap().contains("--agent other"));
        let unregistered = findings(&report, &json!({}), "page", None);
        assert_eq!(unregistered[0].code, "COLAB_QUESTION_UNREGISTERED");
        let injected = crate::page_check::check(&source("$(touch x);'z")).unwrap();
        let refused = findings(&injected, &json!({}), "page", None);
        assert!(
            refused[0]
                .fix
                .as_ref()
                .unwrap()
                .contains("--agent '$(touch x);'\\''z'")
        );
        assert_eq!(meta["questions"], registry);
        assert!(admit_source("<p>no questions</p>", &meta, "page", None).is_ok());
    }

    #[test]
    fn identity_unavailability_is_unverified_and_only_confirmed_not_found_refuses() {
        let registry = apply(&json!([]), "question.add", &registration(), &attribution(1)).unwrap();
        let meta = json!({"questions":registry});
        let report = crate::page_check::check(&source("agent")).unwrap();
        assert!(findings(&report, &meta, "page", None).is_empty());
        assert!(findings(&report, &meta, "page", Some(&Identities::new())).is_empty());
        let mut known = Identities::new();
        known.insert(
            registration()["agent"]["id"].as_str().unwrap().to_owned(),
            None,
        );
        assert_eq!(
            findings(&report, &meta, "page", Some(&known))[0].code,
            "COLAB_QUESTION_AGENT_UNKNOWN"
        );
    }

    #[test]
    fn creator_fallback_requires_the_legacy_identity_pair_and_does_not_backfill_metadata() {
        let registry = apply(&json!([]), "question.add", &registration(), &attribution(1)).unwrap();
        let mut meta = json!({"questions":registry,"originalAuthor":"agent"});
        let report = crate::page_check::check(&source("")).unwrap();
        assert!(creator_agent(&meta).is_none());
        assert!(findings(&report, &meta, "page", None).is_empty());
        meta["creationRecipient"] = json!({"agentId":registration()["agent"]["id"],"machineId":"20000000-0000-4000-8000-000000000001"});
        assert!(findings(&report, &meta, "page", None).is_empty());
        assert!(meta.get("creator").is_none());
        let no_name = json!({"creationRecipient":meta["creationRecipient"]});
        assert_eq!(
            creator_agent(&no_name).unwrap()["name"],
            registration()["agent"]["id"]
        );
    }

    #[test]
    fn creator_changes_do_not_retarget_registered_questions_without_data_agent() {
        let registry = apply(&json!([]), "question.add", &registration(), &attribution(1)).unwrap();
        let mut meta = json!({"questions":registry,"creator":set_creator(
            &json!({"agent":registration()["agent"]}), &attribution(1)
        ).unwrap()});
        let report = crate::page_check::check(&source("")).unwrap();
        assert!(findings(&report, &meta, "page", None).is_empty());
        meta["creator"] = set_creator(
            &json!({"agent":{"id":"50000000-0000-1000-8000-000000000002","name":"New creator"}}),
            &attribution(2),
        )
        .unwrap();
        assert!(findings(&report, &meta, "page", None).is_empty());
        assert_eq!(meta["questions"], registry);
        let explicit = crate::page_check::check(&source("New creator")).unwrap();
        assert_eq!(
            findings(&explicit, &meta, "page", None)[0].code,
            "COLAB_QUESTION_AGENT_MISMATCH"
        );
        meta.as_object_mut().unwrap().remove("creator");
        assert!(findings(&report, &meta, "page", None).is_empty());
    }

    #[test]
    fn unassigned_questions_inherit_creator_without_mutating_assigned_questions() {
        let entry = json!({"id":QUESTION,"title":"Choose"});
        let unassigned = apply(&json!([]), "question.add", &entry, &attribution(1)).unwrap();
        assert!(unassigned[0].get("agent").is_none());
        assert_eq!(
            apply(&unassigned, "question.add", &entry, &attribution(2)).unwrap(),
            unassigned
        );
        let mut meta = json!({"questions":unassigned});
        assert!(question_agent(&unassigned[0], None).is_none());
        assert!(admit_source(&source(""), &meta, "page", None).is_ok());
        assert!(admit_source(&source("agent"), &meta, "page", None).is_err());
        for (id, name) in [
            ("50000000-0000-1000-8000-000000000001", "agent"),
            ("50000000-0000-1000-8000-000000000002", "New creator"),
        ] {
            meta["creator"] =
                set_creator(&json!({"agent":{"id":id,"name":name}}), &attribution(3)).unwrap();
            let creator = creator_agent(&meta).unwrap();
            assert_eq!(
                question_agent(&unassigned[0], Some(&creator)),
                Some(&creator)
            );
            assert!(admit_source(&source(name), &meta, "page", None).is_ok());
            assert_eq!(meta["questions"], unassigned);
        }
        let assigned = apply(
            &unassigned,
            "question.add",
            &registration(),
            &attribution(4),
        )
        .unwrap();
        assert_eq!(
            apply(&assigned, "question.add", &entry, &attribution(5)).unwrap(),
            assigned
        );
        let creator = creator_agent(&meta).unwrap();
        assert_eq!(
            question_agent(&assigned[0], Some(&creator)).unwrap()["name"],
            "agent"
        );
    }

    #[test]
    fn an_ignored_question_still_requires_registration() {
        let report =
            crate::page_check::check(&source("agent").replace("short", "invalid-kind")).unwrap();
        assert!(report.questions.is_empty());
        assert_eq!(report.question_sources.len(), 1);
        assert_eq!(
            findings(&report, &json!({}), "page", None)[0].code,
            "COLAB_QUESTION_UNREGISTERED"
        );
    }
}

#[cfg(test)]
mod refusal_wire_tests {
    use super::*;
    #[test]
    fn bad_authored_labels_do_not_expand_repairs_and_save_details_fit_the_existing_frame() {
        let source: String = (1..=70).map(|n| format!("<tmt-question data-id=\"00000000-0000-0000-0000-{n:012}\" data-kind=\"short\" data-agent=\"{}\"><p data-question>Question</p></tmt-question>\n", "x".repeat(2000))).collect();
        let report = crate::page_check::check(&source).unwrap();
        let mut findings = findings(
            &report,
            &json!({}),
            "10000000-0000-4000-8000-000000000001",
            None,
        );
        assert_eq!(findings.len(), crate::limits::QUESTION_PAGE);
        assert!(
            findings
                .iter()
                .all(|finding| finding.fix.as_ref().unwrap().len() <= 2048)
        );
        let long_repair = format!(
            "tmt colab question add PAGE --id UUID --agent {} --title {}",
            crate::repair::argument(&"'".repeat(128)),
            crate::repair::argument(&"'".repeat(200))
        );
        for finding in &mut findings {
            finding.fix = Some(long_repair.clone());
        }
        let refused = Refused { findings };
        let reply = crate::page::save::SaveResult::rejected(
            "10000000-0000-4000-8000-000000000001",
            &refused,
        );
        assert!(reply.message.as_ref().unwrap().len() <= 512);
        let details = reply.findings.as_ref().unwrap();
        assert!(!details.is_empty() && details.len() < crate::limits::QUESTION_PAGE);
        assert_eq!(details[0].line, 1);
        assert_eq!(details[0].fix, long_repair);
        let mut frame = serde_json::to_value(reply).unwrap();
        frame["version"] = json!(1);
        frame["type"] = json!("saveresult");
        frame["space"] = json!("fixture");
        frame["page"] = json!("10000000-0000-4000-8000-000000000001");
        frame["epoch"] = json!("1");
        assert!(serde_json::to_vec(&frame).unwrap().len() <= crate::limits::WS_FRAME_BYTES);
    }
}

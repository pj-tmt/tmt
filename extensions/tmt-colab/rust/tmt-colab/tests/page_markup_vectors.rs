//! The shared page markup vectors, run natively. The browser client runs the same file through
//! the browser's own parser (`e2e/question-vectors.spec.ts`), so the two cannot drift.

use std::collections::BTreeSet;

use serde_json::{Value, json};
use tmt_colab::{limits, page_check};

fn vectors() -> Value {
    serde_json::from_str(include_str!(
        "../../../contracts/vectors/page-markup-v1.json"
    ))
    .unwrap()
}

fn finding_key(finding: &Value) -> String {
    format!(
        "{} {} {} {} {} {} {}",
        finding["element"],
        finding["ordinal"],
        finding["code"],
        finding["severity"],
        finding["line"],
        finding["id"],
        finding["questionsSkipped"],
    )
}

#[test]
fn every_case_gets_the_verdict_the_browser_gave() {
    let vectors = vectors();
    assert_eq!(vectors["version"], 1);
    let mut checked = 0;
    for case in vectors["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let source: Vec<&str> = case["source"]
            .as_array()
            .unwrap()
            .iter()
            .map(|line| line.as_str().unwrap())
            .collect();
        let report = page_check::check(&source.join("\n")).unwrap();

        let shown: Vec<Value> = report
            .questions
            .iter()
            .map(|q| {
                json!({
                    "id": q.id, "kind": q.kind, "title": q.title, "choices": q.choices,
                    "other": q.other, "agent": q.agent, "group": q.group, "label": q.label,
                })
            })
            .collect();
        assert_eq!(
            case["admitted"]["questions"].as_array().unwrap(),
            &shown,
            "{name}: questions"
        );
        // A lone question is an internal group of itself: its own ID, no title.
        let groups: Vec<Value> = report
            .groups
            .iter()
            .map(|g| {
                let mut group = json!({"id": g.id, "title": g.title, "questions": g.questions});
                // Only an opted-in group carries `form`.
                if g.split {
                    group["split"] = json!(true);
                }
                if g.chip {
                    group["form"] = json!("chip");
                }
                group
            })
            .collect();
        assert_eq!(
            case["admitted"]["groups"].as_array().unwrap(),
            &groups,
            "{name}: groups"
        );

        // A case with no sections omits the key.
        let sections: Vec<Value> = report
            .reviews
            .iter()
            .map(|r| {
                json!({"id": r.id, "title": r.title, "agent": r.agent, "questions": r.questions})
            })
            .collect();
        assert_eq!(
            case["admitted"]
                .get("reviews")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
            sections,
            "{name}: reviews"
        );

        let readings: Vec<Value> = report
            .readings
            .iter()
            .map(|r| json!({"id": r.id, "title": r.title, "agent": r.agent, "why": r.why}))
            .collect();
        assert_eq!(
            case["admitted"]
                .get("readings")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
            readings,
            "{name}: readings"
        );

        let found: BTreeSet<String> = report
            .findings
            .iter()
            .map(|f| {
                finding_key(&json!({
                    "element": f.element, "ordinal": f.ordinal, "code": f.code,
                    "severity": f.severity.as_str(), "line": f.line, "id": f.id,
                    "questionsSkipped": f.questions_skipped,
                }))
            })
            .collect();
        let expected: BTreeSet<String> = case["findings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| {
                let mut f = f.clone();
                f["questionsSkipped"] = f.get("questionsSkipped").cloned().unwrap_or(Value::Null);
                finding_key(&f)
            })
            .collect();
        assert_eq!(found, expected, "{name}: findings");
        assert_eq!(
            report.findings.len(),
            case["findings"].as_array().unwrap().len(),
            "{name}: one finding per element and code"
        );
        checked += 1;
    }
    assert!(checked >= 50, "the corpus shrank to {checked} cases");
}

#[test]
fn the_vector_limits_equal_the_named_constants() {
    let limits_block = &vectors()["limits"];
    for (key, value) in [
        ("pageQuestions", limits::QUESTION_PAGE),
        ("pageReviews", limits::REVIEW_PAGE),
        ("pageReadings", limits::READING_PAGE),
        ("titlePoints", limits::QUESTION_TITLE_POINTS),
        ("choices", limits::QUESTION_CHOICES),
        ("choicePoints", limits::QUESTION_CHOICE_POINTS),
        ("agentPoints", limits::QUESTION_AGENT_POINTS),
        ("groupQuestions", limits::GROUP_QUESTIONS),
        ("labelPoints", limits::GROUP_LABEL_POINTS),
        ("dataImageBytes", limits::DATA_IMAGE_BYTES),
        ("dataImagesTotalBytes", limits::DATA_IMAGES_TOTAL_BYTES),
        ("attachmentRefs", limits::ATTACHMENT_REFS),
    ] {
        assert_eq!(limits_block[key], json!(value), "{key}");
    }
}

#[test]
fn every_reason_code_has_a_case_and_no_case_names_an_unknown_code() {
    let vectors = vectors();
    let used: BTreeSet<&str> = vectors["cases"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|case| case["findings"].as_array().unwrap())
        .map(|finding| finding["code"].as_str().unwrap())
        .collect();
    let known: BTreeSet<&str> = page_check::codes().into_iter().collect();
    // Resolution needs the page's attachments, not a source alone, and the two image sizes need
    // payloads of tens of KiB that do not belong in a contract file: the `page_check` unit tests
    // (and, for resolution, the acceptance suite) pin those four.
    let resolved = [
        "attachment-ref-unresolved",
        "attachment-ref-ambiguous",
        "image-data-oversized",
        "image-data-total",
    ];
    let unknown: Vec<_> = used.difference(&known).collect();
    let uncovered: Vec<_> = known
        .difference(&used)
        .filter(|code| !resolved.contains(code))
        .collect();
    assert!(
        unknown.is_empty(),
        "vectors name unknown codes: {unknown:?}"
    );
    assert!(
        uncovered.is_empty(),
        "codes without a vector: {uncovered:?}"
    );
}

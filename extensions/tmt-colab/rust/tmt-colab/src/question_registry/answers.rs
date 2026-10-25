//! Effective question state from the already verified conversation projection. No writes, Ask
//! outcome, author labels, source titles or timestamps establish whether an answer exists.
use crate::{closed::Result, export::conversations::Conversations};
use serde_json::{Value, json};
use std::collections::BTreeSet;

pub fn project(
    registry: &Value,
    view: &Conversations,
    device_writers: &BTreeSet<String>,
) -> Result<Vec<Value>> {
    super::validate(registry)?;
    Ok(registry
        .as_array()
        .expect("validated registry")
        .iter()
        .map(|entry| {
            let answer = view
                .threads
                .iter()
                .filter(|thread| !thread.deleted && thread.proposal.is_none())
                .flat_map(|thread| &thread.comments)
                .filter(|comment| {
                    !comment.deleted
                        && device_writers.contains(&comment.writer)
                        && comment.question_id.as_deref() == entry["id"].as_str()
                })
                .max_by(|a, b| {
                    (a.sequence.parse::<u64>().unwrap_or(0), &a.writer, &a.id).cmp(&(
                        b.sequence.parse::<u64>().unwrap_or(0),
                        &b.writer,
                        &b.id,
                    ))
                });
            let mut effective = entry.clone();
            if let Some(answer) = answer {
                if effective["state"] == "open" {
                    effective["state"] = json!("answered");
                }
                let summary = crate::summary_passages::excerpt(
                    &answer.body,
                    crate::limits::QUESTION_ANSWER_POINTS,
                );
                effective["answer"] = json!(summary);
                effective["answeredAt"] = json!(answer.at);
                effective["answerRef"] =
                    json!({"writer":answer.writer,"id":answer.id,"revision":answer.revision});
            }
            effective
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn verified_answer_states_and_per_question_counts_match_shared_vectors() {
        let corpus: Value = serde_json::from_str(include_str!(
            "../../../../contracts/vectors/question-answers-v1.json"
        ))
        .unwrap();
        for case in corpus["cases"].as_array().unwrap() {
            let view = crate::summary_pending::tests::view(case);
            let writers = case["deviceWriters"]
                .as_array()
                .unwrap()
                .iter()
                .map(|id| id.as_str().unwrap().to_owned())
                .collect();
            let actual = project(&case["registry"], &view, &writers).unwrap();
            assert_eq!(
                serde_json::to_value(&actual).unwrap(),
                case["expected"],
                "{}",
                case["name"]
            );
            let (report, anchors) = crate::page_check::summary("", &view.threads).unwrap();
            let pending =
                crate::summary_pending::items(&report, &anchors, &view, &writers, &actual);
            let ids: Vec<_> = pending.iter().map(|item| item.id.clone()).collect();
            assert_eq!(
                serde_json::to_value(ids).unwrap(),
                case["pending"],
                "{}",
                case["name"]
            );
        }
    }
}

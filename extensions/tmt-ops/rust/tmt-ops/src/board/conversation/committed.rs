//! Core commits order; this projection only groups adjacent inputs and selects identities once.
use crate::board::recent_responses::timeline::{Entry, Kind};
use std::ops::Range;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Block {
    Inputs(Range<usize>),
    Final(usize),
    Reply(usize),
    Failure(usize),
}

pub(super) fn blocks(entries: &[Entry]) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut finals = std::collections::HashSet::new();
    let mut replies = std::collections::HashSet::new();
    let mut failures = std::collections::HashSet::new();
    let mut index = 0;
    while index < entries.len() {
        let entry = &entries[index];
        match &entry.kind {
            Kind::Input(_) => {
                let start = index;
                index += 1;
                while index < entries.len()
                    && matches!(entries[index].kind, Kind::Input(_))
                    && entry.batch.is_some()
                    && entry.batch == entries[index].batch
                {
                    index += 1;
                }
                blocks.push(Block::Inputs(start..index));
                continue;
            }
            Kind::Final { response_id, .. } => {
                if finals.insert(response_id) {
                    blocks.push(Block::Final(index));
                }
            }
            Kind::Reply { request, .. } => {
                if replies.insert(request) {
                    blocks.push(Block::Reply(index));
                }
            }
            Kind::Failure { .. } => {
                if failures.insert(&entry.id) {
                    blocks.push(Block::Failure(index));
                }
            }
        }
        index += 1;
    }
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::recent_responses::timeline::{Body, Input, Sender};

    fn input(id: &str, batch: Option<&str>) -> Entry {
        Entry {
            id: id.into(),
            acknowledgements: Vec::new(),
            acknowledgements_truncated: false,
            batch: batch.map(str::to_owned),
            kind: Kind::Input(Input {
                request: id.into(),
                sender: Sender {
                    name: Some("core-1".into()),
                    identity: None,
                },
                uncertain: false,
                body: Some(Body {
                    text: id.into(),
                    truncated: false,
                    original_bytes: id.len() as u64,
                }),
            }),
        }
    }
    fn final_entry(id: &str, response: &str) -> Entry {
        Entry {
            id: id.into(),
            acknowledgements: Vec::new(),
            acknowledgements_truncated: false,
            batch: Some("g".into()),
            kind: Kind::Final {
                response_id: response.into(),
                available: true,
            },
        }
    }

    #[test]
    fn the_incident_keeps_a_before_the_digest_input_and_b_after_it() {
        let entries = [
            input("Ben question", Some("g")),
            final_entry("z", "rsp_A"),
            input("Digest question", Some("g")),
            final_entry("a", "rsp_B"),
        ];
        assert_eq!(
            blocks(&entries),
            [
                Block::Inputs(0..1),
                Block::Final(1),
                Block::Inputs(2..3),
                Block::Final(3)
            ]
        );
    }

    #[test]
    fn only_consecutive_inputs_in_the_explicit_same_batch_group_even_before_a_final() {
        let entries = [
            input("one", Some("g")),
            input("two", Some("g")),
            input("new", Some("h")),
            input("legacy", None),
            input("unrelated", None),
        ];
        assert_eq!(
            blocks(&entries),
            [
                Block::Inputs(0..2),
                Block::Inputs(2..3),
                Block::Inputs(3..4),
                Block::Inputs(4..5)
            ]
        );
    }

    #[test]
    fn identity_deduplication_never_groups_inputs_across_a_final() {
        let entries = [
            final_entry("first", "rsp_A"),
            input("before", Some("g")),
            final_entry("duplicate", "rsp_A"),
            input("after", Some("g")),
            final_entry("other", "rsp_B"),
        ];
        assert_eq!(
            blocks(&entries),
            [
                Block::Final(0),
                Block::Inputs(1..2),
                Block::Inputs(3..4),
                Block::Final(4)
            ]
        );
    }

    #[test]
    fn submitted_replies_use_request_identity_and_failures_use_entry_identity() {
        let reply = |id: &str, request: &str| Entry {
            id: id.into(),
            acknowledgements: Vec::new(),
            acknowledgements_truncated: false,
            batch: Some("g".into()),
            kind: Kind::Reply {
                request: request.into(),
                body: Some(Body {
                    text: "same text".into(),
                    truncated: false,
                    original_bytes: 9,
                }),
            },
        };
        let fail = |id: &str| Entry {
            id: id.into(),
            acknowledgements: Vec::new(),
            acknowledgements_truncated: false,
            batch: Some("g".into()),
            kind: Kind::Failure {
                code: "worker_error".into(),
            },
        };
        let entries = [
            reply("a", "req1"),
            reply("b", "req1"),
            reply("c", "req2"),
            final_entry("d", "rsp_A"),
            fail("e"),
            fail("f"),
            fail("e"),
        ];
        assert_eq!(
            blocks(&entries),
            [
                Block::Reply(0),
                Block::Reply(2),
                Block::Final(3),
                Block::Failure(4),
                Block::Failure(5)
            ]
        );
    }
}

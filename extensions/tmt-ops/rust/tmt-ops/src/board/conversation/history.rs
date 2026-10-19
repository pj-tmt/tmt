//! Storage-backed finals remain distinct messages, independent of follow mode.
use super::*;

/// A reply to one of this session's sends whose body is still being read stays
/// invisible for this long; the working line remains until the reply replaces it,
/// so no "reading" placeholder flashes between them. A body unreadable for longer
/// shows its honest placeholder instead of an endless working line.
const PLACEHOLDER_HOLD_MS: u64 = 15_000;

impl Session {
    pub(super) fn observe_finals(&mut self, mut next: Vec<Reply>) {
        let now = crate::status::now_ms();
        self.pending_replies.clear();
        next.retain(|reply| {
            let held = !reply.acquired
                && reply
                    .at_ms
                    .is_some_and(|at| now.saturating_sub(at) < PLACEHOLDER_HOLD_MS)
                && reply.key.as_ref().is_some_and(|key| {
                    self.outgoing
                        .iter()
                        .any(|entry| entry.request() == Some(key.request.as_str()))
                });
            if held && let Some(key) = &reply.key {
                self.pending_replies.insert(key.request.clone());
            }
            !held
        });
        for reply in &mut next {
            // A body refresh cannot erase a source already acquired for this ID.
            if !reply.acquired
                && let Some(old) = self
                    .finals
                    .iter()
                    .find(|old| old.acquired && same_source(old, reply))
            {
                reply.body = old.body.clone();
                reply.acquired = old.acquired;
                reply.source = old.source;
            }
        }
        for retained in self.local_finals() {
            if !next.iter().any(|reply| same_source(retained, reply)) {
                next.push(retained.clone());
            }
        }
        next.sort_by_key(|reply| reply.at_ms);
        // Keep the one message already being read when the bounded source page
        // rolls over. New finals still append; End releases this held message.
        if !self.follow
            && let Some(held) = self.reply.as_ref().filter(|reply| {
                reply
                    .key
                    .as_ref()
                    .is_some_and(|key| key.kind == super::super::home_leads::Kind::Reply)
            })
            && !next.iter().any(|reply| same_source(held, reply))
        {
            next.push(held.clone());
            next.sort_by_key(|reply| reply.at_ms);
        }
        let changed = self.finals.len() != next.len()
            || self.finals.iter().zip(&next).any(|(old, new)| {
                !same_source(old, new)
                    || old.body != new.body
                    || old.from != new.from
                    || old.at_ms != new.at_ms
                    || old.acquired != new.acquired
                    || old.source != new.source
            });
        self.finals = next;
        if changed {
            self.rendered = None;
        }
    }

    /// The local outgoing transcript may already contain this request's final.
    /// Its request/status block stays; only the duplicate final body is replaced.
    pub(super) fn block_final(&self, rows: &[usize], mapped: &Mapped) -> Option<usize> {
        let key = if self.outgoing.is_empty() {
            self.reply.as_ref().and_then(|reply| reply.key.as_ref())
        } else {
            self.response_spans
                .iter()
                .find(|(_, _, range)| {
                    rows.iter()
                        .any(|index| range.contains(&mapped.sources[*index]))
                })
                .map(|(_, key, _)| key)
        };
        self.finals.iter().position(|reply| {
            key.is_some_and(|key| reply.key.as_ref().is_some_and(|held| held.same_source(key)))
        })
    }
}

#[derive(Clone, Copy)]
pub(super) enum Block {
    Existing(usize),
    Final(usize),
}

//! Per-body folding changes the view, never retained text or request identity.
use super::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
};

const PREVIEW_ROWS: usize = 12;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Key {
    Reply(String),
    Outgoing(u64),
    Pane(String),
}

pub(super) struct Body {
    pub key: Key,
    pub sources: Range<usize>,
    pub newest_reply: bool,
}

#[derive(Default)]
pub(super) struct Folds {
    expanded: BTreeMap<Key, bool>,
    pub focused: Option<Key>,
}

pub(super) struct Projection {
    pub mapped: Mapped,
    pub original: Vec<Option<usize>>,
    pub anchors: Vec<usize>,
    pub controls: BTreeMap<usize, Key>,
}

impl Folds {
    pub fn toggle(&mut self, key: &Key) -> bool {
        let Some(expanded) = self.expanded.get_mut(key) else {
            return false;
        };
        *expanded = !*expanded;
        true
    }

    pub fn project(
        &mut self,
        mapped: Mapped,
        original: Option<Vec<Option<usize>>>,
        bodies: &[Body],
        width: usize,
        look: Look,
    ) -> Projection {
        let live: BTreeSet<_> = bodies.iter().map(|body| body.key.clone()).collect();
        self.expanded.retain(|key, _| live.contains(key));
        if self.focused.as_ref().is_some_and(|key| !live.contains(key)) {
            self.focused = None;
        }
        let anchors = mapped.sources.clone();
        let raw: Vec<_> = mapped
            .sources
            .iter()
            .map(|source| {
                original
                    .as_ref()
                    .map_or(Some(*source), |map| map.get(*source).copied().flatten())
            })
            .collect();
        let mut hidden = BTreeSet::new();
        let mut controls_at = BTreeMap::new();
        for body in bodies {
            let expanded = *self
                .expanded
                .entry(body.key.clone())
                .or_insert(body.newest_reply);
            let rows: Vec<_> = body.sources.clone().collect();
            if rows.len() <= PREVIEW_ROWS {
                continue;
            }
            if expanded {
                controls_at.insert(
                    rows[rows.len() - 1],
                    (body.key.clone(), "⏎ collapse".to_owned(), false),
                );
            } else {
                controls_at.insert(
                    rows[PREVIEW_ROWS - 1],
                    (
                        body.key.clone(),
                        format!(
                            "… {} more lines · ⏎ expand",
                            rows.len() - (PREVIEW_ROWS - 1)
                        ),
                        true,
                    ),
                );
                hidden.extend(rows.into_iter().skip(PREVIEW_ROWS - 1));
            }
        }
        if self
            .focused
            .as_ref()
            .is_some_and(|key| !controls_at.values().any(|(control, _, _)| control == key))
        {
            self.focused = None;
        }
        let mut result = Projection {
            mapped: Mapped {
                lines: Vec::new(),
                sources: Vec::new(),
                mentions: Vec::new(),
            },
            original: Vec::new(),
            anchors: Vec::new(),
            controls: BTreeMap::new(),
        };
        for (index, (line, mentions)) in mapped.lines.into_iter().zip(mapped.mentions).enumerate() {
            if let Some((key, caption, before)) = controls_at.get(&index)
                && *before
            {
                result.control(
                    key,
                    caption,
                    width,
                    look,
                    self.focused.as_ref(),
                    anchors[index],
                );
            }
            if !hidden.contains(&index) {
                result.row(line, mentions, raw[index], anchors[index]);
            }
            if let Some((key, caption, before)) = controls_at.get(&index)
                && !*before
            {
                result.control(
                    key,
                    caption,
                    width,
                    look,
                    self.focused.as_ref(),
                    anchors[index],
                );
            }
        }
        result
    }
}

impl Projection {
    fn row(
        &mut self,
        line: Line<'static>,
        mentions: Vec<super::super::member_links::Link>,
        original: Option<usize>,
        anchor: usize,
    ) {
        self.mapped.sources.push(self.mapped.lines.len());
        self.mapped.lines.push(line);
        self.mapped.mentions.push(mentions);
        self.original.push(original);
        self.anchors.push(anchor);
    }
    fn control(
        &mut self,
        key: &Key,
        text: &str,
        width: usize,
        look: Look,
        focused: Option<&Key>,
        anchor: usize,
    ) {
        let mut style = look.role(Role::Muted);
        if focused == Some(key) {
            style = look.role(Role::Text).add_modifier(
                ratatui::style::Modifier::BOLD | ratatui::style::Modifier::UNDERLINED,
            );
        }
        self.controls.insert(self.mapped.lines.len(), key.clone());
        self.row(
            Line::styled(crate::board::view::fit(text, width), style),
            Vec::new(),
            None,
            anchor,
        );
    }
}

#[cfg(test)]
mod tests;

#[derive(Clone)]
pub(super) struct Hit {
    pub area: Rect,
    pub conversation: ConversationKey,
    pub key: Key,
}

mod interaction;

impl Session {
    /// Resolve retained source ranges into this transcript's visual rows once.
    pub(super) fn visual_bodies(
        &self,
        transcript: &super::transcript::RenderedTranscript,
    ) -> Vec<Body> {
        let mut bodies = Vec::new();
        for mut body in self.body_ranges() {
            let rows: Vec<_> = transcript
                .mapped
                .sources
                .iter()
                .enumerate()
                .filter_map(|(index, source)| {
                    let original = transcript
                        .response_sources
                        .as_ref()
                        .map_or(Some(*source), |map| map.get(*source).copied().flatten());
                    original
                        .filter(|source| body.sources.contains(source))
                        .map(|_| index)
                })
                .collect();
            if let (Some(first), Some(last)) = (rows.first(), rows.last()) {
                body.sources = *first..last + 1;
                bodies.push(body);
            }
        }
        bodies.extend(transcript.bodies.iter().map(|body| Body {
            key: body.key.clone(),
            sources: body.sources.clone(),
            newest_reply: body.newest_reply,
        }));
        bodies
    }
}

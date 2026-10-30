//! Admission and presentation reuse the actor-fenced results and existing lanes.
use super::{Count, Page, publish::Seen};
use crate::board::{app::App, workspace::OpsTarget};

impl App {
    fn unread_page(&self) -> Option<&Page> {
        let actor = self.view.as_ref()?.me_id.as_deref()?;
        self.home_leads.unread(actor)
    }

    pub(in crate::board) fn answer_unread(&self, actor: &str, answer: &super::Answer) -> bool {
        !answer.acknowledged || self.unread.local.contains(actor, answer)
    }

    /// Resolve UUID membership once before sorting; local marks share this projection.
    pub(in crate::board) fn unread_identities(&self) -> std::collections::BTreeSet<String> {
        let Some(actor) = self.view.as_ref().and_then(|view| view.me_id.as_deref()) else {
            return Default::default();
        };
        // Historical overlay pages do not change Core's bounded sorting authority.
        // Explicit local marks add their members independently of that head page.
        let mut members: std::collections::BTreeSet<String> = self
            .unread_page()
            .map(|page| {
                page.replies
                    .keys()
                    .filter(|member| page.count(member).total > 0)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        members.extend(self.unread.local.members(actor));
        members
    }

    pub(in crate::board) fn unread_pending(&self) -> bool {
        let Some(actor) = self.view.as_ref().and_then(|view| view.me_id.as_deref()) else {
            return false;
        };
        self.unread.local.has_marks(actor)
            || self.unread_page().is_some_and(|page| {
                page.replies
                    .values()
                    .flatten()
                    .any(|answer| !answer.acknowledged)
            })
    }

    pub(in crate::board) fn unread_count(&self, member: &str) -> Count {
        let Some(actor) = self.view.as_ref().and_then(|view| view.me_id.as_deref()) else {
            return Count::default();
        };
        let page = self.unread_page();
        let mut count = page.map_or_else(Count::default, |page| page.count(member));
        if let (Some(overlay), Some(page)) = (&self.unread_overlay, page) {
            count.add(overlay.additional_count(page, member));
        }
        if !self.unread.local.has_marks(actor) {
            return count;
        }
        let mut known: std::collections::BTreeMap<String, super::Answer> =
            std::collections::BTreeMap::new();
        let held = self
            .unread_overlay
            .as_ref()
            .map(|overlay| overlay.answers(actor, member))
            .into_iter()
            .flatten();
        let head = page.map(|page| page.answers(member)).into_iter().flatten();
        for answer in self
            .unread
            .local
            .answers(actor, member)
            .into_iter()
            .chain(held.cloned())
            .chain(head.cloned())
        {
            let entry = known
                .entry(answer.request.clone())
                .or_insert_with(|| answer.clone());
            if answer.revision > entry.revision
                || answer.revision == entry.revision && answer.acknowledged
            {
                *entry = answer.clone();
            }
        }
        count.total = known
            .values()
            .filter(|answer| self.answer_unread(actor, answer))
            .count();
        count
    }
    pub(in crate::board) fn unread_target(&self, target: &OpsTarget) -> Count {
        if self.unread_page().is_none()
            && !self
                .view
                .as_ref()
                .and_then(|view| view.me_id.as_deref())
                .is_some_and(|actor| self.unread.local.has_marks(actor))
        {
            return Count::default();
        }
        match target {
            OpsTarget::DigestAll | OpsTarget::Hidden => Count::default(),
            OpsTarget::Member { identity_id, .. } => self.unread_count(identity_id),
            OpsTarget::Squad(room) => self
                .workspace_snapshot()
                .and_then(|snapshot| {
                    snapshot
                        .roster
                        .squads
                        .iter()
                        .find(|squad| &squad.room_id == room)
                })
                .map_or_else(Count::default, |squad| {
                    self.unread_members(squad.members.iter().map(String::as_str))
                }),
            OpsTarget::Pool => self
                .workspace_snapshot()
                .map_or_else(Count::default, |snapshot| {
                    self.unread_members(snapshot.roster.pool().map(|member| member.id.as_str()))
                }),
        }
    }
    fn unread_members<'a>(&self, members: impl Iterator<Item = &'a str>) -> Count {
        let mut count = Count::default();
        let mut seen = std::collections::BTreeSet::new();
        for member in members.filter(|member| seen.insert(*member)) {
            count.add(self.unread_count(member));
        }
        count
    }
    pub(in crate::board) fn unread_global(&self) -> Count {
        if self.unread_page().is_none()
            && !self
                .view
                .as_ref()
                .and_then(|view| view.me_id.as_deref())
                .is_some_and(|actor| self.unread.local.has_marks(actor))
        {
            return Count::default();
        }
        self.workspace_snapshot()
            .map_or_else(Count::default, |snapshot| {
                self.unread_members(snapshot.roster.members.keys().map(String::as_str))
            })
    }

    pub(in crate::board) fn local_unread_changed(&mut self) {
        self.unread_overlay_sync();
        if let Some(overlay) = &self.unread_overlay {
            overlay.invalidate();
        }
        self.overview_rows.borrow_mut().clear();
        self.workspace_reconcile();
        self.invalidate_frame_geometry();
    }

    /// Explicit reads alone may remove manual marks; automatic body admission never does.
    pub(in crate::board) fn explicit_answers_read(
        &mut self,
        actor: &str,
        answers: Vec<super::Answer>,
    ) {
        for answer in &answers {
            if self.unread.local.contains(actor, answer) {
                self.unread.local.read_after_ack(actor, answer);
            }
        }
        for mut answer in answers {
            // Explicit reads of local marks still use the public exact writer.
            answer.acknowledged = false;
            self.unread.writer.seen(&Seen {
                actor: actor.into(),
                answer,
            });
        }
        self.local_unread_changed();
    }

    /// Freeze this member's known keys; the existing walk handles older Core-unread keys.
    pub(in crate::board) fn mark_read(&mut self, member: &str) -> bool {
        let Some(actor) = self.view.as_ref().and_then(|view| view.me_id.clone()) else {
            return false;
        };
        let mut answers: std::collections::BTreeMap<_, _> = self
            .unread
            .local
            .answers(&actor, member)
            .into_iter()
            .map(|answer| (answer.request.clone(), answer))
            .collect();
        let mut before = None;
        if let Some(page) = self.unread_page() {
            before = page.next_before.clone();
            for answer in page.answers(member) {
                if (!answer.acknowledged || answers.contains_key(&answer.request))
                    && answers
                        .get(&answer.request)
                        .is_none_or(|held| answer.revision >= held.revision)
                {
                    answers.insert(answer.request.clone(), answer.clone());
                }
            }
        }
        let active = !answers.is_empty() || before.is_some();
        self.explicit_answers_read(&actor, answers.into_values().collect());
        if let Some(before) = before {
            self.unread.writer.mark_member(&actor, member, before);
        }
        active
    }
    /// The key and menu form: the selected member.
    pub(in crate::board) fn mark_read_selected(&mut self) -> crate::board::app::Effect {
        let member = self
            .workspace_selected_member()
            .and_then(|target| match target {
                OpsTarget::Member { identity_id, .. } => Some(identity_id.clone()),
                _ => None,
            });
        match member {
            Some(member) if self.mark_read(&member) => crate::board::app::Effect::None,
            _ => self.say("Nothing unread here."),
        }
    }
    /// Called after a successful draw, outside paint. A hidden/modal/paused or
    /// still-loading conversation cannot mark a reply that was never displayed,
    /// and neither can a selection that only moved onto the row: the reader
    /// must hold focus, which is the explicit act of opening the conversation.
    pub(in crate::board) fn unread_drawn(&mut self) {
        if self.loading() {
            return;
        }
        // The Unread overlay is a modal that paints answers itself: it feeds the writer.
        if self.unread_overlay.is_some() {
            if self.overlay() == Some(crate::board::app::Overlay::Unread)
                && !self.help
                && self.menu.is_none()
            {
                self.unread_overlay_drawn();
            } else {
                self.unread.displayed.clear();
            }
            return;
        }
        if self.modal_open() || self.shell.tab != crate::board::shell::OpsTab::Agents {
            return;
        }
        if self.focused_pane() != Some(crate::config::Pane::Replies) {
            // Leaving the reader closes the conversation: opening it again is
            // the explicit retry that "Reopen the conversation" promises.
            self.unread.displayed.clear();
            return;
        }
        let next = self
            .view
            .as_ref()
            .and_then(|view| view.me_id.as_deref())
            .map(|actor| {
                self.conversation
                    .borrow()
                    .painted_answers
                    .iter()
                    .filter(|body| body.actor == actor)
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for seen in &next {
            if !self.unread.displayed.contains(seen) {
                self.unread.writer.body(seen);
            }
        }
        self.unread.displayed = next;
    }
    pub(in crate::board) fn unread_observed(&self) {
        if let Some(actor) = self.view.as_ref().and_then(|view| view.me_id.as_deref())
            && let Some(page) = self.home_leads.unread(actor)
        {
            self.unread.writer.observed(page);
            self.unread.local.observe(page);
        }
    }

    pub(in crate::board) fn finished_unread(
        &mut self,
        result: Result<super::publish::Completed, String>,
    ) {
        if let Ok(completed) = &result {
            self.home_leads.acknowledged(completed);
            if let Some(overlay) = &mut self.unread_overlay
                && self.view.as_ref().and_then(|view| view.me_id.as_deref())
                    == Some(&completed.actor)
            {
                overlay.acknowledged(completed);
            }
        }
        self.unread_observed();
        self.overview_rows.borrow_mut().clear();
        self.workspace_reconcile();
        self.invalidate_frame_geometry();
        if let Some(error) = self.unread.writer.failure().or_else(|| result.err()) {
            self.set_notice(
                Some(format!(
                    "Unread update failed: {error} {}",
                    self.unread.writer.recovery()
                )),
                true,
            );
        }
        self.unread.writer.settled();
    }
}

#[cfg(test)]
mod tests;

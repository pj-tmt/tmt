//! Admission and presentation reuse the actor-fenced results and existing lanes.
use super::{Count, Page, publish::Seen};
use crate::board::{app::App, workspace::OpsTarget};

impl App {
    fn unread_page(&self) -> Option<&Page> {
        let actor = self.view.as_ref()?.me_id.as_deref()?;
        self.home_leads
            .unread(actor)
            .filter(|page| !page.replies.is_empty())
    }

    /// Sorting resolves the bounded results page once; comparisons use UUID
    /// membership in the held Core acknowledgment projection.
    pub(in crate::board) fn unread_identities(&self) -> std::collections::BTreeSet<&str> {
        let Some(page) = self.unread_page() else {
            return Default::default();
        };
        page.replies
            .keys()
            .map(String::as_str)
            .filter(|member| page.count(member).total > 0)
            .collect()
    }

    pub(in crate::board) fn unread_count(&self, member: &str) -> Count {
        self.unread_page()
            .map_or_else(Count::default, |page| page.count(member))
    }
    pub(in crate::board) fn unread_target(&self, target: &OpsTarget) -> Count {
        // Admit the bounded page before constructing a roster iterator: the
        // Common pool iterator computes assigned membership eagerly.
        let Some(page) = self.unread_page() else {
            return Count::default();
        };
        match target {
            OpsTarget::DigestAll => Count::default(),
            OpsTarget::Member { identity_id, .. } => page.count(identity_id),
            OpsTarget::Squad(room) => {
                let Some(squad) = self.workspace_snapshot().and_then(|snapshot| {
                    snapshot
                        .roster
                        .squads
                        .iter()
                        .find(|squad| &squad.room_id == room)
                }) else {
                    return Count::default();
                };
                self.unread_members(page, squad.members.iter().map(String::as_str))
            }
            OpsTarget::Pool => self
                .workspace_snapshot()
                .map_or_else(Count::default, |snapshot| {
                    self.unread_members(
                        page,
                        snapshot.roster.pool().map(|member| member.id.as_str()),
                    )
                }),
        }
    }
    fn unread_members<'a>(&self, page: &Page, members: impl Iterator<Item = &'a str>) -> Count {
        let mut count = Count::default();
        let mut seen = std::collections::BTreeSet::new();
        for member in members.filter(|member| seen.insert(*member)) {
            count.add(page.count(member));
        }
        count
    }
    pub(in crate::board) fn unread_global(&self) -> Count {
        let Some(page) = self.unread_page() else {
            return Count::default();
        };
        self.workspace_snapshot()
            .map_or_else(Count::default, |snapshot| {
                self.unread_members(page, snapshot.roster.members.keys().map(String::as_str))
            })
    }
    /// Explicit Mark read admits the selected member's unread answer revisions.
    pub(in crate::board) fn mark_read(&mut self, member: &str) -> bool {
        let Some(page) = self.unread_page() else {
            return false;
        };
        if page.count(member).total == 0 && page.next_before.is_none() {
            return false;
        }
        for answer in page.answers(member).filter(|answer| !answer.acknowledged) {
            self.unread.writer.seen(&Seen {
                actor: page.actor.clone(),
                answer: answer.clone(),
            });
        }
        if let Some(before) = &page.next_before {
            self.unread
                .writer
                .mark_member(&page.actor, member, before.clone());
        }
        true
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
        if self.loading()
            || self.modal_open()
            || self.shell.tab != crate::board::shell::OpsTab::Agents
        {
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
        }
    }

    pub(in crate::board) fn finished_unread(
        &mut self,
        result: Result<super::publish::Completed, String>,
    ) {
        if let Ok(completed) = &result {
            self.home_leads.acknowledged(completed);
        }
        self.unread_observed();
        self.overview_rows.borrow_mut().clear();
        self.workspace_reconcile();
        self.invalidate_frame_geometry();
        if let Some(error) = self.unread.writer.failure() {
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

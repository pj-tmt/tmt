//! Admission and presentation reuse the actor-fenced results and existing lanes.
use super::{Count, Page, publish::Seen};
use crate::board::{app::App, home_leads::Kind, workspace::OpsTarget};

impl App {
    fn unread_page(&self) -> Option<&Page> {
        let actor = self.view.as_ref()?.me_id.as_deref()?;
        self.home_leads
            .unread(actor)
            .filter(|page| !page.replies.is_empty())
    }

    /// Sorting resolves the bounded results page once; comparisons use UUID
    /// membership without acquiring read markers again.
    pub(in crate::board) fn unread_identities(&self) -> std::collections::BTreeSet<&str> {
        let Some(page) = self.unread_page() else {
            return Default::default();
        };
        page.replies
            .keys()
            .map(String::as_str)
            .filter(|member| {
                page.newest(member)
                    .is_some_and(|at| at > self.read_marker(&page.actor, member))
            })
            .collect()
    }

    fn read_marker(&self, actor: &str, member: &str) -> u64 {
        self.unread.writer.confirmed(actor, member).max(
            self.workspace_snapshot()
                .map_or(0, |snapshot| snapshot.read_markers.get(actor, member)),
        )
    }
    pub(in crate::board) fn unread_count(&self, member: &str) -> Count {
        self.unread_page().map_or_else(Count::default, |page| {
            page.count(member, self.read_marker(&page.actor, member))
        })
    }
    pub(in crate::board) fn unread_target(&self, target: &OpsTarget) -> Count {
        // Admit the bounded page before constructing a roster iterator: the
        // Common pool iterator computes assigned membership eagerly.
        let Some(page) = self.unread_page() else {
            return Count::default();
        };
        match target {
            OpsTarget::Member { identity_id, .. } => {
                page.count(identity_id, self.read_marker(&page.actor, identity_id))
            }
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
            count.add(page.count(member, self.read_marker(&page.actor, member)));
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
    /// Seal the admitted actor, UUID and observed watermark before leaving the UI.
    /// The lane saves it only after Core's guarded jump succeeds.
    pub(in crate::board) fn unread_jump(&self, member: &str) -> Option<Seen> {
        let page = self.unread_page()?;
        self.admit_seen(&page.actor, member, page.newest(member)?)
    }
    fn admit_seen(&self, actor: &str, member: &str, at: u64) -> Option<Seen> {
        if at <= self.read_marker(actor, member) {
            return None;
        }
        Seen::new(actor, member, at)
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
            self.unread.displayed = None;
            return;
        }
        let next = (|| {
            let area = self.conversation.borrow().area;
            if area.width == 0 || area.height == 0 {
                return None;
            }
            let reply = self.conversation_reply()?;
            if !reply.acquired {
                return None;
            }
            let key = reply.key?;
            if key.kind != Kind::Reply {
                return None;
            }
            let actor = self.view.as_ref()?.me_id.as_deref()?;
            if key.sender != actor {
                return None;
            }
            let page = self.home_leads.unread(actor)?;
            self.admit_seen(actor, &key.lead, page.submitted(&key.lead, &key.request)?)
        })();
        if next != self.unread.displayed {
            if let Some(seen) = &next {
                self.unread.writer.seen(seen);
            }
            self.unread.displayed = next;
        }
    }
    pub(in crate::board) fn finished_unread(
        &mut self,
        result: Result<crate::config::ReadMarkers, String>,
    ) {
        match result {
            Ok(markers) => {
                self.unread.writer.observed(&markers);
                self.overview_rows.borrow_mut().clear();
                self.workspace_reconcile();
                self.invalidate_frame_geometry();
            }
            Err(error) => self.set_notice(
                Some(format!(
                    "Read marker not saved: {error} Reopen the conversation or jump to retry."
                )),
                true,
            ),
        }
    }
}

#[cfg(test)]
mod tests;

//! Home targets share the board's selection, effects, composer and scroll owner.

use super::{Age, Home};
use crate::board::{
    app::{App, Effect, Request},
    home_leads::MessageKey,
};
use serde_json::Value;

/// Section key of the one cron cursor target.
pub const CRON: &str = "cron";
pub const LEADS: &str = "leads";
pub const ALL_LEADS: &str = "all-leads";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub section: String,
    pub squad: String,
    pub member: Option<String>,
}

pub struct HomeEntry<'a> {
    pub target: Target,
    pub row: &'a Value,
    pub lead: Option<&'a str>,
    pub age: Option<&'a Age>,
}

impl Home {
    /// Acquired attention and squads; App inserts its deferred lead/cron targets.
    pub fn entries<'a>(&'a self, document: &'a Value, search: &str) -> Vec<HomeEntry<'a>> {
        let mut entries = Vec::new();
        for section in &self.sections {
            for row in &section.rows {
                if crate::board::app::matches(&row.member, search) {
                    entries.push(HomeEntry {
                        target: Target {
                            section: section.key.clone(),
                            squad: row.squad.clone(),
                            member: row.member["id"].as_str().map(str::to_owned),
                        },
                        row: &row.member,
                        lead: row.lead.as_deref(),
                        age: row.age.as_ref(),
                    });
                }
            }
        }
        for squad in &self.squads {
            let row = document["sections"][0]["rows"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|row| row["squad"] == squad.squad);
            if let Some(row) = row.filter(|row| crate::board::app::matches(row, search)) {
                entries.push(HomeEntry {
                    target: Target {
                        section: "squads".into(),
                        squad: squad.squad.clone(),
                        member: None,
                    },
                    row,
                    lead: squad.lead.as_ref().and_then(|lead| lead["name"].as_str()),
                    age: None,
                });
            }
        }
        entries
    }
}

/// The cron entry has no member row; its line comes from the cron projection.
static NO_ROW: Value = Value::Null;

impl App {
    /// One reading order: attention, leads, audience footer, cron, squads.
    pub(in crate::board) fn home_entries(&self) -> Vec<HomeEntry<'_>> {
        let mut entries = self
            .view
            .as_ref()
            .and_then(|view| {
                view.home
                    .as_ref()
                    .map(|home| home.entries(&view.document, &self.search))
            })
            .unwrap_or_default();
        if self.view.as_ref().is_some_and(|view| view.home.is_some()) {
            let at = entries
                .iter()
                .position(|entry| entry.target.section == "squads")
                .unwrap_or(entries.len());
            let mut leads = self
                .home_leads
                .leads
                .iter()
                .filter(|lead| {
                    crate::board::app::matches(&lead.row, &self.search)
                        || lead
                            .squad
                            .to_lowercase()
                            .contains(&self.search.to_lowercase())
                })
                .map(|lead| HomeEntry {
                    target: Target {
                        section: LEADS.into(),
                        squad: lead.squad.clone(),
                        member: Some(lead.id().into()),
                    },
                    row: &lead.row,
                    lead: Some(lead.name()),
                    age: None,
                })
                .collect::<Vec<_>>();
            if self.search.is_empty() && !leads.is_empty() {
                leads.push(HomeEntry {
                    target: Target {
                        section: ALL_LEADS.into(),
                        squad: String::new(),
                        member: None,
                    },
                    row: &NO_ROW,
                    lead: None,
                    age: None,
                });
            }
            entries.splice(at..at, leads);
        }
        if self.search.is_empty() && self.cron_shown() && !entries.is_empty() {
            let at = entries
                .iter()
                .position(|entry| entry.target.section == "squads")
                .unwrap_or(entries.len());
            entries.insert(
                at,
                HomeEntry {
                    target: Target {
                        section: CRON.into(),
                        squad: String::new(),
                        member: None,
                    },
                    row: &NO_ROW,
                    lead: None,
                    age: None,
                },
            );
        }
        if let Some(crate::board::app::RowFeedback {
            target: crate::board::app::RowTarget::Home(target),
            home: Some(feedback),
        }) = &self.sent
            && !entries.iter().any(|entry| &entry.target == target)
            && crate::board::app::matches(&feedback.row, &self.search)
        {
            entries.insert(
                feedback.index.min(entries.len()),
                HomeEntry {
                    target: target.clone(),
                    row: &feedback.row,
                    lead: feedback.lead.as_deref(),
                    age: None,
                },
            );
        }
        entries
    }

    pub(in crate::board) fn home_enter(&mut self) -> Effect {
        let entries = self.home_entries();
        let Some(entry) = entries.get(self.selected) else {
            return self.say("No home row is selected.");
        };
        if entry.target.section == ALL_LEADS {
            self.home_write()
        } else if entry.target.section == CRON {
            self.open_cron_list(None)
        } else if entry.target.member.is_some() {
            match entry.row["name"].as_str() {
                Some(name) => Effect::Act(Request::Jump(name.into())),
                None => self.say("This row has no member name."),
            }
        } else {
            let squad = entry.target.squad.clone();
            self.go(squad)
        }
    }

    pub(in crate::board) fn home_answer(&mut self) -> Effect {
        if self
            .home_entries()
            .get(self.selected)
            .is_some_and(|entry| entry.target.section == ALL_LEADS)
        {
            return self.home_write();
        }
        if self
            .home_entries()
            .get(self.selected)
            .is_some_and(|entry| entry.target.section == CRON)
        {
            return self.say("Nothing to answer here; c opens the cron list.");
        }
        let Some(send) = self.row_send(self.selected, false) else {
            return self.say("Who is sending? Record yourself with tmt squad me <name>.");
        };
        let squad = match &send.target {
            crate::board::app::RowTarget::Home(target) => target.squad.clone(),
            _ => unreachable!("home target"),
        };
        self.compose_row(send, crate::action::Verb::Annotate, squad)
    }
}

impl App {
    pub(in crate::board) fn home_expand(&mut self) -> crate::board::app::Effect {
        use crate::board::app::{Compose, Input};
        let Some(entry) = self
            .home_entries()
            .get(self.selected)
            .map(|entry| entry.target.clone())
        else {
            return self.say("No lead is selected.");
        };
        let Some(lead) = self.home_leads.leads.iter().find(|lead| {
            entry.section == LEADS
                && entry.squad == lead.squad
                && entry.member.as_deref() == Some(lead.id())
        }) else {
            return self.say("Select a lead to expand its latest message.");
        };
        let Some(exchange) = &lead.exchange else {
            return self.say("No exchange with this lead yet.");
        };
        let Some(request) = exchange.request.clone() else {
            return self.say("This question has no retained request body.");
        };
        let Some(sender) = self.view.as_ref().and_then(|view| view.me_id.clone()) else {
            return self.say("Record yourself with tmt squad me <name>.");
        };
        let key = MessageKey {
            sender,
            lead: lead.id().into(),
            squad: lead.squad.clone(),
            request,
            kind: exchange.kind,
        };
        let name = lead.name().to_owned();
        let row_send = self.row_send(self.selected, false);
        self.input = Some(Input {
            squad: lead.squad.clone(),
            prompt: format!("latest from {name}"),
            text: "(reading message…)".into(),
            compose: Compose::ReadLead { key, offset: 0 },
            row_send,
            others: Vec::new(),
            quote: None,
            link: None,
            hint: None,
        });
        self.notice = None;
        crate::board::app::Effect::None
    }

    pub(in crate::board) fn latest_row_reply(&self) -> Option<&serde_json::Value> {
        let row = self.selected_row()?;
        self.view
            .as_ref()?
            .replies
            .iter()
            .find(|reply| reply["recipientId"] == row["id"])
    }

    pub(in crate::board) fn row_expand(&mut self) -> crate::board::app::Effect {
        use crate::board::app::{Compose, Input};
        let Some(target) = self
            .row_target(self.selected)
            .filter(|_| self.focused_pane() == Some(crate::config::Pane::Rows))
        else {
            return self.say("Select a member row to expand its details.");
        };
        let row = self.selected_row().expect("selected target has a row");
        let name = row["name"].as_str().unwrap_or_default().to_owned();
        let id = row["id"].as_str().unwrap_or_default().to_owned();
        let squad = self.shown_tab().unwrap_or_default().to_owned();
        let reply = self.latest_row_reply();
        let key = reply.and_then(|reply| {
            Some(MessageKey {
                sender: self.view.as_ref()?.me_id.clone()?,
                lead: id,
                squad: squad.clone(),
                request: reply["requestId"].as_str()?.into(),
                kind: crate::board::home_leads::Kind::Reply,
            })
        });
        let text = reply.map_or_else(String::new, |reply| {
            reply["response"]
                .as_str()
                .map(crate::board::notes::sanitize)
                .unwrap_or_else(|| {
                    if key.is_some() {
                        "(reading reply…)".into()
                    } else {
                        "(reply unavailable)".into()
                    }
                })
        });
        self.input = Some(Input {
            squad,
            prompt: format!("details for {name}"),
            text,
            compose: Compose::ReadRow {
                target,
                reply: key,
                offset: 0,
            },
            row_send: self.row_send(self.selected, true),
            others: Vec::new(),
            quote: None,
            link: None,
            hint: None,
        });
        self.notice = None;
        crate::board::app::Effect::None
    }

    pub(in crate::board) fn message_valid(&self) -> bool {
        if let Some(crate::board::app::Input {
            compose: crate::board::app::Compose::ReadRow { target, reply, .. },
            ..
        }) = &self.input
        {
            let row = self.target_row(target);
            return !self.loading()
                && row.is_some()
                && reply.as_ref().is_none_or(|key| {
                    self.view.as_ref().and_then(|view| view.me_id.as_ref()) == Some(&key.sender)
                        && self
                            .view
                            .as_ref()
                            .and_then(|view| {
                                view.replies.iter().find(|reply| {
                                    Some(&reply["recipientId"]) == row.map(|row| &row["id"])
                                })
                            })
                            .is_some_and(|reply| reply["requestId"] == key.request)
                });
        }
        let Some(crate::board::app::Input {
            compose: crate::board::app::Compose::ReadLead { key, .. },
            ..
        }) = &self.input
        else {
            return true;
        };
        self.current.as_deref() == Some(crate::board::ALL)
            && self
                .view
                .as_ref()
                .is_some_and(|view| view.home.is_some() && view.me_id.as_ref() == Some(&key.sender))
            && self.home_leads.leads.iter().any(|lead| {
                lead.squad == key.squad
                    && lead.id() == key.lead
                    && lead.exchange.as_ref().is_some_and(|exchange| {
                        exchange.request.as_ref() == Some(&key.request) && exchange.kind == key.kind
                    })
            })
    }

    pub(in crate::board) fn apply_message(
        &mut self,
        key: &MessageKey,
        body: Result<String, String>,
    ) {
        if !self.message_valid() {
            return;
        }
        if let Some(input) = &mut self.input
            && matches!(&input.compose, crate::board::app::Compose::ReadLead { key: current, .. } | crate::board::app::Compose::ReadRow { reply: Some(current), .. } if current == key)
        {
            input.text = body.unwrap_or_else(|error| {
                format!(
                    "(message unavailable: {})",
                    crate::board::notes::sanitize(&error)
                )
            });
        }
    }

    pub(in crate::board) fn message_key(
        &mut self,
        key: ratatui::crossterm::event::KeyEvent,
    ) -> crate::board::app::Effect {
        use ratatui::crossterm::event::{KeyCode, KeyModifiers};
        let row = self.input.as_ref().is_some_and(|input| {
            matches!(input.compose, crate::board::app::Compose::ReadRow { .. })
        });
        if row {
            let action =
                crate::board::app::event_name(key).and_then(|event| self.bindings().remove(&event));
            match action.as_ref().map(|action| action.verb) {
                Some(crate::action::Verb::HomeMessage) => {
                    self.input = None;
                    self.notice = None;
                    return crate::board::app::Effect::None;
                }
                Some(crate::action::Verb::Annotate) => {
                    self.input = None;
                    if let Some(send) = self.row_send(self.selected, true) {
                        return self.compose_row(
                            send,
                            crate::action::Verb::Talk,
                            self.shown_tab().unwrap_or_default().into(),
                        );
                    }
                    return self.say("Record yourself with tmt squad me <name>.");
                }
                Some(crate::action::Verb::Open) => {
                    return self.perform(action.as_ref().expect("resolved action"));
                }
                _ => {}
            }
            if matches!(key.code, KeyCode::Char('e' | 'a' | 'o')) {
                return crate::board::app::Effect::None;
            }
        }
        match key.code {
            KeyCode::Esc | KeyCode::Char('e') if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.input = None;
                self.notice = None;
            }
            KeyCode::Char('a') if key.modifiers.is_empty() => {
                self.input = None;
                return self.home_answer();
            }
            KeyCode::Up
            | KeyCode::Char('k')
            | KeyCode::Down
            | KeyCode::Char('j')
            | KeyCode::PageUp
            | KeyCode::PageDown => {
                let band = self.input_band.get();
                let row_lines = band.map(|band| {
                    crate::board::view::waiting::read_lines(self, band.width.saturating_sub(4))
                        .len()
                });
                if let Some(crate::board::app::Input {
                    compose:
                        crate::board::app::Compose::ReadLead { offset, .. }
                        | crate::board::app::Compose::ReadRow { offset, .. },
                    text,
                    ..
                }) = &mut self.input
                {
                    let (maximum, page) = band.map_or((usize::MAX, 1), |band| {
                        let lines = crate::board::home_leads::message_lines(
                            text,
                            band.width.saturating_sub(4),
                        );
                        let page = usize::from(band.height.saturating_sub(3));
                        (
                            row_lines.unwrap_or(lines.len()).saturating_sub(page),
                            page.max(1),
                        )
                    });
                    let step = if matches!(key.code, KeyCode::PageUp | KeyCode::PageDown) {
                        page
                    } else {
                        1
                    };
                    if matches!(key.code, KeyCode::Up | KeyCode::Char('k') | KeyCode::PageUp) {
                        *offset = (*offset).min(maximum).saturating_sub(step);
                    } else {
                        *offset = offset.saturating_add(step).min(maximum);
                    }
                }
            }
            _ => {}
        }
        crate::board::app::Effect::None
    }
}

impl App {
    pub(in crate::board) fn lead_audience(&self) -> Vec<crate::send::LeadRecipient> {
        self.home_leads
            .leads
            .iter()
            .map(|lead| crate::send::LeadRecipient {
                squad: lead.squad.clone(),
                id: lead.id().into(),
                name: lead.name().into(),
            })
            .collect()
    }

    pub(in crate::board) fn leads_valid(&self, input: &crate::board::app::Input) -> bool {
        let crate::board::app::Compose::Leads {
            sender,
            recipients,
            all,
        } = &input.compose
        else {
            return false;
        };
        let current = self.lead_audience();
        !self.loading()
            && self.current.as_deref() == Some(crate::board::ALL)
            && self.view.as_ref().and_then(|view| view.me_id.as_ref()) == Some(sender)
            && recipients
                .iter()
                .all(|recipient| current.contains(recipient))
            && (!all || current.len() == recipients.len())
    }

    pub(in crate::board) fn home_write(&mut self) -> Effect {
        self.compose_leads(self.lead_audience(), true)
    }

    pub(in crate::board) fn home_pick(&mut self) -> Effect {
        use crate::board::app::{Choice, Menu, MenuEntry};
        if self.current.as_deref() != Some(crate::board::ALL) {
            return Effect::None;
        }
        let recipients = self.lead_audience();
        if recipients.is_empty() {
            return self.say("No current leads to pick.");
        }
        self.menu = Some(Menu {
            title: "Write to a lead".into(),
            entries: recipients
                .into_iter()
                .enumerate()
                .map(|(index, recipient)| MenuEntry {
                    key: (index + 1).to_string(),
                    label: format!("{} ({})", recipient.name, recipient.squad),
                    choice: Choice::Leads {
                        recipients: vec![recipient],
                        all: false,
                    },
                })
                .collect(),
            row_send: None,
            link: None,
            prefill: String::new(),
            selected: 0,
            surface: Default::default(),
        });
        self.notice = None;
        Effect::None
    }

    pub(in crate::board) fn compose_leads(
        &mut self,
        recipients: Vec<crate::send::LeadRecipient>,
        all: bool,
    ) -> Effect {
        use crate::board::app::{Compose, Input, RowSend, RowTarget};
        if self.current.as_deref() != Some(crate::board::ALL) || self.loading() {
            return Effect::None;
        }
        if recipients.is_empty() {
            return self.say("No current leads to write to.");
        }
        let Some(view) = &self.view else {
            return Effect::None;
        };
        let Some(sender) = view.me_id.clone() else {
            return self.say("Record yourself with tmt squad me <name>.");
        };
        let name = if all {
            "all leads".to_owned()
        } else {
            recipients[0].name.clone()
        };
        let target = Target {
            section: ALL_LEADS.into(),
            squad: String::new(),
            member: None,
        };
        if let Some(index) = self
            .home_entries()
            .iter()
            .position(|entry| entry.target == target)
        {
            self.selected = index;
            self.home_start = false;
            self.home_target = Some(target.clone());
        }
        self.input = Some(Input {
            prompt: format!("→ {name}"),
            text: String::new(),
            squad: String::new(),
            compose: Compose::Leads {
                sender,
                recipients,
                all,
            },
            row_send: Some(RowSend {
                target: RowTarget::Home(target),
                sender: view.me.clone().unwrap_or_default(),
                name,
                note: None,
                note_member: false,
            }),
            others: Vec::new(),
            quote: None,
            link: None,
            hint: None,
        });
        self.notice = None;
        self.follow = true;
        Effect::None
    }
}

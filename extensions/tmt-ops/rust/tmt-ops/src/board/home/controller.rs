//! Home targets share the board's selection, effects, composer and scroll owner.

use super::{Age, Home};
use crate::board::app::{App, Effect, Request};
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
            ..
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
            return self.say("Who is sending? Record yourself with tmt ops squad me <name>.");
        };
        let squad = match &send.target {
            crate::board::app::RowTarget::Home(target) => target.squad.clone(),
            _ => unreachable!("home target"),
        };
        self.compose_row(send, crate::action::Verb::Annotate, squad)
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
            return self.say("Record yourself with tmt ops squad me <name>.");
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

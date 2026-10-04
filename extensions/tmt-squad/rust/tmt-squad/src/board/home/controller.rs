//! Home targets share the board's selection, effects, composer and scroll owner.

use super::{Age, Home};
use crate::board::app::{App, Choice, Compose, Effect, Input, Menu, MenuEntry, Request};
use serde_json::Value;

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
    /// Future reply/cron targets belong before squads in this reading order.
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

impl App {
    pub(in crate::board) fn home_entries(&self) -> Vec<HomeEntry<'_>> {
        self.view
            .as_ref()
            .and_then(|view| {
                view.home
                    .as_ref()
                    .map(|home| home.entries(&view.document, &self.search))
            })
            .unwrap_or_default()
    }

    pub(in crate::board) fn home_section(&mut self, previous: bool) {
        let entries = self.home_entries();
        let mut sections = Vec::new();
        for (index, entry) in entries.iter().enumerate() {
            let key = match entry.target.section.as_str() {
                "needs-you" | "blocked" => "attention",
                other => other,
            };
            if sections.last().is_none_or(|(last, _)| *last != key) {
                sections.push((key, index));
            }
        }
        if !sections.is_empty() {
            let current = sections
                .partition_point(|(_, start)| *start <= self.selected)
                .saturating_sub(1);
            let next = (current + if previous { sections.len() - 1 } else { 1 }) % sections.len();
            self.select(sections[next].1);
        }
    }

    pub(in crate::board) fn home_enter(&mut self) -> Effect {
        let entries = self.home_entries();
        let Some(entry) = entries.get(self.selected) else {
            return self.say("No home row is selected.");
        };
        if entry.target.member.is_some() {
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
        let Some(sender) = self.view.as_ref().and_then(|view| view.me.clone()) else {
            return self.say("Who is sending? Record yourself with tmt squad me <name>.");
        };
        let entries = self.home_entries();
        let Some(entry) = entries.get(self.selected) else {
            return self.say("No home row is selected.");
        };
        let send = Send {
            target: entry.target.clone(),
            sender,
        };
        let name = entry.row["name"].as_str().unwrap_or_default().to_owned();
        let requests = entry.row["waitingOnYou"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
            .filter_map(|(i, request)| {
                Some(MenuEntry {
                    key: (i + 1).to_string(),
                    label: crate::board::notes::sanitize(
                        request["preview"]
                            .as_str()
                            .unwrap_or("(question unavailable)"),
                    ),
                    choice: Choice::Reply {
                        request: request["requestId"].as_str()?.into(),
                        from: name.clone(),
                    },
                })
            })
            .collect::<Vec<_>>();
        if !requests.is_empty() {
            self.menu = Some(Menu {
                home: Some(send),
                link: None,
                prefill: String::new(),
                title: format!("answer {name}"),
                entries: requests,
                selected: 0,
            });
            return Effect::None;
        }
        let Some(to) = entry.lead.map(str::to_owned) else {
            return self.say(format!(
                "This squad has no lead; set one with tmt squad lead <name> --squad {}.",
                entry.target.squad
            ));
        };
        let squad = entry.target.squad.clone();
        self.ask(
            format!("note on {name} for {to}"),
            Compose::Annotate { to, row: name },
            squad,
        );
        self.input.as_mut().expect("opened composer").home = Some(send);
        Effect::None
    }
}

/// Opening authority, rechecked against refreshed home data before submission.
#[derive(Clone)]
pub struct Send {
    pub target: Target,
    pub sender: String,
}
impl Send {
    pub fn valid(&self, app: &App, input: &Input) -> bool {
        let Some(view) = &app.view else {
            return false;
        };
        if app.loading()
            || view.me.as_deref() != Some(&self.sender)
            || input.squad != self.target.squad
        {
            return false;
        }
        let Some(home) = &view.home else {
            return false;
        };
        home.entries(&view.document, "")
            .iter()
            .find(|e| e.target == self.target)
            .is_some_and(|entry| match &input.compose {
                Compose::Reply { request, from } => {
                    entry.row["name"].as_str() == Some(from)
                        && entry.row["waitingOnYou"].as_array().is_some_and(|items| {
                            items
                                .iter()
                                .any(|item| item["requestId"].as_str() == Some(request))
                        })
                }
                Compose::Annotate { to, row } => {
                    entry.lead == Some(to) && entry.row["name"].as_str() == Some(row)
                }
                Compose::Talk { .. } => false,
            })
    }
}

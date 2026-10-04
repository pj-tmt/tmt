//! Home targets share the board's selection, effects, composer and scroll owner.

use super::{Age, Counts, Home};
use crate::board::app::{App, Effect, Request};
use serde_json::Value;

/// Section key of the one cron cursor target.
pub const CRON: &str = "cron";

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
    pub counts: Option<&'a Counts>,
    pub pressing: Option<&'a Value>,
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
                        counts: None,
                        pressing: None,
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
                    counts: Some(&squad.counts),
                    pressing: squad.pressing.as_ref(),
                });
            }
        }
        entries
    }
}

/// The cron entry has no member row; its line comes from the cron projection.
static NO_ROW: Value = Value::Null;

impl App {
    /// Reading order: attention, then the cron line, then squads.
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
                    counts: None,
                    pressing: None,
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
                    counts: None,
                    pressing: None,
                },
            );
        }
        entries
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
        if entry.target.section == CRON {
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

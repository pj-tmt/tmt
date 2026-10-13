//! Retained HOME indices for acquired summaries and source projections.
//! Interactive selection and composition belong to the UUID workspace.

use super::{Age, Home};
use crate::board::app::App;
use serde_json::Value;

/// Section key of the one cron cursor target.
pub const CRON: &str = "cron";
pub const LEADS: &str = "leads";
pub const ALL_LEADS: &str = "all-leads";
/// One overview focus stop; its selection is painted on the usage line only.
pub const USAGE: &str = "usage";

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
    /// One reading order: overview, attention, leads, audience footer, squads, cron.
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
            entries.push(HomeEntry {
                target: Target {
                    section: CRON.into(),
                    squad: String::new(),
                    member: None,
                },
                row: &NO_ROW,
                lead: None,
                age: None,
            });
        }
        if !self.searching
            && self.search.is_empty()
            && self.view.as_ref().is_some_and(|view| view.home.is_some())
        {
            entries.insert(
                0,
                HomeEntry {
                    target: Target {
                        section: USAGE.into(),
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
}

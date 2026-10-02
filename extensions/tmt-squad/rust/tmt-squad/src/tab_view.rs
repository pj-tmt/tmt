//! Cross-squad tab documents shared by the board and `ls --tab`.
//! Acquisition stays behind public core commands; renderers consume one result.

use crate::{
    attention::Attention,
    config::Config,
    core::{Core, SquadError},
    requests,
    rows::Rows,
    squad::Squad,
    status,
    tabs::{self, ALL, LEADS},
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub struct Document {
    pub document: Value,
    pub rows: Rows,
    pub attention: BTreeMap<String, Attention>,
}

/// Resolve only explicitly supported tabs, never a squad with the same name.
pub fn key(name: &str) -> Result<&'static str, SquadError> {
    match name {
        "leads" => Ok(LEADS),
        "all" => Ok(ALL),
        _ => Err(SquadError::new(
            "SQUAD_TAB_NOT_FOUND",
            format!("Tab '{name}' does not exist."),
        )),
    }
}

pub fn load(
    core: &Core,
    config: &Config,
    squads: &[Squad],
    tabs: &[String],
    me: Option<&crate::me::Me>,
    key: &str,
) -> Result<Document, SquadError> {
    let listed = if key == LEADS {
        Some(core.json(&["ls"])?)
    } else {
        None
    };
    let all = squads.iter().collect::<Vec<_>>();
    let documents = roster_documents(core, config, &all, me, listed.as_ref());
    let attention = tab_attention(&documents);
    let (mut document, rows) = match key {
        LEADS => (leads_document(tabs, &documents), Rows::leads()),
        ALL => (all_document(tabs, &documents, &attention), Rows::overview()),
        _ => return Err(SquadError::new("SQUAD_TAB_NOT_FOUND", "Unknown tab key.")),
    };
    let grid = rows.value();
    document["columns"] = grid["columns"].clone();
    document["lines"] = grid["lines"].clone();
    document["squad"]["attention"] = attention.get(key).copied().unwrap_or_default().document();
    Ok(Document {
        document,
        rows,
        attention,
    })
}

/// The `all` tab's document: a row per squad, `name` the squad's, with its
/// lead, member count and attention counts as fields.
pub(crate) fn all_document(
    tabs: &[String],
    documents: &BTreeMap<String, Value>,
    attention: &BTreeMap<String, Attention>,
) -> Value {
    let rows: Vec<Value> = in_tab_order(tabs, documents)
        .into_iter()
        .map(|name| {
            let document = &documents[name];
            let lead = document["squad"]["lead"]["name"].as_str();
            let members = document["sections"]
                .as_array()
                .into_iter()
                .flatten()
                .flat_map(|section| section["rows"].as_array().into_iter().flatten())
                .filter_map(|row| row["name"].as_str())
                .collect::<std::collections::BTreeSet<_>>()
                .len();
            let attention = attention.get(name).copied().unwrap_or_default();
            json!({
                "name": name,
                "squad": name,
                "state": attention.state(),
                "fields": {
                    "squad": name,
                    "lead": lead,
                    "members": members.to_string(),
                    "waiting": attention.waiting.to_string(),
                    "blocked": attention.blocked.to_string(),
                },
            })
        })
        .collect();
    json!({
        "squad": {"name": "all", "lead": null},
        "sections": [{"title": null, "rows": rows}],
    })
}

/// Squads in tab order, then squads off the tab line.
fn in_tab_order<'a>(tabs: &'a [String], documents: &'a BTreeMap<String, Value>) -> Vec<&'a String> {
    let mut order: Vec<&String> = tabs
        .iter()
        .filter(|key| documents.contains_key(*key))
        .collect();
    order.extend(documents.keys().filter(|key| !tabs.contains(key)));
    order
}

/// Each squad's roster-only status document, with what waits on the user
/// from one inbox read shared by all, and presence when an `ls` document is
/// given. A squad that cannot be read is left out.
pub(crate) fn roster_documents(
    core: &Core,
    config: &Config,
    squads: &[&Squad],
    me: Option<&crate::me::Me>,
    listed: Option<&Value>,
) -> BTreeMap<String, Value> {
    if squads.is_empty() {
        return BTreeMap::new();
    }
    let waiting = match me {
        Some(me) => match requests::inbox(core, &me.id) {
            Ok(inbox) => Some((me.id.as_str(), inbox)),
            Err(_) => return BTreeMap::new(),
        },
        None => None,
    };
    squads
        .iter()
        .filter_map(|squad| {
            let mut roster = squad.roster(core).ok()?;
            if let Some(listed) = listed {
                crate::squad::join_presence(&mut roster, listed);
            }
            let waiting = waiting.as_ref().map(|(me, inbox)| (*me, inbox));
            Some((
                squad.name.clone(),
                roster_document(config, squad, roster, waiting).ok()?,
            ))
        })
        .collect()
}

/// One squad's document from its roster: the same document `status`
/// builds, so a tab and `ls --json` never disagree.
pub(crate) fn roster_document(
    config: &Config,
    squad: &Squad,
    roster: Vec<crate::squad::Member>,
    waiting: Option<(&str, &requests::Window)>,
) -> Result<Value, crate::core::SquadError> {
    let layout = config.layout(&squad.name)?;
    let states = config.states(&squad.name, layout)?;
    let sections = config.sections(&squad.name)?;
    let rows = config.rows(&squad.name)?;
    let mut document = status::document(squad, layout, &states, &sections, &rows, roster);
    if let Some((me, inbox)) = waiting {
        requests::apply_waiting(&mut document, &squad.name, me, inbox);
    }
    Ok(document)
}

/// The leads tab's document: one row per squad that has a lead, in tab
/// order (hidden squads last), each carrying its squad as `squad` and as
/// the `squad` field.
pub(crate) fn leads_document(tabs: &[String], documents: &BTreeMap<String, Value>) -> Value {
    let rows: Vec<Value> = in_tab_order(tabs, documents)
        .into_iter()
        .filter_map(|name| {
            let mut lead = documents[name]["squad"]["lead"].clone();
            if !lead.is_object() {
                return None;
            }
            lead["squad"] = json!(name);
            lead["fields"]["squad"] = json!(name);
            Some(lead)
        })
        .collect();
    json!({
        "squad": {"name": "leads", "lead": null},
        "sections": [{"title": null, "rows": rows}],
    })
}

/// Every squad tab's attention, and the leads tab's from its leads.
pub(crate) fn tab_attention(documents: &BTreeMap<String, Value>) -> BTreeMap<String, Attention> {
    let mut attention: BTreeMap<String, Attention> = documents
        .iter()
        .map(|(name, document)| (name.clone(), Attention::of(document)))
        .collect();
    attention.insert(
        LEADS.to_owned(),
        Attention::of(&leads_document(&[], documents)),
    );
    // Every squad's members, each counted once per squad.
    let all = attention
        .iter()
        .filter(|(key, _)| !tabs::builtin(key))
        .fold(Attention::default(), |sum, (_, one)| Attention {
            waiting: sum.waiting + one.waiting,
            blocked: sum.blocked + one.blocked,
        });
    attention.insert(ALL.to_owned(), all);
    attention
}

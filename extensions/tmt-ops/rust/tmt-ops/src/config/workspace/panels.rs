//! Persisted dashboard panel IDs and validity, independent of board rendering.
use std::{collections::BTreeSet, fmt::Debug};
use toml_edit::{Item, Table, Value, value};

mod sealed {
    pub trait Sealed {}
}

pub trait Panel: sealed::Sealed + Copy + Debug + Ord + 'static {
    const ALL: &'static [Self];
    const TARGET: Self;
    const SIDE_KEY: &'static str;
    fn key(self) -> &'static str;

    fn parse(key: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|panel| panel.key() == key)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum OverviewPanel {
    Summary,
    Digest,
    Subscriptions,
    Usage,
    Monitor,
    Exchanges,
}
impl sealed::Sealed for OverviewPanel {}
impl Panel for OverviewPanel {
    const ALL: &'static [Self] = &[
        Self::Summary,
        Self::Digest,
        Self::Subscriptions,
        Self::Usage,
        Self::Monitor,
        Self::Exchanges,
    ];
    const TARGET: Self = Self::Monitor;
    const SIDE_KEY: &'static str = "conversation";

    fn key(self) -> &'static str {
        match self {
            Self::Summary => "summary",
            Self::Digest => "digest",
            Self::Subscriptions => "subscriptions",
            Self::Usage => "usage",
            Self::Monitor => "monitor",
            Self::Exchanges => "exchanges",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DigestPanel {
    Status,
    Usage,
    Subscriptions,
    Chart,
    List,
    Report,
}
impl sealed::Sealed for DigestPanel {}
impl Panel for DigestPanel {
    const ALL: &'static [Self] = &[
        Self::Status,
        Self::Usage,
        Self::Subscriptions,
        Self::Chart,
        Self::List,
        Self::Report,
    ];
    const TARGET: Self = Self::List;
    const SIDE_KEY: &'static str = "details";

    fn key(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Usage => "usage",
            Self::Subscriptions => "subscriptions",
            Self::Chart => "chart",
            Self::List => "list",
            Self::Report => "report",
        }
    }
}

/// An explicit policy stays explicit even when it equals the default.
/// Absence and Reset belong to the enclosing optional workspace preference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Panels<P: Panel> {
    order: Vec<P>,
    hidden: BTreeSet<P>,
    side_shown: bool,
}
pub type OverviewPanels = Panels<OverviewPanel>;
pub type DigestPanels = Panels<DigestPanel>;

impl<P: Panel> Default for Panels<P> {
    fn default() -> Self {
        Self {
            order: P::ALL.to_vec(),
            hidden: BTreeSet::new(),
            side_shown: true,
        }
    }
}

impl<P: Panel> Panels<P> {
    pub fn parse(item: &Item) -> Result<Self, String> {
        let table = item
            .as_table_like()
            .ok_or("must be a table or inline table")?;
        for (key, _) in table.iter() {
            if !matches!(key, "order" | "hidden") && key != P::SIDE_KEY {
                return Err(format!("unknown panel field '{key}'"));
            }
        }
        let mut policy = Self::default();
        if let Some(item) = table.get("order") {
            policy.order = ids::<P>(item, "order")?;
            // Preserve the relative order the user saved across new panel IDs.
            for (index, panel) in P::ALL.iter().copied().enumerate() {
                if !policy.order.contains(&panel) {
                    policy.order.insert(index.min(policy.order.len()), panel);
                }
            }
        }
        if let Some(item) = table.get("hidden") {
            policy.hidden = ids::<P>(item, "hidden")?.into_iter().collect();
        }
        if let Some(item) = table.get(P::SIDE_KEY) {
            policy.side_shown = item
                .as_bool()
                .ok_or_else(|| format!("{} must be true or false", P::SIDE_KEY))?;
        }
        policy.validate()?;
        Ok(policy)
    }

    fn validate(&self) -> Result<(), String> {
        if self.order.len() != P::ALL.len()
            || self.order.iter().copied().collect::<BTreeSet<_>>()
                != P::ALL.iter().copied().collect()
        {
            return Err("order must contain every dashboard panel exactly once".into());
        }
        if !self.side_shown && !self.shown(P::TARGET) {
            return Err(format!("keep {} or {} shown", P::TARGET.key(), P::SIDE_KEY));
        }
        Ok(())
    }

    pub fn order(&self) -> &[P] {
        &self.order
    }

    pub fn hidden(&self) -> &BTreeSet<P> {
        &self.hidden
    }

    pub fn shown(&self, panel: P) -> bool {
        !self.hidden.contains(&panel)
    }

    pub fn side_shown(&self) -> bool {
        self.side_shown
    }

    pub fn set_shown(&mut self, panel: P, shown: bool) -> Result<(), String> {
        let mut next = self.clone();
        if shown {
            next.hidden.remove(&panel);
        } else {
            next.hidden.insert(panel);
        }
        next.validate()?;
        *self = next;
        Ok(())
    }

    pub fn set_side_shown(&mut self, shown: bool) -> Result<(), String> {
        let mut next = self.clone();
        next.side_shown = shown;
        next.validate()?;
        *self = next;
        Ok(())
    }

    /// Moves one slot; a boundary leaves both order and selection unchanged.
    pub fn move_panel(&mut self, panel: P, down: bool) -> bool {
        let index = self.order.iter().position(|item| *item == panel).unwrap();
        let next = if down {
            index.checked_add(1).filter(|next| *next < self.order.len())
        } else {
            index.checked_sub(1)
        };
        if let Some(next) = next {
            self.order.swap(index, next);
            true
        } else {
            false
        }
    }
}

/// Retains presence and malformed input for the workspace compare-and-save guard.
/// Rendering reads only `effective`; an unrelated save preserves the observation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelSetting<P: Panel> {
    // Preferences travel twice in the existing compare-and-save request. Keep
    // their owned observation out of that action's inline payload.
    observed: Box<Observation<P>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Observation<P: Panel> {
    Valid { policy: Panels<P>, fields: u8 },
    Invalid { source: String, error: String },
}

impl<P: Panel> PanelSetting<P> {
    pub fn observe(item: &Item) -> Self {
        let observed = match Panels::<P>::parse(item) {
            Ok(policy) => {
                let table = item.as_table_like().expect("validated table");
                let fields = ["order", "hidden", P::SIDE_KEY]
                    .into_iter()
                    .enumerate()
                    .fold(0, |mask, (bit, key)| {
                        mask | (u8::from(table.get(key).is_some()) << bit)
                    });
                Observation::Valid { policy, fields }
            }
            Err(error) => Observation::Invalid {
                source: item.to_string(),
                error,
            },
        };
        Self {
            observed: Box::new(observed),
        }
    }

    pub fn configured(policy: Panels<P>) -> Self {
        Self {
            observed: Box::new(Observation::Valid { policy, fields: 7 }),
        }
    }

    pub fn effective(&self) -> Panels<P> {
        match self.observed.as_ref() {
            Observation::Valid { policy, .. } => policy.clone(),
            Observation::Invalid { .. } => Panels::default(),
        }
    }

    pub fn error(&self) -> Option<&str> {
        match self.observed.as_ref() {
            Observation::Invalid { error, .. } => Some(error),
            Observation::Valid { .. } => None,
        }
    }

    pub(super) fn writable(&self) -> bool {
        matches!(self.observed.as_ref(), Observation::Valid { fields: 7, .. })
    }

    /// Only an explicit full Save calls this; partial/invalid observations are
    /// retained unchanged by the enclosing writer until Save or Reset.
    pub(super) fn write(&self, item: &mut Item) {
        let Observation::Valid { policy, fields: 7 } = self.observed.as_ref() else {
            unreachable!("only configured panel policies are written");
        };
        if item.as_table_like().is_none() {
            let decor = item.as_value().map(|value| value.decor().clone());
            *item = if let Some(decor) = decor {
                let mut table = toml_edit::InlineTable::new();
                *table.decor_mut() = decor;
                value(table)
            } else {
                Item::Table(Table::new())
            };
        }
        let table = item.as_table_like_mut().expect("table created");
        let obsolete: Vec<_> = table
            .iter()
            .filter(|(key, _)| !matches!(*key, "order" | "hidden") && *key != P::SIDE_KEY)
            .map(|(key, _)| key.to_owned())
            .collect();
        let mut displaced_comments = String::new();
        for key in obsolete {
            if let Some((key, old)) = table.get_key_value(&key) {
                append_comments(key.leaf_decor(), &mut displaced_comments);
                item_comments(old, &mut displaced_comments);
            }
            table.remove(&key);
        }
        for (key, ids) in [
            (
                "order",
                policy.order.iter().map(|id| id.key()).collect::<Vec<_>>(),
            ),
            ("hidden", policy.hidden.iter().map(|id| id.key()).collect()),
        ] {
            let previous = table.get(key).and_then(Item::as_array);
            let mut array = previous.cloned().unwrap_or_default();
            array.clear();
            let mut removed_comments = String::new();
            if let Some(previous) = previous {
                for old in previous
                    .iter()
                    .filter(|old| old.as_str().is_none_or(|id| !ids.contains(&id)))
                {
                    value_comments(old, &mut removed_comments);
                }
            }
            if !removed_comments.is_empty() {
                array.set_trailing(format!(
                    "{removed_comments}{}",
                    array.trailing().as_str().unwrap_or_default()
                ));
                array.set_trailing_comma(true);
            }
            for id in ids {
                let existing = previous
                    .and_then(|array| array.iter().find(|value| value.as_str() == Some(id)));
                array.push_formatted(existing.cloned().unwrap_or_else(|| Value::from(id)));
            }
            replace_value(table, key, Value::Array(array));
        }
        replace_value(table, P::SIDE_KEY, Value::from(policy.side_shown));
        if !displaced_comments.is_empty() {
            let mut key = table.key_mut("order").expect("full policy");
            let existing = key
                .leaf_decor()
                .prefix()
                .and_then(|raw| raw.as_str())
                .unwrap_or_default()
                .to_owned();
            key.leaf_decor_mut()
                .set_prefix(format!("{displaced_comments}{existing}"));
        }
    }
}

// Only decoration is inspected: a # inside an authored string is never a comment.
fn append_comments(decor: &toml_edit::Decor, output: &mut String) {
    for raw in [decor.prefix(), decor.suffix()].into_iter().flatten() {
        comment_lines(raw.as_str().unwrap_or_default(), output);
    }
}
fn comment_lines(raw: &str, output: &mut String) {
    for line in raw.lines() {
        if let Some(start) = line.find('#') {
            output.push('\n');
            output.push_str(&line[start..]);
            output.push('\n');
        }
    }
}
fn value_comments(value: &Value, output: &mut String) {
    append_comments(value.decor(), output);
    if let Some(array) = value.as_array() {
        for value in array.iter() {
            value_comments(value, output);
        }
        comment_lines(array.trailing().as_str().unwrap_or_default(), output);
    }
    if let Some(table) = value.as_inline_table() {
        for (name, value) in table.iter() {
            if let Some(key) = table.key(name) {
                append_comments(key.leaf_decor(), output);
            }
            value_comments(value, output);
        }
    }
}
fn item_comments(item: &Item, output: &mut String) {
    if let Some(value) = item.as_value() {
        value_comments(value, output);
    }
    if let Some(table) = item.as_table() {
        append_comments(table.decor(), output);
        for (name, item) in table.iter() {
            if let Some(key) = table.key(name) {
                append_comments(key.leaf_decor(), output);
            }
            item_comments(item, output);
        }
    }
}

fn replace_value(table: &mut dyn toml_edit::TableLike, key: &str, mut next: Value) {
    if let Some(old) = table.get(key).and_then(Item::as_value) {
        *next.decor_mut() = old.decor().clone();
    }
    if let Some(old) = table.get_mut(key) {
        *old = Item::Value(next);
    } else {
        table.insert(key, Item::Value(next));
    }
}

fn ids<P: Panel>(item: &Item, field: &str) -> Result<Vec<P>, String> {
    let array = item
        .as_array()
        .ok_or_else(|| format!("{field} must be an array of panel IDs"))?;
    if array.len() > P::ALL.len() {
        return Err(format!("{field} has more entries than dashboard panels"));
    }
    let mut seen = BTreeSet::new();
    array
        .iter()
        .map(|value| {
            let key = value
                .as_str()
                .ok_or_else(|| format!("{field} must contain panel ID strings"))?;
            let panel = P::parse(key).ok_or_else(|| format!("unknown {field} panel '{key}'"))?;
            if !seen.insert(panel) {
                return Err(format!("{field} repeats panel '{key}'"));
            }
            Ok(panel)
        })
        .collect()
}

#[cfg(test)]
mod tests;

//! The CLI and board share a validated disposable draft and the existing CAS writer.
use super::*;
use crate::split::{Size, Split};

impl Config {
    fn setting_key_in_scope(key: &str, squad: Option<&str>) -> bool {
        matches!(key, "board.refresh" | "tabs.order" | "tabs.hide")
            || squad.is_some_and(|name| !crate::tabs::aggregate(name))
                && (matches!(
                    key,
                    "layout"
                        | "board.direction"
                        | "board.sizes"
                        | "board.panes"
                        | "board.hidden_columns"
                        | "notes.render"
                        | "reminders.enabled"
                        | "reminders.stale_after"
                ) || key
                    .strip_prefix("states.")
                    .and_then(|name| name.strip_suffix(".color"))
                    .is_some_and(field_name))
    }

    pub fn can_edit_setting(&self, key: &str, squad: Option<&str>) -> bool {
        if !Self::setting_key_in_scope(key, squad) {
            return false;
        }
        if !matches!(key, "board.direction" | "board.sizes" | "board.panes") {
            return true;
        }
        squad.is_some_and(|name| {
            self.squad_table(name).ok().flatten().and_then(|s| s.get("board")).and_then(|b| b.get("layout")).is_none()
                && self.board(name).is_ok_and(|board| {
                    board.mode == BoardMode::Split && matches!(board.split, Split::Group { ref children, .. } if children.iter().all(|(size, child)| matches!(size, Size::Percent(_)) && matches!(child, Split::Pane(_))))
                })
        })
    }

    fn setting_path<'a>(squad: Option<&'a str>, key: &'a str) -> Result<Vec<&'a str>, SquadError> {
        if !Self::setting_key_in_scope(key, squad) {
            return Err(invalid(format!("`{key}` is read-only in this scope.")));
        }
        if let Some(name) = squad
            && !crate::squad::valid_name(name)
        {
            return Err(crate::squad::name_invalid(name));
        }
        let mut path = match squad.filter(|_| !key.starts_with("tabs.")) {
            Some(name) => vec!["squad", name],
            None => Vec::new(),
        };
        path.extend(key.split('.'));
        Ok(path)
    }

    fn parse_setting_value(key: &str, text: &str) -> Result<Item, SquadError> {
        if key == "reminders.enabled" {
            return text
                .parse::<bool>()
                .map(value)
                .map_err(|_| invalid("`reminders.enabled` must be true or false."));
        }
        if !matches!(
            key,
            "board.sizes" | "board.panes" | "board.hidden_columns" | "tabs.order" | "tabs.hide"
        ) {
            return Ok(value(text));
        }
        let parsed: serde_json::Value = serde_json::from_str(text)
            .map_err(|_| invalid(format!("`{key}` must be a JSON array.")))?;
        let mut values = toml_edit::Array::new();
        for item in parsed
            .as_array()
            .ok_or_else(|| invalid(format!("`{key}` must be a JSON array.")))?
        {
            if let Some(text) = item.as_str() {
                values.push(text);
            } else if let Some(number) = item.as_i64() {
                values.push(number);
            } else {
                return Err(invalid(format!(
                    "`{key}` entries must be strings or whole numbers."
                )));
            }
        }
        Ok(value(values))
    }

    fn materialize_simple_split(&mut self, name: &str) -> Result<(), SquadError> {
        let board = self.board(name)?;
        let layout = self.layout(name)?;
        let configured_layout = self.squad_table(name)?.and_then(|s| s.get("layout"));
        if configured_layout.is_none() {
            self.document["squad"][name]["layout"] = value(layout.as_str());
        }
        let resolved = split_value(&board.split);
        for field in ["direction", "panes", "sizes"] {
            if self.document["squad"][name]
                .get("board")
                .and_then(|b| b.get(field))
                .is_none()
            {
                let text = resolved[field]
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| resolved[field].to_string());
                let item = Self::parse_setting_value(&format!("board.{field}"), &text)?;
                self.insert_setting(&["squad", name, "board", field], item)?;
            }
        }
        Ok(())
    }

    fn insert_setting(&mut self, path: &[&str], mut replacement: Item) -> Result<(), SquadError> {
        let mut table: &mut dyn TableLike = self.document.as_table_mut();
        for (index, name) in path[..path.len() - 1].iter().enumerate() {
            if !table.contains_key(name) {
                let mut child = Table::new();
                child.set_implicit(index + 1 < path.len() - 1);
                table.insert(name, Item::Table(child));
            }
            table = table
                .get_mut(name)
                .and_then(Item::as_table_like_mut)
                .ok_or_else(|| {
                    invalid(format!(
                        "`{}` must be a table to edit this setting.",
                        path[..=index].join(".")
                    ))
                })?;
        }
        let name = path.last().unwrap();
        if let (Some(old), Some(new)) = (
            table.get(name).and_then(Item::as_value),
            replacement.as_value_mut(),
        ) {
            *new.decor_mut() = old.decor().clone();
        }
        table.insert(name, replacement);
        Ok(())
    }

    fn validate_setting_draft(&self, squad: Option<&str>, global: bool) -> Result<(), SquadError> {
        // Validate the global layer even if the chosen squad masks it.
        self.refresh("")?;
        self.tabs()?;
        self.settings(squad, false, None)?;
        if global && let Some(squads) = self.document.get("squad").and_then(Item::as_table_like) {
            for (name, _) in squads.iter() {
                self.settings(Some(name), false, None)?;
            }
        }
        Ok(())
    }

    pub fn preview_setting(
        &self,
        squad: Option<&str>,
        key: &str,
        text: &str,
    ) -> Result<Self, SquadError> {
        let path = Self::setting_path(squad, key)?;
        if !self.can_edit_setting(key, squad) {
            return Err(invalid(
                "Nested board layouts are read-only; edit the split tree in squad.toml.",
            ));
        }
        let replacement = Self::parse_setting_value(key, text)?;
        let mut draft = self.clone();
        if matches!(key, "board.direction" | "board.sizes" | "board.panes") {
            draft.materialize_simple_split(squad.unwrap())?;
        }
        draft.insert_setting(&path, replacement)?;
        draft.validate_setting_draft(squad, path.first() != Some(&"squad"))?;
        Ok(draft)
    }

    pub fn set_setting(
        &mut self,
        squad: Option<&str>,
        key: &str,
        text: &str,
    ) -> Result<bool, SquadError> {
        let draft = self.preview_setting(squad, key, text)?;
        let changed = self.document.to_string() != draft.document.to_string();
        self.write(|document| *document = draft.document)?;
        Ok(changed)
    }
}

//! The CLI and board share a validated disposable draft and the existing CAS writer.
use super::*;

impl Config {
    fn editable_setting(key: &str, squad: Option<&str>) -> bool {
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
                ) || key
                    .strip_prefix("states.")
                    .and_then(|name| name.strip_suffix(".color"))
                    .is_some_and(field_name))
    }

    pub fn setting_editable(&self, key: &str, squad: Option<&str>) -> bool {
        if !Self::editable_setting(key, squad) {
            return false;
        }
        if !matches!(key, "board.direction" | "board.sizes" | "board.panes") {
            return true;
        }
        squad.is_some_and(|name| {
            self.squad_table(name).ok().flatten().and_then(|s| s.get("board")).and_then(|b| b.get("layout")).is_none()
                && self.board(name).is_ok_and(|board| {
                    board.mode == BoardMode::Split && matches!(board.split, crate::split::Split::Group { ref children, .. } if children.iter().all(|(size, child)| matches!(size, crate::split::Size::Percent(_)) && matches!(child, crate::split::Split::Pane(_))))
                })
        })
    }

    pub fn preview_setting(
        &self,
        squad: Option<&str>,
        key: &str,
        text: &str,
    ) -> Result<Self, SquadError> {
        if !Self::editable_setting(key, squad) {
            return Err(invalid(format!("`{key}` is read-only in this scope.")));
        }
        if let Some(name) = squad
            && !crate::squad::valid_name(name)
        {
            return Err(crate::squad::name_invalid(name));
        }
        if !self.setting_editable(key, squad) {
            return Err(invalid(
                "Nested board layouts are read-only; edit the split tree in squad.toml.",
            ));
        }
        let array = matches!(
            key,
            "board.sizes" | "board.panes" | "board.hidden_columns" | "tabs.order" | "tabs.hide"
        );
        let mut replacement = if array {
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
            value(values)
        } else {
            value(text)
        };
        let global = key.starts_with("tabs.") || squad.is_none();
        let mut path: Vec<&str> = if global {
            Vec::new()
        } else {
            vec!["squad", squad.unwrap()]
        };
        path.extend(key.split('.'));
        let mut draft = self.clone();
        if matches!(key, "board.direction" | "board.sizes" | "board.panes") {
            let name = squad.unwrap();
            let board = self.board(name)?;
            let layout = self.layout(name)?;
            if draft
                .document
                .get("squad")
                .and_then(|s| s.get(name))
                .and_then(|s| s.get("layout"))
                .is_none()
            {
                draft.document["squad"][name]["layout"] = value(layout.as_str());
            }
            let crate::split::Split::Group {
                direction,
                children,
            } = &board.split
            else {
                unreachable!("flat split validated");
            };
            let mut panes = toml_edit::Array::new();
            let mut sizes = toml_edit::Array::new();
            for (size, pane) in children {
                let crate::split::Split::Pane(pane) = pane else {
                    unreachable!("flat split validated");
                };
                let crate::split::Size::Percent(size) = size else {
                    unreachable!("percent split validated");
                };
                panes.push(pane.title());
                sizes.push(i64::from(*size));
            }
            for (field, item) in [
                ("panes", value(panes)),
                ("sizes", value(sizes)),
                (
                    "direction",
                    value(if *direction == Direction::LeftRight {
                        "left-right"
                    } else {
                        "top-bottom"
                    }),
                ),
            ] {
                if draft.document["squad"][name]
                    .get("board")
                    .and_then(|b| b.get(field))
                    .is_none()
                {
                    draft.document["squad"][name]["board"][field] = item;
                }
            }
        }
        let mut table: &mut dyn TableLike = draft.document.as_table_mut();
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
        // Validate the global layer even if the chosen squad masks it.
        draft.refresh("")?;
        draft.tabs()?;
        let names: Vec<String> = if global {
            draft
                .document
                .get("squad")
                .and_then(Item::as_table_like)
                .into_iter()
                .flat_map(|s| s.iter().map(|(name, _)| name.to_owned()))
                .collect()
        } else {
            vec![squad.unwrap().into()]
        };
        draft.settings(squad, false, None)?;
        for name in names {
            draft.settings(Some(&name), false, None)?;
        }
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

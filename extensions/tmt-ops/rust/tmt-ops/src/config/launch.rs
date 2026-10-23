//! Caller-owned host launch overrides, never Core configuration.
use super::*;
impl Config {
    pub fn launch_override(&self, host: &str) -> Result<Option<Vec<String>>, SquadError> {
        let Some(launch) = self.document.get("launch") else {
            return Ok(None);
        };
        let hosts = launch
            .as_table_like()
            .ok_or_else(|| invalid("launch must be a table of host overrides."))?;
        let Some(host) = hosts.get(host) else {
            return Ok(None);
        };
        let table = host
            .as_table_like()
            .ok_or_else(|| invalid("Each launch host override must be a table."))?;
        let Some(item) = table.get("argv") else {
            return Ok(None);
        };
        let array = item
            .as_array()
            .ok_or_else(|| invalid("Launch argv must be an array."))?;
        let words = array
            .iter()
            .map(|word| {
                word.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| invalid("Launch argv words must be strings."))
            })
            .collect::<Result<Vec<_>, _>>()?;
        crate::launch::validate(&words).map_err(invalid)?;
        Ok(Some(words))
    }
    pub fn set_launch(&mut self, host: &str, argv: Option<&[String]>) -> Result<(), SquadError> {
        if !valid_host(host) {
            return Err(invalid("Invalid host name."));
        }
        let mut draft = self.clone();
        if let Some(argv) = argv {
            crate::launch::validate(argv).map_err(invalid)?;
            let mut array = toml_edit::Array::new();
            for word in argv {
                array.push(word.as_str());
            }
            draft.insert_setting(&["launch", host, "argv"], value(array))?;
        } else if let Some(table) = draft
            .document
            .get_mut("launch")
            .and_then(|v| v.get_mut(host))
            .and_then(Item::as_table_like_mut)
        {
            table.remove("argv");
        }
        self.write(|document| *document = draft.document)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn save_reset_and_conflict_use_the_held_revision_without_losing_comments() {
        let root = std::env::temp_dir().join(format!(
            "ops-launch-config-{}",
            crate::id::new_v4().unwrap()
        ));
        fs::create_dir(&root).unwrap();
        let path = root.join("ops.toml");
        fs::write(&path, "# retained\nopaque = 'kept'\n").unwrap();
        let mut current = Config::read(path.clone()).unwrap();
        let mut stale = current.clone();
        let argv = ["host", "#{pane_id}", "{session}", "{command}"].map(str::to_owned);
        current.set_launch("tmux", Some(&argv)).unwrap();
        let saved = fs::read_to_string(&path).unwrap();
        assert!(saved.starts_with("# retained\nopaque = 'kept'"));
        assert_eq!(
            Config::read(path.clone())
                .unwrap()
                .launch_override("tmux")
                .unwrap(),
            Some(argv.to_vec())
        );
        assert!(stale.set_launch("tmux", None).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), saved);
        current.set_launch("tmux", None).unwrap();
        assert!(current.launch_override("tmux").unwrap().is_none());
        assert!(
            fs::read_to_string(&path)
                .unwrap()
                .contains("opaque = 'kept'")
        );
        let before = fs::read_to_string(&path).unwrap();
        assert!(
            current
                .set_launch("tmux", Some(&["bad".into(), "embedded {command}".into()]))
                .is_err()
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
        fs::remove_dir_all(root).unwrap();
    }
}

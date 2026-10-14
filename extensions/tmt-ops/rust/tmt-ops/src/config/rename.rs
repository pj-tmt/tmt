//! Squad namespace edits share Config's format-preserving CAS writer. A repair
//! never overwrites an independently edited source or destination table.
use super::*;

impl Config {
    pub(crate) fn squad_config_text(&self, name: &str) -> Option<String> {
        self.document
            .get("squad")
            .and_then(|squad| squad.get(name))
            .map(ToString::to_string)
    }

    pub(crate) fn check_squad_rename(
        &self,
        old: &str,
        new: &str,
        expected: Option<&str>,
    ) -> Result<bool, SquadError> {
        let source = self.squad_config_text(old);
        let destination = self.squad_config_text(new);
        if source.as_deref() == expected && destination.is_none() {
            return Ok(source.is_some());
        }
        if source.is_none() && destination.as_deref() == expected {
            return Ok(false); // Repair after a confirmed/uncertain config publication.
        }
        Err(SquadError::new(
            "SQUAD_RENAME_CONFIG_CONFLICT",
            "The squad configuration changed; reconcile the old and new tables before repairing the rename.",
        ))
    }

    pub(crate) fn rename_squad(
        &mut self,
        old: &str,
        new: &str,
        expected: Option<&str>,
    ) -> Result<bool, SquadError> {
        if !self.check_squad_rename(old, new, expected)? {
            return Ok(false);
        }
        self.write(|document| {
            let squads = document
                .get_mut("squad")
                .unwrap()
                .as_table_like_mut()
                .unwrap();
            let original = squads.remove(old).expect("checked source table");
            squads.insert(new, original);
        })?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let directory = std::env::temp_dir().join(format!(
                "ops-config-rename-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&directory).unwrap();
            fs::write(directory.join("ops.toml"), "# keep global\n[bind]\nt='talk'\n[squad.product]\n# keep scope\nlayout='crew'\n[squad.product.theme]\nbase='mono'\n[squad.other]\nlayout='team'\n").unwrap();
            Self(directory)
        }
        fn config(&self) -> Config {
            Config::read(self.0.join("ops.toml")).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn config_rename_preserves_values_comments_and_other_scopes() {
        let fixture = Fixture::new();
        let mut config = fixture.config();
        let expected = config.squad_config_text("product").unwrap();
        assert!(
            config
                .rename_squad("product", "renamed", Some(&expected))
                .unwrap()
        );
        assert!(config.squad_config_text("product").is_none());
        assert_eq!(config.squad_config_text("renamed"), Some(expected.clone()));
        let bytes = fs::read_to_string(config.path()).unwrap();
        assert!(bytes.contains("# keep global\n[bind]\nt='talk'"));
        assert!(bytes.contains("# keep scope"));
        assert!(bytes.contains("[squad.renamed.theme]"));
        assert!(bytes.contains("[squad.other]\nlayout='team'"));
        assert!(
            !config
                .rename_squad("product", "renamed", Some(&expected))
                .unwrap()
        );
    }

    #[test]
    fn config_rename_refuses_intervening_edit_or_destination_conflict() {
        let fixture = Fixture::new();
        let mut config = fixture.config();
        let expected = config.squad_config_text("product").unwrap();
        assert_eq!(
            config
                .rename_squad("product", "other", Some(&expected))
                .unwrap_err()
                .code,
            "SQUAD_RENAME_CONFIG_CONFLICT"
        );
        fs::write(
            config.path(),
            "# user edit\n[squad.product]\nlayout='team'\n",
        )
        .unwrap();
        assert_eq!(
            config
                .rename_squad("product", "renamed", Some(&expected))
                .unwrap_err()
                .code,
            "SQUAD_CONFIG_CHANGED"
        );
        assert!(
            fs::read_to_string(config.path())
                .unwrap()
                .starts_with("# user edit")
        );
        assert!(config.squad_config_text("renamed").is_none());
    }
}

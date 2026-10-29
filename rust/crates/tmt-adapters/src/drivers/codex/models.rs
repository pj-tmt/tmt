//! Codex launch choices (#5539). Codex keeps its own model list in
//! `models_cache.json` under `CODEX_HOME`; only each entry's `slug` and
//! `visibility` are read, never its account or identity fields.
use crate::runtime::launch_choices::{LaunchChoices, ModelSource};
use serde::Deserialize;
use std::path::Path;

const CACHE: &str = "models_cache.json";
/// The cache holds per-model metadata; a larger file is not one Codex wrote.
const CACHE_LIMIT: usize = 1024 * 1024;
/// The models Codex 0.164 lists (`visibility: "list"`), for a first run that has
/// no cache yet. Checked 2026-10-29; the cache, once present, replaces it.
const FALLBACK_MODELS: &[&str] = &[
    "gpt-6.1-sol",
    "gpt-6-astra",
    "gpt-6.1-terra",
    "gpt-6-sol",
    "gpt-6-luna",
    "gpt-5.6-sol",
    "gpt-5.6-terra",
    "gpt-5.6-luna",
];
/// The `model_reasoning_effort` values Codex models accept; support varies by model.
const EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max", "ultra"];

#[derive(Deserialize)]
struct Cache {
    models: Vec<Entry>,
}

#[derive(Deserialize)]
struct Entry {
    slug: String,
    visibility: Option<String>,
}

pub(super) fn launch_choices(codex_home: &Path) -> LaunchChoices {
    let mut choices = LaunchChoices::fallback(FALLBACK_MODELS, EFFORTS);
    if let Some(models) = listed(codex_home) {
        choices.source = ModelSource::Provider;
        choices.models = models;
    }
    choices
}

/// The listed slugs in file order, or `None` when the cache is missing, too
/// large, a symlink, malformed, or lists nothing.
fn listed(codex_home: &Path) -> Option<Vec<String>> {
    let bytes = crate::bounded_file::read_no_follow(&codex_home.join(CACHE), CACHE_LIMIT).ok()?;
    let cache: Cache = serde_json::from_slice(&bytes).ok()?;
    let mut models = Vec::new();
    for entry in cache.models {
        if entry.visibility.as_deref() == Some("list")
            && !entry.slug.is_empty()
            && !models.contains(&entry.slug)
        {
            models.push(entry.slug);
        }
    }
    (!models.is_empty()).then_some(models)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDirectory;

    /// A private Codex home, never the real one.
    fn home() -> TestDirectory {
        let home = TestDirectory::new();
        assert!(home.path.starts_with(std::env::temp_dir()));
        home
    }

    #[test]
    fn listed_slugs_come_from_the_cache_in_file_order() {
        let home = home();
        std::fs::write(
            home.path.join(CACHE),
            include_bytes!("../../runtime/fixtures/codex-models-cache.json"),
        )
        .unwrap();
        let choices = launch_choices(home.path.as_path());
        assert_eq!(choices.source, ModelSource::Provider);
        assert_eq!(choices.models, ["gpt-6.1-sol", "gpt-6-luna"]);
        assert_eq!(choices.efforts, EFFORTS);
    }

    #[test]
    fn an_unusable_cache_falls_back_to_the_maintained_lists() {
        let home = home();
        let path = home.path.join(CACHE);
        let fallback = LaunchChoices::fallback(FALLBACK_MODELS, EFFORTS);
        assert!(!fallback.models.is_empty());
        // Missing.
        assert_eq!(launch_choices(home.path.as_path()), fallback);
        for bytes in [
            b"not json".to_vec(),
            br#"{"models":[{"slug":"hidden","visibility":"hide"}]}"#.to_vec(),
            br#"{"models":[]}"#.to_vec(),
            vec![b' '; CACHE_LIMIT + 1],
        ] {
            std::fs::write(&path, &bytes).unwrap();
            assert_eq!(launch_choices(home.path.as_path()), fallback);
        }
        // A symlink is not followed, even to a valid cache.
        let target = home.path.join("elsewhere.json");
        std::fs::write(
            &target,
            include_bytes!("../../runtime/fixtures/codex-models-cache.json"),
        )
        .unwrap();
        std::fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(&target, &path).unwrap();
        assert_eq!(launch_choices(home.path.as_path()), fallback);
    }
}

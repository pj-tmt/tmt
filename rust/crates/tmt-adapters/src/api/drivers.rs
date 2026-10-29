//! `drivers.models`: best-effort launch choices per AI tool (#5539).
//! Storage-free: each driver reads only its own provider files.
use super::Fault;
use serde::Serialize;

pub(super) fn models() -> Result<Vec<u8>, Fault> {
    let environment = crate::skill_installation::ProviderEnvironment::capture()
        .map_err(|_| Fault::unavailable())?;
    models_in(&environment)
}

fn models_in(
    environment: &crate::skill_installation::ProviderEnvironment,
) -> Result<Vec<u8>, Fault> {
    #[derive(Serialize)]
    struct Entry<'a> {
        driver: &'a str,
        #[serde(flatten)]
        choices: crate::runtime::launch_choices::LaunchChoices,
    }
    let registry = crate::runtime::RuntimeRegistry::first_party();
    let mut drivers = Vec::new();
    for driver in registry.harnesses() {
        let Some(lifecycle) = tmt_core::binding::session::HarnessId::new(driver)
            .ok()
            .and_then(|harness| registry.lifecycle(&harness))
        else {
            continue;
        };
        drivers.push(Entry {
            driver,
            choices: lifecycle.launch_choices(environment),
        });
    }
    drivers.sort_by_key(|entry| entry.driver);
    serde_json::to_vec(&serde_json::json!({"drivers": drivers})).map_err(|_| Fault::unavailable())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skill_installation::ProviderEnvironment;
    use crate::test_support::TestDirectory;
    use serde_json::{Value, json};

    #[test]
    fn every_runtime_driver_reports_its_source_models_and_efforts() {
        let root = TestDirectory::new();
        assert!(root.path.starts_with(std::env::temp_dir()));
        let codex_home = root.path.join("codex");
        std::fs::create_dir(&codex_home).unwrap();
        std::fs::write(
            codex_home.join("models_cache.json"),
            include_bytes!("../runtime/fixtures/codex-models-cache.json"),
        )
        .unwrap();
        let environment = ProviderEnvironment::from_parts(
            &root.path,
            &root.path,
            Vec::new(),
            [("CODEX_HOME", codex_home.clone())],
        );
        let value: Value = serde_json::from_slice(&models_in(&environment).unwrap()).unwrap();
        let drivers = value["drivers"].as_array().unwrap();
        let names: Vec<_> = drivers
            .iter()
            .map(|d| d["driver"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["agy", "claude", "codex"]);
        assert_eq!(
            drivers[0],
            json!({"driver":"agy","source":"fallback","models":[],"efforts":[]})
        );
        assert_eq!(drivers[1]["source"], "fallback");
        assert!(
            drivers[1]["models"]
                .as_array()
                .unwrap()
                .contains(&json!("opus"))
        );
        assert_eq!(
            drivers[1]["efforts"],
            json!(["low", "medium", "high", "xhigh", "max"])
        );
        assert_eq!(
            drivers[2],
            json!({"driver":"codex","source":"provider","models":["gpt-6.1-sol","gpt-6-luna"],
                "efforts":["low","medium","high","xhigh","max","ultra"]})
        );

        // Without the cache Codex falls back and still lists its efforts.
        std::fs::remove_file(codex_home.join("models_cache.json")).unwrap();
        let value: Value = serde_json::from_slice(&models_in(&environment).unwrap()).unwrap();
        assert_eq!(value["drivers"][2]["source"], "fallback");
        assert_eq!(value["drivers"][2]["models"], json!([]));
    }
}

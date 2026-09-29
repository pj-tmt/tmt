//! Antigravity (`agy`): skills only, under its Gemini configuration.

pub static DRIVER: super::DriverDefinition = super::DriverDefinition {
    descriptor: &tmt_core::driver::descriptor::AGY,
    env: &[],
    locate,
    runtime: None,
};

fn locate(environment: &crate::skill_installation::ProviderEnvironment) -> super::Locations {
    let config = environment.home().join(".gemini/config");
    super::Locations {
        skills: config.join("skills"),
        config_dirs: vec![config],
        legacy_skills: Vec::new(),
        hook_settings: None,
    }
}

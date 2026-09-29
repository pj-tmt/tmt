//! Gemini CLI: skills only, read from the shared `~/.agents/skills` root.

pub static DRIVER: super::DriverDefinition = super::DriverDefinition {
    descriptor: &tmt_core::driver::descriptor::GEMINI,
    env: &[],
    locate,
    runtime: None,
};

fn locate(environment: &crate::skill_installation::ProviderEnvironment) -> super::Locations {
    super::Locations {
        config_dirs: vec![environment.home().join(".gemini")],
        skills: environment.universal_skills(),
        legacy_skills: Vec::new(),
        hook_settings: None,
    }
}

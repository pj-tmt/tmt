//! Muse: skills only, read from the shared `~/.agents/skills` root;
//! its configuration lives under `~/.config/muse`.

pub static DRIVER: super::DriverDefinition = super::DriverDefinition {
    descriptor: &tmt_core::driver::descriptor::MUSE,
    env: &[],
    locate,
    runtime: None,
};

fn locate(environment: &crate::skill_installation::ProviderEnvironment) -> super::Locations {
    super::Locations {
        config_dirs: vec![environment.home().join(".config/muse")],
        skills: environment.universal_skills(),
        legacy_skills: Vec::new(),
        hook_settings: None,
    }
}

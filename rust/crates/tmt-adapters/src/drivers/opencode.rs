//! OpenCode: skills only, read from the shared `~/.agents/skills` root. Its
//! configuration is `OPENCODE_CONFIG_DIR`, else `$XDG_CONFIG_HOME/opencode`,
//! else `~/.config/opencode`.

const NAME: &str = tmt_core::driver::descriptor::OPENCODE.name;

pub static DRIVER: super::DriverDefinition = super::DriverDefinition {
    descriptor: &tmt_core::driver::descriptor::OPENCODE,
    env: &["OPENCODE_CONFIG_DIR", "XDG_CONFIG_HOME"],
    locate,
    runtime: None,
};

fn locate(environment: &crate::skill_installation::ProviderEnvironment) -> super::Locations {
    let config = match (
        environment.var("OPENCODE_CONFIG_DIR"),
        environment.var("XDG_CONFIG_HOME"),
    ) {
        (Some(configured), _) => environment.resolve(configured),
        (None, Some(xdg)) => environment.resolve(xdg).join(NAME),
        (None, None) => environment.home().join(".config").join(NAME),
    };
    super::Locations {
        config_dirs: vec![config],
        skills: environment.universal_skills(),
        legacy_skills: Vec::new(),
        hook_settings: None,
    }
}

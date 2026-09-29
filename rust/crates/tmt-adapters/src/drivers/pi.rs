//! Pi coding agent: skills only, under `PI_CODING_AGENT_DIR` (`~` forms
//! expand to the home directory) or `~/.pi/agent`.

use std::path::Path;

pub static DRIVER: super::DriverDefinition = super::DriverDefinition {
    descriptor: &tmt_core::driver::descriptor::PI,
    env: &["PI_CODING_AGENT_DIR"],
    locate,
    runtime: None,
};

fn locate(environment: &crate::skill_installation::ProviderEnvironment) -> super::Locations {
    let home = environment.home();
    let directory = match environment.var("PI_CODING_AGENT_DIR") {
        None => home.join(".pi/agent"),
        Some(configured) if configured == Path::new("~") => home.to_path_buf(),
        Some(configured) => match configured.strip_prefix("~/") {
            Ok(relative) => home.join(relative),
            Err(_) => environment.resolve(configured),
        },
    };
    super::Locations {
        skills: directory.join("skills"),
        config_dirs: vec![directory],
        legacy_skills: Vec::new(),
        hook_settings: None,
    }
}

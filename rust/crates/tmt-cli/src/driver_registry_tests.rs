//! #440: a driver added as one core descriptor plus one adapter module is
//! picked up by setup, detection, completion and `ls` with no other edit.

use std::path::PathBuf;
use tmt_adapters::{
    drivers::{DriverDefinition, Locations, Registry},
    setup,
    skill_installation::ProviderEnvironment,
};
use tmt_cli_style::{AnsiColor, Token};
use tmt_core::driver::descriptor::{DriverDescriptor, HookFormat, Hue};

static DESCRIPTOR: DriverDescriptor = DriverDescriptor {
    name: "fixture-agent",
    executables: &["fixture-agent"],
    hooks: Some(HookFormat::SessionHooksJson),
    hue: Hue::Magenta,
};

static MODULE: DriverDefinition = DriverDefinition {
    descriptor: &DESCRIPTOR,
    env: &[],
    locate: |environment| Locations {
        config_dirs: vec![environment.home().join(".fixture-agent")],
        skills: environment.home().join(".fixture-agent/skills"),
        legacy_skills: Vec::new(),
        hook_settings: Some(environment.home().join(".fixture-agent/settings.json")),
    },
    runtime: None,
};

fn descriptors() -> Vec<&'static DriverDescriptor> {
    tmt_core::driver::ALL
        .into_iter()
        .chain([&DESCRIPTOR])
        .collect()
}

fn values(command: &clap::Command, path: &[&str], argument: &str) -> Vec<String> {
    let mut command = command.clone();
    for name in path {
        command = command.find_subcommand(name).expect("subcommand").clone();
    }
    command
        .get_arguments()
        .find(|arg| arg.get_id() == argument)
        .expect("argument")
        .get_possible_values()
        .iter()
        .map(|value| value.get_name().to_owned())
        .collect()
}

#[test]
fn completion_values_come_from_the_descriptors() {
    let grammar = crate::grammar::grammar_for(&descriptors());
    assert!(values(&grammar, &["install"], "agent").contains(&"fixture-agent".into()));
    assert!(values(&grammar, &["setup"], "provider").contains(&"fixture-agent".into()));
    let builtin = crate::grammar::grammar();
    assert!(!values(&builtin, &["setup"], "provider").contains(&"fixture-agent".into()));
}

#[test]
fn setup_and_detection_come_from_the_registry() {
    let registry = Registry::builtin().with(&MODULE);
    assert_eq!(
        registry.with_hooks().last().map(DriverDefinition::name),
        Some("fixture-agent")
    );
    let home = std::env::temp_dir().join(format!("tmt-440-{}", std::process::id()));
    std::fs::create_dir_all(home.join(".fixture-agent")).unwrap();
    let environment = ProviderEnvironment::from_parts(&home, &home, Vec::new(), []);
    let detected = environment.detect_in(&registry);
    std::fs::remove_dir_all(&home).unwrap();
    assert_eq!(detected, [&MODULE]);
    let plan = setup::plan(
        &MODULE,
        PathBuf::from("/settings.json"),
        None,
        "/stable/tmt".into(),
        false,
        setup::UsageHook::Keep,
    )
    .unwrap();
    assert!(plan.change.after.contains("__hook fixture-agent"));
}

#[test]
fn ls_colors_come_from_the_descriptors() {
    assert_eq!(
        crate::driver_style::token_in(&descriptors(), "fixture-agent"),
        Token::Driver(Some(AnsiColor::Magenta))
    );
    assert_eq!(
        crate::driver_style::token("fixture-agent"),
        Token::Driver(None)
    );
}

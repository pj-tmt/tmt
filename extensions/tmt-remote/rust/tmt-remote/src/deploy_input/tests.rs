use super::*;
use serde_json::json;

const MANIFEST: &[InputSpec] = &[
    InputSpec {
        input: "account",
        flag: "account",
        kind: InputKind::SelectExisting,
        default: DefaultChoice::MustProvide,
        creates: None,
        cost: "Selects an existing account",
        permissions: &["read account"],
    },
    InputSpec {
        input: "region",
        flag: "region",
        kind: InputKind::SelectExisting,
        default: DefaultChoice::Value("west"),
        creates: None,
        cost: "Free-plan region",
        permissions: &["read account"],
    },
    InputSpec {
        input: "site",
        flag: "site",
        kind: InputKind::CreateOrAdopt,
        default: DefaultChoice::DuringPlanning("a new site name"),
        creates: Some("a site if absent"),
        cost: "Free-plan site",
        permissions: &["read account", "create site"],
    },
];
fn values(pairs: &[(&str, &str)]) -> Values {
    pairs
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}
fn given() -> Values {
    values(&[
        ("account", "mine"),
        ("region", "east"),
        ("site", "existing"),
    ])
}
fn required(supplied: &Values, defaults: bool) -> RequiredInputs {
    match select(MANIFEST, supplied, &Values::new(), defaults).unwrap_err() {
        InputError::Required(required) => required,
        other => panic!("{other:?}"),
    }
}
#[test]
fn explicit_inputs_need_no_defaults_and_preserve_manifest_order() {
    let inputs = select(MANIFEST, &given(), &Values::new(), false).unwrap();
    assert_eq!(
        inputs,
        vec![
            SelectedInput {
                input: "account",
                value: SelectedValue::Value("mine".into())
            },
            SelectedInput {
                input: "region",
                value: SelectedValue::Value("east".into())
            },
            SelectedInput {
                input: "site",
                value: SelectedValue::Value("existing".into())
            },
        ]
    );
}
#[test]
fn missing_inputs_refuse_even_when_defaults_exist_and_requirements_are_ordered() {
    let refusal = required(&values(&[("account", "mine")]), false);
    assert_eq!(
        refusal
            .missing
            .iter()
            .map(|input| input.flag.as_str())
            .collect::<Vec<_>>(),
        ["--region", "--site"]
    );
    assert_eq!(refusal.permissions, ["read account", "create site"]);
    assert_eq!(refusal.creates, ["a site if absent"]);
    assert_eq!(refusal.missing[0].default, "west");
}
#[test]
fn defaults_fill_only_omissions_and_state_dependent_values_stay_unresolved() {
    let inputs = select(
        MANIFEST,
        &values(&[("account", "mine")]),
        &Values::new(),
        true,
    )
    .unwrap();
    assert_eq!(inputs[1].value, SelectedValue::Value("west".into()));
    assert_eq!(
        inputs[2].value,
        SelectedValue::DuringPlanning("a new site name")
    );
    assert_eq!(
        select(MANIFEST, &given(), &Values::new(), true).unwrap(),
        select(MANIFEST, &given(), &Values::new(), false).unwrap()
    );
    let missing = required(&Values::new(), true);
    assert_eq!(missing.missing.len(), 1);
    assert_eq!(missing.missing[0].input, "account");
    assert_eq!(missing.missing[0].default, "none: must be given");
}
#[test]
fn records_reuse_exact_values_even_when_defaults_would_choose_another_resource() {
    let record = given();
    for defaults in [false, true] {
        assert_eq!(
            select(MANIFEST, &Values::new(), &record, defaults).unwrap(),
            select(MANIFEST, &record, &Values::new(), false).unwrap()
        );
        assert_eq!(
            select(MANIFEST, &record, &record, defaults).unwrap(),
            select(MANIFEST, &record, &Values::new(), false).unwrap()
        );
        assert_eq!(
            select(
                MANIFEST,
                &values(&[("site", "different")]),
                &record,
                defaults
            ),
            Err(InputError::RecordedConflict {
                input: "site",
                flag: "--site".into()
            })
        );
    }
    assert_eq!(
        record,
        given(),
        "input selection must not rewrite an old record"
    );
}
#[test]
fn missing_input_error_has_the_standard_fixed_shape_without_provider_or_user_text() {
    let supplied = values(&[("account", "canary-https://provider.invalid/private")]);
    let refusal = required(&supplied, false);
    let hint = defaults_hint("sample", MANIFEST, &supplied, &refusal);
    let error = crate::error::RemoteError::deploy_inputs(refusal, &hint);
    assert_eq!(
        error.json_value(),
        json!({"error": {
            "code": "REMOTE_DEPLOY_INPUT_REQUIRED",
            "message": "Missing required deployment inputs: --region, --site.",
            "missing": [
                {"input":"region", "flag":"--region", "default":"west", "creates":null},
                {"input":"site", "flag":"--site", "default":"a new site name", "creates":"a site if absent"}
            ],
            "permissions": ["read account", "create site"],
            "creates": ["a site if absent"]
        }})
    );
    assert!(!error.json_value().to_string().contains("canary"));
    assert_eq!(
        error.hint.unwrap(),
        format!(
            "--region: --defaults uses west; would create: nothing.\n--site: --defaults uses a new site name; would create: a site if absent.\nNeeds these permissions\n  read account\n  create site\nWould create\n  a site if absent\nTry: {hint}\nRerun with the flags above, or add --defaults to use the listed values."
        )
    );
}
fn command() -> Command {
    let backend = tmt_cli_style::command(&tmt_cli_style::CommandSpec {
        name: "sample",
        summary: "Exercise shared deployment inputs",
        examples: &[tmt_cli_style::Example {
            command: "tmt remote deploy sample --account mine --defaults --json",
            note: "Read the selected input plan",
        }],
        outputs: tmt_cli_style::OutputModes::HumanAndJson,
        details: "Test backend; no provider.",
    });
    Command::new("tmt").subcommand(
        Command::new("remote")
            .subcommand(Command::new("deploy").subcommand(register(backend, MANIFEST))),
    )
}
#[test]
fn the_defaults_hint_names_and_quotes_a_missing_must_provide_input() {
    let supplied = Values::new();
    let refusal = required(&supplied, false);
    let hint = defaults_hint("sample", MANIFEST, &supplied, &refusal);
    assert_eq!(
        hint,
        "tmt remote deploy 'sample' --account='<account>' --defaults"
    );
    let argv = tmt_cli_style::Example {
        command: Box::leak(hint.into_boxed_str()),
        note: "printed required-input hint",
    }
    .argv()
    .unwrap();
    let parsed = command().try_get_matches_from(argv).unwrap();
    let parsed = parsed
        .subcommand_matches("remote")
        .unwrap()
        .subcommand_matches("deploy")
        .unwrap()
        .subcommand_matches("sample")
        .unwrap();
    assert_eq!(parsed.get_one::<String>("account").unwrap(), "<account>");
    assert!(parsed.get_flag("defaults"));
    // Once the user replaces the named placeholder, defaults can fill the rest.
    let supplied = values(&[("account", "chosen account")]);
    assert!(
        select(
            MANIFEST,
            &supplied,
            &Values::new(),
            parsed.get_flag("defaults")
        )
        .is_ok()
    );
}
#[test]
fn the_defaults_hint_keeps_recorded_must_provide_inputs_implicit() {
    let supplied = Values::new();
    let recorded = values(&[("account", "saved account")]);
    let InputError::Required(refusal) = select(MANIFEST, &supplied, &recorded, false).unwrap_err()
    else {
        panic!("expected missing region/site");
    };
    let hint = defaults_hint("sample", MANIFEST, &supplied, &refusal);
    assert_eq!(hint, "tmt remote deploy 'sample' --defaults");
    assert!(select(MANIFEST, &supplied, &recorded, true).is_ok());
}
#[test]
fn the_printed_defaults_hint_round_trips_through_registered_help_and_parser() {
    for name in ["-Saved account", "owner's account", "--defaults", ""] {
        let supplied = values(&[("account", name)]);
        let refusal = required(&supplied, false);
        let hint = defaults_hint("sample", MANIFEST, &supplied, &refusal);
        let error = crate::error::RemoteError::deploy_inputs(refusal, &hint);
        let human = error.hint.unwrap();
        let hint = human
            .lines()
            .find_map(|line| line.strip_prefix("Try: "))
            .unwrap();
        let argv = tmt_cli_style::Example {
            command: Box::leak(hint.to_owned().into_boxed_str()),
            note: "printed hint",
        }
        .argv()
        .unwrap();
        let parsed = command().try_get_matches_from(argv).unwrap();
        let parsed = parsed
            .subcommand_matches("remote")
            .unwrap()
            .subcommand_matches("deploy")
            .unwrap()
            .subcommand_matches("sample")
            .unwrap();
        assert_eq!(parsed.get_one::<String>("account").unwrap(), name);
        assert!(parsed.get_flag("defaults"));
        assert_eq!(
            select(
                MANIFEST,
                &supplied,
                &Values::new(),
                parsed.get_flag("defaults")
            )
            .unwrap()[0]
                .value,
            SelectedValue::Value(name.into())
        );
    }
    let mut cmd = command();
    let help = cmd
        .find_subcommand_mut("remote")
        .unwrap()
        .find_subcommand_mut("deploy")
        .unwrap()
        .find_subcommand_mut("sample")
        .unwrap()
        .render_long_help()
        .to_string();
    assert!(help.contains("--defaults"));
    assert!(help.contains("does not authorize changes"));
    assert!(
        command()
            .try_get_matches_from(["tmt", "remote", "deploy", "sample", "--yes"])
            .is_err()
    );
    assert!(
        command()
            .try_get_matches_from([
                "tmt",
                "remote",
                "deploy",
                "sample",
                "--account",
                "--defaults"
            ])
            .is_err()
    );
}

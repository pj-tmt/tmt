use super::*;
use crate::{
    deploy_cli,
    deploy_discovery::{DeclarationSource, DiscoveryRefusal},
    deploy_record::DeployRecordStore,
};
use clap::Command;

#[allow(dead_code)]
#[path = "../../../tests/support/deploy_fixture.rs"]
mod fixture;

fn args() -> FirestoreArgs {
    FirestoreArgs {
        project: "demo-remote-1".into(),
        region: "asia-east1".into(),
        remote: None,
        database: Some("(default)".into()),
        web_app: Some("new".into()),
        site: Some("new".into()),
        sign_in: vec![SignInProvider::Anonymous],
        defaults: false,
        authorize: None,
        json: true,
        details: false,
    }
}
fn parse(argv: &[&str]) -> Result<FirestoreArgs, DeployCliError> {
    let matches = deploy_cli::command().try_get_matches_from(argv).unwrap();
    deploy_cli::arguments(&matches)
}
#[test]
fn all_flags_defaults_and_mixed_inputs_use_the_same_manifest() {
    let all = args();
    let selected = resolve(&all, None, &Values::new(), &[]).unwrap();
    assert_eq!(selected.project, all.project);
    assert_eq!(selected.database, "(default)");
    let defaults = parse(&[
        "firestore",
        "--project",
        "demo-remote-1",
        "--region",
        "asia-east1",
        "--defaults",
        "--json",
    ])
    .unwrap();
    assert_eq!(
        resolve(&defaults, None, &Values::new(), &[]).unwrap(),
        selected
    );
    let mixed = parse(&[
        "firestore",
        "--project",
        "demo-remote-1",
        "--region",
        "asia-east1",
        "--defaults",
        "--site",
        "selected-site",
        "--web-app",
        "1:123:web:abc",
        "--sign-in",
        "google.com",
    ])
    .unwrap();
    let selected = resolve(&mixed, None, &Values::new(), &[]).unwrap();
    assert_eq!(selected.site, "selected-site");
    assert_eq!(selected.web_app, "1:123:web:abc");
    assert_eq!(selected.sign_in, [SignInProvider::Google]);
}
#[test]
fn missing_inputs_use_the_fixed_refusal_and_its_hint_parses_with_placeholders() {
    let missing = parse(&["firestore", "--json"]).unwrap();
    let DeployCliError::Local(error) = resolve(&missing, None, &Values::new(), &[]).unwrap_err()
    else {
        panic!("expected missing inputs");
    };
    assert_eq!(error.code, "REMOTE_DEPLOY_INPUT_REQUIRED");
    assert_eq!(
        error.json_value()["error"]["missing"]
            .as_array()
            .unwrap()
            .len(),
        6
    );
    let human = error.hint.unwrap();
    let hint = human
        .lines()
        .find_map(|line| line.strip_prefix("Try: "))
        .unwrap();
    let argv = tmt_cli_style::Example {
        command: Box::leak(hint.to_owned().into_boxed_str()),
        note: "missing input hint",
    }
    .argv()
    .unwrap();
    let matches = Command::new("tmt")
        .subcommand(
            Command::new("remote")
                .subcommand(Command::new("deploy").subcommand(deploy_cli::command())),
        )
        .try_get_matches_from(argv)
        .unwrap();
    let parsed = deploy_cli::arguments(
        matches
            .subcommand_matches("remote")
            .unwrap()
            .subcommand_matches("deploy")
            .unwrap()
            .subcommand_matches("firestore")
            .unwrap(),
    )
    .unwrap();
    assert_eq!(parsed.project, "<project>");
    assert_eq!(parsed.region, "<region>");
    assert!(parsed.defaults);
    assert!(
        matches!(validate_supplied(&parsed), Err(DeployCliError::Local(error)) if error.code == "REMOTE_DEPLOY_INPUT_INVALID")
    );
}
struct Untouched;
#[test]
fn dedicated_hosted_google_refuses_but_anonymous_and_default_site_remain_supported() {
    let mut selected = resolve(&args(), None, &Values::new(), &[]).unwrap();
    for site in ["new", "selected-site", "demo-remote-1"] {
        selected.site = site.into();
        selected.sign_in = vec![SignInProvider::Google];
        assert!(validate_hosted_sign_in(&selected, false).is_ok());
        let result = validate_hosted_sign_in(&selected, true);
        if site == selected.project {
            assert!(result.is_ok());
        } else {
            assert!(
                matches!(result, Err(DeployCliError::Local(error)) if error.code == "REMOTE_DEPLOY_INPUT_INVALID"
                    && error.message.contains("dedicated Hosting site")
                    && error.message.contains("--site <project-id>")
                    && error.message.contains("--sign-in anonymous"))
            );
        }
        selected.sign_in = vec![SignInProvider::Anonymous];
        assert!(validate_hosted_sign_in(&selected, true).is_ok());
    }
}
impl DeclarationSource for Untouched {
    fn declaration(&mut self, _: &str) -> Result<Option<Vec<u8>>, DiscoveryRefusal> {
        panic!("invalid inputs must precede discovery too");
    }
}
#[test]
fn copied_placeholders_and_named_databases_refuse_before_all_provider_work() {
    let valid = args();
    assert!(validate_supplied(&valid).is_ok());
    for input in ["project", "region", "database", "web-app", "site"] {
        let mut invalid = valid.clone();
        match input {
            "project" => invalid.project = "<project>".into(),
            "region" => invalid.region = "<region>".into(),
            "database" => invalid.database = Some("<database>".into()),
            "web-app" => invalid.web_app = Some("<web-app>".into()),
            _ => invalid.site = Some("<site>".into()),
        }
        let result = deploy_cli::execute::<crate::deploy_firestore::DeployFirestore<'_>>(
            &invalid,
            &mut Untouched,
            &["colab"],
            || panic!("no provider factory/login/inventory/mutation"),
            || panic!("no layout"),
            || panic!("no clock"),
        );
        assert!(
            matches!(result, Err(DeployCliError::Local(error)) if error.code == "REMOTE_DEPLOY_INPUT_INVALID")
        );
    }
    assert!(
        matches!(parse(&["firestore", "--sign-in", "<sign-in>", "--json"]), Err(DeployCliError::Local(error)) if error.code == "REMOTE_DEPLOY_INPUT_INVALID")
    );
    let mut named = valid;
    named.database = Some("named".into());
    let result = deploy_cli::execute::<crate::deploy_firestore::DeployFirestore<'_>>(
        &named,
        &mut Untouched,
        &["colab"],
        || panic!("no provider"),
        || panic!("no layout"),
        || panic!("no clock"),
    );
    assert!(
        matches!(result, Err(DeployCliError::Local(error)) if error.message.contains("only (default) is supported on the free plan"))
    );
}
#[test]
fn draft_identity_and_dedicated_site_stay_stable_and_recorded_inputs_reopen() {
    let root = fixture::Root::new();
    let layout = root.layout();
    let all = args();
    let resources = resolve(&all, None, &Values::new(), &[]).unwrap();
    let mut store = DeployRecordStore::open(&layout, &all.project, "retry fixture").unwrap();
    let first = store.load_or_draft().unwrap();
    let second = store.load_or_draft().unwrap();
    assert_eq!(first.deployment_id, second.deployment_id);
    let selected_site = site(&resources, &first.deployment_id, None).unwrap();
    assert_ne!(selected_site, all.project);
    assert!(crate::hosting::site_id_ok(&selected_site));
    assert_eq!(
        selected_site,
        site(&resources, &second.deployment_id, None).unwrap()
    );
    assert_eq!(
        site(
            &resources,
            &first.deployment_id,
            Some("frozen-earlier-site")
        )
        .unwrap(),
        "frozen-earlier-site"
    );
    store.select_inputs(resources.clone()).unwrap();
    store.bind_target(&all.project, &all.region).unwrap();
    let record = store.load_or_draft().unwrap();
    store.persist(&record).unwrap();
    drop(store);
    let before =
        std::fs::read(root.remote().join("deployments/demo-remote-1/deploy.json")).unwrap();
    let store = DeployRecordStore::open(&layout, &all.project, "retry fixture").unwrap();
    let record = store.load_or_draft().unwrap();
    let saved = record_values(&record, &all.project, store.region());
    let empty = parse(&["firestore", "--defaults"]).unwrap();
    assert_eq!(resolve(&empty, None, &saved, &[]).unwrap(), resources);
    let mut different = all.clone();
    different.site = Some("another-site".into());
    assert!(
        matches!(resolve(&different, None, &saved, &[]), Err(DeployCliError::Local(error)) if error.code == "REMOTE_DEPLOY_PROJECT_CONFLICT")
    );
    assert_eq!(
        std::fs::read(root.remote().join("deployments/demo-remote-1/deploy.json")).unwrap(),
        before
    );
}
#[test]
fn permission_metadata_covers_non_flag_effects_without_hosting_requirements_when_inapplicable() {
    use crate::deploy_plan::{self, CloudBackend, Enabled, Supplied, Target};
    let plan = deploy_plan::compose(
        Target {
            backend: CloudBackend::Firestore,
            physical_ttl: false,
        },
        &[Enabled {
            name: "colab",
            supplied: Some(Supplied {
                declaration: include_bytes!(
                    "../../../tests/fixtures/declarations/colab-firestore.json"
                ),
                artifact: include_bytes!(
                    "../../../tests/fixtures/declarations/rules/colab-admission.rules"
                ),
            }),
        }],
    )
    .unwrap();
    let base = permission_requirements(&plan, false);
    assert!(base.iter().any(|requirement| requirement.step == "rules"
        && requirement.permission == "firebaserules.releases.update"));
    assert!(base.iter().any(|requirement| requirement.step == "indexes"));
    assert!(
        !base.iter().any(|requirement| matches!(
            requirement.step,
            "hosting" | "web-app" | "owner:register"
        ))
    );
    let hosted = permission_requirements(&plan, true);
    for permission in ["datastore.entities.update", "datastore.entities.delete"] {
        assert!(
            hosted
                .iter()
                .any(|requirement| requirement.step == "owner:register"
                    && requirement.permission == permission)
        );
    }
    assert!(
        hosted
            .iter()
            .any(|requirement| requirement.step == "owner:register"
                && requirement.permission == "datastore.entities.create")
    );
    for input in manifest() {
        for permission in input.permissions {
            assert!(
                hosted
                    .iter()
                    .any(|requirement| requirement.permission == *permission)
            );
        }
    }
}

//! The library command owner, driven by injected inputs/provider and real atomic files.
//! All provider/declaration bytes here are fixtures; no real credential or account is read.
#[path = "support/deploy_fixture.rs"]
mod deploy_fixture;
#[path = "support/deploy_port.rs"]
mod deploy_port;
use deploy_fixture::{ID, Root};
use std::{fs, path::PathBuf};
use tmt_remote::deploy_plan::{CloudBackend, Enabled, Plan, Supplied, Target, compose};
const ACCOUNT: &str = "owner@example.test";
pub fn fixture(group: &str, name: &str) -> Vec<u8> {
    fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(group)
            .join(name),
    )
    .unwrap()
}
pub fn plan() -> Plan {
    let colab = fixture("declarations", "colab-firestore.json");
    let notes = fixture("declarations", "notes-firestore.json");
    let ca = fixture("declarations", "rules/colab-admission.rules");
    let na = fixture("declarations", "rules/notes-admission.rules");
    compose(
        Target {
            backend: CloudBackend::Firestore,
            physical_ttl: false,
        },
        &[
            Enabled {
                name: "colab",
                supplied: Some(Supplied {
                    declaration: &colab,
                    artifact: &ca,
                }),
            },
            Enabled {
                name: "notes",
                supplied: Some(Supplied {
                    declaration: &notes,
                    artifact: &na,
                }),
            },
        ],
    )
    .unwrap()
}

use deploy_port::{Fake, When};
use tmt_remote::{
    deploy_command::{DeployCommandError, DeployCommandInput, DeployCommandOptions, execute},
    deploy_record::{self, DeployRecordStore},
    deploy_run::{
        self, DeployError, DeployInput, DeployRecord, DeployRefusal, DeploySink, DeploySinkError,
        RunState, SignInProvider,
    },
};
const BOTH: [SignInProvider; 2] = [SignInProvider::Google, SignInProvider::Anonymous];
fn input<'a>(plan: &'a tmt_remote::deploy_plan::Plan, body: &'a [u8]) -> DeployCommandInput<'a> {
    DeployCommandInput {
        extensions: plan,
        project: "demo-remote-1",
        location: "asia-east1",
        sign_in: &BOTH,
        rules_body: body,
        live_rules: None,
    }
}
#[test]
fn plan_only_matches_independent_human_and_json_goldens_without_provider_effects() {
    let root = Root::new();
    let layout = root.layout();
    let mut store = DeployRecordStore::open(&layout).unwrap();
    store.persist(&DeployRecord::new(ID)).unwrap();
    let plan = plan();
    let body = fixture("deploy_run", "rules-body.rules");
    let mut port = Fake::new(ACCOUNT);
    let out = execute(
        &input(&plan, &body),
        &DeployCommandOptions::default(),
        &mut store,
        &mut port,
        7,
    )
    .unwrap();
    assert_eq!(
        out.human.as_bytes(),
        fixture("deploy_command", "plan-human.txt")
    );
    assert_eq!(
        out.json,
        serde_json::from_slice::<serde_json::Value>(&fixture("deploy_command", "plan.json"))
            .unwrap()
    );
    assert_eq!(
        serde_json::to_vec(&out.json).unwrap(),
        fixture("deploy_command", "plan.json")
    );
    assert!(port.calls.is_empty());
    assert!(port.effects.is_empty());
    assert!(
        deploy_record::read(&layout)
            .unwrap()
            .unwrap()
            .usable_binding()
            .is_none()
    );
    drop(store);
    let bytes = std::fs::read(root.remote().join("deploy.json")).unwrap();
    let mut store = DeployRecordStore::open(&layout).unwrap();
    let again = execute(
        &input(&plan, &body),
        &DeployCommandOptions::default(),
        &mut store,
        &mut port,
        9,
    )
    .unwrap();
    assert_eq!(out.json, again.json);
    assert_eq!(
        std::fs::read(root.remote().join("deploy.json")).unwrap(),
        bytes
    );
}
#[test]
fn first_plan_creates_a_stable_local_draft_but_refused_inputs_create_no_record() {
    let root = Root::new();
    let layout = root.layout();
    let mut store = DeployRecordStore::open(&layout).unwrap();
    let plan = plan();
    let body = fixture("deploy_run", "rules-body.rules");
    let mut port = Fake::new(ACCOUNT);
    let mut request = input(&plan, &body);
    request.sign_in = &[];
    assert!(matches!(
        execute(
            &request,
            &DeployCommandOptions::default(),
            &mut store,
            &mut port,
            1
        ),
        Err(DeployCommandError::Refused(DeployRefusal::NoSignIn))
    ));
    assert!(deploy_record::read(&layout).unwrap().is_none());
    assert!(port.calls.is_empty());
    request.sign_in = &BOTH;
    let first = execute(
        &request,
        &DeployCommandOptions::default(),
        &mut store,
        &mut port,
        1,
    )
    .unwrap();
    drop(store);
    let mut store = DeployRecordStore::open(&layout).unwrap();
    let second = execute(
        &request,
        &DeployCommandOptions::default(),
        &mut store,
        &mut port,
        2,
    )
    .unwrap();
    assert_eq!(first.json, second.json);
    assert!(port.effects.is_empty());
}
#[test]
fn stale_short_changed_account_and_foreign_authorizations_have_zero_effects() {
    let root = Root::new();
    let layout = root.layout();
    let mut store = DeployRecordStore::open(&layout).unwrap();
    store.persist(&DeployRecord::new(ID)).unwrap();
    let plan = plan();
    let body = fixture("deploy_run", "rules-body.rules");
    let mut port = Fake::new(ACCOUNT);
    let mut request = input(&plan, &body);
    let preview = execute(
        &request,
        &DeployCommandOptions::default(),
        &mut store,
        &mut port,
        1,
    )
    .unwrap();
    let digest = preview.json["planDigest"].as_str().unwrap();
    for typed in ["", &digest[..11], &"0".repeat(64)] {
        assert!(matches!(
            execute(
                &request,
                &DeployCommandOptions {
                    authorize: Some(typed),
                },
                &mut store,
                &mut port,
                2
            ),
            Err(DeployCommandError::Refused(
                DeployRefusal::AuthorizationStale
            ))
        ));
    }
    port.account = "other@example.test".into();
    assert!(matches!(
        execute(
            &request,
            &DeployCommandOptions {
                authorize: Some(digest),
            },
            &mut store,
            &mut port,
            2
        ),
        Err(DeployCommandError::Refused(
            DeployRefusal::AuthorizationStale
        ))
    ));
    port.account = ACCOUNT.into();
    let foreign = fixture("deploy_run", "foreign.rules");
    request.live_rules = Some(&foreign);
    let preview = execute(
        &request,
        &DeployCommandOptions::default(),
        &mut store,
        &mut port,
        1,
    )
    .unwrap();
    assert!(matches!(
        execute(
            &request,
            &DeployCommandOptions {
                authorize: Some(digest),
            },
            &mut store,
            &mut port,
            2
        ),
        Err(DeployCommandError::Refused(
            DeployRefusal::AuthorizationStale
        ))
    ));
    assert!(port.calls.is_empty());
    assert!(port.effects.is_empty());
    assert_eq!(
        preview.human.as_bytes(),
        fixture("deploy_command", "foreign-plan-human.txt")
    );
    let replaced = preview.json["plan"]["rules"]["replacedDigest"]
        .as_str()
        .unwrap();
    assert_eq!(replaced.len(), 64);
    assert!(
        preview
            .human
            .contains(&format!("Existing Rules fingerprint: {replaced}"))
    );
    assert!(
        preview
            .human
            .contains("DESTRUCTIVE: Replace the live Rules for project demo-remote-1.")
    );
    assert!(preview.human.contains(&format!(
        "Authorizing plan {} allows this replacement.",
        &preview.json["planDigest"].as_str().unwrap()[..12]
    )));
    port.rules = Some(foreign.clone());
    let done = execute(
        &request_for_foreign(&plan, &body, &foreign),
        &DeployCommandOptions {
            authorize: preview.json["planDigest"].as_str(),
        },
        &mut store,
        &mut port,
        3,
    )
    .unwrap();
    assert_eq!(done.json["record"]["run"]["state"], "complete");
}
fn request_for_foreign<'a>(
    plan: &'a tmt_remote::deploy_plan::Plan,
    body: &'a [u8],
    foreign: &'a [u8],
) -> DeployCommandInput<'a> {
    let mut request = input(plan, body);
    request.live_rules = Some(foreign);
    request
}

// A crash fails one real-file save, not the engine transition. The persisted file
// remains authoritative and the same provider instance preserves actual effects.
struct DeployFailingSink<'a, 'b> {
    owner: &'a mut DeployRecordStore<'b>,
    saves: usize,
    fail_at: usize,
}
impl DeploySink for DeployFailingSink<'_, '_> {
    fn save(&mut self, record: &DeployRecord) -> Result<(), DeploySinkError> {
        let n = self.saves;
        self.saves += 1;
        if n == self.fail_at {
            return Err(DeploySinkError);
        }
        self.owner.save(record)
    }
}
#[test]
fn every_interrupted_save_reloads_the_same_original_and_never_duplicates_provider_effects() {
    let extension = plan();
    let body = fixture("deploy_run", "rules-body.rules");
    let plan = deploy_run::prepare(
        &extension,
        &DeployInput {
            account: ACCOUNT,
            project: "demo-remote-1",
            deployment_id: ID,
            location: "asia-east1",
            sign_in: &BOTH,
            rules_body: &body,
            live_rules: None,
        },
    )
    .unwrap();
    let authorization = deploy_run::authorize(&plan, plan.digest()).unwrap();
    let mut baseline = deploy_port::Mem::default();
    let mut port = Fake::new(ACCOUNT);
    deploy_run::run(
        &plan,
        &authorization,
        DeployRecord::new(ID),
        &mut port,
        &mut baseline,
        10,
    )
    .unwrap();
    for fail_at in 0..baseline.saved.len() {
        let root = Root::new();
        let layout = root.layout();
        let mut store = DeployRecordStore::open(&layout).unwrap();
        store.persist(&DeployRecord::new(ID)).unwrap();
        let mut port = Fake::new(ACCOUNT);
        port.unrelated
            .insert("foreign-collection".into(), "unchanged".into());
        let unrelated = port.unrelated.clone();
        let mut sink = DeployFailingSink {
            owner: &mut store,
            saves: 0,
            fail_at,
        };
        assert_eq!(
            deploy_run::run(
                &plan,
                &authorization,
                DeployRecord::new(ID),
                &mut port,
                &mut sink,
                10
            ),
            Err(DeployError::Interrupted)
        );
        drop(store);
        let reloaded = deploy_record::read(&layout).unwrap().unwrap();
        assert_eq!(reloaded.deployment_id, ID);
        assert!(
            reloaded.usable_binding().is_none(),
            "binding prematurely saved at {fail_at}"
        );
        let mut store = DeployRecordStore::open(&layout).unwrap();
        let done =
            deploy_run::run(&plan, &authorization, reloaded, &mut port, &mut store, 11).unwrap();
        assert!(done.usable_binding().is_some());
        assert_eq!(done.run.as_ref().unwrap().state, RunState::Complete);
        assert_eq!(deploy_record::read(&layout).unwrap().unwrap(), done);
        assert!(
            port.effects.values().all(|count| *count == 1),
            "duplicate at {fail_at}"
        );
        assert_eq!(port.unrelated, unrelated);
    }
}
#[test]
fn lost_effect_returns_unknown_and_same_authorization_observes_before_resuming() {
    let root = Root::new();
    let layout = root.layout();
    let mut store = DeployRecordStore::open(&layout).unwrap();
    let plan = plan();
    let body = fixture("deploy_run", "rules-body.rules");
    let request = input(&plan, &body);
    let mut port = Fake::new(ACCOUNT);
    let preview = execute(
        &request,
        &DeployCommandOptions::default(),
        &mut store,
        &mut port,
        1,
    )
    .unwrap();
    let opts = DeployCommandOptions {
        authorize: preview.json["planDigest"].as_str(),
    };
    port.faults.insert("rules".into(), When::AfterEffectUnknown);
    let out = execute(&request, &opts, &mut store, &mut port, 2).unwrap();
    assert!(out.human.contains("rules: unknown"));
    assert!(out.human.contains("usable binding: false"));
    assert!(
        deploy_record::read(&layout)
            .unwrap()
            .unwrap()
            .usable_binding()
            .is_none()
    );
    drop(store);
    let mut store = DeployRecordStore::open(&layout).unwrap();
    let done = execute(&request, &opts, &mut store, &mut port, 3).unwrap();
    assert_eq!(done.json["record"]["run"]["state"], "complete");
    assert_eq!(port.effects_of("rules"), 1);
}

#[test]
fn every_provider_failure_keeps_real_file_partial_and_only_a_successful_resume_binds() {
    let extension = plan();
    let body = fixture("deploy_run", "rules-body.rules");
    let request = input(&extension, &body);
    for step in [
        "database",
        "sign-in:anonymous",
        "sign-in:google.com",
        "index:x/colab/checkpoints#epoch:desc",
        "index:x/colab/log#expiresAt:asc",
        "index:x/colab/log#sequence:asc",
        "rules",
    ] {
        for when in [When::BeforeEffect, When::AfterEffectUnknown] {
            let root = Root::new();
            let layout = root.layout();
            let mut store = DeployRecordStore::open(&layout).unwrap();
            let mut port = Fake::new(ACCOUNT);
            port.unrelated.insert("other".into(), "untouched".into());
            let preview = execute(
                &request,
                &DeployCommandOptions::default(),
                &mut store,
                &mut port,
                1,
            )
            .unwrap();
            let opts = DeployCommandOptions {
                authorize: preview.json["planDigest"].as_str(),
            };
            port.faults.insert(step.into(), when);
            let out = execute(&request, &opts, &mut store, &mut port, 2).unwrap();
            assert_eq!(out.json["record"]["run"]["state"], "partial");
            assert!(
                deploy_record::read(&layout)
                    .unwrap()
                    .unwrap()
                    .usable_binding()
                    .is_none()
            );
            assert!(out.human.contains(if when == When::BeforeEffect {
                "REMOTE_DEPLOY_PROVIDER_REJECTED"
            } else {
                "unknown"
            }));
            drop(store);
            let mut store = DeployRecordStore::open(&layout).unwrap();
            execute(&request, &opts, &mut store, &mut port, 3).unwrap();
            assert!(
                deploy_record::read(&layout)
                    .unwrap()
                    .unwrap()
                    .usable_binding()
                    .is_some()
            );
            assert!(port.effects.values().all(|n| *n == 1), "{step} {when:?}");
            assert_eq!(port.unrelated["other"], "untouched");
        }
    }
}
#[test]
fn old_open_reader_sees_the_draft_while_new_readers_see_the_verified_binding() {
    use std::io::Read;
    let root = Root::new();
    let layout = root.layout();
    let mut store = DeployRecordStore::open(&layout).unwrap();
    let plan = plan();
    let body = fixture("deploy_run", "rules-body.rules");
    let request = input(&plan, &body);
    let mut port = Fake::new(ACCOUNT);
    let preview = execute(
        &request,
        &DeployCommandOptions::default(),
        &mut store,
        &mut port,
        1,
    )
    .unwrap();
    let mut opened = layout.read_file("deploy.json").unwrap().unwrap();
    execute(
        &request,
        &DeployCommandOptions {
            authorize: preview.json["planDigest"].as_str(),
        },
        &mut store,
        &mut port,
        2,
    )
    .unwrap();
    let mut bytes = Vec::new();
    opened.read_to_end(&mut bytes).unwrap();
    let old: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(old["record"]["binding"].is_null());
    assert!(
        deploy_record::read(&layout)
            .unwrap()
            .unwrap()
            .usable_binding()
            .is_some()
    );
}
#[test]
fn existing_binding_survives_pre_rules_failure_but_not_a_possible_new_rules_effect() {
    let root = Root::new();
    let layout = root.layout();
    let mut store = DeployRecordStore::open(&layout).unwrap();
    let plan = plan();
    let body = fixture("deploy_run", "rules-body.rules");
    let mut port = Fake::new(ACCOUNT);
    let request = input(&plan, &body);
    let preview = execute(
        &request,
        &DeployCommandOptions::default(),
        &mut store,
        &mut port,
        1,
    )
    .unwrap();
    execute(
        &request,
        &DeployCommandOptions {
            authorize: preview.json["planDigest"].as_str(),
        },
        &mut store,
        &mut port,
        2,
    )
    .unwrap();
    let binding = deploy_record::read(&layout).unwrap().unwrap().binding;
    let changed = [body.as_slice(), b"\n"].concat();
    let mut request = input(&plan, &changed);
    let old_rules = port.rules.clone().unwrap();
    request.live_rules = Some(&old_rules);
    let preview = execute(
        &request,
        &DeployCommandOptions::default(),
        &mut store,
        &mut port,
        3,
    )
    .unwrap();
    let opts = DeployCommandOptions {
        authorize: preview.json["planDigest"].as_str(),
    };
    port.observe_faults
        .insert("database".into(), deploy_run::DeployProviderError::Unknown);
    execute(&request, &opts, &mut store, &mut port, 4).unwrap();
    assert_eq!(
        deploy_record::read(&layout)
            .unwrap()
            .unwrap()
            .usable_binding(),
        binding.as_ref()
    );
    port.faults.insert("rules".into(), When::AfterEffectUnknown);
    execute(&request, &opts, &mut store, &mut port, 5).unwrap();
    assert!(
        deploy_record::read(&layout)
            .unwrap()
            .unwrap()
            .usable_binding()
            .is_none()
    );
}
#[test]
fn owner_action_and_building_are_reported_without_a_usable_binding() {
    let root = Root::new();
    let layout = root.layout();
    let mut store = DeployRecordStore::open(&layout).unwrap();
    let plan = plan();
    let body = fixture("deploy_run", "rules-body.rules");
    let request = input(&plan, &body);
    let mut port = Fake::new(ACCOUNT);
    let preview = execute(
        &request,
        &DeployCommandOptions::default(),
        &mut store,
        &mut port,
        1,
    )
    .unwrap();
    let opts = DeployCommandOptions {
        authorize: preview.json["planDigest"].as_str(),
    };
    port.owner_pending.insert(
        "sign-in:google.com".into(),
        deploy_run::DeployOwnerAction::EnableGoogleSignIn,
    );
    let action = execute(&request, &opts, &mut store, &mut port, 2).unwrap();
    assert!(
        action
            .human
            .contains("REMOTE_DEPLOY_OWNER_ENABLE_GOOGLE_SIGN_IN")
    );
    assert!(action.human.contains("Firebase console"));
    port.owner_pending.clear();
    port.present.insert("index:x/colab/log#sequence:asc".into());
    port.building
        .insert("index:x/colab/log#sequence:asc".into());
    let building = execute(&request, &opts, &mut store, &mut port, 3).unwrap();
    assert!(building.human.contains("verify: building"));
    assert!(
        deploy_record::read(&layout)
            .unwrap()
            .unwrap()
            .usable_binding()
            .is_none()
    );
}

mod cli_composition {
    use super::*;
    use serde_json::json;
    use sha2::{Digest, Sha256};
    use tmt_remote::{
        deploy_cli::{self, DeployCliError, FirestoreCommandPort},
        deploy_discovery::{DeclarationSource, DiscoveryRefusal},
        deploy_record::{self, DeployRecordStore},
        deploy_run::{
            DeployApplied, DeployObserved, DeployPlan, DeployPort, DeployProviderError,
            DeployRecord, DeployStep,
        },
    };
    struct DeployCliSource(usize, bool);
    impl DeclarationSource for DeployCliSource {
        fn declaration(&mut self, extension: &str) -> Result<Option<Vec<u8>>, DiscoveryRefusal> {
            self.0 += 1;
            assert_eq!(extension, "colab");
            if !self.1 {
                return Ok(None);
            }
            let read = |path: &str| {
                fs::read_to_string(
                    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                        .join("tests/fixtures/rules")
                        .join(path),
                )
                .unwrap()
            };
            let declaration = read("colab.json");
            let digest: String = Sha256::digest(declaration.as_bytes())
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            Ok(Some(serde_json::to_vec(&json!({"version":1,"extension":"colab","backend":"firestore","declaration":declaration,"declarationDigest":digest,"artifact":read("colab.rules")})).unwrap()))
        }
    }
    struct DeployCliProvider(deploy_port::Fake, usize);
    impl DeployPort for &mut DeployCliProvider {
        fn account(&mut self) -> Result<String, DeployProviderError> {
            self.1 += 1;
            self.0.account()
        }
        fn observe(
            &mut self,
            plan: &DeployPlan,
            step: &DeployStep,
        ) -> Result<DeployObserved, DeployProviderError> {
            self.0.observe(plan, step)
        }
        fn apply(
            &mut self,
            plan: &DeployPlan,
            step: &DeployStep,
        ) -> Result<DeployApplied, DeployProviderError> {
            self.0.apply(plan, step)
        }
    }
    impl FirestoreCommandPort for &mut DeployCliProvider {
        fn login(&mut self) -> Result<(), tmt_remote::deploy_firestore::DeploySetupError> {
            self.1 += 1;
            Ok(())
        }
        fn check_index_budget(
            &mut self,
            _: &str,
            _: &tmt_remote::deploy_plan::Plan,
        ) -> Result<(), DeployProviderError> {
            self.1 += 1;
            Ok(())
        }
        fn live_rules(&mut self, _: &str) -> Result<Option<Vec<u8>>, DeployProviderError> {
            self.1 += 1;
            Ok(self.0.rules.clone())
        }
    }
    fn args(extra: &[&str]) -> Result<deploy_cli::FirestoreArgs, DeployCliError> {
        let mut words = vec![
            "firestore",
            "--project",
            "demo-remote-1",
            "--region",
            "asia-east1",
            "--sign-in",
            "anonymous",
        ];
        words.extend_from_slice(extra);
        let matches = deploy_cli::command()
            .try_get_matches_from(words)
            .map_err(|_| DeployCliError::Usage)?;
        deploy_cli::arguments(&matches)
    }
    #[test]
    fn authorization_shape_is_usage_before_discovery_or_setup() {
        for extra in [
            vec![
                "--replace-rules",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ],
            vec!["--authorize", "abc"],
            vec!["--authorize", "ABCDEF012345"],
            vec!["--authorize", "not-a-digest"],
        ] {
            assert!(matches!(args(&extra), Err(DeployCliError::Usage)));
        }
        assert!(args(&["--authorize", "abcdef012345"]).is_ok());
        assert!(
            deploy_cli::command()
                .try_get_matches_from([
                    "firestore",
                    "--project",
                    "demo-remote-1",
                    "--region",
                    "asia-east1"
                ])
                .is_err()
        );
        assert!(
            deploy_cli::command()
                .try_get_matches_from([
                    "firestore",
                    "--project",
                    "demo-remote-1",
                    "--region",
                    "asia-east1",
                    "--sign-in",
                    "anonymous",
                    "--yes"
                ])
                .is_err()
        );
    }
    #[test]
    fn no_declaration_reports_unavailable_without_provider_or_record_effect() {
        let root = Root::new();
        let layout = root.layout();
        let before = deploy_record::read(&layout).unwrap();
        let mut source = DeployCliSource(0, false);
        let provider = DeployCliProvider(deploy_port::Fake::new("owner@example.test"), 0);
        let out = deploy_cli::execute(
            &args(&[]).unwrap(),
            &mut source,
            &["colab"],
            || -> Result<&mut DeployCliProvider, DeployCliError> {
                panic!("no declaration must not create a provider")
            },
            || panic!("no declaration must not open the record"),
            || panic!("no declaration must not need a timestamp"),
        )
        .unwrap();
        assert_eq!(out.json["available"], false);
        assert_eq!(out.json["extensions"]["unavailable"][0]["name"], "colab");
        assert_eq!(provider.1, 0);
        assert!(provider.0.calls.is_empty());
        assert!(provider.0.effects.is_empty());
        assert_eq!(source.0, 1);
        assert_eq!(deploy_record::read(&layout).unwrap(), before);
    }
    #[test]
    fn discovered_plan_authorizes_whole_envelope_and_recovers_without_resending() {
        let root = Root::new();
        let layout = root.layout();
        let mut store = DeployRecordStore::open(&layout).unwrap();
        store.persist(&DeployRecord::new(ID)).unwrap();
        drop(store);
        let mut source = DeployCliSource(0, true);
        let mut provider = DeployCliProvider(deploy_port::Fake::new("owner@example.test"), 0);
        let preview = deploy_cli::execute(
            &args(&["--json"]).unwrap(),
            &mut source,
            &["colab"],
            || Ok(&mut provider),
            || Ok(root.layout()),
            || Ok(1),
        )
        .unwrap();
        assert!(provider.0.effects.is_empty());
        assert_eq!(source.0, 1);
        assert_eq!(preview.json["authorized"], false);
        assert!(preview.human.contains("Not authorized; nothing changed"));
        let digest = preview.json["planDigest"].as_str().unwrap();
        let opts = args(&["--authorize", &digest[..12]]).unwrap();
        provider
            .0
            .faults
            .insert("rules".into(), deploy_port::When::AfterEffectUnknown);
        let partial = deploy_cli::execute(
            &opts,
            &mut source,
            &["colab"],
            || Ok(&mut provider),
            || Ok(root.layout()),
            || Ok(2),
        )
        .unwrap();
        assert!(partial.human.contains("rules: unknown"));
        assert!(partial.json["record"]["binding"].is_null());
        assert!(
            partial
                .human
                .contains("some changes may already be applied")
        );
        assert!(partial.human.contains(&format!(
            "cat '{}'",
            layout.directory.join("deploy.json").display()
        )));
        assert!(
            !partial
                .human
                .contains("Nothing changed in your Firebase project.")
        );
        let before = fs::read(root.remote().join("deploy.json")).unwrap();
        let calls = provider.1;
        let mut conflicting = args(&[]).unwrap();
        conflicting.project = "demo-remote-2".into();
        assert!(matches!(deploy_cli::execute(
            &conflicting, &mut source, &["colab"],
            || -> Result<&mut DeployCliProvider, DeployCliError> { panic!("partial target must fence provider setup") },
            || Ok(root.layout()), || Ok(3),
        ), Err(DeployCliError::Local(e)) if e.code == "REMOTE_DEPLOY_PROJECT_CONFLICT"));
        assert_eq!(fs::read(root.remote().join("deploy.json")).unwrap(), before);
        assert_eq!(provider.1, calls);
        let done = deploy_cli::execute(
            &opts,
            &mut source,
            &["colab"],
            || Ok(&mut provider),
            || Ok(root.layout()),
            || Ok(3),
        )
        .unwrap();
        assert_eq!(done.json["record"]["run"]["state"], "complete");
        assert_eq!(provider.0.effects_of("rules"), 1);
        assert_eq!(source.0, 4);
    }
    #[test]
    fn plan_only_a_then_plan_and_authorize_b_binds_only_b() {
        let root = Root::new();
        let mut source = DeployCliSource(0, true);
        let mut provider = DeployCliProvider(deploy_port::Fake::new(ACCOUNT), 0);
        let request_a = args(&[]).unwrap();
        let plan_a = deploy_cli::execute(
            &request_a,
            &mut source,
            &["colab"],
            || Ok(&mut provider),
            || Ok(root.layout()),
            || Ok(1),
        )
        .unwrap();
        let path = root.remote().join("deploy.json");
        let draft: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(draft["version"], 1);
        assert!(draft.get("target").is_none());
        let mut request_b = args(&[]).unwrap();
        request_b.project = "demo-remote-2".into();
        request_b.region = "us-central1".into();
        let plan_b = deploy_cli::execute(
            &request_b,
            &mut source,
            &["colab"],
            || Ok(&mut provider),
            || Ok(root.layout()),
            || Ok(2),
        )
        .unwrap();
        assert_eq!(
            plan_a.json["record"]["deploymentId"],
            plan_b.json["record"]["deploymentId"]
        );
        assert_ne!(plan_a.json["planDigest"], plan_b.json["planDigest"]);
        assert_eq!(
            fs::read(&path).unwrap(),
            serde_json::to_vec(&draft).unwrap()
        );
        assert!(provider.0.effects.is_empty());
        request_b.authorize = Some(plan_b.json["planDigest"].as_str().unwrap()[..12].to_owned());
        let done = deploy_cli::execute(
            &request_b,
            &mut source,
            &["colab"],
            || Ok(&mut provider),
            || Ok(root.layout()),
            || Ok(3),
        )
        .unwrap();
        assert_eq!(done.json["record"]["run"]["state"], "complete");
        let bound: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(bound["version"], 2);
        assert_eq!(
            bound["target"],
            serde_json::json!({"project":"demo-remote-2", "region":"us-central1"})
        );
    }
    #[test]
    fn retained_target_conflict_precedes_provider_creation_and_preserves_the_record() {
        let root = Root::new();
        let mut source = DeployCliSource(0, true);
        let mut provider = DeployCliProvider(deploy_port::Fake::new(ACCOUNT), 0);
        let request = args(&[]).unwrap();
        let preview = deploy_cli::execute(
            &request,
            &mut source,
            &["colab"],
            || Ok(&mut provider),
            || Ok(root.layout()),
            || Ok(1),
        )
        .unwrap();
        let digest = preview.json["planDigest"].as_str().unwrap();
        deploy_cli::execute(
            &args(&["--authorize", &digest[..12]]).unwrap(),
            &mut source,
            &["colab"],
            || Ok(&mut provider),
            || Ok(root.layout()),
            || Ok(2),
        )
        .unwrap();
        let path = root.remote().join("deploy.json");
        let before = fs::read(&path).unwrap();
        for region in [false, true] {
            let mut changed = args(&[]).unwrap();
            if region {
                changed.region = "us-central1".into();
            } else {
                changed.project = "demo-remote-2".into();
            }
            let result = deploy_cli::execute(
                &changed,
                &mut source,
                &["colab"],
                || -> Result<&mut DeployCliProvider, DeployCliError> {
                    panic!("conflicting target must refuse before provider creation")
                },
                || Ok(root.layout()),
                || Ok(2),
            );
            assert!(
                matches!(result, Err(DeployCliError::Local(e)) if e.code == "REMOTE_DEPLOY_PROJECT_CONFLICT")
            );
            assert_eq!(fs::read(&path).unwrap(), before);
        }
        assert!(!provider.0.effects.is_empty());
    }
}

#[test]
fn a_second_home_sees_the_first_homes_rules_as_foreign_and_must_authorize_its_own_plan() {
    let first = Root::new();
    let first_layout = first.layout();
    let mut first_store = DeployRecordStore::open(&first_layout).unwrap();
    let extensions = plan();
    let body = fixture("deploy_run", "rules-body.rules");
    let mut provider = Fake::new(ACCOUNT);
    let request = input(&extensions, &body);
    let first_plan = execute(
        &request,
        &DeployCommandOptions::default(),
        &mut first_store,
        &mut provider,
        1,
    )
    .unwrap();
    let first_done = execute(
        &request,
        &DeployCommandOptions {
            authorize: first_plan.json["planDigest"].as_str(),
        },
        &mut first_store,
        &mut provider,
        2,
    )
    .unwrap();
    assert_eq!(first_done.json["record"]["run"]["state"], "complete");
    let first_bytes = fs::read(first.remote().join("deploy.json")).unwrap();
    let live = provider.rules.clone().unwrap();
    let effects = provider.effects.clone();
    let second = Root::new();
    let second_layout = second.layout();
    let mut second_store = DeployRecordStore::open(&second_layout).unwrap();
    let second_request = request_for_foreign(&extensions, &body, &live);
    let takeover = execute(
        &second_request,
        &DeployCommandOptions::default(),
        &mut second_store,
        &mut provider,
        3,
    )
    .unwrap();
    assert_ne!(
        first_plan.json["record"]["deploymentId"],
        takeover.json["record"]["deploymentId"]
    );
    assert_eq!(takeover.json["plan"]["rules"]["replaces"], "foreign");
    assert_eq!(
        takeover.json["plan"]["rules"]["replacedDigest"]
            .as_str()
            .unwrap()
            .len(),
        64
    );
    assert!(
        takeover
            .human
            .contains("DESTRUCTIVE: Replace the live Rules")
    );
    assert!(takeover.human.contains("This affects every tenant"));
    assert_eq!(provider.effects, effects);
    assert!(matches!(
        execute(
            &second_request,
            &DeployCommandOptions {
                authorize: first_plan.json["planDigest"].as_str()
            },
            &mut second_store,
            &mut provider,
            4
        ),
        Err(DeployCommandError::Refused(
            DeployRefusal::AuthorizationStale
        ))
    ));
    assert_eq!(provider.effects, effects);
    let authorized = execute(
        &second_request,
        &DeployCommandOptions {
            authorize: takeover.json["planDigest"].as_str(),
        },
        &mut second_store,
        &mut provider,
        5,
    )
    .unwrap();
    assert_eq!(authorized.json["record"]["run"]["state"], "complete");
    assert_eq!(provider.effects_of("rules"), 2);
    assert_eq!(
        fs::read(first.remote().join("deploy.json")).unwrap(),
        first_bytes
    );
    assert_ne!(provider.rules.as_ref().unwrap(), &live);
}

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
    let mut store =
        DeployRecordStore::open(&layout, "demo-remote-1", "retry deploy fixture").unwrap();
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
    let bytes = std::fs::read(root.remote().join("deployments/demo-remote-1/deploy.json")).unwrap();
    let mut store =
        DeployRecordStore::open(&layout, "demo-remote-1", "retry deploy fixture").unwrap();
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
        std::fs::read(root.remote().join("deployments/demo-remote-1/deploy.json")).unwrap(),
        bytes
    );
}
#[test]
fn first_plan_creates_a_stable_local_draft_but_refused_inputs_create_no_record() {
    let root = Root::new();
    let layout = root.layout();
    let mut store =
        DeployRecordStore::open(&layout, "demo-remote-1", "retry deploy fixture").unwrap();
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
    let mut store =
        DeployRecordStore::open(&layout, "demo-remote-1", "retry deploy fixture").unwrap();
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
    let mut store =
        DeployRecordStore::open(&layout, "demo-remote-1", "retry deploy fixture").unwrap();
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
    assert_eq!(
        preview.json["plan"]["destructive"],
        serde_json::json!([format!("replaces-rules:{replaced}")])
    );
    assert!(!preview.human.contains(replaced));
    assert!(
        !preview
            .human
            .contains("Destructive change: replaces-rules:")
    );
    assert!(preview.human.contains("(replaces the live Rules)"));
    assert!(preview.human.contains(&format!(
        "To deploy this plan and replace the live Rules, run the same command with --authorize {}",
        &preview.json["planDigest"].as_str().unwrap()[..12]
    )));

    assert!(
        preview
            .human
            .contains(&format!("Existing Rules fingerprint: {}", &replaced[..12]))
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
#[test]
fn interactive_foreign_rules_warning_names_y_while_plain_output_keeps_digest() {
    let root = Root::new();
    let layout = root.layout();
    let mut store =
        DeployRecordStore::open(&layout, "demo-remote-1", "retry deploy fixture").unwrap();
    store.persist(&DeployRecord::new(ID)).unwrap();
    let extensions = plan();
    let body = fixture("deploy_run", "rules-body.rules");
    let foreign = fixture("deploy_run", "foreign.rules");
    let request = request_for_foreign(&extensions, &body, &foreign);
    let mut port = Fake::new(ACCOUNT);
    let prepared =
        tmt_remote::deploy_command::prepare_with_hosting(&request, &mut store, &mut port, None)
            .unwrap();
    let interactive = prepared.preview(&request, &mut store, None).unwrap();
    assert!(interactive.human.contains("Answering y replaces them."));
    assert!(!interactive.human.contains("Authorizing plan"));
    let plain = execute(
        &request,
        &DeployCommandOptions::default(),
        &mut store,
        &mut port,
        1,
    )
    .unwrap();
    assert_eq!(interactive.json, plain.json);
    assert_eq!(
        plain.human.as_bytes(),
        fixture("deploy_command", "foreign-plan-human.txt")
    );
    assert!(!plain.human.contains("Answering y replaces them."));
    assert!(port.effects.is_empty());
    port.rules = Some(foreign.clone());
    let authorized = execute(
        &request,
        &DeployCommandOptions {
            authorize: plain.json["planDigest"].as_str(),
        },
        &mut store,
        &mut port,
        2,
    )
    .unwrap();
    assert!(authorized.human.contains(&format!(
        "Authorizing plan {} allows this replacement.",
        &plain.json["planDigest"].as_str().unwrap()[..12],
    )));
    assert!(!authorized.human.contains("Answering y replaces them."));
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
struct DeployFailingSink<'a> {
    owner: &'a mut DeployRecordStore,
    saves: usize,
    fail_at: usize,
}
impl DeploySink for DeployFailingSink<'_> {
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
        let mut store =
            DeployRecordStore::open(&layout, "demo-remote-1", "retry deploy fixture").unwrap();
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
        let mut store =
            DeployRecordStore::open(&layout, "demo-remote-1", "retry deploy fixture").unwrap();
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
    let mut store =
        DeployRecordStore::open(&layout, "demo-remote-1", "retry deploy fixture").unwrap();
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
    let mut store =
        DeployRecordStore::open(&layout, "demo-remote-1", "retry deploy fixture").unwrap();
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
            let mut store =
                DeployRecordStore::open(&layout, "demo-remote-1", "retry deploy fixture").unwrap();
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
            let mut store =
                DeployRecordStore::open(&layout, "demo-remote-1", "retry deploy fixture").unwrap();
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
    let mut store =
        DeployRecordStore::open(&layout, "demo-remote-1", "retry deploy fixture").unwrap();
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
    let mut opened =
        std::fs::File::open(root.remote().join("deployments/demo-remote-1/deploy.json")).unwrap();
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
    let mut store =
        DeployRecordStore::open(&layout, "demo-remote-1", "retry deploy fixture").unwrap();
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
    let mut store =
        DeployRecordStore::open(&layout, "demo-remote-1", "retry deploy fixture").unwrap();
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
    use serde_json::{Value, json};
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
    struct DeployCliProvider<const HOSTING: bool = false>(
        deploy_port::Fake,
        usize,
        Option<tmt_remote::hosting::HostingInventory>,
        Option<bool>,
    );
    impl<const HOSTING: bool> DeployPort for &mut DeployCliProvider<HOSTING> {
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
    impl<const HOSTING: bool> FirestoreCommandPort for &mut DeployCliProvider<HOSTING> {
        const SUPPORTS_HOSTING: bool = HOSTING;
        fn hosting_inventory(
            &mut self,
            _: &str,
            _: &str,
        ) -> Result<tmt_remote::hosting::HostingInventory, deploy_run::DeployRefusal> {
            self.2
                .clone()
                .ok_or(deploy_run::DeployRefusal::HostingUnavailable)
        }
        fn login(&mut self) -> Result<(), tmt_remote::deploy_firestore::DeploySetupError> {
            self.1 += 1;
            Ok(())
        }
        fn check_index_budget(
            &mut self,
            _: &str,
            _: &tmt_remote::deploy_plan::Plan,
        ) -> Result<Option<bool>, tmt_remote::deploy_firestore::DeployPreflightError> {
            self.1 += 1;
            Ok(self.3)
        }
        fn live_rules(
            &mut self,
            _: &str,
        ) -> Result<Option<Vec<u8>>, tmt_remote::deploy_firestore::DeployPreflightError> {
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
            "--defaults",
        ];
        words.extend_from_slice(extra);
        let matches = deploy_cli::command()
            .try_get_matches_from(words)
            .map_err(|_| DeployCliError::Usage)?;
        deploy_cli::arguments(&matches)
    }
    #[test]
    fn interactive_preview_is_short_and_keeps_unknown_targets() {
        let root = Root::new();
        let layout = root.layout();
        let mut source = DeployCliSource(0, true);
        let mut provider = DeployCliProvider::<false>(Fake::new(ACCOUNT), 0, None, None);
        let mut asked = false;
        let mut no = |out: &tmt_remote::deploy_command::DeployCommandOutput| {
            asked = true;
            assert_eq!(
                out.human,
                "Deploy to Firebase\n  Account         owner@example.test\n  Project         demo-remote-1 (asia-east1)\n\n  Database        asia-east1\n  Security rules  will be created\n  Sign-in         anonymous\n  Indexes         3 needed\n\nFull plan: run the same command with --details.\n"
            );
            assert!(!out.json["planDigest"].as_str().unwrap().is_empty());
            Ok(false)
        };
        let out = deploy_cli::execute_with_confirmation(
            &args(&[]).unwrap(),
            &mut source,
            &["colab"],
            || Ok(&mut provider),
            || Ok(layout.clone()),
            || Ok(1),
            Some(&mut no),
        )
        .unwrap();
        assert!(asked, "unknown targets cannot suppress confirmation");
        assert_eq!(out.json["declined"], true);
        assert!(provider.0.effects.is_empty());
    }
    #[test]
    fn summary_uses_captured_rules_and_database_evidence_but_still_asks() {
        let root = Root::new();
        let layout = root.layout();
        let mut store =
            DeployRecordStore::open(&layout, "demo-remote-1", "retry deploy fixture").unwrap();
        store.persist(&DeployRecord::new(ID)).unwrap();
        drop(store);
        let mut source = DeployCliSource(0, true);
        let discovered = tmt_remote::deploy_discovery::discover(&mut source, &["colab"]).unwrap();
        let expected = deploy_run::prepare(
            &discovered.extensions,
            &DeployInput {
                account: ACCOUNT,
                project: "demo-remote-1",
                deployment_id: ID,
                location: "asia-east1",
                sign_in: &[SignInProvider::Anonymous],
                rules_body: discovered.artifacts.rules.as_bytes(),
                live_rules: None,
            },
        )
        .unwrap();
        for (live, database, rules_line) in [
            (None, true, "will be created"),
            (Some(expected.deployed_rules().to_vec()), true, "no change"),
            (
                Some(b"foreign rules".to_vec()),
                false,
                "will replace rules set outside TMT",
            ),
        ] {
            for details in [false, true] {
                let mut provider =
                    DeployCliProvider::<false>(Fake::new(ACCOUNT), 0, None, Some(database));
                provider.0.rules = live.clone();
                let mut asked = false;
                let mut no = |out: &tmt_remote::deploy_command::DeployCommandOutput| {
                    asked = true;
                    if details {
                        assert!(out.human.starts_with("Firestore sharing plan\n"));
                        assert!(out.human.contains("Plan digest:"));
                        assert!(out.human.contains("Extensions and resources:"));
                        assert!(
                            out.human
                                .contains(&format!("  Security rules  {rules_line}\n")),
                            "{}",
                            out.human
                        );
                        assert!(
                            !out.human.contains("Security rules:"),
                            "the interactive delta uses the shared states"
                        );
                    } else {
                        assert!(
                            out.human
                                .contains(&format!("  Security rules  {rules_line}\n"))
                        );
                        assert!(out.human.contains(if database {
                            "Database        no change\n"
                        } else {
                            "Database        will be created\n"
                        }));
                        assert!(!out.human.contains("Plan digest:"));
                        assert!(!out.human.contains("Extensions and resources:"));
                        assert!(!out.human.contains("Nothing to deploy"));
                        if !database {
                            assert!(out.human.contains(
                                "will replace rules set outside TMT\n                  Answering y replaces them.\n"
                            ));
                        }
                    }
                    assert_eq!(out.json["authorized"], false);
                    Ok(false)
                };
                deploy_cli::execute_with_confirmation(
                    &args(if details { &["--details"] } else { &[] }).unwrap(),
                    &mut source,
                    &["colab"],
                    || Ok(&mut provider),
                    || Ok(layout.clone()),
                    || Ok(1),
                    Some(&mut no),
                )
                .unwrap();
                assert!(asked, "sign-in/index targets still need consent");
                assert!(provider.0.effects.is_empty());
                assert_eq!(
                    provider.1, 4,
                    "one login, budget, Rules and account snapshot"
                );
            }
        }
    }
    #[test]
    fn details_preserves_json_and_explicit_authorization_output() {
        let run = |extra: &[&str]| {
            let root = Root::new();
            let layout = root.layout();
            let mut store =
                DeployRecordStore::open(&layout, "demo-remote-1", "retry deploy fixture").unwrap();
            store.persist(&DeployRecord::new(ID)).unwrap();
            drop(store);
            let mut source = DeployCliSource(0, true);
            let mut provider = DeployCliProvider::<false>(Fake::new(ACCOUNT), 0, None, None);
            let out = deploy_cli::execute(
                &args(extra).expect("--details is a presentation option"),
                &mut source,
                &["colab"],
                || Ok(&mut provider),
                || Ok(layout.clone()),
                || Ok(1),
            )
            .unwrap();
            assert_eq!(source.0, 1);
            (out, provider.0.effects)
        };
        let (plain, effects) = run(&["--json"]);
        assert!(effects.is_empty());
        let (details, effects) = run(&["--json", "--details"]);
        assert!(effects.is_empty());
        assert_eq!(details.json, plain.json);
        assert_eq!(details.human, plain.human);

        let digest = plain.json["planDigest"].as_str().unwrap();
        let (plain, effects) = run(&["--authorize", digest]);
        assert!(!effects.is_empty());
        let (details, detailed_effects) = run(&["--authorize", digest, "--details"]);
        assert_eq!(detailed_effects, effects);
        assert_eq!(details.json, plain.json);
        assert_eq!(details.human, plain.human);
    }
    #[test]
    fn website_notice_uses_the_frozen_replacement_view_even_for_identical_foreign_files() {
        use base64::{Engine, engine::general_purpose::STANDARD};
        use tmt_remote::hosting::{self, HostingInventory, HostingLiveRelease, HostingManifest};
        struct Source {
            envelope: Vec<u8>,
            bundle: Vec<u8>,
        }
        impl DeclarationSource for Source {
            fn declaration(&mut self, _: &str) -> Result<Option<Vec<u8>>, DiscoveryRefusal> {
                Ok(Some(self.envelope.clone()))
            }
            fn hosting_bundle(&mut self, _: &str) -> Result<Vec<u8>, DiscoveryRefusal> {
                Ok(self.bundle.clone())
            }
        }
        let mut envelope: serde_json::Value = serde_json::from_slice(
            &DeployCliSource(0, true)
                .declaration("colab")
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        let mut declaration: serde_json::Value =
            serde_json::from_str(envelope["declaration"].as_str().unwrap()).unwrap();
        let hash = |bytes: &[u8]| {
            Sha256::digest(bytes)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        };
        let manifest = HostingManifest::parse(&json!({"version":1,"files":[{"path":"/index.html","sha256":hash(b"hello"),"length":5,"contentType":"text/html"}]})).unwrap();
        declaration["hosting"] = serde_json::to_value(&manifest).unwrap();
        let bytes = serde_json::to_vec(&declaration).unwrap();
        envelope["declaration"] = json!(String::from_utf8(bytes.clone()).unwrap());
        envelope["declarationDigest"] = json!(hash(&bytes));
        let mut source = Source {
            envelope: serde_json::to_vec(&envelope).unwrap(),
            bundle: serde_json::to_vec(&json!({"version":1,"manifestDigest":manifest.digest(),"files":[{"path":"/index.html","bytesBase64":STANDARD.encode(b"hello")}]})).unwrap(),
        };
        let content = tmt_remote::deploy_discovery::discover(&mut source, &["colab"])
            .unwrap()
            .hosting
            .unwrap();
        for (kind, details) in ["foreign", "own", "absent", "empty"]
            .into_iter()
            .flat_map(|kind| [(kind, false), (kind, true)])
        {
            let root = Root::new();
            let layout = root.layout();
            let mut store =
                DeployRecordStore::open(&layout, "demo-remote-1", "retry deploy fixture").unwrap();
            store.persist(&DeployRecord::new(ID)).unwrap();
            drop(store);
            let inventory = HostingInventory {
                site_exists: kind != "absent",
                site_app_id: None,
                site_app_config: None,
                web_apps: vec![],
                live: matches!(kind, "foreign" | "own").then(|| HostingLiveRelease {
                    version: "version-1".into(),
                    deployment_id: (kind == "own").then(|| ID.into()),
                    plan_digest_prefix: (kind == "own").then(|| "a".repeat(12)),
                    content_digest: (kind == "own").then(|| content.live_digest()[..32].into()),
                    files: content.live_files(),
                    config: content.config().clone(),
                }),
            };
            let view = hosting::deployment_view(
                &content,
                "demo-remote-1",
                ID,
                &inventory,
                "demo-remote-1",
                None,
                true,
            )
            .unwrap();
            assert_eq!(
                view.replaces,
                if kind == "foreign" { "foreign" } else { "none" }
            );
            let mut provider =
                DeployCliProvider::<true>(Fake::new(ACCOUNT), 0, Some(inventory), None);
            let mut asked = false;
            let mut no = |out: &tmt_remote::deploy_command::DeployCommandOutput| {
                asked = true;
                if kind == "absent" {
                    assert_ne!(out.json["plan"]["hosting"]["site"], "demo-remote-1");
                    assert_eq!(
                        out.json["plan"]["hosting"]["publicUrl"],
                        format!(
                            "https://{}.web.app",
                            out.json["plan"]["hosting"]["site"].as_str().unwrap()
                        )
                    );
                } else {
                    assert_eq!(out.json["plan"]["hosting"]["site"], "demo-remote-1");
                }
                assert_eq!(out.json["plan"]["hosting"]["replaces"], view.replaces);
                assert_eq!(
                    out.human.matches("Register this machine as owner").count(),
                    1,
                    "both short and full interactive previews disclose owner access exactly once"
                );
                assert!(
                    out.json["plan"]["steps"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|id| id == "owner:register")
                );
                assert!(
                    out.human.contains(match kind {
                        "own" => "Website files   no change",
                        "foreign" => "Website files   will replace the live website\n                  Answering y replaces it.",
                        _ => "Website files   will be published for the first time",
                    }),
                    "{kind}: {}",
                    out.human
                );
                Ok(false)
            };
            let mut selected = args(if details { &["--details"] } else { &[] }).unwrap();
            if kind != "absent" {
                selected.site = Some("demo-remote-1".into());
            }
            deploy_cli::execute_with_confirmation(
                &selected,
                &mut source,
                &["colab"],
                || Ok(&mut provider),
                || Ok(layout.clone()),
                || Ok(1),
                Some(&mut no),
            )
            .unwrap();
            assert!(asked);
            assert!(provider.0.effects.is_empty());
        }
    }
    #[test]
    fn interactive_yes_executes_only_the_displayed_snapshot_once() {
        let root = Root::new();
        let layout = root.layout();
        let mut source = DeployCliSource(0, true);
        let mut provider = DeployCliProvider::<false>(Fake::new(ACCOUNT), 0, None, None);
        let mut shown = None;
        let mut prompt = |out: &tmt_remote::deploy_command::DeployCommandOutput| {
            assert!(shown.is_none());
            shown = Some(out.json["planDigest"].as_str().unwrap().to_string());
            Ok(true)
        };
        let out = deploy_cli::execute_with_confirmation(
            &args(&[]).unwrap(),
            &mut source,
            &["colab"],
            || Ok(&mut provider),
            || Ok(layout.clone()),
            || Ok(1),
            Some(&mut prompt),
        )
        .unwrap();
        assert_eq!(out.json["authorized"], true);
        assert_eq!(out.json["planDigest"].as_str(), shown.as_deref());
        assert_eq!(
            out.json["record"]["run"]["planDigest"],
            out.json["planDigest"]
        );
        assert_eq!(source.0, 1, "no rediscovery between display and answer");
        assert_eq!(provider.0.effects_of("rules"), 1);
    }
    #[test]
    fn interactive_decline_has_no_effect_and_plain_or_json_never_asks() {
        let root = Root::new();
        let layout = root.layout();
        let mut source = DeployCliSource(0, true);
        let mut provider = DeployCliProvider::<false>(Fake::new(ACCOUNT), 0, None, None);
        let mut asked = 0;
        let mut no = |_: &tmt_remote::deploy_command::DeployCommandOutput| {
            asked += 1;
            Ok(false)
        };
        let out = deploy_cli::execute_with_confirmation(
            &args(&[]).unwrap(),
            &mut source,
            &["colab"],
            || Ok(&mut provider),
            || Ok(layout.clone()),
            || Ok(1),
            Some(&mut no),
        )
        .unwrap();
        assert_eq!(asked, 1);
        assert!(
            out.human
                .ends_with("Not deployed. Nothing in your Firebase project changed.\n")
        );
        assert!(provider.0.effects.is_empty());
        let mut json_args = args(&[]).unwrap();
        json_args.json = true;
        let mut never =
            |_: &tmt_remote::deploy_command::DeployCommandOutput| -> Result<bool, DeployCliError> {
                panic!("JSON cannot prompt")
            };
        let out = deploy_cli::execute_with_confirmation(
            &json_args,
            &mut source,
            &["colab"],
            || Ok(&mut provider),
            || Ok(layout.clone()),
            || Ok(1),
            Some(&mut never),
        )
        .unwrap();
        assert_eq!(out.json["authorized"], false);
        assert!(provider.0.effects.is_empty());
    }
    #[test]
    fn same_digest_partial_run_continues_without_another_question() {
        let root = Root::new();
        let layout = root.layout();
        let mut source = DeployCliSource(0, true);
        let mut provider = DeployCliProvider::<false>(Fake::new(ACCOUNT), 0, None, None);
        provider.0.owner_pending.insert(
            "sign-in:anonymous".into(),
            deploy_run::DeployOwnerAction::InitializeAuth,
        );
        let mut yes = |_: &tmt_remote::deploy_command::DeployCommandOutput| Ok(true);
        let partial = deploy_cli::execute_with_confirmation(
            &args(&[]).unwrap(),
            &mut source,
            &["colab"],
            || Ok(&mut provider),
            || Ok(layout.clone()),
            || Ok(1),
            Some(&mut yes),
        )
        .unwrap();
        assert_eq!(partial.json["record"]["run"]["state"], "partial");
        provider.0.owner_pending.clear();
        let mut never =
            |_: &tmt_remote::deploy_command::DeployCommandOutput| -> Result<bool, DeployCliError> {
                panic!("already authorized")
            };
        let completed = deploy_cli::execute_with_confirmation(
            &args(&[]).unwrap(),
            &mut source,
            &["colab"],
            || Ok(&mut provider),
            || Ok(layout.clone()),
            || Ok(2),
            Some(&mut never),
        )
        .unwrap();
        assert_eq!(partial.json["planDigest"], completed.json["planDigest"]);
        assert_eq!(completed.json["record"]["run"]["state"], "complete");
        assert!(
            completed
                .human
                .starts_with("Continuing the deploy you already approved.\n")
        );
    }
    #[test]
    fn plain_same_digest_preview_keeps_unknown_effect_guidance() {
        let root = Root::new();
        let layout = root.layout();
        let mut source = DeployCliSource(0, true);
        let mut provider = DeployCliProvider::<false>(Fake::new(ACCOUNT), 0, None, None);
        provider
            .0
            .faults
            .insert("rules".into(), When::AfterEffectUnknown);
        let mut yes = |_: &tmt_remote::deploy_command::DeployCommandOutput| Ok(true);
        let partial = deploy_cli::execute_with_confirmation(
            &args(&[]).unwrap(),
            &mut source,
            &["colab"],
            || Ok(&mut provider),
            || Ok(layout.clone()),
            || Ok(1),
            Some(&mut yes),
        )
        .unwrap();
        let effects = provider.0.effects.clone();
        let before = deploy_record::read(&layout).unwrap();
        let preview = deploy_cli::execute(
            &args(&[]).unwrap(),
            &mut source,
            &["colab"],
            || Ok(&mut provider),
            || Ok(layout.clone()),
            || Ok(2),
        )
        .unwrap();
        assert_eq!(preview.json["planDigest"], partial.json["planDigest"]);
        assert_eq!(preview.json["authorized"], false);
        assert!(
            preview.human.contains(
                "Deployment could not be confirmed; some changes may already be applied."
            )
        );
        assert_eq!(deploy_record::read(&layout).unwrap(), before);
        assert_eq!(provider.0.effects, effects);
    }
    #[test]
    fn completed_same_digest_asks_before_a_fresh_check() {
        let root = Root::new();
        let layout = root.layout();
        let mut source = DeployCliSource(0, true);
        let mut provider = DeployCliProvider::<false>(Fake::new(ACCOUNT), 0, None, None);
        let mut yes = |_: &tmt_remote::deploy_command::DeployCommandOutput| Ok(true);
        let completed = deploy_cli::execute_with_confirmation(
            &args(&[]).unwrap(),
            &mut source,
            &["colab"],
            || Ok(&mut provider),
            || Ok(layout.clone()),
            || Ok(1),
            Some(&mut yes),
        )
        .unwrap();
        assert_eq!(completed.json["record"]["run"]["state"], "complete");
        let before = deploy_record::read(&layout).unwrap();
        let effects = provider.0.effects.clone();
        let mut asked = false;
        let mut no = |_: &tmt_remote::deploy_command::DeployCommandOutput| {
            asked = true;
            Ok(false)
        };
        let declined = deploy_cli::execute_with_confirmation(
            &args(&[]).unwrap(),
            &mut source,
            &["colab"],
            || Ok(&mut provider),
            || Ok(layout.clone()),
            || Ok(2),
            Some(&mut no),
        )
        .unwrap();
        assert!(
            asked,
            "a completed run is a fresh check, not a continuation"
        );
        assert_eq!(
            declined.human,
            "Not deployed. Nothing in your Firebase project changed.\n"
        );
        assert_eq!(deploy_record::read(&layout).unwrap(), before);
        assert_eq!(provider.0.effects, effects);
    }
    #[test]
    fn interrupted_confirmation_saves_only_a_resumable_draft() {
        let root = Root::new();
        let layout = root.layout();
        let mut source = DeployCliSource(0, true);
        let mut provider = DeployCliProvider::<false>(Fake::new(ACCOUNT), 0, None, None);
        let mut interrupted =
            |_: &tmt_remote::deploy_command::DeployCommandOutput| -> Result<bool, DeployCliError> {
                Err(DeployCliError::Local(
                    std::io::Error::from(std::io::ErrorKind::Interrupted).into(),
                ))
            };
        assert!(
            deploy_cli::execute_with_confirmation(
                &args(&[]).unwrap(),
                &mut source,
                &["colab"],
                || Ok(&mut provider),
                || Ok(layout.clone()),
                || Ok(1),
                Some(&mut interrupted)
            )
            .is_err()
        );
        assert!(provider.0.effects.is_empty());
        assert!(deploy_record::read(&layout).unwrap().unwrap().run.is_none());
    }
    #[test]
    fn declining_changed_work_does_not_relabel_an_earlier_unknown_effect() {
        let root = Root::new();
        let layout = root.layout();
        let mut source = DeployCliSource(0, true);
        let mut provider = DeployCliProvider::<false>(Fake::new(ACCOUNT), 0, None, None);
        provider
            .0
            .faults
            .insert("rules".into(), When::AfterEffectUnknown);
        let mut yes = |_: &tmt_remote::deploy_command::DeployCommandOutput| Ok(true);
        let old = deploy_cli::execute_with_confirmation(
            &args(&[]).unwrap(),
            &mut source,
            &["colab"],
            || Ok(&mut provider),
            || Ok(layout.clone()),
            || Ok(1),
            Some(&mut yes),
        )
        .unwrap();
        assert_eq!(old.json["record"]["run"]["state"], "partial");
        let before = deploy_record::read(&layout).unwrap();
        let effects = provider.0.effects.clone();
        let mut changed = args(&[]).unwrap();
        changed.sign_in.push(SignInProvider::Google);
        let mut constructed = false;
        let conflict = deploy_cli::execute(
            &changed,
            &mut source,
            &["colab"],
            || {
                constructed = true;
                Ok(&mut provider)
            },
            || Ok(layout.clone()),
            || Ok(2),
        );
        assert!(
            matches!(conflict, Err(DeployCliError::Local(error)) if error.code == "REMOTE_DEPLOY_PROJECT_CONFLICT")
        );
        assert!(!constructed);
        assert_eq!(deploy_record::read(&layout).unwrap(), before);
        // Resource selections stay frozen. An extension declaration can still
        // change the next plan, and declining it must preserve the earlier outcome.
        struct ChangedDeclaration<'a>(&'a mut DeployCliSource);
        impl DeclarationSource for ChangedDeclaration<'_> {
            fn declaration(
                &mut self,
                extension: &str,
            ) -> Result<Option<Vec<u8>>, DiscoveryRefusal> {
                let bytes = self.0.declaration(extension)?.unwrap();
                let mut envelope: Value = serde_json::from_slice(&bytes).unwrap();
                let mut declaration: Value =
                    serde_json::from_str(envelope["declaration"].as_str().unwrap()).unwrap();
                declaration["resources"][0]["limits"]["maxEntries"] = json!(1023);
                let declaration = serde_json::to_string(&declaration).unwrap();
                let digest: String = Sha256::digest(declaration.as_bytes())
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect();
                envelope["declaration"] = json!(declaration);
                envelope["declarationDigest"] = json!(digest);
                Ok(Some(serde_json::to_vec(&envelope).unwrap()))
            }
        }
        let changed = args(&[]).unwrap();
        let mut source = ChangedDeclaration(&mut source);
        let mut no = |_: &tmt_remote::deploy_command::DeployCommandOutput| Ok(false);
        let output = deploy_cli::execute_with_confirmation(
            &changed,
            &mut source,
            &["colab"],
            || Ok(&mut provider),
            || Ok(layout.clone()),
            || Ok(2),
            Some(&mut no),
        )
        .unwrap();
        assert_eq!(
            output.human,
            "Not deployed. Nothing in your Firebase project changed.\n"
        );
        assert_eq!(deploy_record::read(&layout).unwrap(), before);
        assert_eq!(provider.0.effects, effects);
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
        let missing = deploy_cli::command()
            .try_get_matches_from([
                "firestore",
                "--project",
                "demo-remote-1",
                "--region",
                "asia-east1",
            ])
            .unwrap();
        let missing = deploy_cli::arguments(&missing).unwrap();
        assert!(matches!(
            deploy_cli::inputs::resolve(&missing, None, &Default::default(), &[]),
            Err(DeployCliError::Local(error)) if error.code == "REMOTE_DEPLOY_INPUT_REQUIRED"
        ));
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
        let provider =
            DeployCliProvider::<false>(deploy_port::Fake::new("owner@example.test"), 0, None, None);
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
    fn a_known_owner_action_is_not_described_as_an_unknown_effect() {
        let root = Root::new();
        let layout = root.layout();
        let mut store =
            DeployRecordStore::open(&layout, "demo-remote-1", "retry deploy fixture").unwrap();
        store.persist(&DeployRecord::new(ID)).unwrap();
        drop(store);
        let mut source = DeployCliSource(0, true);
        let mut provider =
            DeployCliProvider::<false>(deploy_port::Fake::new("owner@example.test"), 0, None, None);
        let preview = deploy_cli::execute(
            &args(&[]).unwrap(),
            &mut source,
            &["colab"],
            || Ok(&mut provider),
            || Ok(root.layout()),
            || Ok(1),
        )
        .unwrap();
        let digest = preview.json["planDigest"].as_str().unwrap();
        provider.0.owner_pending.insert(
            "sign-in:anonymous".into(),
            tmt_remote::deploy_run::DeployOwnerAction::InitializeAuth,
        );
        let output = deploy_cli::execute(
            &args(&["--authorize", digest]).unwrap(),
            &mut source,
            &["colab"],
            || Ok(&mut provider),
            || Ok(root.layout()),
            || Ok(2),
        )
        .unwrap();
        assert_eq!(output.json["record"]["run"]["state"], "partial");
        assert!(output.human.contains("REMOTE_DEPLOY_OWNER_INITIALIZE_AUTH"));
        assert!(!output.human.contains("Deployment could not be confirmed"));
        provider.0.owner_pending.clear();
        provider
            .0
            .present
            .insert("index:x/colab/log#sequence:asc".into());
        provider
            .0
            .building
            .insert("index:x/colab/log#sequence:asc".into());
        let building = deploy_cli::execute(
            &args(&["--authorize", digest]).unwrap(),
            &mut source,
            &["colab"],
            || Ok(&mut provider),
            || Ok(root.layout()),
            || Ok(3),
        )
        .unwrap();
        assert_eq!(building.json["record"]["run"]["state"], "partial");
        assert!(building.human.contains("verify: building"));
        assert!(!building.human.contains("Deployment could not be confirmed"));
    }

    #[test]
    fn discovered_plan_authorizes_whole_envelope_and_recovers_without_resending() {
        let root = Root::new();
        let layout = root.layout();
        let mut store =
            DeployRecordStore::open(&layout, "demo-remote-1", "retry deploy fixture").unwrap();
        store.persist(&DeployRecord::new(ID)).unwrap();
        drop(store);
        let mut source = DeployCliSource(0, true);
        let mut provider =
            DeployCliProvider::<false>(deploy_port::Fake::new("owner@example.test"), 0, None, None);
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
            layout.directory.join("deployments/demo-remote-1/deploy.json").display()
        )));
        assert!(
            !partial
                .human
                .contains("Nothing changed in your Firebase project.")
        );
        let before = fs::read(root.remote().join("deployments/demo-remote-1/deploy.json")).unwrap();
        let calls = provider.1;
        let mut conflicting = args(&[]).unwrap();
        conflicting.region = "us-central1".into();
        assert!(matches!(deploy_cli::execute(
            &conflicting, &mut source, &["colab"],
            || -> Result<&mut DeployCliProvider, DeployCliError> { panic!("partial target must fence provider setup") },
            || Ok(root.layout()), || Ok(3),
        ), Err(DeployCliError::Local(e)) if e.code == "REMOTE_DEPLOY_PROJECT_CONFLICT"));
        assert_eq!(
            fs::read(root.remote().join("deployments/demo-remote-1/deploy.json")).unwrap(),
            before
        );
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
        let mut provider =
            DeployCliProvider::<false>(deploy_port::Fake::new(ACCOUNT), 0, None, None);
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
        let path = root.remote().join("deployments/demo-remote-1/deploy.json");
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
        assert_ne!(
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
        let bound: serde_json::Value = serde_json::from_slice(
            &fs::read(root.remote().join("deployments/demo-remote-2/deploy.json")).unwrap(),
        )
        .unwrap();
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
        let mut provider =
            DeployCliProvider::<false>(deploy_port::Fake::new(ACCOUNT), 0, None, None);
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
        let path = root.remote().join("deployments/demo-remote-1/deploy.json");
        let before = fs::read(&path).unwrap();
        for region in ["us-central1", "europe-west1"] {
            let mut changed = args(&[]).unwrap();
            changed.region = region.into();
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
    #[test]
    fn unsupported_hosting_refuses_before_lazy_provider_login_record_and_clock() {
        use base64::{Engine, engine::general_purpose::STANDARD};
        struct HostedSource;
        impl DeclarationSource for HostedSource {
            fn declaration(&mut self, _: &str) -> Result<Option<Vec<u8>>, DiscoveryRefusal> {
                let mut source = DeployCliSource(0, true);
                let mut reply: serde_json::Value =
                    serde_json::from_slice(&source.declaration("colab")?.unwrap()).unwrap();
                let mut declaration: serde_json::Value =
                    serde_json::from_str(reply["declaration"].as_str().unwrap()).unwrap();
                let hash = |b: &[u8]| {
                    Sha256::digest(b)
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect::<String>()
                };
                declaration["hosting"] = json!({"version":1,"files":[{"path":"/index.html","sha256":hash(b"hello"),"length":5,"contentType":"text/html"}]});
                let bytes = serde_json::to_string(&declaration).unwrap();
                reply["declarationDigest"] = json!(hash(bytes.as_bytes()));
                reply["declaration"] = json!(bytes);
                Ok(Some(serde_json::to_vec(&reply).unwrap()))
            }
            fn hosting_bundle(&mut self, _: &str) -> Result<Vec<u8>, DiscoveryRefusal> {
                let reply = self.declaration("colab")?.unwrap();
                let parsed: serde_json::Value = serde_json::from_slice(&reply).unwrap();
                let declaration: serde_json::Value =
                    serde_json::from_str(parsed["declaration"].as_str().unwrap()).unwrap();
                let manifest =
                    tmt_remote::hosting::HostingManifest::parse(&declaration["hosting"]).unwrap();
                Ok(serde_json::to_vec(&json!({"version":1,"manifestDigest":manifest.digest(),"files":[{"path":"/index.html","bytesBase64":STANDARD.encode(b"hello")}]})).unwrap())
            }
        }
        let mut source = HostedSource;
        let result = deploy_cli::execute(
            &args(&[]).unwrap(),
            &mut source,
            &["colab"],
            || -> Result<&mut DeployCliProvider, DeployCliError> {
                panic!("unsupported Hosting touched provider/tools/login")
            },
            || -> Result<tmt_remote::state::Layout, DeployCliError> {
                panic!("unsupported Hosting touched record")
            },
            || -> Result<u64, DeployCliError> { panic!("unsupported Hosting touched clock") },
        );
        assert!(matches!(
            result,
            Err(DeployCliError::Command(
                DeployCommandError::Refused(DeployRefusal::HostingUnavailable),
                _
            ))
        ));
    }
}

#[test]
fn a_second_home_sees_the_first_homes_rules_as_foreign_and_must_authorize_its_own_plan() {
    let first = Root::new();
    let first_layout = first.layout();
    let mut first_store =
        DeployRecordStore::open(&first_layout, "demo-remote-1", "retry deploy fixture").unwrap();
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
    let first_bytes =
        fs::read(first.remote().join("deployments/demo-remote-1/deploy.json")).unwrap();
    let live = provider.rules.clone().unwrap();
    let effects = provider.effects.clone();
    let second = Root::new();
    let second_layout = second.layout();
    let mut second_store =
        DeployRecordStore::open(&second_layout, "demo-remote-1", "retry deploy fixture").unwrap();
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
        fs::read(first.remote().join("deployments/demo-remote-1/deploy.json")).unwrap(),
        first_bytes
    );
    assert_ne!(provider.rules.as_ref().unwrap(), &live);
}

#[test]
fn frozen_hosting_plan_covers_content_and_foreign_release_but_cannot_run_rules_alone() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use serde_json::json;
    use sha2::{Digest, Sha256};
    use tmt_remote::hosting::{
        self, HostingBundle, HostingInventory, HostingLiveRelease, HostingManifest,
    };
    let root = Root::new();
    let layout = root.layout();
    let mut store =
        DeployRecordStore::open(&layout, "demo-remote-1", "retry deploy fixture").unwrap();
    store.persist(&DeployRecord::new(ID)).unwrap();
    let mut declaration: serde_json::Value =
        serde_json::from_slice(&fixture("declarations", "colab-firestore.json")).unwrap();
    let artifact = fixture("declarations", "rules/colab-admission.rules");
    let hash = |bytes: &[u8]| {
        Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    };
    let manifest=HostingManifest::parse(&json!({"version":1,"files":[{"path":"/index.html","sha256":hash(b"hello"),"length":5,"contentType":"text/html"}]})).unwrap();
    declaration["hosting"] = serde_json::to_value(&manifest).unwrap();
    let bytes = serde_json::to_vec(&declaration).unwrap();
    let plan = compose(
        Target {
            backend: CloudBackend::Firestore,
            physical_ttl: false,
        },
        &[Enabled {
            name: "colab",
            supplied: Some(Supplied {
                declaration: &bytes,
                artifact: &artifact,
            }),
        }],
    )
    .unwrap();
    let reply = json!({"version":1,"manifestDigest":manifest.digest(),"files":[{"path":"/index.html","bytesBase64":STANDARD.encode(b"hello")}]});
    let content = hosting::compose(&[hosting::HostingInput {
        extension: "colab",
        declaration: &bytes,
        bundle: &HostingBundle::parse(&manifest, &serde_json::to_vec(&reply).unwrap()).unwrap(),
    }])
    .unwrap()
    .unwrap();
    let mut inventory = HostingInventory {
        site_exists: true,
        site_app_id: None,
        site_app_config: None,
        web_apps: vec![tmt_remote::hosting::HostingWebApp {
            id: "1:123:web:abc".into(),
            display_name: format!("tmt Remote ({ID})"),
            state: "ACTIVE".into(),
        }],
        live: Some(HostingLiveRelease {
            version: "version-1".into(),
            deployment_id: None,
            plan_digest_prefix: None,
            content_digest: None,
            files: content.live_files(),
            config: content.config().clone(),
        }),
    };
    let body = fixture("deploy_run", "rules-body.rules");
    let mut request = input(&plan, &body);
    request.live_rules = Some(b"foreign rules");
    let mut port = Fake::new(ACCOUNT);
    let view = hosting::deployment_view(
        &content,
        request.project,
        ID,
        &inventory,
        request.project,
        None,
        true,
    )
    .unwrap();
    let preview = tmt_remote::deploy_command::execute_with_hosting(
        &request,
        &DeployCommandOptions::default(),
        &mut store,
        &mut port,
        1,
        Some(&view),
    )
    .unwrap();
    assert_eq!(
        preview.json["plan"]["steps"]
            .as_array()
            .unwrap()
            .iter()
            .rev()
            .take(4)
            .collect::<Vec<_>>(),
        vec![
            &json!("verify"),
            &json!("hosting:release"),
            &json!("rules"),
            &json!("hosting:stage:finalize")
        ]
    );
    assert_eq!(
        preview.json["plan"]["destructive"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(preview.human.contains("Replace the live Hosting release"));
    assert!(
        preview
            .human
            .contains("Create site: no\nCreate web app: no\n")
    );
    assert!(
        !preview
            .human
            .contains("Hosting publishing is not available")
    );
    assert!(preview.human.contains("To deploy this plan"));
    let prefix = &preview.json["planDigest"].as_str().unwrap()[..12];
    let saved = fs::read(root.remote().join("deployments/demo-remote-1/deploy.json")).unwrap();
    inventory.live.as_mut().unwrap().version = "version-2".into();
    let changed = hosting::deployment_view(
        &content,
        request.project,
        ID,
        &inventory,
        request.project,
        None,
        true,
    )
    .unwrap();
    assert!(matches!(
        tmt_remote::deploy_command::execute_with_hosting(
            &request,
            &DeployCommandOptions {
                authorize: Some(prefix)
            },
            &mut store,
            &mut port,
            2,
            Some(&changed)
        ),
        Err(DeployCommandError::Refused(
            DeployRefusal::AuthorizationStale
        ))
    ));
    assert!(matches!(
        tmt_remote::deploy_command::execute_with_hosting(
            &request,
            &DeployCommandOptions {
                authorize: Some(prefix)
            },
            &mut store,
            &mut port,
            2,
            Some(&view)
        ),
        Err(DeployCommandError::Refused(
            DeployRefusal::HostingUnavailable
        ))
    ));
    assert_eq!(
        fs::read(root.remote().join("deployments/demo-remote-1/deploy.json")).unwrap(),
        saved
    );
    assert!(port.calls.is_empty() && port.effects.is_empty());
    // The engine boundary also refuses this future step set before any save or call.
    let prepared = deploy_run::prepare_with_hosting(
        &plan,
        &DeployInput {
            account: ACCOUNT,
            project: request.project,
            deployment_id: ID,
            location: request.location,
            sign_in: &BOTH,
            rules_body: &body,
            live_rules: request.live_rules,
        },
        Some(&view),
    )
    .unwrap();
    let authorization = deploy_run::authorize(&prepared, prefix).unwrap();
    assert!(matches!(
        deploy_run::run(
            &prepared,
            &authorization,
            DeployRecord::new(ID),
            &mut port,
            &mut store,
            3
        ),
        Err(DeployError::Refused(DeployRefusal::HostingUnavailable))
    ));
    assert_eq!(
        fs::read(root.remote().join("deployments/demo-remote-1/deploy.json")).unwrap(),
        saved
    );
    assert!(port.calls.is_empty() && port.effects.is_empty());
    // A real atomic partial checkpoint from an older bundle must not block preview.
    let mut saved_record = preview.json["record"].clone();
    saved_record["run"] = json!({"planDigest":preview.json["planDigest"],"account":ACCOUNT,"authorizedAtMs":1,"state":"partial","rulesAttempted":false,"steps":preview.json["plan"]["steps"].as_array().unwrap().iter().map(|id|json!({"id":id,"state":"pending"})).collect::<Vec<_>>(),"hosting":{"envelope":preview.json["plan"],"appId":"1:123:web:abc","operation":null,"version":null,"versionCreatedMs":null,"publication":null}});
    let record: DeployRecord = serde_json::from_value(saved_record).unwrap();
    store
        .bind_target(request.project, request.location)
        .unwrap();
    store.persist(&record).unwrap();
    let original = fs::read(root.remote().join("deployments/demo-remote-1/deploy.json")).unwrap();
    let mut new_content = view.clone();
    new_content.content["digest"] = json!(hash(b"updated bundle"));
    let current = tmt_remote::deploy_command::execute_with_hosting(
        &request,
        &DeployCommandOptions::default(),
        &mut store,
        &mut port,
        3,
        Some(&new_content),
    )
    .unwrap();
    assert_ne!(current.json["planDigest"], preview.json["planDigest"]);
    assert!(current.human.contains("unfinished run"));
    assert!(current.human.contains("Nothing has been undone."));
    assert_eq!(current.json["supersededRun"]["safe"], true);
    assert_eq!(
        fs::read(root.remote().join("deployments/demo-remote-1/deploy.json")).unwrap(),
        original
    );
    assert!(port.calls.is_empty() && port.effects.is_empty());
}

#[test]
fn refused_replacement_keeps_typed_recovery_instead_of_the_preview_loop() {
    for reason in [
        DeployRefusal::PreviousCreateUnconfirmed,
        DeployRefusal::PreviousVersionStaged,
    ] {
        let path = std::path::Path::new("/private/fixture/deploy.json");
        for error in [
            DeployCommandError::Refused(reason),
            DeployCommandError::Run(DeployError::Refused(reason)),
        ] {
            let projected = tmt_remote::deploy_cli::command_error(error, path);
            assert_eq!(projected.code, reason.code());
            assert!(!projected.message.contains("Run without --authorize"));
            if reason == DeployRefusal::PreviousCreateUnconfirmed {
                assert_eq!(
                    projected.message,
                    tmt_remote::deploy_command::unconfirmed_creation_message(path)
                );
                assert!(projected.message.contains("tmt will not repeat it"));
                assert!(projected.message.contains("do not delete"));
            } else {
                assert!(
                    projected
                        .message
                        .contains("restore the earlier extension build")
                );
                assert!(projected.message.contains("12 hours"));
            }
        }
    }
}

#[test]
fn only_an_interactive_person_can_implicitly_authorize_and_the_copy_is_pinned() {
    use tmt_remote::deploy_cli::*;
    let terminal = tmt_cli_style::Interaction {
        json: false,
        stdin: true,
        stdout: true,
        stderr: true,
        dumb: false,
    };
    assert!(may_prompt(terminal, false, false));
    assert!(!may_prompt(terminal, true, false));
    assert!(!may_prompt(terminal, false, true));
    assert!(!may_prompt(
        tmt_cli_style::Interaction {
            json: true,
            ..terminal
        },
        false,
        false
    ));
    assert!(!may_prompt(
        tmt_cli_style::Interaction {
            stdin: false,
            ..terminal
        },
        false,
        false
    ));
    assert!(!may_prompt(
        tmt_cli_style::Interaction {
            stderr: false,
            ..terminal
        },
        false,
        false
    ));
    assert_eq!(
        DEPLOY_PROMPT,
        "Deploy these changes to your Firebase project? [y/N]"
    );
    assert_eq!(
        DECLINED,
        "Not deployed. Nothing in your Firebase project changed."
    );
    assert_eq!(CONTINUING, "Continuing the deploy you already approved.");
    assert_eq!(
        REPLACING,
        "An earlier deploy did not finish. This one replaces it. Anything the earlier one already changed stays as it is; nothing is undone."
    );
    assert_eq!(
        KNOWN_VERSION,
        "The earlier deploy left an uploaded website version that is not live. It stays in your project, unchanged."
    );
    assert_eq!(
        UNKNOWN_VERSION,
        "Your project may already hold one uploaded website version that is not live. It stays that way, and this deploy may upload another."
    );
}

#[test]
fn account_changed_after_the_displayed_snapshot_refuses_before_effects() {
    let root = Root::new();
    let layout = root.layout();
    let mut store =
        DeployRecordStore::open(&layout, "demo-remote-1", "retry deploy fixture").unwrap();
    let extensions = plan();
    let body = fixture("deploy_run", "rules-body.rules");
    let input = input(&extensions, &body);
    let mut provider = Fake::new(ACCOUNT);
    let prepared =
        tmt_remote::deploy_command::prepare_with_hosting(&input, &mut store, &mut provider, None)
            .unwrap();
    let preview = prepared.preview(&input, &mut store, None).unwrap();
    let consent = deploy_run::authorize_interactive(prepared.plan(), prepared.record()).unwrap();
    assert_eq!(preview.json["planDigest"], prepared.plan().digest());
    provider.account = "different@example.test".into();
    assert!(matches!(
        tmt_remote::deploy_command::execute_prepared(
            &input,
            prepared,
            Some(consent),
            &mut store,
            &mut provider,
            1,
            None
        ),
        Err(DeployCommandError::Run(DeployError::Refused(
            DeployRefusal::AccountChanged
        )))
    ));
    assert!(provider.calls.is_empty() && provider.effects.is_empty());
    assert!(store.load_or_draft().unwrap().run.is_none());
}

#[test]
fn preview_names_and_digest_binds_the_native_agent_talk_policy() {
    let root = Root::new();
    let layout = root.layout();
    let mut store =
        DeployRecordStore::open(&layout, "demo-remote-1", "retry deploy fixture").unwrap();
    store.persist(&DeployRecord::new(ID)).unwrap();
    let plan = compose(
        Target {
            backend: CloudBackend::Firestore,
            physical_ttl: false,
        },
        &[],
    )
    .unwrap();
    let composed = tmt_remote::rules::compose(&plan, &[]).unwrap();
    let mut port = Fake::new(ACCOUNT);
    let out = execute(
        &input(&plan, composed.rules.as_bytes()),
        &DeployCommandOptions::default(),
        &mut store,
        &mut port,
        7,
    )
    .unwrap();
    assert!(out.human.contains(&format!(
        "Agent talk Rules: /m/{{machine}}; carrier bound {} B; descriptor bound {} B; enrollment bound {} B",
        tmt_remote::limits::CLOUD_CARRIER_BYTES,
        tmt_remote::limits::CLOUD_DESCRIPTOR_BYTES,
        tmt_remote::limits::CLOUD_ENROLLMENT_BYTES
    )));
    assert_eq!(
        out.json["plan"]["rules"]["digest"],
        deploy_run::rules_digest(composed.rules.as_bytes())
    );
    assert!(
        port.effects.is_empty(),
        "preview cannot deploy the added policy"
    );
    let other = execute(
        &input(&plan, b"different Rules"),
        &DeployCommandOptions::default(),
        &mut store,
        &mut port,
        7,
    )
    .unwrap();
    assert_ne!(
        out.json["planDigest"], other.json["planDigest"],
        "authorization binds the composed Rules bytes"
    );
}

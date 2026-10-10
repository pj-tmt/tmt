//! The authorized deploy run (#2164 slice 1). The envelope bytes, digests and deployed
//! Rules come from `fixtures/deploy_run/reference.py`, not from the code under test. The
//! provider is a fake that logs calls; no real account is involved. The property under
//! test: a usable binding exists only after every step of the authorized plan finished.
use std::{fs, path::PathBuf};
use tmt_remote::{
    deploy_plan::{CloudBackend, Enabled, Plan, Supplied, Target, compose},
    deploy_run::{
        DeployBinding, DeployError, DeployFault, DeployInput, DeployOwnerAction, DeployPlan,
        DeployProviderError, DeployRecord, DeployRefusal, LiveRules, RunState, SignInProvider,
        StepState, authorize, prepare, run,
    },
};
#[path = "support/deploy_port.rs"]
mod deploy_port;
use deploy_port::{Fake, Mem, When};

const ACCOUNT: &str = "owner@example.test";
const PROJECT: &str = "demo-remote-1";
const DEPLOYMENT: &str = "3f2b8c1e-5d4a-4e7b-9c1d-2a6f8e0b4c11";
const LOCATION: &str = "asia-east1";
const BOTH: [SignInProvider; 2] = [SignInProvider::Google, SignInProvider::Anonymous];

fn read(dir: &str, name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(dir)
        .join(name);
    fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}
fn deploy_fixture(name: &str) -> Vec<u8> {
    read("deploy_run", name)
}
fn text(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).unwrap()
}
fn extension_plan(with_notes: bool) -> Plan {
    let colab = read("declarations", "colab-firestore.json");
    let notes = read("declarations", "notes-firestore.json");
    let colab_artifact = read("declarations", "rules/colab-admission.rules");
    let notes_artifact = read("declarations", "rules/notes-admission.rules");
    let mut enabled = vec![Enabled {
        name: "colab",
        supplied: Some(Supplied {
            declaration: &colab,
            artifact: &colab_artifact,
        }),
    }];
    if with_notes {
        enabled.push(Enabled {
            name: "notes",
            supplied: Some(Supplied {
                declaration: &notes,
                artifact: &notes_artifact,
            }),
        });
    }
    compose(
        Target {
            backend: CloudBackend::Firestore,
            physical_ttl: false,
        },
        &enabled,
    )
    .unwrap()
}
struct Inputs {
    body: Vec<u8>,
    live: Option<Vec<u8>>,
    account: &'static str,
    project: &'static str,
    deployment: &'static str,
    location: &'static str,
    sign_in: Vec<SignInProvider>,
}
impl Inputs {
    fn new() -> Self {
        Self {
            body: deploy_fixture("rules-body.rules"),
            live: None,
            account: ACCOUNT,
            project: PROJECT,
            deployment: DEPLOYMENT,
            location: LOCATION,
            sign_in: BOTH.to_vec(),
        }
    }
    fn prepare(&self, plan: &Plan) -> Result<DeployPlan, DeployRefusal> {
        prepare(
            plan,
            &DeployInput {
                account: self.account,
                project: self.project,
                deployment_id: self.deployment,
                location: self.location,
                sign_in: &self.sign_in,
                rules_body: &self.body,
                live_rules: self.live.as_deref(),
            },
        )
    }
}
fn fresh() -> DeployPlan {
    Inputs::new().prepare(&extension_plan(true)).unwrap()
}
fn authorized(plan: &DeployPlan) -> tmt_remote::deploy_run::Authorization {
    authorize(plan, &plan.digest()[..16]).unwrap()
}
fn go(
    plan: &DeployPlan,
    fake: &mut Fake,
    sink: &mut Mem,
    record: DeployRecord,
) -> Result<DeployRecord, DeployError> {
    run(plan, &authorized(plan), record, fake, sink, 1_000)
}
fn states(record: &DeployRecord) -> Vec<(String, StepState)> {
    record
        .run
        .as_ref()
        .unwrap()
        .steps
        .iter()
        .map(|step| (step.id.clone(), step.state.clone()))
        .collect()
}
fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn ids(plan: &DeployPlan) -> Vec<String> {
    plan.steps().iter().map(|step| step.id.clone()).collect()
}

#[test]
fn the_envelope_matches_the_independent_reference() {
    let plan = fresh();
    assert_eq!(
        text(plan.bytes().to_vec()),
        text(deploy_fixture("view-fresh.json"))
    );
    assert_eq!(plan.digest(), text(deploy_fixture("view-fresh.sha256")));
    // The deployed bytes are one definition: marker line, then the composed body.
    assert_eq!(plan.deployed_rules(), deploy_fixture("deployed.rules"));
    let mut inputs = Inputs::new();
    inputs.live = Some(deploy_fixture("foreign.rules"));
    let foreign = inputs.prepare(&extension_plan(true)).unwrap();
    assert_eq!(
        text(foreign.bytes().to_vec()),
        text(deploy_fixture("view-foreign.json"))
    );
    assert_eq!(
        foreign.digest(),
        text(deploy_fixture("view-foreign.sha256"))
    );
    assert_eq!(foreign.view().destructive.len(), 1);
}

#[test]
fn every_part_of_the_envelope_changes_the_digest() {
    let base = fresh();
    let plans = [
        Inputs {
            account: "other@example.test",
            ..Inputs::new()
        },
        Inputs {
            project: "demo-remote-2",
            ..Inputs::new()
        },
        Inputs {
            deployment: "3f2b8c1e-5d4a-4e7b-9c1d-2a6f8e0b4c12",
            ..Inputs::new()
        },
        Inputs {
            location: "us-central1",
            ..Inputs::new()
        },
        Inputs {
            sign_in: vec![SignInProvider::Anonymous],
            ..Inputs::new()
        },
        Inputs {
            body: b"rules_version = '2';\n".to_vec(),
            ..Inputs::new()
        },
    ];
    for inputs in plans {
        let other = inputs.prepare(&extension_plan(true)).unwrap();
        assert_ne!(other.digest(), base.digest());
    }
    let without_notes = Inputs::new().prepare(&extension_plan(false)).unwrap();
    assert_ne!(without_notes.digest(), base.digest());
    // The order the caller lists providers in does not matter.
    let reversed = Inputs {
        sign_in: vec![SignInProvider::Anonymous, SignInProvider::Google],
        ..Inputs::new()
    };
    assert_eq!(
        reversed.prepare(&extension_plan(true)).unwrap().digest(),
        base.digest()
    );
}

#[test]
fn an_envelope_with_an_unusable_part_is_refused() {
    let plan = extension_plan(true);
    let cases: [(Inputs, DeployRefusal); 7] = [
        (
            Inputs {
                project: "Demo-Remote",
                ..Inputs::new()
            },
            DeployRefusal::InvalidProject,
        ),
        (
            Inputs {
                project: "short",
                ..Inputs::new()
            },
            DeployRefusal::InvalidProject,
        ),
        (
            Inputs {
                project: "demo-remote-",
                ..Inputs::new()
            },
            DeployRefusal::InvalidProject,
        ),
        (
            Inputs {
                location: "",
                ..Inputs::new()
            },
            DeployRefusal::InvalidLocation,
        ),
        (
            Inputs {
                deployment: "3f2b8c1e-5d4a-3e7b-9c1d-2a6f8e0b4c11",
                ..Inputs::new()
            },
            DeployRefusal::InvalidDeployment,
        ),
        (
            Inputs {
                deployment: "not-a-uuid",
                ..Inputs::new()
            },
            DeployRefusal::InvalidDeployment,
        ),
        (
            Inputs {
                sign_in: vec![],
                ..Inputs::new()
            },
            DeployRefusal::NoSignIn,
        ),
    ];
    for (inputs, expected) in cases {
        assert_eq!(inputs.prepare(&plan).unwrap_err(), expected);
    }
}

#[test]
fn authorization_names_the_digest_and_the_replaced_rules() {
    let plan = fresh();
    for typed in ["", "abc", &plan.digest()[..11], &"0".repeat(16)] {
        assert_eq!(
            authorize(&plan, typed).unwrap_err(),
            DeployRefusal::AuthorizationStale,
            "{typed:?}"
        );
    }
    assert!(authorize(&plan, &plan.digest()[..12]).is_ok());
    assert!(authorize(&plan, plan.digest()).is_ok());

    let mut inputs = Inputs::new();
    inputs.live = Some(deploy_fixture("foreign.rules"));
    let foreign = inputs.prepare(&extension_plan(true)).unwrap();
    let replaced = foreign.view().rules.replaced_digest.clone().unwrap();
    assert_eq!(replaced, sha256(inputs.live.as_deref().unwrap()));
    assert!(authorize(&foreign, foreign.digest()).is_ok());
    inputs.live = Some([inputs.live.as_deref().unwrap(), b"\n"].concat());
    let changed = inputs.prepare(&extension_plan(true)).unwrap();
    assert_ne!(foreign.digest(), changed.digest());
    assert_eq!(
        authorize(&changed, foreign.digest()).unwrap_err(),
        DeployRefusal::AuthorizationStale
    );
}

/// An earlier, untouched release of this deployment: its marker names the digest of its body.
fn earlier_release(deployment: &str, body: &[u8]) -> Vec<u8> {
    let mut bytes = format!(
        "// tmt-remote deployment {deployment} rules {}\n",
        sha256(body)
    )
    .into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

#[test]
fn live_rules_are_classified_by_ownership() {
    let plan = fresh();
    let earlier = earlier_release(DEPLOYMENT, b"rules_version = '2';\n// first release\n");
    let mut edited = plan.deployed_rules().to_vec();
    edited.extend_from_slice(b"// edited\n");
    let mut edited_earlier = earlier.clone();
    edited_earlier.extend_from_slice(b"// edited\n");
    let foreign = |bytes: &[u8]| LiveRules::Foreign(sha256(bytes));
    let cases = [
        (None, LiveRules::Absent),
        (Some(plan.deployed_rules().to_vec()), LiveRules::Current),
        (Some(earlier.clone()), LiveRules::Own),
        // Edited after it was published: no longer Remote's.
        (Some(edited.clone()), foreign(&edited)),
        (Some(edited_earlier.clone()), foreign(&edited_earlier)),
        // A marker whose digest names other bytes is not intact either.
        (
            Some(
                text(deploy_fixture("deployed.rules"))
                    .replace("rules d84f", "rules 0000")
                    .into_bytes(),
            ),
            foreign(
                &text(deploy_fixture("deployed.rules"))
                    .replace("rules d84f", "rules 0000")
                    .into_bytes(),
            ),
        ),
        // Another deployment's release is not this deployment's.
        (
            Some(earlier_release(
                "3f2b8c1e-5d4a-4e7b-9c1d-2a6f8e0b4c12",
                b"x\n",
            )),
            foreign(&earlier_release(
                "3f2b8c1e-5d4a-4e7b-9c1d-2a6f8e0b4c12",
                b"x\n",
            )),
        ),
    ];
    for (live, expected) in cases {
        let mut inputs = Inputs::new();
        inputs.live = live;
        assert_eq!(
            inputs.prepare(&extension_plan(true)).unwrap().live_rules(),
            &expected
        );
    }
}

#[test]
fn rules_edited_after_publishing_are_replaced_only_with_their_digest() {
    let mut live = fresh().deployed_rules().to_vec();
    live.extend_from_slice(b"// edited in the console\n");
    let mut inputs = Inputs::new();
    inputs.live = Some(live.clone());
    let plan = inputs.prepare(&extension_plan(true)).unwrap();
    assert_eq!(plan.view().rules.replaces, "foreign");
    assert_ne!(plan.digest(), fresh().digest());
    assert_eq!(
        authorize(&plan, fresh().digest()).unwrap_err(),
        DeployRefusal::AuthorizationStale
    );
    let authorization = authorize(&plan, plan.digest()).unwrap();
    let mut fake = Fake::new(ACCOUNT);
    fake.rules = Some(live);
    let mut sink = Mem::default();
    let record = run(
        &plan,
        &authorization,
        DeployRecord::new(DEPLOYMENT),
        &mut fake,
        &mut sink,
        1,
    )
    .unwrap();
    assert_eq!(record.run.as_ref().unwrap().state, RunState::Complete);
    assert_eq!(fake.rules.as_deref(), Some(plan.deployed_rules()));
}

#[test]
fn a_complete_run_publishes_the_binding_and_nothing_else_changes() {
    let plan = fresh();
    let mut fake = Fake::new(ACCOUNT);
    fake.unrelated
        .insert("index:x/other#a:asc".into(), "kept".into());
    let before = fake.unrelated.clone();
    let mut sink = Mem::default();
    let record = go(&plan, &mut fake, &mut sink, DeployRecord::new(DEPLOYMENT)).unwrap();

    assert_eq!(record.run.as_ref().unwrap().state, RunState::Complete);
    assert!(states(&record).iter().all(|(_, s)| *s == StepState::Done));
    assert_eq!(
        record.binding,
        Some(DeployBinding {
            plan_digest: plan.digest().to_owned(),
            project: PROJECT.into(),
            completed_at_ms: 1_000,
        })
    );
    assert_eq!(record.usable_binding(), record.binding.as_ref());
    for id in ids(&plan).iter().filter(|id| *id != "verify") {
        assert_eq!(fake.effects_of(id), 1, "{id}");
    }
    assert_eq!(fake.rules.as_deref(), Some(plan.deployed_rules()));
    assert_eq!(fake.unrelated, before);
    // Every call named a step of the plan: nothing else was observed or changed.
    let known = ids(&plan);
    assert!(
        fake.calls
            .iter()
            .all(|call| known.contains(&call.split_once(':').unwrap().1.to_owned()))
    );
    // Rules are the last change: every additive step landed before it.
    let applies: Vec<&String> = fake
        .calls
        .iter()
        .filter(|c| c.starts_with("apply:"))
        .collect();
    assert_eq!(applies.last().unwrap().as_str(), "apply:rules");
}

/// The step order of the fixture plan, with the Verify read-back last.
fn order() -> Vec<String> {
    ids(&fresh())
}

#[test]
fn a_failure_at_any_step_leaves_no_usable_binding_and_resume_finishes() {
    let steps = order();
    let last = steps.len() - 1;
    for (k, id) in steps.iter().enumerate().take(last) {
        for when in [When::BeforeEffect, When::AfterEffectUnknown] {
            let plan = fresh();
            let mut fake = Fake::new(ACCOUNT);
            fake.faults.insert(id.clone(), when);
            let mut sink = Mem::default();
            let record = go(&plan, &mut fake, &mut sink, DeployRecord::new(DEPLOYMENT)).unwrap();

            let label = format!("{id} {when:?}");
            assert_eq!(
                record.run.as_ref().unwrap().state,
                RunState::Partial,
                "{label}"
            );
            assert_eq!(record.binding, None, "{label}");
            assert_eq!(record.usable_binding(), None, "{label}");
            let found = states(&record);
            for (index, (_, state)) in found.iter().enumerate() {
                match index.cmp(&k) {
                    std::cmp::Ordering::Less => assert_eq!(*state, StepState::Done, "{label}"),
                    std::cmp::Ordering::Equal => assert_eq!(
                        *state,
                        match when {
                            When::BeforeEffect => StepState::Failed(DeployFault::ProviderRejected),
                            When::AfterEffectUnknown => StepState::Unknown,
                        },
                        "{label}"
                    ),
                    std::cmp::Ordering::Greater => {
                        assert_eq!(*state, StepState::Pending, "{label}")
                    }
                }
            }
            // Nothing after the failed step was observed or applied.
            let touched: Vec<_> = fake
                .calls
                .iter()
                .map(|c| c.split_once(':').unwrap().1)
                .collect();
            assert!(
                !touched
                    .iter()
                    .any(|t| steps[k + 1..].contains(&(*t).to_owned())),
                "{label}"
            );

            // The same authorization resumes; each step's effect lands exactly once.
            let record = go(&plan, &mut fake, &mut sink, record).unwrap();
            assert_eq!(
                record.run.as_ref().unwrap().state,
                RunState::Complete,
                "{label}"
            );
            assert!(record.usable_binding().is_some(), "{label}");
            for step in steps.iter().take(last) {
                assert_eq!(fake.effects_of(step), 1, "{label}: {step}");
            }
        }
    }
}

/// An installed deploy and an upgrade that adds a sign-in provider and changes the Rules.
fn upgrade() -> (DeployPlan, DeployPlan) {
    let old = Inputs {
        sign_in: vec![SignInProvider::Anonymous],
        body: b"rules_version = '2';\n// first release\n".to_vec(),
        ..Inputs::new()
    };
    (
        old.prepare(&extension_plan(false)).unwrap(),
        Inputs::new().prepare(&extension_plan(true)).unwrap(),
    )
}

#[test]
fn the_rules_call_withdraws_the_binding_but_an_earlier_failure_keeps_it() {
    let (old, new) = upgrade();
    for (failing, keeps) in [("sign-in:google.com", true), ("rules", false)] {
        let mut fake = Fake::new(ACCOUNT);
        let mut sink = Mem::default();
        let first = go(&old, &mut fake, &mut sink, DeployRecord::new(DEPLOYMENT)).unwrap();
        let old_binding = first.binding.clone().unwrap();
        fake.faults.insert(failing.into(), When::BeforeEffect);
        let partial = go(&new, &mut fake, &mut sink, first).unwrap();
        assert_eq!(
            partial.run.as_ref().unwrap().state,
            RunState::Partial,
            "{failing}"
        );
        let attempted = partial.run.as_ref().unwrap().rules_attempted;
        if keeps {
            // Nothing switched yet: the installed deploy is still fully described.
            assert_eq!(partial.usable_binding(), Some(&old_binding), "{failing}");
            assert!(!attempted);
            assert_eq!(fake.rules.as_deref(), Some(old.deployed_rules()));
        } else {
            assert_eq!(partial.usable_binding(), None, "{failing}");
            assert!(attempted);
        }
        let done = go(&new, &mut fake, &mut sink, partial).unwrap();
        assert_eq!(
            done.run.as_ref().unwrap().state,
            RunState::Complete,
            "{failing}"
        );
        assert_eq!(done.usable_binding().unwrap().plan_digest, new.digest());
    }
}

#[test]
fn a_new_plan_after_a_partial_rules_switch_withdraws_the_old_binding() {
    let (old, new) = upgrade();
    let mut fake = Fake::new(ACCOUNT);
    let mut sink = Mem::default();
    let first = go(&old, &mut fake, &mut sink, DeployRecord::new(DEPLOYMENT)).unwrap();
    fake.faults.insert("rules".into(), When::AfterEffectUnknown);
    let partial = go(&new, &mut fake, &mut sink, first).unwrap();
    assert!(partial.binding.is_some());
    assert_eq!(partial.usable_binding(), None);
    // The owner now authorizes the older plan again instead: the project served new Rules
    // meanwhile, so the stored binding is dropped when the other plan's run begins.
    let mut stuck = Fake::new(ACCOUNT);
    stuck.present = fake.present.clone();
    stuck.rules = fake.rules.clone();
    stuck.faults.insert("rules".into(), When::BeforeEffect);
    let again = go(&old, &mut stuck, &mut sink, partial).unwrap();
    assert_eq!(again.binding, None);
    assert_eq!(again.usable_binding(), None);
}

#[test]
fn running_a_finished_plan_again_keeps_the_binding_until_rules_are_touched() {
    let plan = fresh();
    for error in [
        DeployProviderError::Rejected(DeployFault::ProviderRejected),
        DeployProviderError::Unknown,
    ] {
        let mut fake = Fake::new(ACCOUNT);
        let mut sink = Mem::default();
        let done = go(&plan, &mut fake, &mut sink, DeployRecord::new(DEPLOYMENT)).unwrap();
        let binding = done.binding.clone().unwrap();
        // The check itself fails on the first read: the Rules were never touched again.
        fake.observe_faults.insert("database".into(), error);
        let again = go(&plan, &mut fake, &mut sink, done).unwrap();
        let run = again.run.as_ref().unwrap();
        assert_eq!(run.state, RunState::Partial, "{error:?}");
        assert!(!run.rules_attempted, "{error:?}");
        assert_eq!(again.usable_binding(), Some(&binding), "{error:?}");
        // The retry resumes this unfinished check and completes it.
        let third = go(&plan, &mut fake, &mut sink, again).unwrap();
        assert_eq!(third.run.as_ref().unwrap().state, RunState::Complete);
        assert_eq!(third.usable_binding().unwrap().plan_digest, plan.digest());
    }
}

#[test]
fn drift_found_on_a_finished_plan_withdraws_the_binding_when_rules_are_applied() {
    let plan = fresh();
    let earlier = earlier_release(DEPLOYMENT, b"rules_version = '2';\n// first release\n");
    let foreign = deploy_fixture("foreign.rules");
    for drifted in [earlier, foreign] {
        let mut fake = Fake::new(ACCOUNT);
        let mut sink = Mem::default();
        let done = go(&plan, &mut fake, &mut sink, DeployRecord::new(DEPLOYMENT)).unwrap();
        let mut inputs = Inputs::new();
        inputs.live = Some(drifted.clone());
        let plan = inputs.prepare(&extension_plan(true)).unwrap();
        fake.rules = Some(drifted);
        fake.faults.insert("rules".into(), When::BeforeEffect);
        let authorization = authorize(&plan, plan.digest()).unwrap();
        let partial = run(&plan, &authorization, done, &mut fake, &mut sink, 2).unwrap();
        assert!(partial.run.as_ref().unwrap().rules_attempted);
        assert_eq!(partial.usable_binding(), None);
        let finished = run(&plan, &authorization, partial, &mut fake, &mut sink, 3).unwrap();
        assert_eq!(finished.run.as_ref().unwrap().state, RunState::Complete);
        assert!(finished.usable_binding().is_some());
        assert_eq!(fake.rules.as_deref(), Some(plan.deployed_rules()));
    }
}

#[test]
fn a_changed_plan_or_account_is_refused_before_any_provider_call() {
    let plan = fresh();
    let authorization = authorized(&plan);
    let other = Inputs {
        sign_in: vec![SignInProvider::Anonymous],
        ..Inputs::new()
    }
    .prepare(&extension_plan(true))
    .unwrap();
    let mut fake = Fake::new(ACCOUNT);
    let mut sink = Mem::default();
    let refused = run(
        &other,
        &authorization,
        DeployRecord::new(DEPLOYMENT),
        &mut fake,
        &mut sink,
        1,
    );
    assert_eq!(
        refused.unwrap_err(),
        DeployError::Refused(DeployRefusal::AuthorizationStale)
    );
    // A record of another deployment is not this plan's either.
    let refused = run(
        &plan,
        &authorization,
        DeployRecord::new("3f2b8c1e-5d4a-4e7b-9c1d-2a6f8e0b4c12"),
        &mut fake,
        &mut sink,
        1,
    );
    assert_eq!(
        refused.unwrap_err(),
        DeployError::Refused(DeployRefusal::AuthorizationStale)
    );

    let mut elsewhere = Fake::new("someone-else@example.test");
    let refused = go(
        &plan,
        &mut elsewhere,
        &mut sink,
        DeployRecord::new(DEPLOYMENT),
    );
    assert_eq!(
        refused.unwrap_err(),
        DeployError::Refused(DeployRefusal::AccountChanged)
    );
    assert!(fake.calls.is_empty() && elsewhere.calls.is_empty());
    assert!(sink.saved.is_empty());
}

#[test]
fn foreign_rules_are_replaced_only_with_their_digest_and_never_by_a_race() {
    let mut inputs = Inputs::new();
    inputs.live = Some(deploy_fixture("foreign.rules"));
    let plan = inputs.prepare(&extension_plan(true)).unwrap();
    let authorization = authorize(&plan, plan.digest()).unwrap();

    let mut fake = Fake::new(ACCOUNT);
    fake.rules = Some(deploy_fixture("foreign.rules"));
    let mut sink = Mem::default();
    let record = run(
        &plan,
        &authorization,
        DeployRecord::new(DEPLOYMENT),
        &mut fake,
        &mut sink,
        1,
    )
    .unwrap();
    assert_eq!(record.run.as_ref().unwrap().state, RunState::Complete);
    assert_eq!(fake.rules.as_deref(), Some(plan.deployed_rules()));

    // Between the plan and the run the live Rules changed to something else: not authorized.
    let mut fake = Fake::new(ACCOUNT);
    fake.rules = Some(b"rules_version = '2';\n// a third release\n".to_vec());
    let record = run(
        &plan,
        &authorization,
        DeployRecord::new(DEPLOYMENT),
        &mut fake,
        &mut sink,
        1,
    )
    .unwrap();
    let rules = states(&record)
        .into_iter()
        .find(|(id, _)| id == "rules")
        .unwrap();
    assert_eq!(rules.1, StepState::Failed(DeployFault::RulesForeign));
    assert_eq!(fake.effects_of("rules"), 0);
    assert!(!fake.calls.contains(&"apply:rules".to_owned()));
    assert!(!record.run.as_ref().unwrap().rules_attempted);
    assert_eq!(record.usable_binding(), None);
}

#[test]
fn existing_objects_are_adopted_and_unrelated_ones_are_untouched() {
    let plan = fresh();
    let mut fake = Fake::new(ACCOUNT);
    for id in ["database", "index:x/colab/checkpoints#epoch:desc"] {
        fake.present.insert(id.into());
        fake.adopted.insert(id.into());
    }
    for (name, value) in [
        ("index:x/other/log#t:asc", "mine"),
        ("collection:users", "documents"),
        ("provider:facebook.com", "enabled"),
    ] {
        fake.unrelated.insert(name.into(), value.into());
    }
    let before = fake.unrelated.clone();
    let mut sink = Mem::default();
    let record = go(&plan, &mut fake, &mut sink, DeployRecord::new(DEPLOYMENT)).unwrap();
    assert_eq!(record.run.as_ref().unwrap().state, RunState::Complete);
    let found = states(&record);
    assert_eq!(found[0], ("database".into(), StepState::Adopted));
    assert_eq!(fake.effects_of("database"), 0);
    assert_eq!(fake.effects_of("index:x/colab/checkpoints#epoch:desc"), 0);
    assert_eq!(fake.unrelated, before);
}

#[test]
fn a_database_that_does_not_match_stops_the_run_before_anything_else() {
    let plan = fresh();
    let mut fake = Fake::new(ACCOUNT);
    fake.database_mismatch = true;
    let mut sink = Mem::default();
    let record = go(&plan, &mut fake, &mut sink, DeployRecord::new(DEPLOYMENT)).unwrap();
    assert_eq!(
        states(&record)[0].1,
        StepState::Failed(DeployFault::DatabaseMismatch)
    );
    assert_eq!(fake.calls, vec!["observe:database".to_owned()]);
    assert_eq!(record.usable_binding(), None);
}

#[test]
fn an_owner_step_stops_the_run_with_fixed_text_and_resume_continues_after_it() {
    let plan = fresh();
    let mut fake = Fake::new(ACCOUNT);
    fake.owner_pending.insert(
        "sign-in:google.com".into(),
        DeployOwnerAction::EnableGoogleSignIn,
    );
    let mut sink = Mem::default();
    let record = go(&plan, &mut fake, &mut sink, DeployRecord::new(DEPLOYMENT)).unwrap();
    let found = states(&record);
    let google = found
        .iter()
        .position(|(id, _)| id == "sign-in:google.com")
        .unwrap();
    assert_eq!(
        found[google].1,
        StepState::OwnerAction(DeployOwnerAction::EnableGoogleSignIn)
    );
    assert!(
        found[google + 1..]
            .iter()
            .all(|(_, s)| *s == StepState::Pending)
    );
    assert_eq!(fake.rules, None);
    assert_eq!(record.usable_binding(), None);
    assert_eq!(
        DeployOwnerAction::EnableGoogleSignIn.instruction(),
        "In the Firebase console open Authentication, Sign-in method, enable Google, then run the deploy again."
    );
    assert_eq!(
        DeployOwnerAction::EnableGoogleSignIn.code(),
        "REMOTE_DEPLOY_OWNER_ENABLE_GOOGLE_SIGN_IN"
    );

    // The owner acts in the console; the provider now has the sign-in method.
    fake.owner_pending.clear();
    fake.present.insert("sign-in:google.com".into());
    let record = go(&plan, &mut fake, &mut sink, record).unwrap();
    assert_eq!(record.run.as_ref().unwrap().state, RunState::Complete);
    assert_eq!(fake.effects_of("sign-in:google.com"), 0);
}

#[test]
fn building_indexes_and_a_failed_read_back_never_publish_a_binding() {
    let plan = fresh();
    let index = "index:x/colab/log#sequence:asc";
    let mut fake = Fake::new(ACCOUNT);
    fake.present.insert(index.into());
    fake.building.insert(index.into());
    let mut sink = Mem::default();
    let record = go(&plan, &mut fake, &mut sink, DeployRecord::new(DEPLOYMENT)).unwrap();
    let found = states(&record);
    assert_eq!(
        found.iter().find(|(id, _)| id == index).unwrap().1,
        StepState::Building
    );
    assert_eq!(found.last().unwrap().1, StepState::Building);
    assert_eq!(record.run.as_ref().unwrap().state, RunState::Partial);
    assert_eq!(record.usable_binding(), None);
    fake.building.clear();
    let record = go(&plan, &mut fake, &mut sink, record).unwrap();
    assert_eq!(record.run.as_ref().unwrap().state, RunState::Complete);

    // Someone else publishes Rules right after ours: the read-back catches it.
    let mut fake = Fake::new(ACCOUNT);
    fake.drift_after = Some(("rules".into(), deploy_fixture("foreign.rules")));
    let mut sink = Mem::default();
    let record = go(&plan, &mut fake, &mut sink, DeployRecord::new(DEPLOYMENT)).unwrap();
    assert_eq!(
        states(&record).last().unwrap().1,
        StepState::Failed(DeployFault::VerifyFailed)
    );
    assert_eq!(record.binding, None);
    assert_eq!(record.usable_binding(), None);
    // A retry looks at every step again: the drifted Rules are not Remote's, so the owner
    // must name them before they are replaced.
    fake.drift_after = None;
    let record = go(&plan, &mut fake, &mut sink, record).unwrap();
    let rules = states(&record)
        .into_iter()
        .find(|(id, _)| id == "rules")
        .unwrap();
    assert_eq!(rules.1, StepState::Failed(DeployFault::RulesForeign));
    let mut inputs = Inputs::new();
    inputs.live = fake.rules.clone();
    let plan = inputs.prepare(&extension_plan(true)).unwrap();
    let authorization = authorize(&plan, plan.digest()).unwrap();
    let record = run(&plan, &authorization, record, &mut fake, &mut sink, 2).unwrap();
    assert_eq!(record.run.as_ref().unwrap().state, RunState::Complete);
    assert_eq!(fake.rules.as_deref(), Some(plan.deployed_rules()));
}

#[test]
fn a_crash_after_any_save_resumes_from_the_last_saved_record() {
    let plan = fresh();
    let mut clean = Mem::default();
    go(
        &plan,
        &mut Fake::new(ACCOUNT),
        &mut clean,
        DeployRecord::new(DEPLOYMENT),
    )
    .unwrap();
    let saves = clean.saved.len();
    assert!(saves > ids(&plan).len() * 2);
    for crash_at in 0..saves {
        let mut fake = Fake::new(ACCOUNT);
        let mut dying = Mem {
            fail_from: Some(crash_at),
            ..Mem::default()
        };
        let result = go(&plan, &mut fake, &mut dying, DeployRecord::new(DEPLOYMENT));
        assert_eq!(
            result.unwrap_err(),
            DeployError::Interrupted,
            "crash {crash_at}"
        );
        let saved = dying
            .saved
            .last()
            .cloned()
            .unwrap_or_else(|| DeployRecord::new(DEPLOYMENT));
        let mut sink = Mem::default();
        let record = go(&plan, &mut fake, &mut sink, saved).unwrap();
        assert_eq!(
            record.run.as_ref().unwrap().state,
            RunState::Complete,
            "crash {crash_at}"
        );
        for id in ids(&plan).iter().filter(|id| *id != "verify") {
            assert_eq!(fake.effects_of(id), 1, "crash {crash_at}: {id}");
        }
    }
}

#[test]
fn a_binding_is_never_saved_before_the_last_step_and_the_record_round_trips() {
    let plan = fresh();
    let mut fake = Fake::new(ACCOUNT);
    fake.owner_pending.insert(
        "sign-in:anonymous".into(),
        DeployOwnerAction::InitializeAuth,
    );
    fake.faults.insert("rules".into(), When::AfterEffectUnknown);
    let mut sink = Mem::default();
    let first = go(&plan, &mut fake, &mut sink, DeployRecord::new(DEPLOYMENT)).unwrap();
    fake.owner_pending.clear();
    fake.present.insert("sign-in:anonymous".into());
    let second = go(&plan, &mut fake, &mut sink, first).unwrap();
    assert_eq!(
        states(&second)
            .into_iter()
            .find(|(id, _)| id == "rules")
            .unwrap()
            .1,
        StepState::Unknown
    );
    let record = go(&plan, &mut fake, &mut sink, second).unwrap();
    assert_eq!(record.run.as_ref().unwrap().state, RunState::Complete);
    for saved in &sink.saved {
        let finished = saved.run.as_ref().is_some_and(|run| {
            run.steps
                .iter()
                .all(|s| matches!(s.state, StepState::Done | StepState::Adopted))
        });
        // Never a binding ahead of the last step; the converse may lag by one save.
        assert!(finished || saved.binding.is_none());
        let json = serde_json::to_string(saved).unwrap();
        assert_eq!(&serde_json::from_str::<DeployRecord>(&json).unwrap(), saved);
    }
    // Fixed codes only: the stored states carry no provider text.
    let json = serde_json::to_string(&sink.saved[2]).unwrap();
    assert!(json.contains(r#""state":"pending""#), "{json}");
}

#[path = "support/deploy_fixture.rs"]
mod deploy_fixture_root;

#[test]
fn emulator_artifact_is_the_exact_verified_deployment_output() {
    use tmt_remote::{
        deploy_record::{DeployRecordEvidence, DeployRecordStore},
        readiness::{FirestoreEvidenceSource, Observed},
    };
    let colab = read("rules", "colab.rules");
    let notes = read("rules", "notes.rules");
    let declaration = |name: &str, artifact: &[u8]| {
        let mut value: serde_json::Value =
            serde_json::from_slice(&read("rules", &format!("{name}.json"))).unwrap();
        value["admission"]["digest"] = serde_json::json!(sha256(artifact));
        serde_json::to_vec(&value).unwrap()
    };
    let colab_decl = declaration("colab", &colab);
    let notes_decl = declaration("notes", &notes);
    let extensions = compose(
        Target {
            backend: CloudBackend::Firestore,
            physical_ttl: false,
        },
        &[
            Enabled {
                name: "colab",
                supplied: Some(Supplied {
                    declaration: &colab_decl,
                    artifact: &colab,
                }),
            },
            Enabled {
                name: "notes",
                supplied: Some(Supplied {
                    declaration: &notes_decl,
                    artifact: &notes,
                }),
            },
        ],
    )
    .unwrap();
    let composed = tmt_remote::rules::compose(
        &extensions,
        &[
            tmt_remote::rules::Fragment {
                extension: "colab",
                source: &colab,
            },
            tmt_remote::rules::Fragment {
                extension: "notes",
                source: &notes,
            },
        ],
    )
    .unwrap();
    assert_eq!(composed.rules.as_bytes(), read("rules", "composed.rules"));
    let plan = prepare(
        &extensions,
        &DeployInput {
            account: ACCOUNT,
            project: PROJECT,
            deployment_id: deploy_fixture_root::ID,
            location: LOCATION,
            sign_in: &BOTH,
            rules_body: composed.rules.as_bytes(),
            live_rules: None,
        },
    )
    .unwrap();
    let root = deploy_fixture_root::Root::new();
    let layout = root.layout();
    let mut sink = DeployRecordStore::open(&layout).unwrap();
    let mut fake = Fake::new(ACCOUNT);
    let record = run(
        &plan,
        &authorized(&plan),
        DeployRecord::new(deploy_fixture_root::ID),
        &mut fake,
        &mut sink,
        1_000,
    )
    .unwrap();
    assert_eq!(record.run.as_ref().unwrap().state, RunState::Complete);
    assert!(record.usable_binding().is_some());
    let captured = fake.rules.as_deref().unwrap();
    assert_eq!(captured, plan.deployed_rules());
    assert_eq!(
        captured,
        read("rules", "deployed.rules"),
        "native deployment output must bind the emulator artifact"
    );
    let (marker, body) = std::str::from_utf8(captured)
        .unwrap()
        .split_once('\n')
        .unwrap();
    assert_eq!(
        marker,
        format!(
            "// tmt-remote deployment {} rules {}",
            deploy_fixture_root::ID,
            sha256(body.as_bytes())
        )
    );
    assert_eq!(body, composed.rules);
    assert_eq!(
        tmt_remote::deploy_record::read(&layout).unwrap(),
        Some(record)
    );
    assert!(root.remote().join("deploy.json").exists());
    let evidence = DeployRecordEvidence::new(&layout).evidence().unwrap();
    assert_eq!(
        (evidence.project, evidence.sign_in, evidence.rules),
        (Observed::Enabled, Observed::Enabled, Observed::Enabled)
    );
    assert_eq!(evidence.quota, Observed::Unknown);
}

// Executable Hosting order uses the existing fake provider and original record owner.
fn hosted_plan() -> DeployPlan {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use serde_json::json;
    use tmt_remote::hosting::{self, HostingBundle, HostingInventory, HostingManifest};
    let mut declaration: serde_json::Value =
        serde_json::from_slice(&read("declarations", "colab-firestore.json")).unwrap();
    let manifest = HostingManifest::parse(&json!({"version":1,"files":[{"path":"/index.html","sha256":sha256(b"hello"),"length":5,"contentType":"text/html"}]})).unwrap();
    declaration["hosting"] = serde_json::to_value(&manifest).unwrap();
    let bytes = serde_json::to_vec(&declaration).unwrap();
    let artifact = read("declarations", "rules/colab-admission.rules");
    let extensions = compose(
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
    let bundle = HostingBundle::parse(&manifest, &serde_json::to_vec(&reply).unwrap()).unwrap();
    let content = hosting::compose(&[bundle]).unwrap().unwrap();
    let hosting = hosting::deployment_view(
        &content,
        PROJECT,
        DEPLOYMENT,
        &HostingInventory {
            site_exists: false,
            web_apps: vec![],
            live: None,
        },
    )
    .unwrap();
    let inputs = Inputs::new();
    tmt_remote::deploy_run::prepare_with_hosting(
        &extensions,
        &DeployInput {
            account: ACCOUNT,
            project: PROJECT,
            deployment_id: DEPLOYMENT,
            location: LOCATION,
            sign_in: &BOTH,
            rules_body: &inputs.body,
            live_rules: None,
        },
        Some(&hosting),
    )
    .unwrap()
}
struct HostedFake {
    inner: Fake,
    checkpoint: Option<tmt_remote::deploy_run::HostingCheckpoint>,
    publish: bool,
}
impl HostedFake {
    fn new() -> Self {
        Self {
            inner: Fake::new(ACCOUNT),
            checkpoint: None,
            publish: true,
        }
    }
}
impl tmt_remote::deploy_run::DeployPort for HostedFake {
    fn supports_hosting(&self) -> bool {
        true
    }
    fn account(&mut self) -> Result<String, DeployProviderError> {
        Ok(ACCOUNT.into())
    }
    fn restore_hosting(
        &mut self,
        c: &tmt_remote::deploy_run::HostingCheckpoint,
        _: &[tmt_remote::deploy_run::StepRecord],
    ) {
        self.checkpoint = Some(c.clone());
    }
    fn hosting_checkpoint(&self) -> Option<tmt_remote::deploy_run::HostingCheckpoint> {
        self.checkpoint.clone()
    }
    fn observe(
        &mut self,
        p: &DeployPlan,
        s: &tmt_remote::deploy_run::DeployStep,
    ) -> Result<tmt_remote::deploy_run::DeployObserved, DeployProviderError> {
        use tmt_remote::deploy_run::{DeployObserved, StepKind};
        let result = tmt_remote::deploy_run::DeployPort::observe(&mut self.inner, p, s)?;
        if s.kind == StepKind::Verify && result == DeployObserved::Satisfied && self.publish {
            // Fixed provider-verification fixture, not a production publication source.
            let cp = self.checkpoint.as_mut().unwrap();
            cp.app_id = Some("mine".into());
            cp.version = Some(format!("sites/{PROJECT}/versions/one"));
            cp.publication=Some(serde_json::from_value(serde_json::json!({"project":PROJECT,"region":LOCATION,"deploymentId":DEPLOYMENT,"planDigest":p.digest(),"appId":"mine","publicConfig":{"apiKey":"public-key","authDomain":format!("{PROJECT}.firebaseapp.com"),"projectId":PROJECT,"appId":"mine"},"entryUrl":format!("https://{PROJECT}.web.app"),"version":cp.version,"release":format!("sites/{PROJECT}/releases/one"),"rulesDigest":p.view().rules.digest,"contentDigest":p.view().hosting.as_ref().unwrap().content["digest"]})).unwrap());
        }
        Ok(result)
    }
    fn apply(
        &mut self,
        p: &DeployPlan,
        s: &tmt_remote::deploy_run::DeployStep,
    ) -> Result<tmt_remote::deploy_run::DeployApplied, DeployProviderError> {
        tmt_remote::deploy_run::DeployPort::apply(&mut self.inner, p, s)
    }
}
#[test]
fn hosted_order_is_executable_and_only_joint_verification_publishes() {
    let plan = hosted_plan();
    let ids = ids(&plan);
    assert_eq!(ids, plan.view().steps);
    assert_eq!(&ids[..2], &["web-app:create", "hosting-site:create"]);
    let at = |id: &str| ids.iter().position(|s| s == id).unwrap();
    assert!(at("hosting:stage:create") < at("hosting:stage:populate"));
    assert!(at("hosting:stage:finalize") < at("rules"));
    assert!(at("rules") < at("hosting:release") && at("hosting:release") < at("verify"));
    for publish in [false, true] {
        let mut fake = HostedFake::new();
        fake.publish = publish;
        let mut sink = Mem::default();
        let record = run(
            &plan,
            &authorized(&plan),
            DeployRecord::new(DEPLOYMENT),
            &mut fake,
            &mut sink,
            1000,
        )
        .unwrap();
        assert_eq!(
            record.run.as_ref().unwrap().state,
            if publish {
                RunState::Complete
            } else {
                RunState::Partial
            }
        );
        assert_eq!(record.usable_binding().is_some(), publish);
        assert_eq!(record.verified_publication().is_some(), publish);
        assert_eq!(
            record
                .run
                .as_ref()
                .unwrap()
                .hosting
                .as_ref()
                .unwrap()
                .publication
                .is_some(),
            publish
        );
    }
}
#[test]
fn every_hosted_effect_loss_stops_dependents_and_recovery_observes_original_without_resending() {
    let plan = hosted_plan();
    for step in plan
        .steps()
        .iter()
        .filter(|s| matches!(s.kind, tmt_remote::deploy_run::StepKind::Hosting(_)))
    {
        let mut fake = HostedFake::new();
        fake.inner
            .faults
            .insert(step.id.clone(), When::AfterEffectUnknown);
        let mut sink = Mem::default();
        let partial = run(
            &plan,
            &authorized(&plan),
            DeployRecord::new(DEPLOYMENT),
            &mut fake,
            &mut sink,
            1000,
        )
        .unwrap();
        assert_eq!(partial.run.as_ref().unwrap().state, RunState::Partial);
        let last = fake.inner.calls.last().unwrap();
        assert_eq!(last, &format!("apply:{}", step.id));
        assert_eq!(fake.inner.effects_of(&step.id), 1);
        let complete = run(
            &plan,
            &authorized(&plan),
            partial,
            &mut fake,
            &mut sink,
            2000,
        )
        .unwrap();
        assert_eq!(complete.run.as_ref().unwrap().state, RunState::Complete);
        assert_eq!(
            fake.inner.effects_of(&step.id),
            1,
            "{} was sent twice",
            step.id
        );
    }
}
#[test]
fn unknown_create_without_a_provider_resource_cannot_be_sent_a_second_time() {
    let plan = hosted_plan();
    let mut fake = HostedFake::new();
    fake.inner
        .faults
        .insert("web-app:create".into(), When::AfterEffectUnknown);
    let mut sink = Mem::default();
    let partial = run(
        &plan,
        &authorized(&plan),
        DeployRecord::new(DEPLOYMENT),
        &mut fake,
        &mut sink,
        1000,
    )
    .unwrap();
    fake.inner.present.remove("web-app:create");
    fake.inner.calls.clear();
    let partial = run(
        &plan,
        &authorized(&plan),
        partial,
        &mut fake,
        &mut sink,
        2000,
    )
    .unwrap();
    assert_eq!(partial.run.as_ref().unwrap().state, RunState::Partial);
    assert_eq!(fake.inner.calls, vec!["observe:web-app:create"]);
    assert_eq!(fake.inner.effects_of("web-app:create"), 1);
}
#[test]
fn frozen_hosted_envelope_survives_own_inventory_changes_but_not_intent_changes() {
    let plan = hosted_plan();
    let mut fake = HostedFake::new();
    fake.inner
        .faults
        .insert("hosting:release".into(), When::AfterEffectUnknown);
    let mut sink = Mem::default();
    let record = run(
        &plan,
        &authorized(&plan),
        DeployRecord::new(DEPLOYMENT),
        &mut fake,
        &mut sink,
        1,
    )
    .unwrap();
    let restored = tmt_remote::deploy_run::retain_hosting_plan(hosted_plan(), &record).unwrap();
    assert_eq!(restored.bytes(), plan.bytes());
    assert_eq!(restored.view().steps, plan.view().steps);
    let mut changed = record.clone();
    changed
        .run
        .as_mut()
        .unwrap()
        .hosting
        .as_mut()
        .unwrap()
        .envelope["account"] = serde_json::json!("another@example.test");
    assert!(tmt_remote::deploy_run::retain_hosting_plan(hosted_plan(), &changed).is_err());
}

#[test]
fn hosting_checkpoint_survives_every_save_interruption_without_duplicate_effects() {
    let plan = hosted_plan();
    let mut baseline = Mem::default();
    run(
        &plan,
        &authorized(&plan),
        DeployRecord::new(DEPLOYMENT),
        &mut HostedFake::new(),
        &mut baseline,
        1000,
    )
    .unwrap();
    for boundary in 0..baseline.saved.len() {
        let mut fake = HostedFake::new();
        let mut sink = Mem {
            saved: Vec::new(),
            fail_from: Some(boundary),
        };
        assert_eq!(
            run(
                &plan,
                &authorized(&plan),
                DeployRecord::new(DEPLOYMENT),
                &mut fake,
                &mut sink,
                1000
            ),
            Err(DeployError::Interrupted)
        );
        let durable = sink
            .saved
            .last()
            .cloned()
            .unwrap_or_else(|| DeployRecord::new(DEPLOYMENT));
        assert!(
            durable.verified_publication().is_none(),
            "boundary {boundary}"
        );
        sink.fail_from = None;
        let complete = run(
            &plan,
            &authorized(&plan),
            durable,
            &mut fake,
            &mut sink,
            2000,
        )
        .unwrap();
        assert_eq!(
            complete.run.as_ref().unwrap().state,
            RunState::Complete,
            "boundary {boundary}"
        );
        assert!(complete.verified_publication().is_some());
        for step in plan
            .steps()
            .iter()
            .filter(|s| matches!(s.kind, tmt_remote::deploy_run::StepKind::Hosting(_)))
        {
            assert_eq!(
                fake.inner.effects_of(&step.id),
                1,
                "boundary {boundary}: {}",
                step.id
            );
        }
    }
}

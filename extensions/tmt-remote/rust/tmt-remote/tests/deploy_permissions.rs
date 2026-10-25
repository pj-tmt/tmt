//! Permission preflight through injected CLI ports and the real embedded helper.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf, sync::atomic::AtomicBool};
use tmt_remote::{
    deploy_cli::{self, DeployCliError, FirestoreCommandPort, inputs::PermissionRequirement},
    deploy_discovery::{DeclarationSource, DiscoveryRefusal},
    deploy_firestore::{DeployFirestore, DeployPreflightError, DeploySetupError},
    deploy_permissions,
    deploy_run::{
        DeployApplied, DeployObserved, DeployPlan, DeployPort, DeployProviderError, DeployStep,
    },
};
#[allow(dead_code)]
#[path = "support/deploy_fixture.rs"]
mod deploy_fixture;
#[allow(dead_code)]
#[path = "support/deploy_port.rs"]
mod deploy_port;
use deploy_fixture::Root;

struct Source;
impl DeclarationSource for Source {
    fn declaration(&mut self, extension: &str) -> Result<Option<Vec<u8>>, DiscoveryRefusal> {
        assert_eq!(extension, "colab");
        let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rules");
        let declaration = fs::read_to_string(fixtures.join("colab.json")).unwrap();
        let hash: String = Sha256::digest(declaration.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        Ok(Some(serde_json::to_vec(&json!({"version":1,"extension":"colab","backend":"firestore","declaration":declaration,"declarationDigest":hash,"artifact":fs::read_to_string(fixtures.join("colab.rules")).unwrap()})).unwrap()))
    }
}
enum Grants {
    All,
    Missing,
    Unreadable,
}
struct Port {
    fake: deploy_port::Fake,
    calls: Vec<&'static str>,
    grants: Grants,
    names: Vec<String>,
}
impl Port {
    fn new(grants: Grants) -> Self {
        Self {
            fake: deploy_port::Fake::new("owner@example.test"),
            calls: vec![],
            grants,
            names: vec![],
        }
    }
}
impl DeployPort for &mut Port {
    fn account(&mut self) -> Result<String, DeployProviderError> {
        self.calls.push("account");
        self.fake.account()
    }
    fn observe(
        &mut self,
        plan: &DeployPlan,
        step: &DeployStep,
    ) -> Result<DeployObserved, DeployProviderError> {
        self.fake.observe(plan, step)
    }
    fn apply(
        &mut self,
        plan: &DeployPlan,
        step: &DeployStep,
    ) -> Result<DeployApplied, DeployProviderError> {
        self.fake.apply(plan, step)
    }
}
impl FirestoreCommandPort for &mut Port {
    fn login(&mut self) -> Result<(), DeploySetupError> {
        self.calls.push("login");
        Ok(())
    }
    fn check_permissions(
        &mut self,
        project: &str,
        permissions: &[&str],
    ) -> Result<Vec<String>, DeployPreflightError> {
        assert_eq!(project, "demo-permissions");
        self.calls.push("permissions");
        self.names = permissions.iter().map(|name| (*name).to_owned()).collect();
        match self.grants {
            Grants::All => Ok(self.names.clone()),
            Grants::Missing => Ok(vec![]),
            Grants::Unreadable => Err(DeployProviderError::Unknown.into()),
        }
    }
    fn check_index_budget(
        &mut self,
        _: &str,
        _: &tmt_remote::deploy_plan::Plan,
    ) -> Result<Option<bool>, DeployPreflightError> {
        self.calls.push("inventory");
        Ok(None)
    }
    fn live_rules(&mut self, _: &str) -> Result<Option<Vec<u8>>, DeployPreflightError> {
        self.calls.push("rules");
        Ok(self.fake.rules.clone())
    }
}
fn args(extra: &[&str]) -> deploy_cli::FirestoreArgs {
    let mut words = vec![
        "firestore",
        "--project",
        "demo-permissions",
        "--region",
        "asia-east1",
        "--defaults",
    ];
    words.extend_from_slice(extra);
    deploy_cli::arguments(&deploy_cli::command().try_get_matches_from(words).unwrap()).unwrap()
}
#[test]
fn missing_or_unreadable_permissions_stop_before_inventory_confirmation_and_effects_in_every_mode()
{
    for extra in [
        vec![],
        vec!["--json"],
        vec!["--authorize", "aaaaaaaaaaaa"],
        vec!["--details"],
    ] {
        for grants in [Grants::Missing, Grants::Unreadable] {
            let root = Root::new();
            let mut port = Port::new(grants);
            let mut confirmed = false;
            let mut confirm = |_: &tmt_remote::deploy_command::DeployCommandOutput| {
                confirmed = true;
                Ok(true)
            };
            let error = deploy_cli::execute_with_confirmation(
                &args(&extra),
                &mut Source,
                &["colab"],
                || Ok(&mut port),
                || Ok(root.layout()),
                || Ok(1),
                Some(&mut confirm),
            )
            .err()
            .expect("permission gate refuses");
            assert!(matches!(
                error,
                DeployCliError::Local(_) | DeployCliError::Provider(_)
            ));
            if let DeployCliError::Local(error) = error {
                assert_eq!(error.code, "REMOTE_DEPLOY_PERMISSION_DENIED");
                assert!(error.message.contains("firebase.projects.get (project)"));
                assert!(error.message.contains("rules"));
            }
            assert_eq!(port.calls, ["login", "permissions"]);
            assert!(!confirmed);
            assert!(port.fake.effects.is_empty());
        }
    }
}
#[test]
fn granted_preview_and_authorized_rerun_recheck_the_manifest_before_inventory() {
    let root = Root::new();
    let mut port = Port::new(Grants::All);
    let preview = deploy_cli::execute(
        &args(&["--json", "--details"]),
        &mut Source,
        &["colab"],
        || Ok(&mut port),
        || Ok(root.layout()),
        || Ok(1),
    )
    .unwrap();
    assert_eq!(port.calls[0..3], ["login", "permissions", "inventory"]);
    assert!(port.fake.effects.is_empty());
    assert_eq!(preview.json["permissions"]["checked"], true);
    let required = preview.json["permissions"]["required"].as_array().unwrap();
    assert_eq!(
        required
            .iter()
            .map(|r| r["permission"].as_str().unwrap())
            .collect::<Vec<_>>(),
        port.names
    );
    assert!(preview.human.contains("Permissions required:"));
    assert!(preview.human.contains("Permissions: checked"));
    let digest = preview.json["planDigest"].as_str().unwrap();
    port.calls.clear();
    port.grants = Grants::Missing;
    let refusal = deploy_cli::execute(
        &args(&["--authorize", digest]),
        &mut Source,
        &["colab"],
        || Ok(&mut port),
        || Ok(root.layout()),
        || Ok(2),
    )
    .err()
    .expect("permission gate refuses");
    assert!(matches!(refusal, DeployCliError::Local(_)));
    assert!(port.fake.effects.is_empty());
    assert_eq!(port.calls, ["login", "permissions"]);
    port.grants = Grants::All;
    port.calls.clear();
    let result = deploy_cli::execute(
        &args(&["--authorize", digest]),
        &mut Source,
        &["colab"],
        || Ok(&mut port),
        || Ok(root.layout()),
        || Ok(3),
    )
    .unwrap();
    assert_eq!(result.json["authorized"], true);
    assert!(!port.fake.effects.is_empty());
    assert_eq!(result.json["planDigest"], digest);
    assert_eq!(port.calls[0..3], ["login", "permissions", "inventory"]);
    // A saved/complete record cannot bypass a new permission check either.
    port.grants = Grants::Missing;
    port.calls.clear();
    assert!(
        deploy_cli::execute(
            &args(&[]),
            &mut Source,
            &["colab"],
            || Ok(&mut port),
            || Ok(root.layout()),
            || Ok(4)
        )
        .is_err()
    );
    assert_eq!(port.calls, ["login", "permissions"]);
}
#[test]
fn missing_permission_names_keep_all_manifest_step_associations_and_first_occurrence() {
    let required = [
        PermissionRequirement {
            permission: "firebase.projects.get",
            step: "project",
        },
        PermissionRequirement {
            permission: "firebase.projects.get",
            step: "hosting",
        },
        PermissionRequirement {
            permission: "datastore.entities.get",
            step: "owner:register",
        },
    ];
    assert_eq!(
        deploy_permissions::requested(&required),
        ["firebase.projects.get", "datastore.entities.get"]
    );
    let error =
        deploy_permissions::check(&required, &["datastore.entities.get".into()]).unwrap_err();
    assert!(
        error
            .message
            .contains("firebase.projects.get (project), firebase.projects.get (hosting)")
    );
    assert!(!error.message.contains("owner:register"));
    assert_eq!(
        deploy_permissions::check(&required, &["TOKEN_CANARY".into()])
            .unwrap_err()
            .code,
        "REMOTE_DEPLOY_UNAVAILABLE"
    );
}

fn package(root: &Root, mode: &str) -> PathBuf {
    let package = root.0.join("firebase-tools");
    fs::create_dir_all(package.join("lib")).unwrap();
    fs::write(
        package.join("package.json"),
        br#"{"name":"firebase-tools","version":"15.29.0"}"#,
    )
    .unwrap();
    for file in ["api", "apiv2", "configstore"] {
        fs::write(package.join(format!("lib/{file}.js")), "module.exports={};").unwrap();
    }
    fs::write(package.join("lib/logger.js"), "exports.logger={clear(){}};").unwrap();
    fs::write(package.join("mode"), mode).unwrap();
    fs::write(package.join("lib/auth.js"), r#"
const fs=require('node:fs'), path=require('node:path');
if(process.cwd()!=='/') throw Error('TOKEN_CANARY');
const mode=fs.readFileSync(path.join(__dirname,'../mode'),'utf8');
exports.selectAccount=()=>({user:{email:'owner@example.test'},tokens:{refresh_token:'TOKEN_CANARY'}});
exports.getAccessToken=async()=>({access_token:'TOKEN_CANARY'});
global.fetch=async(url,options)=>{
  fs.appendFileSync(path.join(__dirname,'../calls'),JSON.stringify({url,method:options.method,body:options.body})+'\n');
  if(url==='https://www.googleapis.com/oauth2/v3/userinfo') return new Response(JSON.stringify({email:'owner@example.test',email_verified:true}),{status:200});
  if(url!=='https://cloudresourcemanager.googleapis.com/v3/projects/demo-permissions:testIamPermissions'||options.method!=='POST') throw Error('TOKEN_CANARY');
  if(mode==='throw') throw Error('TOKEN_CANARY');
  const permissions=JSON.parse(options.body).permissions;
  const body=mode==='malformed'?{permissions:['TOKEN_CANARY']}:mode==='disabled'?{error:{details:[{reason:'SERVICE_DISABLED'}],message:'TOKEN_CANARY'}}:{permissions:mode==='missing'?[]:permissions};
  return new Response(JSON.stringify(body),{status:mode==='disabled'?403:200});
};
"#).unwrap();
    package
}
#[test]
fn embedded_helper_process_probes_exact_names_and_keeps_errors_sanitized() {
    let node = tmt_invoke::find_executable(
        std::ffi::OsStr::new("node"),
        &std::env::var_os("PATH").unwrap(),
    )
    .unwrap();
    let names = ["firebase.projects.get", "datastore.databases.create"];
    for mode in ["granted", "missing", "disabled", "malformed", "throw"] {
        let root = Root::new();
        let package = package(&root, mode);
        let stop = AtomicBool::new(false);
        let mut adapter = DeployFirestore::at(node.clone(), package.clone(), &stop).unwrap();
        let result = adapter.check_permissions("demo-permissions", &names);
        match mode {
            "granted" => assert_eq!(result.unwrap(), names),
            "missing" => assert!(result.unwrap().is_empty()),
            _ => {
                let error = result.unwrap_err().remote_error();
                assert!(!format!("{error:?}").contains("TOKEN_CANARY"));
                assert_eq!(
                    error.code,
                    if mode == "disabled" {
                        "REMOTE_DEPLOY_API_DISABLED"
                    } else {
                        "REMOTE_DEPLOY_UNAVAILABLE"
                    }
                );
            }
        }
        let calls: Vec<Value> = fs::read_to_string(package.join("calls"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0]["method"], "GET");
        assert_eq!(calls[1]["method"], "POST");
        assert_eq!(
            serde_json::from_str::<Value>(calls[1]["body"].as_str().unwrap()).unwrap(),
            json!({"permissions":names})
        );
    }
}
#[test]
fn adapter_input_bounds_and_cancellation_refuse_without_provider_calls() {
    let root = Root::new();
    let package = package(&root, "granted");
    let stop = AtomicBool::new(false);
    let node = tmt_invoke::find_executable(
        std::ffi::OsStr::new("node"),
        &std::env::var_os("PATH").unwrap(),
    )
    .unwrap();
    let mut adapter = DeployFirestore::at(node, package.clone(), &stop).unwrap();
    for names in [
        vec![],
        vec!["*"],
        vec!["firebase.projects.get", "firebase.projects.get"],
    ] {
        assert!(
            adapter
                .check_permissions("demo-permissions", &names)
                .is_err()
        );
    }
    assert!(!package.join("calls").exists());
    stop.store(true, std::sync::atomic::Ordering::Release);
    assert!(
        adapter
            .check_permissions("demo-permissions", &["firebase.projects.get"])
            .is_err()
    );
    assert!(!package.join("calls").exists());
    let helper = include_str!("../src/deploy_firestore/helper.cjs");
    assert!(helper.contains(&format!(
        "const PERMISSION_COUNT = {};",
        tmt_remote::limits::DEPLOY_PERMISSION_COUNT
    )));
    assert!(helper.contains(&format!(
        "const PERMISSION_BYTES = {};",
        tmt_remote::limits::DEPLOY_PERMISSION_BYTES
    )));
}

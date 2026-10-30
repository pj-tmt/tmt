//! Signed browser issuance over the actual coordinator, Store and injected provider.
use super::browser_ceremony_tests::Browser;
use super::*;
use crate::{
    deploy_plan::{self, CloudBackend, Enabled, Supplied, Target},
    deploy_record::DeployRecordStore,
    deploy_run::{
        self, DeployBinding, DeployInput, HostingCheckpoint, Run, RunState, SignInProvider,
        StepRecord, StepState,
    },
    hosting::{self, HostingBundle, HostingInventory, HostingManifest},
    state::Layout,
};
use base64::Engine;
fn now() -> u64 {
    crate::pairing::now_ms().unwrap()
}
fn hash(bytes: &[u8]) -> String {
    crate::deploy_plan::sha256_hex(bytes)
}
// Synthetic local deployment metadata, admitted through the real record owner.
// It supplies the invite's owned public config, not evidence of a real deployment.
fn bootstrap_record(layout: &Layout) {
    let mut records = DeployRecordStore::open(layout, PROJECT, "fixture bootstrap").unwrap();
    let mut record = records.load_or_draft().unwrap();
    let html = b"<p>synthetic public configuration</p>";
    let manifest = HostingManifest::parse(&json!({"version":1,"files":[{
        "path":"/index.html","sha256":hash(html),"length":html.len(),"contentType":"text/html"}]}))
    .unwrap();
    let mut declaration: Value = serde_json::from_slice(include_bytes!(
        "../../tests/fixtures/declarations/colab-firestore.json"
    ))
    .unwrap();
    declaration["hosting"] = serde_json::to_value(&manifest).unwrap();
    let declaration = serde_json::to_vec(&declaration).unwrap();
    let plan = deploy_plan::compose(
        Target {
            backend: CloudBackend::Firestore,
            physical_ttl: false,
        },
        &[Enabled {
            name: "colab",
            supplied: Some(Supplied {
                declaration: &declaration,
                artifact: include_bytes!(
                    "../../tests/fixtures/declarations/rules/colab-admission.rules"
                ),
            }),
        }],
    )
    .unwrap();
    let bundle = HostingBundle::parse(&manifest, &serde_json::to_vec(&json!({"version":1,
        "manifestDigest":manifest.digest(),"files":[{"path":"/index.html","bytesBase64":STANDARD.encode(html)}]})).unwrap()).unwrap();
    let content = hosting::compose(&[hosting::HostingInput {
        extension: "colab",
        declaration: &declaration,
        bundle: &bundle,
    }])
    .unwrap()
    .unwrap();
    let hosted = hosting::deployment_view(
        &content,
        PROJECT,
        &record.deployment_id,
        &HostingInventory {
            site_exists: false,
            site_app_id: None,
            site_app_config: None,
            web_apps: vec![],
            live: None,
        },
        PROJECT,
        None,
        true,
    )
    .unwrap();
    let prepared = deploy_run::prepare_with_hosting(
        &plan,
        &DeployInput {
            account: "fixture@example.test",
            project: PROJECT,
            deployment_id: &record.deployment_id,
            location: "us-central1",
            sign_in: &[SignInProvider::Anonymous],
            rules_body: crate::cloud_inbox::rules::artifact().as_bytes(),
            live_rules: None,
        },
        Some(&hosted),
    )
    .unwrap();
    let version = format!("sites/{PROJECT}/versions/fixture");
    let publication = serde_json::from_value(json!({"project":PROJECT,"region":"us-central1",
        "deploymentId":record.deployment_id,"planDigest":prepared.digest(),"appId":config()["appId"],
        "siteAppId":config()["appId"],"publicConfig":config(),"entryUrl":format!("https://{PROJECT}.web.app"),
        "version":version,"release":format!("sites/{PROJECT}/releases/fixture"),
        "rulesDigest":prepared.view().rules.digest,"contentDigest":hosted.content["digest"]})).unwrap();
    record.binding = Some(DeployBinding {
        plan_digest: prepared.digest().into(),
        project: PROJECT.into(),
        completed_at_ms: now(),
    });
    record.run = Some(Run {
        plan_digest: prepared.digest().into(),
        account: "fixture@example.test".into(),
        authorized_at_ms: now(),
        state: RunState::Complete,
        rules_attempted: true,
        steps: prepared
            .view()
            .steps
            .iter()
            .map(|id| StepRecord {
                id: id.clone(),
                state: StepState::Done,
                cause: None,
            })
            .collect(),
        hosting: Some(HostingCheckpoint {
            envelope: serde_json::to_value(prepared.view()).unwrap(),
            app_id: Some(config()["appId"].as_str().unwrap().into()),
            operation: None,
            version: Some(version),
            version_created_ms: Some(now()),
            publication: Some(publication),
        }),
    });
    records.bind_target(PROJECT, "us-central1").unwrap();
    records.persist(&record).unwrap();
    let selected = crate::remote_selector::client(layout, Some(PROJECT), None).unwrap();
    assert!(matches!(
        selected.kind,
        crate::remote_selector::ClientKind::Owned
    ));
    assert_eq!(selected.public_config, config());
}

fn absent(bytes: &[u8], secrets: &[Vec<u8>]) {
    for secret in secrets {
        assert!(
            !bytes.windows(secret.len()).any(|part| part == secret),
            "Invitation secret entered persisted/provider/log bytes"
        );
    }
}
fn assert_private_rows_and_logs(owner: &Service, core: &core_fixture::Core, secrets: &[Vec<u8>]) {
    assert!(
        core.calls().is_empty(),
        "Invitation management must not run a Core command"
    );
    let store = owner.store().unwrap();
    let path = std::path::PathBuf::from(store.connection.path().unwrap())
        .canonicalize()
        .unwrap();
    let private = std::path::PathBuf::from(std::env::var_os("TMT_HOME").expect("private TMT_HOME"))
        .canonicalize()
        .unwrap();
    assert!(
        private
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("tmt-remote-private-")
    );
    assert!(private.starts_with(std::env::temp_dir().canonicalize().unwrap()));
    assert!(path.starts_with(&private));
    let mut names = store
        .connection
        .prepare("SELECT name FROM sqlite_master WHERE type='table'")
        .unwrap();
    let tables = names
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    drop(names);
    for table in tables {
        let mut query = store
            .connection
            .prepare(&format!(
                "SELECT * FROM \"{}\"",
                table.replace('\"', "\"\"")
            ))
            .unwrap();
        let count = query.column_count();
        let mut rows = query.query([]).unwrap();
        while let Some(row) = rows.next().unwrap() {
            for i in 0..count {
                match row.get_ref(i).unwrap() {
                    rusqlite::types::ValueRef::Text(bytes)
                    | rusqlite::types::ValueRef::Blob(bytes) => absent(bytes, secrets),
                    _ => {}
                }
            }
        }
    }
    drop(store);
    let mut dirs = vec![core.root.clone(), private];
    while let Some(path) = dirs.pop() {
        for entry in std::fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dirs.push(path);
            } else {
                absent(&std::fs::read(path).unwrap(), secrets);
            }
        }
    }
}
#[test]
fn browser_create_is_once_only_and_cancel_history_has_no_secret_or_replay() {
    crate::private_test_case::private_case(
        "cloud_pairing::service::tests::browser_issuance_tests::browser_create_is_once_only_and_cancel_history_has_no_secret_or_replay",
        |private| {
            let core = core_fixture::Core::new();
            let wire = Arc::new(Wire::new());
            let ports = Arc::new(Ports {
                wire: wire.clone(),
                clock: SystemClock::default(),
            });
            let (owner, lease) = service(
                &private.join("issuer"),
                &core,
                ProjectPorts {
                    http: ports.clone(),
                    clock: ports.clone(),
                    jitter: ports,
                },
            );
            bootstrap_record(&owner.layout);
            let mut browser = Browser::new(&owner);
            let id = crate::store::uuid_v4().unwrap();
            let input = json!({"operationId":id});
            let created = browser
                .request(&owner, "remote.invites.create", input.clone())
                .0;
            assert_eq!(created["state"], "committed");
            let offer = created["result"]["invitation"]["offerId"]
                .as_str()
                .unwrap()
                .to_owned();
            assert_ne!(offer, id);
            let token = created["secret"]["code"]
                .as_str()
                .expect("first creation returns one code")
                .to_owned();
            let invite = wire::v2::Invite::read(&token, now()).unwrap();
            assert_eq!(invite.context.offer, offer);
            assert_eq!(invite.context.owner, owner.machine);
            assert_eq!(invite.public_config, config());
            let mut secrets = vec![token.as_bytes().to_vec()];
            invite.with_code(|code| {
                secrets.push(code.to_vec());
                secrets.push(crate::canonical::base64url(code).into_bytes());
            });
            assert_private_rows_and_logs(&owner, &core, &secrets);
            absent(
                &serde_json::to_vec(&*wire.commits.lock().unwrap()).unwrap(),
                &secrets,
            );
            let commits = wire.commits.lock().unwrap().len();
            // Treat the creating response as lost: only the metadata original is recoverable.
            let recovered = browser
                .request(
                    &owner,
                    "remote.management.operation",
                    json!({"operationId":id}),
                )
                .0;
            assert_eq!(recovered["state"], "committed");
            assert!(recovered.get("secret").is_none());
            absent(&serde_json::to_vec(&recovered).unwrap(), &secrets);
            let replay = browser.request(&owner, "remote.invites.create", input).0;
            assert_eq!(replay, recovered);
            assert_eq!(wire.commits.lock().unwrap().len(), commits);
            let page = browser
                .request(&owner, "remote.invites.list", json!({"cursor":null}))
                .0;
            assert_eq!(page["invitations"][0]["offerId"], offer);
            absent(&serde_json::to_vec(&page).unwrap(), &secrets);
            let cancel_input =
                json!({"operationId":crate::store::uuid_v4().unwrap(),"offerId":offer});
            let cancelled = browser
                .request(&owner, "remote.invites.cancel", cancel_input.clone())
                .0;
            assert_eq!(
                cancelled["result"]["invitation"]["code"],
                "REMOTE_INVITATION_CANCELLED"
            );
            let commits_after_cancel = wire.commits.lock().unwrap().len();
            assert_eq!(
                browser
                    .request(&owner, "remote.invites.cancel", cancel_input.clone())
                    .0,
                cancelled
            );
            assert_eq!(wire.commits.lock().unwrap().len(), commits_after_cancel);
            let mut conflict = cancel_input;
            conflict["offerId"] = json!(crate::store::uuid_v4().unwrap());
            assert_eq!(
                browser.request(&owner, "remote.invites.cancel", conflict).0["error"]["code"],
                "REMOTE_INTENT_CONFLICT"
            );
            for offer in [offer.clone(), crate::store::uuid_v4().unwrap()] {
                let unchanged = browser
                    .request(
                        &owner,
                        "remote.invites.cancel",
                        json!({"operationId":crate::store::uuid_v4().unwrap(),"offerId":offer}),
                    )
                    .0;
                assert_eq!(
                    unchanged["result"]["invitation"]["code"],
                    "REMOTE_INVITATION_NOT_ACTIVE"
                );
            }
            let refused = browser
                .request(
                    &owner,
                    "remote.invites.create",
                    json!({"operationId":crate::store::uuid_v4().unwrap(),"selector":"other"}),
                )
                .0;
            assert_eq!(refused["error"]["code"], "REMOTE_INPUT_INVALID");
            assert_private_rows_and_logs(&owner, &core, &secrets);
            absent(
                &serde_json::to_vec(&*wire.commits.lock().unwrap()).unwrap(),
                &secrets,
            );
            owner.store().unwrap().undesignate().unwrap();
            let refused = browser
                .request(
                    &owner,
                    "remote.invites.create",
                    json!({"operationId":crate::store::uuid_v4().unwrap()}),
                )
                .0;
            assert_eq!(refused["error"]["code"], "REMOTE_MANAGEMENT_READ_ONLY");
            drop(invite);
            drop(owner);
            drop(lease);
        },
    );
}

#[test]
fn browser_created_offer_confirms_through_the_original_journal_and_retains_known_history() {
    crate::private_test_case::private_case(
        "cloud_pairing::service::tests::browser_issuance_tests::browser_created_offer_confirms_through_the_original_journal_and_retains_known_history",
        |private| {
            let core = core_fixture::Core::new();
            let wire = Arc::new(Wire::new());
            let ports = Arc::new(Ports {
                wire: wire.clone(),
                clock: SystemClock::default(),
            });
            let make = || ProjectPorts {
                http: ports.clone(),
                clock: ports.clone(),
                jitter: ports.clone(),
            };
            let (owner, owner_lease) = service(&private.join("issuer"), &core, make());
            let (peer, peer_lease) = service(&private.join("guest"), &core, make());
            bootstrap_record(&owner.layout);
            let mut browser = Browser::new(&owner);
            let creation_id = crate::store::uuid_v4().unwrap();
            let issued = browser
                .request(
                    &owner,
                    "remote.invites.create",
                    json!({"operationId":creation_id}),
                )
                .0;
            let token = issued["secret"]["code"].as_str().unwrap().to_owned();
            let invite = wire::v2::Invite::read(&token, now()).unwrap();
            assert_ne!(
                invite.context.offer, creation_id,
                "creation must not occupy the confirmation original ID"
            );
            let provider = Arc::new(peer.provider(&config(), &owner.machine, true).unwrap());
            let principal = provider.uid().as_bytes().to_vec();
            let (backend, _, _) = peer
                .invitation_backend(provider, &invite.context, &config(), &[], &peer.io())
                .unwrap();
            let candidate = crate::invitations::Candidate {
                machine: peer.machine.clone(),
                public_key: peer.key.public(),
                principal,
                name: "Guest".into(),
                nonce: [5; 16],
            };
            let mut attempt = invite
                .with_code(|code| {
                    crate::invitations::submission::SubmissionAttempt::new(
                        backend,
                        invite.context.clone(),
                        candidate,
                        code,
                        &peer.key,
                        now(),
                    )
                })
                .unwrap();
            attempt.submit(&peer.io()).unwrap();
            // The existing reconciliation entry observes the offer; no terminal presenter exists.
            owner.reconcile().unwrap();
            let pending = browser
                .request(&owner, "remote.invites.pending", json!({}))
                .0;
            assert_eq!(pending["phase"], "awaiting-owner");
            wire.receipt_answers
                .lose_next
                .store(true, Ordering::Release);
            let confirmed=browser.request(&owner,"remote.invites.confirm",json!({"operationId":crate::store::uuid_v4().unwrap(),"offerId":invite.context.offer,"snapshot":pending["candidate"]["snapshot"],"matched":true})).0;
            assert_eq!(confirmed["state"], "committed");
            assert_eq!(confirmed["result"]["invitation"]["state"], "confirmed");
            let known: bool = owner
                .store()
                .unwrap()
                .connection
                .query_row(
                    "SELECT confirmation_known FROM issued_invitations WHERE offer_id=?1",
                    [&invite.context.offer],
                    |r| r.get(0),
                )
                .unwrap();
            assert!(
                known,
                "known confirmation is durable before any status/list refresh"
            );
            assert!(
                !owner
                    .store()
                    .unwrap()
                    .grant(&peer.machine)
                    .unwrap()
                    .unwrap()
                    .permits_scope("talk")
            );
            let original = browser
                .request(
                    &owner,
                    "remote.management.operation",
                    json!({"operationId":creation_id}),
                )
                .0;
            assert!(original.get("secret").is_none());
            let no_op = browser
        .request(
            &owner,
            "remote.invites.cancel",
            json!({"operationId":crate::store::uuid_v4().unwrap(),"offerId":invite.context.offer}),
        )
        .0;
            assert_eq!(
                no_op["result"]["invitation"]["code"],
                "REMOTE_INVITATION_NOT_ACTIVE"
            );
            assert_eq!(owner.store().unwrap().grants().unwrap().len(), 2);
            let mut secrets = vec![token.into_bytes()];
            invite.with_code(|code| {
                secrets.push(code.to_vec());
                secrets.push(crate::canonical::base64url(code).into_bytes());
            });
            assert_private_rows_and_logs(&owner, &core, &secrets);
            drop(attempt);
            drop(invite);
            drop(peer);
            drop(owner);
            drop(peer_lease);
            drop(owner_lease);
        },
    );
}

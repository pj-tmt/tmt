//! Public owner commands use a private Store and the actual running/stopped control paths.
use super::*;
use tmt_remote::{
    members::Member,
    state::Layout,
    store::{DEFAULT_SCOPES, Grant, Store, uuid_v4},
};

pub(super) fn seed_member(pilot: &Pilot) -> (Grant, Member) {
    seed_kind(pilot, "cli")
}
fn seed_kind(pilot: &Pilot, kind: &str) -> (Grant, Member) {
    let serving = Layout::open(&pilot.root.join("state"))
        .unwrap()
        .serve_lock()
        .unwrap();
    let mut store = Store::open(&serving).unwrap();
    store.machine().unwrap();
    let mut seed = [0u8; 32];
    getrandom::fill(&mut seed).unwrap();
    let grant = Grant {
        client_id: uuid_v4().unwrap(),
        public_key: ed25519_dalek::SigningKey::from_bytes(&seed)
            .verifying_key()
            .to_bytes(),
        kind: kind.into(),
        origin: if kind == "browser" {
            format!(
                "http://127.0.0.1:{}",
                store.remembered_port().unwrap().unwrap()
            )
        } else {
            "cli".into()
        },
        name: if kind == "browser" {
            "Owner browser".into()
        } else {
            "Person".into()
        },
        agents: "all".into(),
        scopes: DEFAULT_SCOPES.map(str::to_owned).into(),
        mode: "hold".into(),
        issued_at_ms: 1,
        expires_at_ms: None,
        revision: 1,
        disabled: false,
    };
    store.insert_grant(&grant).unwrap();
    store.rename(&grant.client_id, "Laptop").unwrap().unwrap();
    let member = store.member_for_client(&grant.client_id).unwrap().unwrap();
    (grant, member)
}
pub(super) fn json_command(pilot: &Pilot, words: &[&str], accepted: bool) -> Value {
    let output = pilot.command().args(words).output().unwrap();
    assert_eq!(
        output.status.success(),
        accepted,
        "{words:?}: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
fn revision(value: &Value) -> String {
    value["member"]["revision"].as_u64().unwrap().to_string()
}
fn original_rows(pilot: &Pilot) -> (i64, String, i64, i64) {
    let sql = rusqlite::Connection::open(pilot.root.join("state/remote/remote.db")).unwrap();
    (
        sql.query_row("SELECT COUNT(*) FROM management_receipts", [], |r| r.get(0))
            .unwrap(),
        sql.query_row("SELECT name FROM grants", [], |r| r.get(0))
            .unwrap(),
        sql.query_row("SELECT revision FROM grants", [], |r| r.get(0))
            .unwrap(),
        sql.query_row("SELECT disabled FROM grants", [], |r| r.get(0))
            .unwrap(),
    )
}
#[test]
fn public_member_effects_are_revision_fenced_with_no_terminal_receipts() {
    let mut pilot = Pilot::new();
    let (grant, member) = seed_member(&pilot);
    let id = member.member_id.as_str();
    let first = json_command(
        &pilot,
        &["members", "--json", "rename", id, "Renamed person"],
        true,
    );
    assert_eq!(first["state"], "committed");
    assert_eq!(first["changed"], true);
    assert_eq!(first["member"]["displayName"], "Renamed person");
    assert!(first.get("operationId").is_none());
    let rev = revision(&first);
    let same = json_command(
        &pilot,
        &[
            "members",
            "rename",
            id,
            "Renamed person",
            "--member-revision",
            &rev,
            "--json",
        ],
        true,
    );
    assert_eq!(same["changed"], false);
    assert_eq!(revision(&same), rev);
    let before = original_rows(&pilot);
    let stale = json_command(
        &pilot,
        &[
            "members",
            "sending",
            id,
            "on",
            "--member-revision",
            "1",
            "--json",
        ],
        false,
    );
    assert_eq!(stale["error"]["code"], "REMOTE_MEMBER_CONFLICT");
    assert_eq!(original_rows(&pilot), before);
    assert_eq!(before.0, 0);
    assert_eq!(before.1, "Laptop");
    start_door(&mut pilot, &["--background"], false);
    let enabled = json_command(
        &pilot,
        &[
            "members",
            "sending",
            "Renamed person",
            "on",
            "--member-revision",
            &rev,
            "--json",
        ],
        true,
    );
    assert_eq!(enabled["member"]["sendingEnabled"], true);
    let rev = revision(&enabled);
    let same = json_command(
        &pilot,
        &[
            "members",
            "sending",
            id,
            "on",
            "--member-revision",
            &rev,
            "--json",
        ],
        true,
    );
    assert_eq!(same["changed"], false);
    assert_eq!(revision(&same), rev);
    let machine = json_command(&pilot, &["members", "machines", id, "--json"], true);
    assert_eq!(machine["machines"][0]["delivery"], "hold");
    assert_eq!(machine["machines"][0]["sendingScope"], true);
    let disabled = json_command(
        &pilot,
        &[
            "members",
            "sending",
            id,
            "off",
            "--member-revision",
            &rev,
            "--json",
        ],
        true,
    );
    assert_eq!(disabled["member"]["sendingEnabled"], false);
    let rev = revision(&disabled);
    let revoked = json_command(
        &pilot,
        &["members", "revoke", id, "--member-revision", &rev, "--json"],
        true,
    );
    assert_eq!(revoked["changed"], true);
    let old = rev;
    let rev = revision(&revoked);
    let again = json_command(
        &pilot,
        &["members", "revoke", id, "--member-revision", &rev, "--json"],
        true,
    );
    assert_eq!(again["alreadyRevoked"], true);
    assert_eq!(again["changed"], false);
    assert_eq!(revision(&again), rev);
    let before = original_rows(&pilot);
    assert_eq!(
        json_command(
            &pilot,
            &["members", "revoke", id, "--member-revision", &old, "--json"],
            false
        )["error"]["code"],
        "REMOTE_MEMBER_CONFLICT"
    );
    assert_eq!(original_rows(&pilot), before);
    assert_eq!(before.0, 0);
    assert_eq!(before.1, "Laptop");
    assert_eq!(before.3, 1);
    assert_eq!(stop_json(&pilot), serde_json::json!({"stopped":true}));
    wait_stopped(pilot.child.as_mut().unwrap());
    let machines = json_command(&pilot, &["members", "machines", id, "--json"], true);
    assert_eq!(machines["machines"][0]["clientId"], grant.client_id);
    assert_eq!(machines["machines"][0]["revoked"], true);
    assert_eq!(machines["machines"][0]["grantLive"], false);
    assert!(machines["machines"][0]["liveSessionCount"].is_null());
    assert_eq!(original_rows(&pilot).0, 0);
}

#[test]
fn malformed_member_mutation_answer_is_unconfirmed_without_resend() {
    let pilot = Pilot::new();
    let (_, member) = seed_member(&pilot);
    let before = original_rows(&pilot);
    let socket = pilot.root.join("state/remote/control.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = Arc::clone(&stop);
    let marker = pilot.root.join("mutation-accepted");
    let marker_written = marker.clone();
    let target = serde_json::json!({"remoteId":member.remote_id,"memberId":member.member_id,"memberRevision":member.revision});
    let server = std::thread::spawn(move || {
        let until = Instant::now() + Duration::from_secs(10);
        let mut calls = Vec::new();
        while calls.len() < 2 && !worker_stop.load(Ordering::Relaxed) {
            let (mut stream, _) = match listener.accept() {
                Ok(value) => value,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        Instant::now() < until,
                        "private member control caller did not arrive"
                    );
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                }
                Err(error) => panic!("private member accept: {error}"),
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut line = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            calls.push(request["op"].as_str().unwrap().to_owned());
            if calls.len() == 1 {
                assert_eq!(request["op"], "members.read");
                assert_eq!(request["input"]["view"], "resolve");
                writeln!(stream, "{}", serde_json::json!({"target":target})).unwrap();
            } else {
                assert_eq!(request["op"], "members.mutate");
                assert_eq!(request["input"]["action"], "sending");
                assert_eq!(request["input"]["target"], target);
                fs::write(&marker_written, "accepted original request").unwrap();
                // An accepted request with no verifiable final is uncertain, never refused.
                writeln!(
                    stream,
                    "{{\"state\":\"committed\",\"changed\":true,\"member\":{{}}}}"
                )
                .unwrap();
            }
        }
        calls
    });
    let answer = pilot
        .command()
        .args(["members", "sending", &member.member_id, "on", "--json"])
        .output()
        .unwrap();
    stop.store(true, Ordering::Relaxed);
    let calls = server.join().unwrap();
    fs::remove_file(socket).unwrap();
    assert!(!answer.status.success());
    let answer: Value = serde_json::from_slice(&answer.stdout).unwrap();
    assert_eq!(answer["error"]["code"], "REMOTE_MEMBER_UNCONFIRMED");
    assert_eq!(calls, ["members.read", "members.mutate"]);
    assert!(marker.exists());
    assert_eq!(
        original_rows(&pilot),
        before,
        "an invalid live-owner answer never falls back to another Store writer"
    );
}

#[test]
fn public_exact_machine_commands_and_offline_browser_recovery_preserve_receipts() {
    use tmt_remote::members::{Association, Owner};
    fn journal(pilot: &Pilot) -> Vec<Vec<rusqlite::types::Value>> {
        let sql = rusqlite::Connection::open(pilot.root.join("state/remote/remote.db")).unwrap();
        let mut query = sql
            .prepare("SELECT * FROM management_receipts ORDER BY id")
            .unwrap();
        let columns = query.column_count();
        query
            .query_map([], |row| (0..columns).map(|i| row.get(i)).collect())
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }
    let mut pilot = Pilot::new();
    let (first, member) = seed_member(&pilot);
    let (second, _) = seed_member(&pilot);
    {
        let serving = Layout::open(&pilot.root.join("state"))
            .unwrap()
            .serve_lock()
            .unwrap();
        let mut store = Store::open(&serving).unwrap();
        store
            .associate_member(
                Owner::LocalOwner,
                &Association {
                    operation_id: uuid_v4().unwrap(),
                    remote_id: member.remote_id,
                    member_id: member.member_id.clone(),
                    member_revision: member.revision,
                    client_id: second.client_id.clone(),
                    grant_revision: second.revision,
                },
                1,
            )
            .unwrap();
    }
    let baseline = journal(&pilot);
    assert_eq!(baseline.len(), 1);
    let renamed = json_command(
        &pilot,
        &[
            "members",
            "rename-machine",
            &second.client_id,
            "Travel laptop",
            "--json",
        ],
        true,
    );
    assert_eq!(renamed["changed"], true);
    let stale = json_command(
        &pilot,
        &[
            "members",
            "remove-machine",
            &second.client_id,
            "--grant-revision",
            "1",
            "--json",
        ],
        false,
    );
    assert_eq!(stale["error"]["code"], "REMOTE_MEMBER_CONFLICT");
    let stale = json_command(
        &pilot,
        &[
            "members",
            "remove-machine",
            &second.client_id,
            "--member-revision",
            "1",
            "--json",
        ],
        false,
    );
    assert_eq!(stale["error"]["code"], "REMOTE_MEMBER_CONFLICT");
    let removed = json_command(
        &pilot,
        &["members", "remove-machine", &second.client_id, "--json"],
        true,
    );
    assert_eq!(removed["changed"], true);
    let same = json_command(
        &pilot,
        &["members", "remove-machine", &second.client_id, "--json"],
        true,
    );
    assert_eq!(same["changed"], false);
    let machines = json_command(
        &pilot,
        &["members", "machines", &member.member_id, "--json"],
        true,
    );
    let rows = machines["machines"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows.iter()
            .find(|m| m["clientId"] == first.client_id)
            .unwrap()["revoked"],
        false
    );
    assert_eq!(
        rows.iter()
            .find(|m| m["clientId"] == second.client_id)
            .unwrap()["revoked"],
        true
    );
    let roster = json_command(&pilot, &["members", "--json"], true);
    let person = roster["members"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["memberId"] == member.member_id)
        .unwrap();
    assert_eq!(person["displayName"], "Person");
    assert!(person["revokedAtMs"].is_null());
    start_door(&mut pilot, &["--background"], false);
    assert_eq!(
        json_command(
            &pilot,
            &["members", "designate", &first.client_id, "--json"],
            false
        )["error"]["code"],
        "REMOTE_INPUT_INVALID"
    );
    assert_eq!(
        json_command(&pilot, &["members", "undesignate", "--json"], true)["changed"],
        false
    );
    assert_eq!(
        json_command(
            &pilot,
            &[
                "members",
                "rename-machine",
                &first.client_id,
                "Live laptop",
                "--json"
            ],
            true
        )["machine"]["name"],
        "Live laptop"
    );
    assert_eq!(stop_json(&pilot), serde_json::json!({"stopped":true}));
    wait_stopped(pilot.child.as_mut().unwrap());
    let (browser, _) = seed_kind(&pilot, "browser");
    let designated = json_command(
        &pilot,
        &["members", "designate", &browser.client_id, "--json"],
        true,
    );
    assert_eq!(designated["designatedClientId"], browser.client_id);
    assert_eq!(designated["changed"], true);
    let legacy = pilot
        .command()
        .args(["devices", "designate", &browser.client_id, "--json"])
        .output()
        .unwrap();
    assert!(legacy.status.success());
    assert_eq!(
        String::from_utf8(legacy.stdout).unwrap(),
        format!(
            "{}\n",
            serde_json::json!({"designatedClientId":browser.client_id})
        )
    );
    let same = json_command(
        &pilot,
        &["members", "designate", &browser.client_id, "--json"],
        true,
    );
    assert_eq!(same["changed"], false);
    let stale = json_command(
        &pilot,
        &["members", "undesignate", "--member-revision", "1", "--json"],
        false,
    );
    assert_eq!(stale["error"]["code"], "REMOTE_MEMBER_CONFLICT");
    let cleared = json_command(&pilot, &["members", "undesignate", "--json"], true);
    assert_eq!(cleared["changed"], true);
    assert!(cleared["designatedClientId"].is_null());
    assert_eq!(
        json_command(&pilot, &["members", "undesignate", "--json"], true)["changed"],
        false
    );
    assert_eq!(
        journal(&pilot),
        baseline,
        "terminal commands preserve every old receipt column and byte"
    );
}

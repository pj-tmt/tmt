//! Public refusals and expired observations must not become authority or another effect.
use super::{
    member_commands::{json_command, seed_member},
    *,
};
use tmt_remote::state::Layout;

fn durable_rows(pilot: &Pilot) -> Vec<Vec<Vec<rusqlite::types::Value>>> {
    let sql = rusqlite::Connection::open(pilot.root.join("state/remote/remote.db")).unwrap();
    [
        "grants",
        "members",
        "member_machines",
        "management_receipts",
    ]
    .iter()
    .map(|table| {
        let mut query = sql
            .prepare(&format!("SELECT * FROM {table} ORDER BY 1,2"))
            .unwrap();
        let columns = query.column_count();
        query
            .query_map([], |row| (0..columns).map(|i| row.get(i)).collect())
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    })
    .collect()
}

#[test]
fn ambiguous_and_foreign_members_commands_refuse_without_durable_changes() {
    let mut pilot = Pilot::new();
    let (first, member) = seed_member(&pilot);
    seed_member(&pilot);
    let layout = Layout::open(&pilot.root.join("state")).unwrap();
    tmt_remote::remote_catalog::add(&layout, "elsewhere", "http://127.0.0.1:9/").unwrap();
    let before = durable_rows(&pilot);
    for running in [false, true] {
        if running {
            start_door(&mut pilot, &["--background"], false);
        }
        for words in [
            vec!["members", "machines", "Person", "--json"],
            vec!["members", "rename", "Person", "Unexpected", "--json"],
            vec!["members", "sending", "Person", "on", "--json"],
            vec!["members", "revoke", "Person", "--json"],
        ] {
            assert_eq!(
                json_command(&pilot, &words, false)["error"]["code"],
                "REMOTE_MEMBER_AMBIGUOUS"
            );
        }
        for (selected, code) in [
            ("elsewhere", "REMOTE_SELECTOR_LOCAL_REQUIRED"),
            ("absent", "REMOTE_SELECTOR_NOT_FOUND"),
        ] {
            assert_eq!(
                json_command(
                    &pilot,
                    &[
                        "--remote",
                        selected,
                        "members",
                        "remove-machine",
                        &first.client_id,
                        "--json"
                    ],
                    false
                )["error"]["code"],
                code
            );
        }
        let machines = json_command(
            &pilot,
            &[
                "--remote",
                "local",
                "members",
                "machines",
                &member.member_id,
                "--json",
            ],
            true,
        );
        assert_eq!(machines["machines"][0]["clientId"], first.client_id);
        assert_eq!(durable_rows(&pilot), before);
    }
    assert_eq!(stop_json(&pilot), serde_json::json!({"stopped":true}));
    wait_stopped(pilot.child.as_mut().unwrap());
}

#[test]
fn expired_and_revoked_machine_reads_do_not_restore_access() {
    let pilot = Pilot::new();
    let (expired, member) = seed_member(&pilot);
    let (revoked, _) = seed_member(&pilot);
    {
        let sql = rusqlite::Connection::open(pilot.root.join("state/remote/remote.db")).unwrap();
        sql.execute(
            "UPDATE grants SET expires_at_ms=2 WHERE client_id=?1",
            [&expired.client_id],
        )
        .unwrap();
    }
    let answer = json_command(
        &pilot,
        &["members", "sending", &member.member_id, "on", "--json"],
        true,
    );
    assert_eq!(answer["member"]["sendingEnabled"], true);
    let machines = json_command(
        &pilot,
        &["members", "machines", &member.member_id, "--json"],
        true,
    );
    assert_eq!(machines["machines"][0]["expiresAtMs"], 2);
    assert_eq!(machines["machines"][0]["grantLive"], false);
    assert_eq!(machines["machines"][0]["revoked"], false);
    assert!(machines["machines"][0]["liveSessionCount"].is_null());
    json_command(
        &pilot,
        &["members", "remove-machine", &revoked.client_id, "--json"],
        true,
    );
    let before = durable_rows(&pilot);
    assert_eq!(
        json_command(
            &pilot,
            &["members", "sending", &revoked.client_id, "on", "--json"],
            false
        )["error"]["code"],
        "REMOTE_MEMBER_CONFLICT"
    );
    let machines = json_command(
        &pilot,
        &["members", "machines", &revoked.client_id, "--json"],
        true,
    );
    assert_eq!(machines["machines"][0]["revoked"], true);
    assert_eq!(machines["machines"][0]["grantLive"], false);
    assert_eq!(durable_rows(&pilot), before);
}

#[test]
fn invalid_member_input_and_page_options_refuse_without_effect() {
    let pilot = Pilot::new();
    let (grant, member) = seed_member(&pilot);
    let before = durable_rows(&pilot);
    for words in [
        vec!["members", "--limit", "0", "--json"],
        vec!["members", "--limit", "51", "--json"],
        vec![
            "members",
            "--limit",
            "1",
            "rename",
            &member.member_id,
            "Unexpected",
            "--json",
        ],
        vec!["members", "rename", &member.member_id, "", "--json"],
        vec!["members", "rename-machine", &grant.client_id, "", "--json"],
        vec![
            "members",
            "machines",
            &member.member_id,
            "--cursor",
            "invalid",
            "--json",
        ],
    ] {
        assert!(
            !pilot
                .command()
                .args(&words)
                .output()
                .unwrap()
                .status
                .success(),
            "{words:?}"
        );
        assert_eq!(durable_rows(&pilot), before);
    }
    for action in [
        "machines",
        "rename",
        "sending",
        "revoke",
        "rename-machine",
        "remove-machine",
        "designate",
        "undesignate",
    ] {
        let output = pilot
            .command()
            .args(["members", action, "--help"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{action}");
        assert!(String::from_utf8_lossy(&output.stdout).contains("members"));
    }
    assert_eq!(durable_rows(&pilot), before);
}

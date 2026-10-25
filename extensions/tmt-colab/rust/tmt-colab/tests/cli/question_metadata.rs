//! Native command scenarios, using the CLI suite's isolated root and immutable Core fixtures.
use super::*;
const AGENT: &str = "50000000-0000-1000-8000-000000000001";
const OTHER: &str = "50000000-0000-1000-8000-000000000002";
const QUESTION: &str = "00000000-0000-0000-0000-000000000001";
fn refused(pilot: &Pilot, args: &[&str], code: &str) -> Value {
    let output = pilot.command().args(args).output().unwrap();
    assert!(!output.status.success(), "{output:?}");
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["error"]["code"], code, "{value}");
    value
}
fn new_page(pilot: &Pilot) -> String {
    pilot.call(&[
        "page", "create", "--title", "Metadata", "--layout", "column", "--json",
    ])["pageId"]
        .as_str()
        .unwrap()
        .into()
}
fn add(pilot: &Pilot, page: &str) -> Value {
    pilot.call(&[
        "question",
        "add",
        page,
        "--id",
        QUESTION,
        "--agent",
        AGENT,
        "--title",
        "First question",
        "--json",
    ])
}
#[test]
fn metadata_registry_lifecycle_is_conditional_and_preserves_source_and_original_author() {
    for serving in [false, true] {
        let mut pilot = Pilot::new(None);
        let page = new_page(&pilot);
        if serving {
            pilot.start();
        }
        let before = pilot.call(&["page", "read", &page, "--json"]);
        let registered = add(&pilot, &page);
        assert_eq!(registered["changed"], true);
        assert_eq!(
            registered["identityVerified"], false,
            "stored reference reused while lookup is unavailable"
        );
        let repeated = add(&pilot, &page);
        assert_eq!(repeated["changed"], false);
        let source = format!(
            "<p>Intro</p>\n<tmt-question data-id=\"{QUESTION}\" data-kind=\"short\" data-agent=\"Fixture agent\"><p data-question>First question</p></tmt-question>"
        );
        let file = pilot.root.join("question.html");
        fs::write(&file, &source).unwrap();
        pilot.call(&[
            "page",
            "write",
            &page,
            "--file",
            file.to_str().unwrap(),
            "--json",
        ]);
        let read = pilot.call(&["page", "read", &page, "--json"]);
        assert_eq!(read["source"], source);
        assert_eq!(read["questions"][0]["state"], "open");
        assert_eq!(read["questionStates"][0]["state"], "open");
        refused(
            &pilot,
            &[
                "question", "handle", &page, QUESTION, "--task", "T-99", "--json",
            ],
            "COLAB_INPUT_INVALID",
        );
        pilot.call(&["question", "handle", &page, QUESTION, "--json"]);
        assert_eq!(
            pilot.call(&["question", "ls", &page, "--json"])["questions"][0]["state"],
            "handled"
        );
        pilot.call(&["question", "reopen", &page, QUESTION, "--json"]);
        assert_eq!(
            pilot.call(&["question", "ls", &page, "--json"])["questions"][0]["state"],
            "open"
        );
        fs::write(
            pilot.root.join("selected-identity"),
            json!({"identity":{"id":OTHER,"name":"New creator"}}).to_string(),
        )
        .unwrap();
        pilot.call(&["page", "creator", &page, "New creator", "--json"]);
        let changed = pilot.call(&["page", "read", &page, "--json"]);
        assert_eq!(
            changed["creator"]["agent"],
            json!({"id":OTHER,"name":"New creator"})
        );
        assert!(changed["creator"]["attribution"]["writerId"].is_string());
        assert_eq!(changed["originalAuthor"], before["originalAuthor"]);
        assert_eq!(changed["source"], source);
        let destination = pilot.root.join("metadata-export");
        fs::create_dir(&destination).unwrap();
        let exported = pilot.call(&[
            "export",
            &page,
            "--dir",
            destination.to_str().unwrap(),
            "--json",
        ]);
        let manifest: Value = serde_json::from_slice(
            &fs::read(PathBuf::from(exported["directory"].as_str().unwrap()).join("manifest.json"))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(manifest["creator"], changed["creator"]);
        assert_eq!(manifest["questions"], changed["questions"]);
        assert_eq!(manifest["originalAuthor"], before["originalAuthor"]);
        assert!(manifest.get("questionStates").is_none());
        // A malformed selected-ID reply must not bind a different identity to an explicit UUID.
        add(&pilot, &page);
        assert_eq!(
            pilot.call(&["question", "ls", &page, "--json"])["questions"][0]["agent"]["id"],
            AGENT
        );

        refused(
            &pilot,
            &[
                "page",
                "creator",
                &page,
                "New creator",
                "--expected-revision",
                before["revision"].as_str().unwrap(),
                "--json",
            ],
            "COLAB_STALE_BASE",
        );
    }
}
#[test]
fn authored_file_mismatches_refuse_before_create_and_before_write_without_partial_state() {
    let pilot = Pilot::new(None);
    let file = pilot.root.join("authored.html");
    let source = format!(
        "<p>Intro</p>\n<tmt-question data-id=\"{QUESTION}\" data-kind=\"short\" data-agent=\"Fixture agent\"><p data-question>First question</p></tmt-question>"
    );
    fs::write(&file, &source).unwrap();
    let refusal = refused(
        &pilot,
        &[
            "page",
            "create",
            "--title",
            "Authored",
            "--layout",
            "column",
            "--file",
            file.to_str().unwrap(),
            "--json",
        ],
        "COLAB_QUESTION_UNREGISTERED",
    );
    let message = refusal["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("page create")
            && message.contains("question add PAGE")
            && message.contains("--title 'First question'")
            && message.contains("page write PAGE")
    );
    assert!(!pilot.root.join("selected/colab").exists());
    refused(
        &pilot,
        &[
            "page",
            "create",
            "--title",
            "Strict authored",
            "--layout",
            "column",
            "--file",
            file.to_str().unwrap(),
            "--strict",
            "--json",
        ],
        "COLAB_QUESTION_UNREGISTERED",
    );
    assert!(!pilot.root.join("selected/colab").exists());
    let page = new_page(&pilot);
    let before = pilot.call(&["page", "read", &page, "--json"]);
    for (body, code) in [
        (source.clone(), "COLAB_QUESTION_UNREGISTERED"),
        (
            source.replace("Fixture agent", "Someone else"),
            "COLAB_QUESTION_AGENT_MISMATCH",
        ),
    ] {
        if code.ends_with("MISMATCH") {
            add(&pilot, &page);
        }
        fs::write(&file, body).unwrap();
        let refusal = refused(
            &pilot,
            &[
                "page",
                "write",
                &page,
                "--file",
                file.to_str().unwrap(),
                "--json",
            ],
            code,
        );
        assert_eq!(refusal["findings"][0]["line"], 2);
        assert!(
            refusal["findings"][0]["fix"]
                .as_str()
                .unwrap()
                .contains("question add")
        );
        let checked = refused(
            &pilot,
            &[
                "page",
                "check",
                file.to_str().unwrap(),
                "--page",
                &page,
                "--json",
            ],
            code,
        );
        assert_eq!(checked["findings"][0]["line"], 2);
        assert_eq!(
            pilot.call(&["page", "read", &page, "--json"])["source"],
            before["source"]
        );
    }
    // A markup-only check needs neither a root nor Core identity discovery.
    let offline = pilot
        .command()
        .env_remove("TMT_EXECUTABLE")
        .args(["page", "check", file.to_str().unwrap(), "--json"])
        .output()
        .unwrap();
    assert!(offline.status.success(), "{offline:?}");
}
#[test]
fn missing_creator_and_unavailable_name_lookup_offer_repairs_before_local_initialization() {
    let pilot = Pilot::new(None);
    fs::remove_file(pilot.root.join("publisher")).unwrap();
    refused(
        &pilot,
        &[
            "page",
            "create",
            "--title",
            "No identity",
            "--layout",
            "column",
            "--json",
        ],
        "COLAB_CREATOR_REQUIRED",
    );
    let refusal = refused(
        &pilot,
        &[
            "page",
            "create",
            "--title",
            "No identity",
            "--layout",
            "column",
            "--creator",
            "new name",
            "--json",
        ],
        "COLAB_IDENTITY_UNVERIFIED",
    );
    let message = refusal["error"]["message"].as_str().unwrap();
    assert!(message.contains("--creator UUID") && message.contains("--creator 'new name'"));
    assert!(!pilot.root.join("selected/colab").exists());
    let created = pilot.call(&[
        "page",
        "create",
        "--title",
        "Explicit ID",
        "--layout",
        "column",
        "--creator",
        AGENT,
        "--json",
    ]);
    assert_eq!(created["identityVerified"], false);
    let read = pilot.call(&[
        "page",
        "read",
        created["pageId"].as_str().unwrap(),
        "--json",
    ]);
    assert_eq!(read["creator"]["agent"]["id"], AGENT);
    assert!(read.get("originalAuthor").is_none());
}
#[test]
fn compiled_templates_create_their_questions_and_creator_in_one_commit() {
    for template in ["review", "compare"] {
        let pilot = Pilot::new(None);
        let created = pilot.call(&[
            "page",
            "create",
            "--title",
            "First party",
            "--template",
            template,
            "--json",
        ]);
        let read = pilot.call(&[
            "page",
            "read",
            created["pageId"].as_str().unwrap(),
            "--json",
        ]);
        let source = read["source"].as_str().unwrap();
        let report = tmt_colab::page_check::check(source).unwrap();
        assert!(!report.questions.is_empty());
        let registry = read["questions"].as_array().unwrap();
        assert_eq!(registry.len(), report.questions.len());
        for question in report.questions {
            assert_eq!(question.agent, "Fixture agent");
            assert!(registry.iter().any(|entry| entry["id"] == question.id
                && entry["title"] == question.title
                && entry["agent"]["id"] == AGENT
                && entry["state"] == "open"));
        }
        assert_eq!(created["membershipHead"]["revision"], "2");
    }
}

#[test]
fn verified_own_answers_change_registry_state_without_content_or_ask_publication() {
    use tmt_colab::{
        decoder::{Decoder, OwnRecord},
        keyring::{Keyring, Layout},
        page,
        store::Store,
    };
    let pilot = Pilot::new(None);
    let id = new_page(&pilot);
    add(&pilot, &id);
    let initial = pilot.call(&["page", "read", &id, "--json"]);
    let thread = "10000000-0000-4000-8000-000000000002";
    let message = "10000000-0000-4000-8000-000000000003";
    // A genuine root-local own publication exercises signatures, fold provenance and the
    // production read projection. No Ask/intents/replies record is supplied.
    let answer = |revision: &str, body: &str, deleted: bool| {
        let layout = Layout::existing(&pilot.root.join("selected"))
            .unwrap()
            .unwrap();
        let _lock = layout.serve_lock().unwrap();
        let key = Keyring::read(&layout).unwrap();
        let writer = initial["creator"]["attribution"]["writerId"]
            .as_str()
            .unwrap();
        let mut store = Store::write_existing(&layout).unwrap();
        let mut decoder = Decoder::with_config(support::decoder_config(BINARY.into())).unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let scope = json!({"version":1,"spaceId":key.space_id,"pageId":id,"epoch":initial["epoch"],"senderDevice":writer,"revision":revision,"deleted":deleted,"deviceName":"Fixture person","at":now.to_string()});
        let mut comment = scope.clone();
        comment["kind"] = json!("comment");
        comment["messageId"] = json!(message);
        comment["thread"] = json!({"writer":writer,"id":thread});
        comment["questionId"] = json!(QUESTION);
        comment["body"] = json!(body);
        comment["sequence"] = json!("1");
        let mut records = vec![];
        if revision == "1" {
            let mut thread_record = scope;
            thread_record["kind"] = json!("thread");
            thread_record["threadId"] = json!(thread);
            thread_record["anchor"] = Value::Null;
            thread_record["resolved"] = json!(false);
            records.push(OwnRecord {
                root: "threads".into(),
                key: format!("{thread}:1"),
                value: thread_record,
            });
        }
        records.push(OwnRecord {
            root: "messages".into(),
            key: format!("{message}:{revision}"),
            value: comment,
        });
        let frozen =
            page::prepare_own_records(&store, &key, &id, &records, &mut decoder, now).unwrap();
        page::commit_publication(
            &mut store,
            &key,
            frozen.job(),
            frozen.packet(),
            frozen.chain(),
            now,
        )
        .unwrap();
        store.close().unwrap();
    };
    answer("1", "The first answer", false);
    let first = pilot.call(&["page", "read", &id, "--json"]);
    assert_eq!(first["source"], initial["source"]);
    assert_eq!(first["questions"], initial["questions"]);
    assert_eq!(first["questionStates"][0]["state"], "answered");
    assert_eq!(first["questionStates"][0]["answer"], "The first answer");
    assert_eq!(first["questionStates"][0]["answerRef"]["id"], message);
    answer("2", "The changed answer", false);
    let changed = pilot.call(&["question", "ls", &id, "--json"]);
    assert_eq!(changed["questions"][0]["answer"], "The changed answer");
    assert_eq!(changed["questions"][0]["answerRef"]["revision"], "2");
    pilot.call(&["question", "handle", &id, QUESTION, "--json"]);
    answer("3", "", true);
    assert_eq!(
        pilot.call(&["question", "ls", &id, "--json"])["questions"][0]["state"],
        "handled"
    );
    pilot.call(&["question", "reopen", &id, QUESTION, "--json"]);
    let reopened = pilot.call(&["question", "ls", &id, "--json"]);
    assert_eq!(reopened["questions"][0]["state"], "open");
    assert!(reopened["questions"][0].get("answerRef").is_none());
    assert_eq!(
        pilot.call(&["page", "read", &id, "--json"])["source"],
        initial["source"]
    );
}

#[test]
fn registry_repairs_preserve_leading_hyphen_names_and_titles_as_cli_values() {
    let pilot = Pilot::new(None);
    let page = new_page(&pilot);
    let file = pilot.root.join("hyphen.html");
    let source = format!(
        "<tmt-question data-id=\"{QUESTION}\" data-kind=\"short\" data-agent=\"-Agent\"><p data-question>-Title</p></tmt-question>"
    );
    fs::write(&file, &source).unwrap();
    let error = refused(
        &pilot,
        &[
            "page",
            "write",
            &page,
            "--file",
            file.to_str().unwrap(),
            "--json",
        ],
        "COLAB_QUESTION_UNREGISTERED",
    );
    let fix = error["findings"][0]["fix"].as_str().unwrap();
    assert!(
        fix.contains("--agent=-Agent") && fix.contains("--title=-Title"),
        "{fix}"
    );
    fs::write(
        pilot.root.join("selected-identity"),
        json!({"identity":{"id":OTHER,"name":"-Agent"}}).to_string(),
    )
    .unwrap();
    // Use the repaired option syntax as structured argv, never execute a repair string.
    pilot.call(&[
        "question",
        "add",
        &page,
        "--id",
        QUESTION,
        "--agent=-Agent",
        "--title=-Title",
        "--json",
    ]);
    pilot.call(&[
        "page",
        "write",
        &page,
        "--file",
        file.to_str().unwrap(),
        "--json",
    ]);
    pilot.call(&["page", "creator", &page, "--json", "--", "-Agent"]);
    assert_eq!(
        pilot.call(&["page", "read", &page, "--json"])["source"],
        source
    );
    fs::remove_file(pilot.root.join("selected-identity")).unwrap();
    let unavailable = refused(
        &pilot,
        &["page", "creator", &page, "--json", "--", "-New"],
        "COLAB_IDENTITY_UNVERIFIED",
    );
    assert!(
        unavailable["error"]["message"]
            .as_str()
            .unwrap()
            .contains(" -- -New")
    );
}

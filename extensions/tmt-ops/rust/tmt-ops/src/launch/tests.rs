use super::*;
#[test]
fn published_literals_and_each_slot_stay_one_word() {
    let argv = [
        "sh",
        "-c",
        "printf '%s' '#{pane_id}' ${value}",
        "launch",
        "{session}",
        "{command}",
    ]
    .map(str::to_owned);
    let command = line(&["tmt", "run", "a'b", "codex", "--model=a $b; c"].map(str::to_owned));
    let filled = fill(&argv, "my session", &command).unwrap();
    assert_eq!(filled.len(), 6);
    assert_eq!(filled[2], argv[2]);
    assert_eq!(filled[4], "my session");
    assert_eq!(filled[5], command);
    assert_eq!(command, "tmt run 'a'\\''b' codex '--model=a $b; c'");
    for session in ["", "a.b", "a:b", "a\n"] {
        assert!(fill(&argv, session, "tmt run n codex").is_err());
    }
}
#[test]
fn unavailable_states_never_default_and_published_order_is_preserved() {
    let template = serde_json::json!({"argv":["host","{session}","{command}"],"placeholders":["session","command"]});
    let hosts=hosts(&serde_json::json!({"version":1,"hosts":[{"host":"changed","state":"changed","launch":null},{"host":"herdr","state":"ok","launch":null},{"host":"first","state":"ok","launch":template},{"host":"tmux","state":"builtin","launch":template}]})).unwrap();
    assert_eq!(hosts.iter().find(|h| h.eligible()).unwrap().name, "first");
}

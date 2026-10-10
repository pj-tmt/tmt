use super::*;

#[test]
fn notes_path_has_one_typed_identity_selector_and_json_mode() {
    assert_eq!(
        parsed(&["notes", "path", "--identity", "Research & QA", "--json"]),
        Parsed {
            legacy_hook: false,
            invocation: Invocation::NotesPath {
                identity: Some("Research & QA".into()),
            },
            mode: OutputMode { json: true },
        }
    );
    assert_eq!(
        parsed(&["notes", "path"]).invocation,
        Invocation::NotesPath { identity: None }
    );
    assert_eq!(parse_error(&["notes"]).code, "USAGE_ERROR");
    assert_eq!(
        parse_error(&["notes", "path", "unexpected"]).code,
        "USAGE_ERROR"
    );
    assert_eq!(
        parse_error(&["notes", "path", "--force"]).code,
        "USAGE_ERROR"
    );
}

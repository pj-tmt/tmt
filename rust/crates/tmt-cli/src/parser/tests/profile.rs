use super::*;

#[test]
fn preamble_operands_join_without_reparsing_literal_content() {
    use crate::invocation::PreambleRequest;
    for (args, content) in [
        (
            vec!["preamble", "set", "Alice", "first", "second"],
            "first second",
        ),
        (
            vec!["preamble", "set", "Alice", "--", "--json", "literal"],
            "--json literal",
        ),
    ] {
        let parsed = parsed(&args);
        assert_eq!(
            parsed.invocation,
            Invocation::Preamble(PreambleRequest::Set {
                name: "Alice".into(),
                content: content.into()
            })
        );
        assert!(!parsed.mode.json);
    }
    assert_eq!(
        parsed(&["preamble"]).invocation,
        Invocation::Preamble(PreambleRequest::Show(None))
    );
    assert_eq!(
        parsed(&["preamble", "show", "Alice"]).invocation,
        Invocation::Preamble(PreambleRequest::Show(Some("Alice".into())))
    );
}

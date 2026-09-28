use super::*;

fn render<const N: usize>(headers: [&str; N], rows: Vec<[&str; N]>) -> String {
    let mut output = Vec::new();
    write(
        &mut output,
        headers,
        rows.into_iter().map(|row| row.map(str::to_owned)),
    )
    .unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn aligns_long_ascii_values_without_truncation() {
    assert_eq!(
        render(
            ["NAME", "STATE"],
            vec![["a", "offline"], ["long-agent-name", "active"]]
        ),
        "NAME             STATE\na                offline\nlong-agent-name  active\n"
    );
}

#[test]
fn measures_wide_combining_and_emoji_sequences_as_display_columns() {
    // Non-English fixture characters exercise terminal width, not translated UI.
    assert_eq!(
        render(
            ["NAME", "STATE"],
            vec![["測試", "wide"], ["e\u{301}", "combining"], ["👩‍💻", "emoji"]]
        ),
        "NAME  STATE\n測試  wide\ne\u{301}     combining\n👩‍💻    emoji\n"
    );
}

#[test]
fn escapes_controls_before_measuring_and_preserves_literal_backslashes() {
    assert_eq!(
        render(
            ["KEY", "VALUE"],
            vec![
                ["a\tb", "\u{1b}[31m\n\r\0\u{7f}\u{85}\u{2028}"],
                ["x", r"\n"]
            ]
        ),
        "KEY   VALUE\na\\tb  \\u{1b}[31m\\n\\r\\u{0}\\u{7f}\\u{85}\\u{2028}\nx     \\n\n"
    );
}

#[test]
fn retains_empty_middle_cells_without_trailing_padding() {
    assert_eq!(
        render(
            ["A", "B", "C"],
            vec![["x", "", "z"], ["y", "", ""], ["", "", ""]]
        ),
        "A  B  C\nx     z\ny\n\n"
    );
    assert_eq!(render(["A", "B"], vec![]), "A  B\n");
}

#[test]
fn propagates_output_errors() {
    struct Broken;
    impl Write for Broken {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::ErrorKind::BrokenPipe.into())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    assert_eq!(
        write(&mut Broken, ["NAME"], Vec::<[String; 1]>::new())
            .unwrap_err()
            .kind(),
        io::ErrorKind::BrokenPipe
    );
}

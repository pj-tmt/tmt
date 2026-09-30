use super::*;

fn parse(text: &str) -> Result<Rows, SquadError> {
    let config: toml_edit::DocumentMut = text.parse().unwrap();
    read(config["p"].as_table_like(), "p")
}

fn error(text: &str) -> String {
    let error = parse(text).expect_err(text);
    assert_eq!(error.code, "SQUAD_CONFIG_INVALID", "{text}");
    error.message
}

const HANDBOOK: &str = r#"
[p.rows]
columns = [
  { name = "member", min = 10 },
  { name = "state",  width = 9 },
  { name = "task",   grow = 1, min = 12 },
  { name = "pr",     width = 10, align = "right", priority = 2 },
]
lines = [
  ["member", "state", "task", "pr"],
  ["",       { field = "pending", span = 3 }],
]
"#;

#[test]
fn the_handbook_example_reads_as_columns_and_lines_with_a_span() {
    let rows = parse(HANDBOOK).unwrap();
    assert_eq!(
        rows.columns
            .iter()
            .map(|c| c.field.as_str())
            .collect::<Vec<_>>(),
        ["member", "state", "task", "pr"]
    );
    assert_eq!(rows.columns[3].align, Align::Right);
    assert_eq!(rows.columns[3].priority, Some(2));
    assert_eq!(rows.columns[3].title, "PR");
    assert_eq!(
        rows.lines[1],
        [
            Cell {
                field: None,
                span: 1
            },
            Cell {
                field: Some("pending".into()),
                span: 3
            }
        ]
    );
    assert_eq!(rows.fields(), ["member", "state", "task", "pr", "pending"]);
    let value = rows.value();
    assert_eq!(value["columns"][2]["grow"], 1);
    assert_eq!(value["columns"][3]["align"], "right");
    assert_eq!(value["lines"][1][1], json!({"field": "pending", "span": 3}));
    assert_eq!(value["lines"][1][0], json!({"field": null, "span": 1}));
}

#[test]
fn columns_become_solver_tracks() {
    let rows = parse(HANDBOOK).unwrap();
    // No width and no grow: its widest value, shrinking to `min`.
    assert_eq!(
        rows.columns[0].track(7),
        Track {
            basis: 7,
            min: 10,
            max: None,
            grow: 0,
            shrink: 0,
            priority: None
        }
    );
    assert_eq!(rows.columns[1].track(99), Track::fixed(9).with_max(None));
    let task = rows.columns[2].track(40);
    assert_eq!((task.basis, task.min, task.grow), (0, 12, 1));
    let bare = parse("[p.rows]\ncolumns = [{ name = \"note\" }]\n").unwrap();
    assert_eq!(
        bare.columns[0].track(2).min,
        2,
        "never raised past its value"
    );
    assert_eq!(bare.columns[0].track(30).min, 4);
}

trait WithMax {
    fn with_max(self, max: Option<usize>) -> Self;
}

impl WithMax for Track {
    fn with_max(mut self, max: Option<usize>) -> Self {
        self.max = max;
        self
    }
}

#[test]
fn the_preset_and_the_older_columns_table_read_as_the_same_model() {
    let preset = read(None, "p").unwrap();
    assert_eq!(preset, Rows::preset());
    assert_eq!(preset.lines.len(), 1);
    // A width is fixed; no width grows into what is left, from four cells.
    assert_eq!(preset.columns[0].track(3), Track::fixed(14).with_max(None));
    let task = preset.columns[2].track(80);
    assert_eq!((task.basis, task.min, task.grow), (0, 4, 1));
    let legacy = parse(
        "[p.columns]\nshow = [\"member\", \"note\"]\nnote = { title = \"WHY\", width = 30 }\n",
    )
    .unwrap();
    assert_eq!(legacy.columns[1].title, "WHY");
    assert_eq!(
        (legacy.columns[1].width, legacy.columns[1].grow),
        (Some(30), 0)
    );
    assert_eq!(
        legacy.columns[0].width,
        Some(14),
        "known fields keep their preset width"
    );
    assert_eq!(legacy.fields(), ["member", "note"]);
    // A legacy table that shows the link keeps the preset's step-aside
    // priority, also with its own width.
    let link =
        parse("[p.columns]\nshow = [\"member\", \"pr_link\"]\npr_link = { width = 20 }\n").unwrap();
    assert_eq!(
        (link.columns[1].width, link.columns[1].priority),
        (Some(20), Some(1))
    );
    assert_eq!(link.columns[0].priority, None);
    let tables = parse(
        "[[p.rows.columns]]\nname = \"member\"\n[[p.rows.columns]]\nname = \"task\"\ngrow = 2\n",
    )
    .unwrap();
    assert_eq!(tables.columns[1].grow, 2);
}

#[test]
fn a_column_binds_a_documented_path_in_a_format() {
    let rows = parse(
        "[p.rows]\ncolumns = [\n  { name = \"member\" },\n  \
         { name = \"ctx\", from = \"session.usage.tokens\", format = \"tokens\", align = \"right\" },\n  \
         { name = \"model\", from = \"session.model\" },\n  \
         { name = \"seen\", format = \"age\" },\n]\n",
    )
    .unwrap();
    let ctx = &rows.columns[1];
    assert_eq!(ctx.from.as_ref().unwrap().path, "session.usage.tokens");
    assert_eq!(ctx.format, Format::Tokens);
    assert_eq!(rows.columns[2].format, Format::Text);
    // A plain column has no binding; a format alone reads its own field.
    assert_eq!(rows.columns[0].source(), None);
    assert_eq!(rows.columns[3].source().unwrap().path, "meta.squad.seen");
    assert!(!rows.reads_metadata());
    let value = rows.value();
    assert_eq!(value["columns"][1]["from"], "session.usage.tokens");
    assert_eq!(value["columns"][1]["format"], "tokens");
    assert_eq!(value["columns"][3]["from"], Value::Null);
    let meta = parse("[p.rows]\ncolumns = [{ name = \"team_role\", from = \"meta.team.role\" }]\n")
        .unwrap();
    assert!(meta.reads_metadata());
}

#[test]
fn an_unknown_path_or_format_is_refused_with_the_choices() {
    let message = error("[p.rows]\ncolumns = [{ name = \"m\", from = \"resume.model\" }]\n");
    assert!(
        message.contains("`squad.p.rows.columns[0].from` must be one of: member, presence"),
        "{message}"
    );
    assert!(message.contains("meta.squad.<field>"), "{message}");
    let message = error("[p.rows]\ncolumns = [{ name = \"m\", format = \"bytes\" }]\n");
    assert!(message.contains("text, tokens, age or count"), "{message}");
    for name in ["member", "role", "state", "pending", "note"] {
        let message = error(&format!(
            "[p.rows]\ncolumns = [{{ name = \"{name}\", from = \"meta.team.role\" }}]\n"
        ));
        assert!(message.contains("a field Squad reads itself"), "{message}");
        let message = error(&format!(
            "[p.rows]\ncolumns = [{{ name = \"{name}\", format = \"count\" }}]\n"
        ));
        assert!(message.contains("a field Squad reads itself"), "{message}");
    }
    let message = error("[p.rows]\ncolumns = [{ name = \"m\", color = \"red\" }]\n");
    assert!(message.contains("#514"), "{message}");
}

#[test]
fn mistakes_are_refused_with_their_place() {
    for (text, place) in [
        (
            "[p.rows]\ncolumns = [{ name = \"a\" }]\n[p.columns]\nshow = [\"a\"]\n",
            "both `rows` and `columns`",
        ),
        ("[p.rows]\ncolumns = []\n", "p.rows.columns"),
        ("[p.rows]\nlines = [[\"a\"]]\n", "p.rows.columns"),
        (
            "[p.rows]\ncolumns = [{ name = \"a\" }]\nsort = 1\n",
            "p.rows.sort",
        ),
        (
            "[p.rows]\ncolumns = [{ name = \"A\" }]\n",
            "p.rows.columns[0].name",
        ),
        (
            "[p.rows]\ncolumns = [{ name = \"a\" }, { name = \"a\" }]\n",
            "repeats",
        ),
        (
            "[p.rows]\ncolumns = [{ name = \"a\", width = 0 }]\n",
            "p.rows.columns[0].width",
        ),
        (
            "[p.rows]\ncolumns = [{ name = \"a\", align = \"top\" }]\n",
            "p.rows.columns[0].align",
        ),
        (
            "[p.rows]\ncolumns = [{ name = \"a\", truncate = \"start\" }]\n",
            "truncate",
        ),
        (
            "[p.rows]\ncolumns = [{ name = \"a\", min = 9, max = 3 }]\n",
            "min <= width <= max",
        ),
        (
            "[p.rows]\ncolumns = [{ name = \"a\", width = 5, min = 9 }]\n",
            "min <= width <= max",
        ),
        (
            "[p.rows]\ncolumns = [{ name = \"a\", wrap = true }]\n",
            "not a column setting",
        ),
        (
            "[p.rows]\ncolumns = [{ name = \"a\" }]\nlines = []\n",
            "p.rows.lines",
        ),
        (
            "[p.rows]\ncolumns = [{ name = \"a\" }]\nlines = [[\"a\", \"b\"]]\n",
            "spans more than",
        ),
        (
            "[p.rows]\ncolumns = [{ name = \"a\" }]\nlines = [[{ field = \"a\", span = 2 }]]\n",
            "span",
        ),
        (
            "[p.rows]\ncolumns = [{ name = \"a\" }]\nlines = [[\"Bad\"]]\n",
            "p.rows.lines[0][0]",
        ),
        (
            "[p.rows]\ncolumns = [{ name = \"a\" }]\nlines = [[{ field = \"a\", size = 1 }]]\n",
            "not a cell setting",
        ),
        (
            "[p.columns]\nmember = { align = \"left\" }\n",
            "not a column setting",
        ),
    ] {
        let message = error(text);
        assert!(message.contains(place), "{text}: {message}");
    }
}

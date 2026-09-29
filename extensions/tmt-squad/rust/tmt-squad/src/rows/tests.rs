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
    let tables = parse(
        "[[p.rows.columns]]\nname = \"member\"\n[[p.rows.columns]]\nname = \"task\"\ngrow = 2\n",
    )
    .unwrap();
    assert_eq!(tables.columns[1].grow, 2);
}

#[test]
fn field_sources_keys_are_reserved_for_503() {
    for key in ["from = \"x\"", "format = \"x\"", "color = \"red\""] {
        let message = error(&format!(
            "[p.rows]\ncolumns = [{{ name = \"task\", {key} }}]\n"
        ));
        assert!(message.contains("#503"), "{message}");
    }
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

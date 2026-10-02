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
            basis: Basis::Cells(7),
            min: 10,
            max: None,
            grow: 0,
            shrink: 0,
            priority: None
        }
    );
    assert_eq!(rows.columns[1].track(99), Track::fixed(9).with_max(None));
    let task = rows.columns[2].track(40);
    assert_eq!((task.basis, task.min, task.grow), (Basis::Cells(0), 12, 1));
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
    assert_eq!((task.basis, task.min, task.grow), (Basis::Cells(0), 4, 1));
    let legacy = parse(
        "[p.columns]\nshow = [\"member\", \"note\"]\nnote = { title = \"WHY\", width = 30 }\n",
    )
    .unwrap();
    assert_eq!(legacy.columns[1].title, "WHY");
    assert_eq!(
        (legacy.columns[1].width, legacy.columns[1].grow),
        (Some(Basis::Cells(30)), 0)
    );
    assert_eq!(
        legacy.columns[0].width,
        Some(Basis::Cells(14)),
        "known fields keep their preset width"
    );
    assert_eq!(legacy.fields(), ["member", "note"]);
    // A legacy table that shows the link keeps the preset's step-aside
    // priority, also with its own width.
    let link =
        parse("[p.columns]\nshow = [\"member\", \"pr_link\"]\npr_link = { width = 20 }\n").unwrap();
    assert_eq!(
        (link.columns[1].width, link.columns[1].priority),
        (Some(Basis::Cells(20)), Some(1))
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
}

#[test]
fn numeric_color_thresholds_name_theme_tokens_in_increasing_order() {
    let rows = parse(
        "[p.rows]\ncolumns = [{ name = \"ctx\", color = [{ at = 400000, token = \"review\" }, { at = 6e5, token = \"red\" }] }]\n",
    )
    .unwrap();
    let column = &rows.columns[0];
    assert_eq!(column.threshold(399_999.0), None);
    assert_eq!(column.threshold(400_000.0), Some("review"));
    assert_eq!(column.threshold(599_999.5), Some("review"));
    assert_eq!(
        column.threshold(600_000.0),
        Some("red"),
        "aliases stay names"
    );
    assert!(
        parse("[p.rows]\ncolumns = [{ name = \"ctx\" }]\n")
            .unwrap()
            .columns[0]
            .color
            .is_empty()
    );
    let place = "squad.p.rows.columns[0].color";
    for (setting, expected) in [
        ("\"red\"", format!("`{place}` must be a list")),
        ("[1]", format!("`{place}` must be a list")),
        (
            "[{ at = \"1\", token = \"red\" }]",
            format!("`{place}[0].at` must be a number"),
        ),
        (
            "[{ at = 1, token = \"orange\" }]",
            format!("`{place}[0].token` must be a theme token"),
        ),
        (
            "[{ at = 1, token = \"default\" }]",
            format!("`{place}[0].token` must be a theme token"),
        ),
        (
            "[{ at = 1, token = \"red\", over = 2 }]",
            format!("`{place}[0].over` is not a threshold setting"),
        ),
        (
            "[{ at = 2, token = \"red\" }, { at = 2, token = \"blocked\" }]",
            format!("`{place}[1].at` must be greater than the threshold before it"),
        ),
        (
            "[{ at = 5, token = \"red\" }, { at = 1, token = \"blocked\" }]",
            format!("`{place}[1].at` must be greater"),
        ),
    ] {
        let message = error(&format!(
            "[p.rows]\ncolumns = [{{ name = \"ctx\", color = {setting} }}]\n"
        ));
        assert!(message.starts_with(&expected), "{setting}: {message}");
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

#[test]
fn percentage_and_overflow_settings_are_validated_and_published() {
    let rows = parse(
        r#"[p.rows]
columns = [{ name = "member", width = "30%", min = 3, max = 20 },
           { name = "task", width = "70%", overflow = "wrap" }]
"#,
    )
    .unwrap();
    assert_eq!(rows.columns[0].width, Some(Basis::Percent(30)));
    assert_eq!(
        rows.columns[1].overflow,
        Some(Overflow::Wrap { max_lines: 2 })
    );
    let value = rows.value();
    assert_eq!(value["columns"][0]["width"], "30%");
    assert!(value["columns"][0].get("overflow").is_none());
    assert!(value["columns"][0].get("max_lines").is_none());
    assert_eq!(value["columns"][1]["overflow"], "wrap");
    assert_eq!(value["columns"][1]["max_lines"], 2);
    let decoded = Column::display("member", &value["columns"][0]);
    assert_eq!(decoded.track(90), rows.columns[0].track(90));
    let legacy = parse(
        r#"[p.columns]
show = ["member", "task"]
member = { width = "30%" }
task = { width = "70%", overflow = "wrap", max_lines = 8 }
"#,
    )
    .unwrap();
    assert_eq!(
        legacy.columns[1].overflow,
        Some(Overflow::Wrap { max_lines: 8 })
    );
    for settings in [
        "width = \"0%\"",
        "width = \"101%\"",
        "width = \"3.5%\"",
        "width = \"x%\"",
        "width = \"30\"",
        "width = \"030%\"",
        "overflow = \"wrap\", lines = 2",
        "overflow = \"clip\"",
        "max_lines = 2",
        "overflow = \"ellipsis\", max_lines = 2",
        "overflow = \"wrap\", max_lines = 0",
        "overflow = \"wrap\", max_lines = 9",
    ] {
        let input = format!("[p.rows]\ncolumns = [{{ name = \"task\", {settings} }}]\n");
        assert!(error(&input).contains("p.rows.columns"), "{input}");
    }
    assert!(
        error(
            r#"[p.rows]
columns = [{ name = "member", width = "60%" }, { name = "task", width = "41%" }]
"#
        )
        .contains("100%")
    );
}

#[test]
fn percent_tracks_keep_a_flexible_minimum_among_fixed_cell_columns() {
    let rows = parse(
        r#"[p.rows]
columns = [{ name = "member", width = 6 }, { name = "task", width = "60%" }]
"#,
    )
    .unwrap();
    let tracks: Vec<_> = rows.columns.iter().map(|column| column.track(80)).collect();
    assert_eq!(tracks[1].min, NARROWEST);
    assert_eq!(
        tmt_cli_style::grid::solve(&tracks, Some(11), 1),
        [Some(6), Some(4)]
    );
    for available in 7..25 {
        assert!(
            tmt_cli_style::grid::solve(&tracks, Some(available), 1)
                .iter()
                .flatten()
                .all(|width| *width > 0)
        );
    }
    let bounded = parse(
        r#"[p.rows]
columns = [{ name = "task", width = "60%", max = 2 }]
"#,
    )
    .unwrap();
    let track = bounded.columns[0].track(80);
    assert_eq!(tmt_cli_style::grid::solve(&[track], Some(80), 1), [Some(2)]);
    let explicit = parse(
        r#"[p.rows]
columns = [{ name = "task", width = "60%", min = 1 }]
"#,
    )
    .unwrap();
    assert_eq!(explicit.columns[0].track(80).min, 1);
}

#[test]
fn published_column_settings_round_trip_into_projected_display_settings() {
    let rows = parse(r#"[p.rows]
columns = [
    { name = "member", width = 10, min = 4, max = 14, grow = 1, align = "center", truncate = "middle", overflow = "ellipsis", priority = 3 },
    { name = "state", width = "30%", min = 2, max = 20, grow = 2, align = "right", overflow = "wrap", max_lines = 7 },
    { name = "task", width = "25%", max = 2, overflow = "wrap" },
    { name = "note", min = 1, max = 12, grow = 2, align = "center", truncate = "middle", priority = 1 },
]
"#).unwrap();
    let value = rows.value();
    for (index, column) in rows.columns.iter().enumerate() {
        let decoded = Column::display(&column.field, &value["columns"][index]);
        assert_eq!(decoded.field, column.field);
        assert_eq!(decoded.title, column.title);
        assert_eq!(decoded.width, column.width);
        assert_eq!((decoded.min, decoded.max), (column.min, column.max));
        assert_eq!(decoded.track(101), column.track(101));
        assert_eq!(
            (decoded.align, decoded.truncate, decoded.overflow),
            (column.align, column.truncate, column.overflow)
        );
    }
}

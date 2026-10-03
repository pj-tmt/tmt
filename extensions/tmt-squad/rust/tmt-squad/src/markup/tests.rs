//! Literal app-adapter cases; no fixture regeneration or provider acquisition.
use super::*;
use serde_json::json;
use tmt_cli_style::grid::Align;
use tmt_tui::{geometry::Space, style::TextFlow, text};

fn configured(source: &str) -> crate::rows::Rows {
    let document: toml_edit::DocumentMut =
        format!("[p.rows]\ncolumns=[{source}]\n").parse().unwrap();
    crate::rows::read(document["p"].as_table_like(), "p").unwrap()
}

#[test]
fn projected_values_and_scoped_identities_survive_filtering_and_reordering() {
    let rows = configured(
        "{name='member',width=10},{name='tokens',width=6,from='session.usage.tokens',format='tokens'}",
    );
    let a = json!({"id":"member-a", "name":"文件", "squad":"product", "fields":{"tokens":"487k"}});
    let b = json!({"id":"member-b", "name":"rin", "squad":"product", "fields":{"tokens":"?"},"failed":["tokens"]});
    let acquired = row_values(&rows, "@custom", vec![(0, &a), (0, &b), (1, &a)]).unwrap();
    assert_eq!(
        acquired[0].children[0].children[1].text.as_deref(),
        Some("487k")
    );
    assert_eq!(
        acquired[1].children[0].children[1].text.as_deref(),
        Some("?")
    );
    assert_eq!(acquired[0].row_id.as_deref(), Some("member-a"));
    assert_eq!(
        acquired[0].children[0].children[1].id.as_ref().unwrap(),
        &[
            "tab:@custom",
            "section-0",
            "squad:product",
            "member-a",
            "line-0",
            "column-1"
        ]
    );
    let filtered = row_values(&rows, "@custom", vec![(1, &a), (0, &a)]).unwrap();
    assert_eq!(filtered[0].id, acquired[2].id);
    assert_eq!(filtered[1].id, acquired[0].id);
    let mut other = a.clone();
    other["squad"] = json!("infra");
    let shared = row_values(&rows, "@custom", vec![(0, &a), (0, &other)]).unwrap();
    assert_ne!(shared[0].id, shared[1].id);
    assert!(
        row_values(&rows, "@custom", vec![(0, &a), (0, &a)])
            .unwrap_err()
            .contains("duplicate")
    );
    let mut malformed = a.clone();
    malformed["id"] = json!("42");
    assert!(row_values(&rows, "@custom", vec![(0, &malformed)]).is_err());
    other["squad"] = json!("7");
    let numeric_scope = row_values(&rows, "42", vec![(0, &other)]).unwrap();
    assert_eq!(
        &numeric_scope[0].id.as_ref().unwrap()[..3],
        &["tab:42", "section-0", "squad:7"]
    );
    other.as_object_mut().unwrap().remove("squad");
    let named = row_values(&rows, "42", vec![(0, &other)]).unwrap();
    assert_eq!(named[0].id.as_ref().unwrap()[2], "squad:42");
    let bare = json!({"name":"all", "fields":{}});
    let plain = row_values(&rows, "@all", vec![(0, &bare)]).unwrap();
    assert!(plain[0].id.is_none() && plain[0].row_id.is_none());
    assert!(plain[0].children[0].children[1].text.is_none());
}

#[test]
fn css_clamp_caps_win_over_weight_and_uncovered_values_reserve_no_width() {
    for weight in [1, 9] {
        let mut rows = configured(&format!(
            "{{name='a',width=10,min=4,max=14,grow={weight}}},{{name='b',min=4,grow=1}},{{name='extra',width=90}}"
        ));
        rows.lines[0].truncate(2);
        rows.lines.push(vec![crate::rows::Cell {
            field: Some("b".into()),
            span: 2,
        }]);
        let grid = Grid::compile(&rows, |_| 0, 40).unwrap();
        assert_eq!(grid.columns, [Some(14), Some(25), None]);
        assert_eq!(grid.span(0..2).unwrap().visible, 40);
        assert!(grid.span(2..3).is_none());
    }
    let rows = configured("{name='a',width='30%',min=2,max=20,grow=2},{name='b',min=4,grow=1}");
    assert_eq!(
        Grid::compile(&rows, |_| 0, 100).unwrap().columns,
        [Some(20), Some(79)]
    );
    let rows = configured("{name='a',max=12,grow=2},{name='b',min=4,grow=1}");
    assert_eq!(
        Grid::compile(&rows, |_| 6, 30).unwrap().columns,
        [Some(12), Some(17)]
    );
    let rows = configured("{name='a',max=2,grow=1},{name='b',grow=1}");
    assert_eq!(
        Grid::compile(&rows, |_| 0, 20).unwrap().columns,
        [Some(4), Some(15)]
    );
    let rows = configured("{name='a',max=12},{name='b',min=4,grow=1}");
    assert_eq!(
        Grid::compile(&rows, |_| 6, 30).unwrap().columns,
        [Some(12), Some(17)]
    );
}

#[test]
fn priority_precedes_geometry_and_only_mandatory_overflow_can_cut() {
    let optional = configured("{name='a',width=10},{name='b',width=10,priority=1}");
    assert_eq!(
        Grid::compile(&optional, |_| 0, 15).unwrap().columns,
        [Some(10), None]
    );
    let mandatory = configured("{name='a',width=10},{name='b',width=10}");
    let four = Grid::compile(&mandatory, |_| 0, 15).unwrap();
    assert_eq!(four.columns, [Some(10), Some(4)]);
    assert_eq!(four.span(1..2).unwrap().text, 10);
    assert!(four.span(1..2).unwrap().cut);
    assert_eq!(
        Grid::compile(&mandatory, |_| 0, 14).unwrap().columns,
        [Some(10), None]
    );
    let ties = configured(
        "{name='a',width=8},{name='b',width=8,priority=2},{name='c',width=8,priority=2}",
    );
    assert_eq!(
        Grid::compile(&ties, |_| 0, 17).unwrap().columns,
        [Some(8), Some(8), None]
    );
    let model = configured(
        "{name='member',width='22%',min=12,max=24},{name='state',width='14%',min=9,max=10},{name='task',min=18,grow=1},{name='pr',width='24%',min=12,max=28,priority=2},{name='model',width='16%',min=18,max=18,priority=3}",
    );
    assert_eq!(
        Grid::compile(&model, |_| 0, 70).unwrap().columns,
        [Some(15), Some(9), Some(27), Some(16), None]
    );
}

#[test]
fn logical_text_budget_wraps_before_cut_and_preserves_grapheme_alignment() {
    assert_eq!(
        text::measure("a a a", TextFlow::Wrap, Space::Cells(4)),
        [4, 2]
    );
    let box_width = BoxWidth {
        visible: 4,
        text: 8,
        cut: true,
    };
    assert_eq!(
        fitted("abcdefgh ijklmnop", box_width, TextFlow::Wrap, Align::Left),
        ["abc…", "ijk…"]
    );
    assert_eq!(
        fitted(
            "e\u{301}👩‍💻界",
            BoxWidth {
                visible: 6,
                text: 6,
                cut: false
            },
            TextFlow::Truncate,
            Align::Right
        ),
        [" e\u{301}👩‍💻界"]
    );
}

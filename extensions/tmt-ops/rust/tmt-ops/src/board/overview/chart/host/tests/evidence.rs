//! Opt-in cell captures are review evidence, never a replacement frozen oracle.
use super::*;
use serde_json::json;
use tmt_cli_style::{Depth, Theme, theme::Base};
#[test]
fn terminal_cell_evidence() {
    let Some(root) = std::env::var_os("TMT_OPS_M08_CAPTURE") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    std::fs::create_dir_all(&root).unwrap();
    crate::status::with_now_ms(fixtures::NOW, || {
        for tab in [OpsTab::Overview, OpsTab::Digest] {
            for (base, depth) in [
                (Base::Tmt, Depth::TrueColor),
                (Base::TmtLight, Depth::TrueColor),
                (Base::Tmt, Depth::None),
            ] {
                for (width, height) in [
                    (160, 36),
                    (140, 36),
                    (100, 36),
                    (80, 36),
                    (60, 36),
                    (60, 18),
                ] {
                    let mut app = board();
                    fixtures::populate(&mut app);
                    app.shell.tab = tab;
                    let look = crate::look::Look {
                        theme: Theme::new(base),
                        depth,
                    };
                    let view = app.view.as_mut().unwrap();
                    view.look = look;
                    for value in view
                        .home
                        .as_mut()
                        .unwrap()
                        .workspace
                        .squad_looks
                        .values_mut()
                    {
                        *value = look;
                    }
                    assert_eq!(app.look().depth, depth);
                    assert_eq!(app.look().role(Role::Text), look.role(Role::Text));
                    let buffer = paint(&mut app, width, height);
                    let cells: Vec<_> = buffer.content.iter().map(|cell| json!({"symbol":cell.symbol(),"fg":format!("{:?}",cell.fg),"bg":format!("{:?}",cell.bg),"modifier":format!("{:?}",cell.modifier)})).collect();
                    let lines: Vec<_> = buffer
                        .content
                        .chunks(usize::from(width))
                        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
                        .collect();
                    let state = app.usage_chart.borrow();
                    assert!(buffer.content.len() == usize::from(width) * usize::from(height));
                    if height == 36 {
                        assert!(state.visible);
                        assert_eq!(state.observed.as_ref().unwrap().buckets.len(), 12);
                    }
                    let value = json!({"tab":format!("{tab:?}"),"width":width,"height":height,"theme":if base == Base::TmtLight {"tmt-light"} else {"tmt"},"depth":format!("{depth:?}"),"cells":cells,"lines":lines,"chartVisible":state.visible,"barHits":state.hits.iter().filter_map(|hit| if let Target::Bar(index) = hit.target { Some(json!({"index":index,"x":hit.area.x,"y":hit.area.y,"width":hit.area.width,"height":hit.area.height})) } else { None }).collect::<Vec<_>>(),"sourceTotals":state.observed.as_ref().map(|source| source.buckets.iter().map(|bucket| json!({"fromMs":bucket.from_ms,"toMs":bucket.to_ms,"tokens":bucket.tokens.map(|value|value.to_string()),"coverageComplete":bucket.complete(),"identities":bucket.identities,"observedIdentities":bucket.observed_identities})).collect::<Vec<_>>())});
                    let file =
                        root.join(format!("{tab:?}-{base:?}-{depth:?}-{width}x{height}.json"));
                    std::fs::write(file, serde_json::to_vec(&value).unwrap()).unwrap();
                }
            }
        }
    });
}

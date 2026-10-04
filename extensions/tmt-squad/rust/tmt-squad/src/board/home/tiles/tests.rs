use super::*;
use ratatui::{Terminal, backend::TestBackend, style::Modifier, widgets::Paragraph};
use serde_json::json;
use tmt_cli_style::{Depth, Theme, theme::Base};

fn squad(name: &str) -> SquadLine {
    SquadLine {
        squad: name.into(),
        lead: Some(
            json!({"name": "lead", "fields": {"task": "private task", "pr_link": "private PR"}}),
        ),
        counts: Counts {
            waiting: 1,
            blocked: 2,
            ..Default::default()
        },
        members: Counts::default(),
    }
}

fn members() -> Counts {
    Counts {
        members: 6,
        waiting: 1,
        blocked: 2,
        review: 1,
        working: 1,
        idle: 1,
    }
}

#[test]
fn other_members_keep_the_marks_row_informative_without_inventing_a_state() {
    let counts = Counts {
        members: 2,
        ..Default::default()
    };
    let line = member_line(&counts, 100, false, Look::default()).to_string();
    assert!(line.contains("2 other · 2 members"), "{line}");
    assert!(!line.contains('◌'));
    let mixed = Counts {
        working: 1,
        ..counts
    };
    let line = member_line(&mixed, 100, false, Look::default()).to_string();
    assert!(
        line.starts_with("● ") && line.contains("1 other · 2 members"),
        "{line}"
    );
}

#[test]
fn large_partial_token_values_keep_a_guaranteed_gap_and_short_models() {
    let squad = squad("remote");
    let counts = members();
    let mut usage = usage(TokenWindow::DEFAULTS);
    usage.lead_model = Some("claude-opus-4-7");
    usage.lead = [3_000, 3_700_000, 12_400_000].map(|tokens| {
        Some(crate::board::rate::Reading {
            tokens,
            partial: true,
            span: 60_000,
        })
    });
    let item = TileItem {
        squad: &squad,
        members: &counts,
        lead_model: None,
        usage: Some(usage),
    };
    for width in [80, 100, 113, 160, 200] {
        let painted = paint(std::slice::from_ref(&item), width, Look::default(), None);
        let output = text(&painted.lines).join("\n");
        assert!(
            output.contains("opus") && !output.contains("claude"),
            "{output}"
        );
        assert!(output.contains("~3.7M ~12.4M"), "{output}");
        assert!(
            painted
                .lines
                .iter()
                .all(|line| line.width() == usize::from(width))
        );
    }
}

fn text(lines: &[Line<'_>]) -> Vec<String> {
    lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect()
        })
        .collect()
}

#[test]
fn responsive_regions_share_row_starts_and_keep_reading_order() {
    let names = ["a", "b", "c", "d", "e"];
    for width in [80, 99, 100, 113, 160, 169, 170, 200] {
        let regions = placement(width, names.len());
        let height = if width < 100 { 1 } else { 3 };
        let step = if width < 100 { 1 } else { 4 };
        for (index, region) in regions.iter().enumerate() {
            assert_eq!(region.item, index);
            assert_eq!(region.x, 0);
            assert_eq!(region.width, width);
            assert_eq!(region.lines, index * step..index * step + height);
        }
    }
}

#[test]
fn squad_count_never_reintroduces_multiple_columns_or_compact_wide_tiles() {
    for width in [80, 100, 113, 160, 200] {
        let regions = placement(width, 14);
        assert_eq!(regions.len(), 14);
        assert!(
            regions
                .iter()
                .all(|region| region.x == 0 && region.width == width)
        );
        assert!(
            regions
                .iter()
                .all(|region| region.lines.len() == if width < 100 { 1 } else { 3 })
        );
    }
    assert!(placement(0, 10).is_empty());
    assert!(placement(160, 0).is_empty());
}

#[test]
fn members_are_urgency_sorted_and_private_row_values_never_appear() {
    let squad = squad("squad");
    let counts = members();
    let item = TileItem {
        squad: &squad,
        members: &counts,
        lead_model: None,
        usage: None,
    };
    for width in [160, 100, 80] {
        let painted = paint(std::slice::from_ref(&item), width, Look::default(), None);
        let output = text(&painted.lines).join("\n");
        assert!(output.contains("◆ ✗ ✗ ◐ ● ○ "));
        assert!(output.contains("6 members"));
        assert!(!output.contains("private"));
        assert!(
            painted
                .lines
                .iter()
                .all(|line| line.width() <= usize::from(width))
        );
        if width >= 100 {
            assert!(output.contains("◆ 1 ✗ 2"));
        }
    }
}

#[test]
fn selection_covers_the_whole_block_but_leaves_gaps_and_neighbours_alone() {
    let squads = (0..10)
        .map(|index| squad(&format!("squad-{index}")))
        .collect::<Vec<_>>();
    let counts = members();
    let items = squads
        .iter()
        .map(|squad| TileItem {
            squad,
            members: &counts,
            lead_model: None,
            usage: None,
        })
        .collect::<Vec<_>>();
    for (base, depth) in [
        (Base::Tmt, Depth::TrueColor),
        (Base::TmtLight, Depth::TrueColor),
        (Base::Tmt, Depth::None),
    ] {
        let look = Look {
            theme: Theme::new(base),
            depth,
        };
        for (width, count) in [(160, 3), (100, 3), (80, 3), (160, 10)] {
            let items = &items[..count];
            let painted = paint(items, width, look, Some(1));
            let selected = &painted.regions[1];
            let height = painted.lines.len() as u16 + 1;
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| {
                    frame.render_widget(Paragraph::new(painted.lines.clone()), frame.area())
                })
                .unwrap();
            let buffer = terminal.backend().buffer();
            for y in 0..height {
                for x in 0..width {
                    let cell = &buffer[(x, y)];
                    let inside = selected.lines.contains(&usize::from(y))
                        && (selected.x..selected.x + selected.width).contains(&x);
                    if depth == Depth::None {
                        assert_eq!(cell.modifier.contains(Modifier::REVERSED), inside);
                    } else {
                        assert_eq!(cell.bg == look.selection().bg.unwrap(), inside);
                    }
                }
            }
            assert_eq!(
                text(&paint(items, width, look, None).lines),
                text(&painted.lines)
            );
        }
    }
}

#[test]
fn escaped_unicode_and_missing_lead_fit_even_tiny_widths() {
    let mut squad = squad("👩‍💻 e\u{301} long squad");
    squad.lead = None;
    let counts = members();
    let item = TileItem {
        squad: &squad,
        members: &counts,
        lead_model: None,
        usage: None,
    };
    assert!(
        text(&paint(std::slice::from_ref(&item), 100, Look::default(), None).lines)
            .join("")
            .contains("no lead")
    );
    squad.squad.push('\n');
    let item = TileItem {
        squad: &squad,
        members: &counts,
        lead_model: None,
        usage: None,
    };
    for width in 1..=160 {
        let painted = paint(std::slice::from_ref(&item), width, Look::default(), Some(0));
        assert!(
            painted
                .lines
                .iter()
                .all(|line| line.width() <= usize::from(width))
        );
        assert!(text(&painted.lines).iter().all(|line| !line.contains('\n')));
        for region in painted.regions {
            assert!(region.x + region.width <= width);
            assert!(region.lines.end <= painted.lines.len());
        }
    }
}

fn usage(windows: [TokenWindow; 3]) -> HomeUsage<'static> {
    HomeUsage {
        lead_model: Some("gpt"),
        windows,
        lead: [1_000, 2_000, 3_000].map(|tokens| {
            Some(crate::board::rate::Reading {
                tokens,
                partial: false,
                span: 60_000,
            })
        }),
        // The formatter consumes the supplied share, never derives it from totals.
        squad: [None; 3],
        share: Some(crate::board::app::UsageShare {
            fraction: 0.375,
            partial: true,
        }),
    }
}

#[test]
fn runtime_observations_keep_missing_zero_partial_model_and_supplied_share_distinct() {
    let squad = squad("squad");
    let counts = members();
    let mut usage = usage(TokenWindow::DEFAULTS);
    usage.lead = [
        Some(crate::board::rate::Reading {
            tokens: 0,
            partial: false,
            span: 60_000,
        }),
        None,
        Some(crate::board::rate::Reading {
            tokens: 7_500,
            partial: true,
            span: 60_000,
        }),
    ];
    let item = TileItem {
        squad: &squad,
        members: &counts,
        lead_model: None,
        usage: Some(usage),
    };
    let painted = paint(std::slice::from_ref(&item), 160, Look::default(), None);
    let line = text(&painted.lines)[1].clone();
    assert!(line.contains("gpt"));
    assert!(line.contains("      0      –    ~8k   ~38%"), "{line}");
    let missing = TileItem {
        squad: &squad,
        members: &counts,
        lead_model: None,
        usage: Some(HomeUsage {
            lead_model: None,
            windows: TokenWindow::DEFAULTS,
            lead: [None; 3],
            squad: [None; 3],
            share: None,
        }),
    };
    for width in [80, 100, 113, 160, 200] {
        let missing = paint(std::slice::from_ref(&missing), width, Look::default(), None);
        let lines = text(&missing.lines);
        let lead_line = &lines[usize::from(width >= 100)];
        assert_eq!(lead_line.matches('–').count(), 1, "{width}: {lead_line}");
        assert!(!lead_line.contains('0'));
    }
}

#[test]
fn disabled_sampling_hides_cells_and_only_sampled_squads_define_the_legend() {
    let squads = [squad("off"), squad("on"), squad("other")];
    let counts = members();
    let mut items = vec![TileItem {
        squad: &squads[0],
        members: &counts,
        lead_model: None,
        usage: None,
    }];
    for width in [160, 100, 80] {
        assert_eq!(legend(&items, width), "");
        let output = text(&paint(&items, width, Look::default(), None).lines).join("\n");
        assert!(output.contains("lead") && output.contains("6 members"));
        assert!(
            !output.contains('–'),
            "disabled sampling invents no missing cells: {output}"
        );
    }
    items[0].lead_model = Some("off-model");
    for width in [160, 100, 80] {
        let output = text(&paint(&items, width, Look::default(), None).lines).join("\n");
        assert!(
            output.contains("off-mod"),
            "known model survives sampling off: {output}"
        );
        assert!(!output.contains('–'));
    }
    let windows = [
        TokenWindow::FIVE_MINUTES,
        TokenWindow::HOUR,
        TokenWindow::parse("24h").unwrap(),
    ];
    items.push(TileItem {
        squad: &squads[1],
        members: &counts,
        lead_model: None,
        usage: Some(usage(windows)),
    });
    for width in [160, 100, 80] {
        assert!(legend(&items, width).contains("24h"));
        assert!(!legend(&items, width).contains("vary"));
        let output = paint(&items, width, Look::default(), None);
        let off = &output.regions[0];
        for line in &output.lines[off.lines.clone()] {
            let mut terminal = Terminal::new(TestBackend::new(width, 1)).unwrap();
            terminal
                .draw(|frame| frame.render_widget(Paragraph::new(line.clone()), frame.area()))
                .unwrap();
            let cells = &terminal.backend().buffer().content;
            let off_text = cells[usize::from(off.x)..usize::from(off.x + off.width)]
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            assert!(
                !off_text.contains('–') && !off_text.contains("3k"),
                "{off_text}"
            );
        }
    }
    items.push(TileItem {
        squad: &squads[2],
        members: &counts,
        lead_model: None,
        usage: Some(usage(TokenWindow::DEFAULTS)),
    });
    assert_eq!(legend(&items, 160), "lead tokens · windows vary");
}

#[test]
fn uniform_labels_appear_once_and_narrow_rows_retain_the_last_two_windows() {
    let squad = squad("squad");
    let counts = members();
    let windows = [
        TokenWindow::FIVE_MINUTES,
        TokenWindow::HOUR,
        TokenWindow::parse("24h").unwrap(),
    ];
    let item = TileItem {
        squad: &squad,
        members: &counts,
        lead_model: None,
        usage: Some(usage(windows)),
    };
    for width in [160, 100, 80] {
        let items = std::slice::from_ref(&item);
        let label = legend(items, width);
        assert_eq!(
            label,
            match width {
                160 => "lead tokens · 5m · 1h · 24h · share (24h)",
                100 => "lead tokens · 5m · 1h · 24h · share (24h)",
                _ => "lead tokens · 1h · 24h",
            }
        );
        let painted = text(&paint(items, width, Look::default(), None).lines).join("\n");
        assert!(painted.contains("2k"));
        assert!(painted.contains("3k"));
        assert_eq!(painted.contains("1k"), width >= 100);
        assert!(!painted.contains("1h"));
        assert!(!painted.contains("24h"));
    }
}

#[test]
fn mixed_windows_label_each_observation_and_share_without_reordering_tiles() {
    let squads = [squad("first"), squad("second")];
    let counts = members();
    let items = [
        TileItem {
            squad: &squads[0],
            members: &counts,
            lead_model: None,
            usage: Some(usage(TokenWindow::DEFAULTS)),
        },
        TileItem {
            squad: &squads[1],
            members: &counts,
            lead_model: None,
            usage: Some(usage([
                TokenWindow::FIVE_MINUTES,
                TokenWindow::HOUR,
                TokenWindow::parse("24h").unwrap(),
            ])),
        },
    ];
    for width in [160, 100, 80] {
        assert_eq!(legend(&items, width), "lead tokens · windows vary");
        let painted = paint(&items, width, Look::default(), None);
        let output = text(&painted.lines).join("\n");
        assert!(output.contains("5m:2k"), "{output}");
        assert!(output.contains("1h:3k"), "{output}");
        assert!(output.contains("1h:2k"), "{output}");
        assert!(output.contains("24h:3k"), "{output}");
        assert!(output.contains("1h:~38%"), "{output}");
        assert!(output.contains("24h:~38%"), "{output}");
        assert_eq!(
            painted
                .regions
                .iter()
                .map(|region| region.item)
                .collect::<Vec<_>>(),
            [0, 1]
        );
        assert!(
            painted
                .lines
                .iter()
                .all(|line| line.width() <= usize::from(width))
        );
    }
}

#[test]
fn oversized_compact_totals_cannot_hide_the_next_window() {
    let squad = squad("squad");
    let counts = members();
    let mut usage = usage(TokenWindow::DEFAULTS);
    usage.lead[1].as_mut().unwrap().tokens = u128::MAX;
    let item = TileItem {
        squad: &squad,
        members: &counts,
        lead_model: None,
        usage: Some(usage),
    };
    let painted = paint(&[item], 80, Look::default(), None);
    let output = text(&painted.lines).join("");
    assert!(output.ends_with("    3k"), "{output}");
    assert!(output.contains('…'));
    assert_eq!(painted.lines[0].width(), 80);
}

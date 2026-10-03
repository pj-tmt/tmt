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
        pressing: None,
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
    for (width, expected) in [
        (
            160,
            vec![
                (0, 0, 52),
                (0, 54, 52),
                (0, 108, 52),
                (4, 0, 52),
                (4, 54, 52),
            ],
        ),
        (
            100,
            vec![(0, 0, 49), (0, 51, 49), (4, 0, 49), (4, 51, 49), (8, 0, 49)],
        ),
        (
            80,
            vec![(0, 0, 80), (1, 0, 80), (2, 0, 80), (3, 0, 80), (4, 0, 80)],
        ),
    ] {
        let regions = placement(width, &names).unwrap();
        assert_eq!(
            regions
                .iter()
                .map(|(r, _)| (r.lines.start, r.x, r.width))
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(
            regions.iter().map(|(r, _)| r.item).collect::<Vec<_>>(),
            [0, 1, 2, 3, 4]
        );
        assert!(
            regions
                .iter()
                .all(|(r, _)| r.lines.len() == if width < 100 { 1 } else { 3 })
        );
    }
    let at_160 = placement(160, &names).unwrap();
    placement(80, &names).unwrap();
    assert_eq!(placement(160, &names).unwrap(), at_160);
}

#[test]
fn exact_width_and_many_squad_boundaries_choose_the_required_form() {
    let names = ["a"; 10];
    assert_eq!(placement(149, &names[..9]).unwrap()[1].0.lines.start, 0);
    assert_eq!(placement(150, &names[..9]).unwrap()[2].0.lines.start, 0);
    assert_eq!(placement(99, &names[..9]).unwrap()[1].0.lines.start, 1);
    for width in [100, 149, 150, 160] {
        let regions = placement(width, &names).unwrap();
        assert!(regions.iter().all(|(r, _)| r.lines.len() == 1));
        assert_eq!(regions[1].0.lines.start, usize::from(width < 150));
        assert_eq!(regions[2].0.lines.start, if width < 150 { 2 } else { 1 });
    }
    assert!(placement(0, &names).unwrap().is_empty());
    assert!(placement(160, &[]).unwrap().is_empty());
}

#[test]
fn members_are_urgency_sorted_and_private_row_values_never_appear() {
    let squad = squad("squad");
    let counts = members();
    let item = TileItem {
        squad: &squad,
        members: &counts,
    };
    for width in [160, 100, 80] {
        let painted = paint(std::slice::from_ref(&item), width, Look::default(), None).unwrap();
        let output = text(&painted.lines).join("\n");
        assert!(output.contains("◆✗✗◐●○"));
        assert!(output.contains("6 members"));
        assert!(!output.contains("private"));
        assert!(
            painted
                .lines
                .iter()
                .all(|line| line.width() <= usize::from(width))
        );
        if width >= 100 {
            assert!(output.contains("◆1 ✗2"));
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
            let painted = paint(items, width, look, Some(1)).unwrap();
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
                text(&paint(items, width, look, None).unwrap().lines),
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
    };
    assert!(
        text(
            &paint(std::slice::from_ref(&item), 100, Look::default(), None)
                .unwrap()
                .lines
        )
        .join("")
        .contains("no lead")
    );
    squad.squad.push('\n');
    let item = TileItem {
        squad: &squad,
        members: &counts,
    };
    for width in 1..=160 {
        let painted = paint(std::slice::from_ref(&item), width, Look::default(), Some(0)).unwrap();
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

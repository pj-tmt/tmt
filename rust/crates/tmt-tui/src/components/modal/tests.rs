use super::*;

#[test]
fn centered_and_docked_sheets_are_full_width_below_the_md_breakpoint() {
    let md = tmt_cli_style::breakpoint::MD.cells;
    for placement in [Placement::Center, Placement::Docked] {
        let modal = Modal {
            title: String::new(),
            placement,
        };
        let outer = |width| {
            modal
                .areas(Rect::new(0, 0, width, 30), [width, 10], false, false)
                .outer
        };
        assert_eq!(outer(md - 1).width, md - 1, "{placement:?} below md");
        // From md the sheet is nine tenths of the body, never more than its demand.
        assert_eq!(outer(md).width, md * 9 / 10, "{placement:?} at md");
        assert_eq!(outer(160).width, 144, "{placement:?} wide");
        let narrow_demand = modal
            .areas(Rect::new(0, 0, md, 30), [40, 10], false, false)
            .outer;
        assert_eq!(narrow_demand.width, 40);
    }
}

#[test]
fn flat_modal_keeps_areas_and_the_complete_opaque_mask_at_every_depth() {
    use ratatui::style::{Color, Modifier, Style};
    for (base, depth) in [
        ("tmt", Depth::TrueColor),
        ("tmt-light", Depth::TrueColor),
        ("terminal", Depth::Ansi16),
        ("tmt", Depth::None),
    ] {
        let theme = Theme::new(tmt_cli_style::Base::parse(base).unwrap());
        for buffer_area in [Rect::new(0, 0, 20, 10), Rect::new(3, 2, 10, 5)] {
            for outer in [
                Rect::new(1, 1, 16, 8),
                Rect::new(4, 3, 2, 2),
                Rect::new(30, 30, 2, 2),
            ] {
                let modal = Modal {
                    title: "a\x1bb".into(),
                    placement: Placement::Body,
                };
                let areas = modal.areas(outer, [outer.width, outer.height], true, true);
                assert_eq!(areas.outer, outer);
                let mut square = Buffer::empty(buffer_area);
                for cell in &mut square.content {
                    cell.set_symbol("X").set_style(
                        Style::new()
                            .fg(Color::Red)
                            .bg(Color::Green)
                            .add_modifier(Modifier::REVERSED),
                    );
                }
                let mut flat = square.clone();
                modal.paint(areas, &mut square, &theme, depth);
                modal.paint_flat(areas, &mut flat, &theme, depth);
                for (offset, (before, after)) in
                    square.content.iter().zip(&flat.content).enumerate()
                {
                    let mut expected = before.clone();
                    match before.symbol() {
                        "┌" | "┐" | "└" | "┘" => {
                            expected.set_symbol("─");
                        }
                        "│" => {
                            expected.set_symbol(" ");
                        }
                        _ => {}
                    }
                    assert_eq!(&expected, after, "{base}/{depth:?}/{outer:?}");
                    let x = buffer_area.x + offset as u16 % buffer_area.width;
                    let y = buffer_area.y + offset as u16 / buffer_area.width;
                    if outer.intersection(buffer_area).contains((x, y).into()) {
                        assert_ne!(after.symbol(), "X", "entire mask clears");
                        assert!(!after.modifier.contains(Modifier::REVERSED));
                        assert_ne!(after.bg, Color::Green);
                    }
                }
            }
        }
    }
}

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

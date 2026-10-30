use super::*;

/// tmux 3.3a's default root binding.
const DEFAULT_3_3: &str = r##"bind-key    -T root         WheelUpPane          if-shell -F "#{||:#{mouse_any_flag},#{&&:#{pane_in_mode},#{?#{m/r:(copy|view)-mode,#{pane_mode}},0,1}}}" { send-keys -M } { copy-mode -e }"##;
/// tmux 3.8's default root binding.
const DEFAULT_3_8: &str = r##"bind-key    -T root         WheelUpPane          if-shell -F "#{||:#{alternate_on},#{pane_in_mode},#{mouse_any_flag}}" { send-keys -M } { copy-mode -e }"##;
/// The owner's custom binding (#5595): only nvim and lazygit get the wheel.
const NAMES_ONLY: &str = r##"bind-key    -T root         WheelUpPane          if-shell -F "#{||:#{m:*nvim*,#{pane_current_command}},#{m:*lazygit*,#{pane_current_command}}}" { send-keys -M } { copy-mode -e }"##;
/// The same binding with the generic conditions added, as the tmt-ops skill shows.
const NAMES_FIXED: &str = r##"bind-key    -T root         WheelUpPane          if-shell -F "#{||:#{||:#{alternate_on},#{mouse_any_flag}},#{||:#{m:*nvim*,#{pane_current_command}},#{m:*lazygit*,#{pane_current_command}}}}" { send-keys -M } { copy-mode -e }"##;

#[test]
fn a_binding_that_skips_the_mouse_and_screen_checks_escapes_and_the_defaults_do_not() {
    assert!(escapes(NAMES_ONLY));
    assert!(escapes(
        "bind-key    -T root         WheelUpPane          copy-mode -e"
    ));
    assert!(escapes(
        r##"bind-key -T root WheelUpPane if-shell -F "#{pane_in_mode}" { send-keys -M } { copy-mode -e }"##
    ));
    for kept in [
        DEFAULT_3_3,
        DEFAULT_3_8,
        NAMES_FIXED,
        "bind-key    -T root         WheelUpPane          send-keys -M",
        "",
    ] {
        assert!(!escapes(kept), "{kept}");
    }
}

#[test]
fn the_hint_takes_a_free_notice_line_once_and_never_replaces_another_notice() {
    let mut app = crate::board::app::workspace::tests::board();
    app.wheel_up_escapes();
    assert_eq!(app.notice.as_deref(), Some(HINT));

    let mut app = crate::board::app::workspace::tests::board();
    app.set_notice(Some("Reloaded.".into()), false);
    app.wheel_up_escapes();
    assert_eq!(app.notice.as_deref(), Some("Reloaded."));

    let mut app = crate::board::app::workspace::tests::board();
    app.migration_notice = Some("Retired binding requires migration.".into());
    app.wheel_up_escapes();
    assert_eq!(app.notice, None);
}

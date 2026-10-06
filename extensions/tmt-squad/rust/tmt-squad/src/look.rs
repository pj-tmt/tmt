//! How Squad draws: core's resolved theme, then `[board.theme]`, then
//! `[squad.<name>.theme]` in Squad's own configuration. Squad names no colors of its
//! own; every color is a design token that tmt-cli-style renders.

use ratatui::{
    buffer::Buffer,
    style::{Color, Modifier, Style},
};
use tmt_cli_style::{
    Depth, Role, Theme,
    theme::{
        background::{self, Background},
        screen,
    },
};

/// The state marks from `design/tokens/tokens.json` that a selected row keeps in
/// color: a mark is one glyph, never part of a word.
const MARKS: [char; 8] = ['◆', '✗', '◐', '●', '○', '✓', '◌', '!'];

static BACKGROUND: std::sync::OnceLock<Option<Background>> = std::sync::OnceLock::new();

/// The signal shared by tabs, refreshes and picker previews in this process.
/// Plain commands only inspect COLORFGBG; the board initializes before workers.
pub(crate) fn background() -> Option<Background> {
    *BACKGROUND.get_or_init(|| {
        std::env::var("COLORFGBG")
            .ok()
            .as_deref()
            .and_then(background::colorfgbg)
    })
}

pub(crate) fn configure_background(signal: Option<Background>) {
    let first = BACKGROUND.set(signal).is_ok();
    debug_assert!(first, "background detection precedes every board reader");
}

/// A color named in `squad.toml` or a layout default: a design token, or one
/// of the names Squad accepted before tokens, kept as their token.
pub fn role(name: &str) -> Option<Role> {
    Role::parse(name).or(match name {
        "red" => Some(Role::Blocked),
        "amber" => Some(Role::Waiting),
        "green" => Some(Role::Working),
        "blue" => Some(Role::Accent),
        "cyan" => Some(Role::Link),
        "magenta" => Some(Role::Review),
        _ => None,
    })
}

/// Whether a user may name this color: `default` (no color) or a token.
pub fn known(name: &str) -> bool {
    name == "default" || role(name).is_some()
}

/// The names a color setting accepts, for error messages.
pub fn names() -> String {
    let tokens: Vec<&str> = Role::ALL.iter().map(|role| role.name()).collect();
    format!(
        "default or a token ({}); red, amber, green, blue, cyan and magenta still work",
        tokens.join(", ")
    )
}

/// Which layer of the board's theme is wrong, with its message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// TMT's global `theme`: not the squad file's mistake.
    Global(String),
    /// The board-wide `[board.theme]`.
    Board(String),
    /// The squad's own `[squad.<name>.theme]`.
    Squad(String),
}

/// Resolve each token and base through core, board and squad layers.
pub fn board_theme(
    global: &[(String, String)],
    board: &[(String, String)],
    squad: &[(String, String)],
    place: &str,
) -> Result<Theme, Problem> {
    let check = |place: &str, settings: &[(String, String)]| {
        Theme::parse(
            place,
            settings
                .iter()
                .map(|(key, value)| (key.as_str(), value.as_str())),
        )
        .map_err(|error| error.to_string())
    };
    check("theme", global).map_err(Problem::Global)?;
    check("board.theme", board).map_err(Problem::Board)?;
    check(place, squad).map_err(Problem::Squad)?;
    let base = |settings: &[(String, String)]| {
        settings
            .iter()
            .find(|(key, _)| key == "base")
            .map(|(_, value)| value.clone())
    };
    let base = base(squad)
        .or_else(|| base(board))
        .or_else(|| base(global))
        .unwrap_or_else(|| "auto".into());
    let mut merged = vec![("base".to_owned(), base)];
    merged.extend(
        global
            .iter()
            .chain(board)
            .chain(squad)
            .filter(|(key, _)| key != "base")
            .cloned(),
    );
    // Both layers are valid on their own, so the merge is too; were it not,
    // the squad's layer is the one that changed it.
    check(place, &merged).map_err(Problem::Squad)
}

/// A theme at the terminal's depth, for the board.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Look {
    pub theme: Theme,
    pub depth: Depth,
}

impl Default for Look {
    fn default() -> Self {
        Self {
            theme: Theme::default(),
            depth: Depth::TrueColor,
        }
    }
}

impl Look {
    /// Identify a selected occurrence in an existing leading blank. Callers
    /// supply only admitted content prefixes, never hit or reveal envelopes.
    pub(crate) fn selected_prefix<'a>(
        &self,
        prefix: &'a str,
        selected: bool,
    ) -> std::borrow::Cow<'a, str> {
        if selected && let Some(rest) = prefix.strip_prefix(' ') {
            std::borrow::Cow::Owned(format!(">{rest}"))
        } else {
            std::borrow::Cow::Borrowed(prefix)
        }
    }

    /// The board draws on a terminal; `NO_COLOR` still turns color off.
    pub fn new(theme: Theme) -> Self {
        Self {
            theme: theme.resolve(background()),
            depth: Depth::from_env(std::env::var_os("NO_COLOR").is_none()),
        }
    }

    pub fn role(&self, role: Role) -> Style {
        screen::style(&self.theme, role, self.depth)
    }

    /// Keep the row's foreground while applying the selection background.
    /// A terminal without a background color still gets visible selection.
    pub fn selection(&self) -> Style {
        let style = self.role(Role::Text).patch(self.role(Role::Selection));
        if style.bg.is_none() {
            style.add_modifier(Modifier::REVERSED)
        } else {
            style
        }
    }

    /// A selected reverse row has one foreground; its semantic spans use bold.
    /// Real selection backgrounds and unselected spans keep their exact style.
    pub fn row_span(&self, selected: bool, style: Style, emphasize: bool) -> Style {
        if selected && self.selection().bg.is_none() {
            Style::new().add_modifier(if emphasize {
                Modifier::BOLD
            } else {
                Modifier::empty()
            })
        } else {
            style
        }
    }

    /// On the selection background `muted`, `dim`, the state colors, `accent`
    /// and `link` hold less than 4.5:1, so there their words paint in `text`.
    /// Marks keep their color (non-text, 3:1), except a dim or muted one. One
    /// pass over the finished frame, keyed on the background, so no surface
    /// classifies its own spans and a new surface follows the rule. Bold,
    /// underline and the background stay. Without a selection background
    /// (reverse fallback) `row_span` has already decided.
    pub fn selected_words(&self, buffer: &mut Buffer) {
        let Some(selection) = self.role(Role::Selection).bg else {
            return;
        };
        let Some(text) = self.role(Role::Text).fg else {
            return;
        };
        let weak = [Role::Muted, Role::Dim].map(|role| self.role(role).fg);
        let colored = [
            Role::Accent,
            Role::Waiting,
            Role::Working,
            Role::Review,
            Role::Blocked,
            Role::Link,
        ]
        .map(|role| self.role(role).fg);
        for cell in &mut buffer.content {
            // A reversed cell shows its foreground as the background: not a word.
            if cell.bg != selection
                || cell.fg == Color::Reset
                || cell.modifier.contains(Modifier::REVERSED)
            {
                continue;
            }
            let fg = Some(cell.fg);
            let mut symbol = cell.symbol().chars();
            let mark =
                matches!((symbol.next(), symbol.next()), (Some(c), None) if MARKS.contains(&c));
            if weak.contains(&fg) || (colored.contains(&fg) && !mark) {
                cell.fg = text;
            }
        }
    }

    /// A named color: its token's style, or no style for `default` and
    /// anything unknown.
    pub fn named(&self, name: &str) -> Style {
        role(name).map_or_else(Style::new, |role| self.role(role))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    #[test]
    fn occurrence_prefix_preserves_occupied_marks_and_existing_width() {
        let look = Look::default();
        for prefix in [" ◆ ", "   task", " ", "◆ ", "", ">"] {
            assert_eq!(look.selected_prefix(prefix, false), prefix);
            let selected = look.selected_prefix(prefix, true);
            assert_eq!(selected.chars().count(), prefix.chars().count());
            if let Some(rest) = prefix.strip_prefix(' ') {
                assert!(selected.starts_with('>'));
                assert_eq!(&selected[1..], rest);
            } else {
                assert_eq!(selected, prefix, "occupied prefix is not admitted");
            }
        }
    }

    #[test]
    fn names_are_tokens_or_the_older_color_names() {
        assert_eq!(role("blocked"), Some(Role::Blocked));
        assert_eq!(role("red"), Some(Role::Blocked));
        assert_eq!(role("amber"), Some(Role::Waiting));
        assert_eq!(role("magenta"), Some(Role::Review));
        assert_eq!(role("default"), None);
        assert!(known("default") && known("link") && known("cyan"));
        assert!(!known("orange") && !known("Red"));
    }

    #[test]
    fn the_board_is_tmt_unless_a_base_is_chosen_and_the_squad_wins() {
        let fg = |theme: Theme, role: Role| theme.style(role, Depth::TrueColor).get_fg_color();
        let none = board_theme(&[], &[], &[], "squad.p.theme").unwrap();
        assert_eq!(none.base, tmt_cli_style::Base::Auto);
        let global = settings(&[("base", "mono"), ("waiting", "red"), ("accent", "blue")]);
        let squad = settings(&[("waiting", "#010203")]);
        let merged = board_theme(&global, &[], &squad, "squad.p.theme").unwrap();
        assert_eq!(merged.base, tmt_cli_style::Base::Mono);
        let expected = Theme::parse(
            "theme",
            [("base", "mono"), ("waiting", "#010203"), ("accent", "blue")],
        )
        .unwrap();
        assert_eq!(
            fg(merged, Role::Waiting),
            fg(expected, Role::Waiting),
            "the squad's override wins"
        );
        assert_ne!(fg(merged, Role::Waiting), fg(none, Role::Waiting));
        assert_eq!(fg(merged, Role::Accent), fg(expected, Role::Accent));
        let own = board_theme(
            &global,
            &[],
            &settings(&[("base", "tmt-light")]),
            "squad.p.theme",
        )
        .unwrap();
        assert_eq!(own.base, tmt_cli_style::Base::TmtLight);
        assert!(matches!(
            board_theme(&[], &[], &settings(&[("waiting", "orange")]), "squad.p.theme"),
            Err(Problem::Squad(message)) if message.starts_with("`squad.p.theme.waiting`")
        ));
        assert!(matches!(
            board_theme(&settings(&[("base", "dark")]), &[], &[], "squad.p.theme"),
            Err(Problem::Global(message)) if message.starts_with("`theme.base`")
        ));
    }

    #[test]
    fn selection_has_a_background_or_a_visible_reverse_fallback() {
        for base in tmt_cli_style::Base::ALL {
            for depth in [Depth::TrueColor, Depth::Ansi16, Depth::None] {
                let look = Look {
                    theme: Theme::new(base),
                    depth,
                };
                let selection = look.selection();
                let background = look.role(Role::Selection).bg;
                assert_eq!(selection.bg, background);
                assert_eq!(selection.fg, look.role(Role::Text).fg);
                assert_eq!(
                    selection.add_modifier.contains(Modifier::REVERSED),
                    background.is_none(),
                    "{base:?} {depth:?}: selection stays visible",
                );
            }
        }
    }

    #[test]
    fn row_spans_change_only_for_selected_reverse_fallbacks() {
        for base in tmt_cli_style::Base::ALL {
            for depth in [Depth::TrueColor, Depth::Ansi16, Depth::None] {
                let look = Look {
                    theme: Theme::new(base),
                    depth,
                };
                for role in Role::ALL {
                    let original = look.role(role);
                    for emphasize in [false, true] {
                        assert_eq!(look.row_span(false, original, emphasize), original);
                        let selected = look.row_span(true, original, emphasize);
                        if look.selection().bg.is_some() {
                            assert_eq!(selected, original);
                        } else {
                            assert_eq!(selected.fg, None);
                            assert_eq!(selected.bg, None);
                            assert!(!selected.add_modifier.contains(Modifier::DIM));
                            assert_eq!(selected.add_modifier.contains(Modifier::BOLD), emphasize);
                        }
                    }
                }
            }
        }
    }

    fn frame(look: &Look, cells: &[(&str, Role, bool)]) -> Buffer {
        let mut buffer = Buffer::empty(ratatui::layout::Rect::new(0, 0, cells.len() as u16, 1));
        for (x, (symbol, role, selected)) in cells.iter().enumerate() {
            let style = if *selected {
                look.selection().patch(look.role(*role))
            } else {
                look.role(*role)
            };
            buffer[(x as u16, 0)].set_symbol(symbol).set_style(style);
        }
        look.selected_words(&mut buffer);
        buffer
    }

    #[test]
    fn selected_words_use_text_and_marks_keep_their_state_color() {
        for base in [tmt_cli_style::Base::Tmt, tmt_cli_style::Base::TmtLight] {
            let look = Look {
                theme: Theme::new(base),
                depth: Depth::TrueColor,
            };
            let fg = |role: Role| look.role(role).fg.unwrap();
            let words = [
                Role::Muted,
                Role::Dim,
                Role::Waiting,
                Role::Working,
                Role::Review,
                Role::Blocked,
            ];
            let cells = words
                .iter()
                .map(|role| ("x", *role, true))
                .chain([
                    ("◆", Role::Waiting, true),
                    ("✗", Role::Blocked, true),
                    ("◐", Role::Review, true),
                    ("●", Role::Working, true),
                    ("○", Role::Dim, true),
                    ("x", Role::Accent, true),
                    ("x", Role::Link, true),
                    ("x", Role::Text, true),
                    ("x", Role::Blocked, false),
                    ("●", Role::Accent, true),
                    ("x", Role::Accent, false),
                ])
                .collect::<Vec<_>>();
            let buffer = frame(&look, &cells);
            let at = |x: u16| &buffer[(x, 0)];
            let bg = look.role(Role::Selection).bg.unwrap();
            for x in 0..6 {
                assert_eq!(at(x).fg, fg(Role::Text), "{base:?} word {x}");
                assert_eq!(at(x).bg, bg);
            }
            for (x, role) in [
                (6, Role::Waiting),
                (7, Role::Blocked),
                (8, Role::Review),
                (9, Role::Working),
            ] {
                assert_eq!(at(x).fg, fg(role), "{base:?} mark {x} keeps its color");
            }
            assert_eq!(
                at(10).fg,
                fg(Role::Text),
                "a dim mark is not readable on the selection"
            );
            assert_eq!(at(11).fg, fg(Role::Text), "accent words paint in text");
            assert_eq!(at(12).fg, fg(Role::Text), "link words paint in text");
            assert_eq!(at(13).fg, fg(Role::Text));
            assert_eq!(
                at(14).fg,
                fg(Role::Blocked),
                "unselected cells are untouched"
            );
        }
    }

    #[test]
    fn a_reversed_cell_is_a_block_not_a_word() {
        let look = Look::default();
        let mut buffer = Buffer::empty(ratatui::layout::Rect::new(0, 0, 1, 1));
        buffer[(0, 0)].set_symbol("x").set_style(
            look.selection()
                .patch(look.role(Role::Accent))
                .add_modifier(Modifier::REVERSED),
        );
        look.selected_words(&mut buffer);
        assert_eq!(buffer[(0, 0)].fg, look.role(Role::Accent).fg.unwrap());
    }

    #[test]
    fn selected_words_keep_modifiers_and_do_nothing_without_a_selection_background() {
        let look = Look::default();
        let mut buffer = Buffer::empty(ratatui::layout::Rect::new(0, 0, 1, 1));
        buffer[(0, 0)].set_symbol("x").set_style(
            look.role(Role::Blocked)
                .patch(look.selection())
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        );
        look.selected_words(&mut buffer);
        assert_eq!(buffer[(0, 0)].fg, look.role(Role::Text).fg.unwrap());
        assert!(
            buffer[(0, 0)]
                .modifier
                .contains(Modifier::BOLD | Modifier::UNDERLINED)
        );
        for look in [
            Look {
                theme: Theme::new(tmt_cli_style::Base::Terminal),
                depth: Depth::Ansi16,
            },
            Look {
                theme: Theme::default(),
                depth: Depth::None,
            },
        ] {
            let mut buffer = Buffer::empty(ratatui::layout::Rect::new(0, 0, 1, 1));
            buffer[(0, 0)]
                .set_symbol("x")
                .set_style(look.selection().patch(look.role(Role::Blocked)));
            let before = buffer.clone();
            look.selected_words(&mut buffer);
            assert_eq!(buffer, before);
        }
    }

    #[test]
    fn no_color_draws_no_color() {
        let look = Look {
            theme: Theme::default(),
            depth: Depth::None,
        };
        assert_eq!(look.named("blocked"), Style::new());
        let look = Look::default();
        assert_ne!(look.named("blocked"), Style::new());
        assert_eq!(look.named("default"), Style::new());
    }
}

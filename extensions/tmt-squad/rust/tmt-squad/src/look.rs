//! How Squad draws: the user's theme, from TMT's global config, with a
//! squad's own `[squad.<name>.theme]` over it. Squad names no colors of its
//! own; every color is a design token that tmt-cli-style renders.

use ratatui::style::Style;
use tmt_cli_style::{Depth, Role, Theme, theme::screen};

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
    /// The squad's own `[squad.<name>.theme]`.
    Squad(String),
}

/// The board's theme: the squad's base, else the global one, else `tmt`;
/// then the global overrides, then the squad's, the later winning. Each
/// layer is checked on its own, so a mistake names its layer and its place
/// (`theme.<key>` or `squad.<name>.theme.<key>`).
pub fn theme(
    global: &[(String, String)],
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
    check(place, squad).map_err(Problem::Squad)?;
    let base = |settings: &[(String, String)]| {
        settings
            .iter()
            .find(|(key, _)| key == "base")
            .map(|(_, value)| value.clone())
    };
    let base = base(squad)
        .or_else(|| base(global))
        .unwrap_or_else(|| "tmt".into());
    let mut merged = vec![("base".to_owned(), base)];
    merged.extend(
        global
            .iter()
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
    /// The board draws on a terminal; `NO_COLOR` still turns color off.
    pub fn new(theme: Theme) -> Self {
        Self {
            theme,
            depth: Depth::from_env(std::env::var_os("NO_COLOR").is_none()),
        }
    }

    pub fn role(&self, role: Role) -> Style {
        screen::style(&self.theme, role, self.depth)
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
        let none = theme(&[], &[], "squad.p.theme").unwrap();
        assert_eq!(none.base, tmt_cli_style::Base::Tmt);
        let global = settings(&[("base", "mono"), ("waiting", "red"), ("accent", "blue")]);
        let squad = settings(&[("waiting", "#010203")]);
        let merged = theme(&global, &squad, "squad.p.theme").unwrap();
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
        let own = theme(
            &global,
            &settings(&[("base", "tmt-light")]),
            "squad.p.theme",
        )
        .unwrap();
        assert_eq!(own.base, tmt_cli_style::Base::TmtLight);
        assert!(matches!(
            theme(&[], &settings(&[("waiting", "orange")]), "squad.p.theme"),
            Err(Problem::Squad(message)) if message.starts_with("`squad.p.theme.waiting`")
        ));
        assert!(matches!(
            theme(&settings(&[("base", "dark")]), &[], "squad.p.theme"),
            Err(Problem::Global(message)) if message.starts_with("`theme.base`")
        ));
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

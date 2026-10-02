//! The command-line theme. The terminal's own 16 colors stay the default, so
//! the user's terminal theme wins; a user who sets `theme.base` in the
//! global config gets that theme on the command line too (the board always
//! defaults to `auto`). It is read once, at startup, and only when output may
//! be colored.

use tmt_adapters::config::{ConfigFiles, ConfigPaths};
use tmt_cli_style::{Interaction, Theme};

/// Sets the process's theme when output may be colored and the user chose
/// a base. A missing or invalid theme changes nothing here: every command
/// keeps working, and `tmt config show` reports what is wrong.
pub fn configure(json: bool) {
    if !Interaction::detect(json).may_color() {
        return;
    }
    let Ok(paths) = ConfigPaths::discover() else {
        return;
    };
    let files = ConfigFiles { paths };
    if let Some(theme) = files
        .theme()
        .ok()
        .and_then(Result::ok)
        .and_then(|settings| chosen(&settings))
    {
        tmt_cli_style::theme::configure(theme);
    }
}

/// The theme for the command line: only once the user set a base.
pub fn chosen(settings: &[(String, String)]) -> Option<Theme> {
    if !settings.iter().any(|(key, _)| key == "base") {
        return None;
    }
    parse(settings).ok()
}

/// The global file's theme with its meaning checked, for every key.
pub fn parse(settings: &[(String, String)]) -> Result<Theme, tmt_cli_style::theme::ThemeError> {
    let theme = Theme::parse(
        "theme",
        settings
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str())),
    )?;
    if theme.base == tmt_cli_style::Base::Auto {
        return Err(tmt_cli_style::theme::ThemeError {
            key: "theme.base".into(),
            message: "auto is a board theme; use tmt sq theme set auto or the board picker. It is valid in squad.toml [board.theme] and [squad.<name>.theme], not global config.json".into(),
        });
    }
    Ok(theme)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tmt_cli_style::{Base, Depth, Role};

    fn settings(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    #[test]
    fn the_command_line_is_themed_only_once_a_base_is_chosen() {
        assert_eq!(chosen(&[]), None);
        assert_eq!(
            chosen(&settings(&[("waiting", "#e0a458")])),
            None,
            "overrides alone keep the terminal's colors"
        );
        let theme = chosen(&settings(&[("base", "tmt-light"), ("waiting", "red")])).unwrap();
        assert_eq!(theme.base, Base::TmtLight);
        assert_eq!(
            theme.style(Role::Waiting, Depth::TrueColor).get_fg_color(),
            Some(tmt_cli_style::AnsiColor::Red.into())
        );
        assert_eq!(
            chosen(&settings(&[("base", "dark")])),
            None,
            "invalid: unchanged"
        );
    }

    #[test]
    fn global_auto_is_rejected_without_changing_the_shared_board_parser() {
        let global = settings(&[("base", "auto"), ("waiting", "red")]);
        let error = parse(&global).unwrap_err();
        assert_eq!(error.key, "theme.base");
        assert!(error.message.contains("auto is a board theme"));
        assert!(error.message.contains("tmt sq theme set auto"));
        assert!(error.message.contains("[board.theme]"));
        assert!(error.message.contains("[squad.<name>.theme]"));
        assert_eq!(
            chosen(&global),
            None,
            "invalid global themes do not color the CLI"
        );
        for layer in ["board.theme", "squad.product.theme"] {
            let board = Theme::parse(layer, [("base", "auto"), ("waiting", "red")]).unwrap();
            assert_eq!(board.base, Base::Auto);
            assert_eq!(
                board.style(Role::Waiting, Depth::TrueColor).get_fg_color(),
                Some(tmt_cli_style::AnsiColor::Red.into())
            );
        }
    }

    #[test]
    fn a_mistake_names_its_key() {
        assert_eq!(
            parse(&settings(&[("base", "tmt"), ("waiting", "orange")]))
                .unwrap_err()
                .key,
            "theme.waiting"
        );
    }
}

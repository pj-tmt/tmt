//! Semantic color tokens over the terminal's own 16-color palette, and the
//! single per-stream decision whether to use them.

use crate::theme::{Depth, Theme};
use anstream::{AutoStream, ColorChoice};
use anstyle::{AnsiColor, Effects, Style};
use clap::builder::Styles;
use std::io;

/// A meaning, never a literal color. Each maps to one of the terminal's 16
/// palette entries (or an effect), so the user's theme decides the shade.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token {
    Accent,
    Ok,
    Warn,
    Error,
    Dim,
    /// Section titles: bold, uncolored.
    Title,
    /// Commands and flags a reader types: bold, uncolored.
    Literal,
    /// An agent driver's address, in the design token its descriptor's hue
    /// names; dimmed when it declares none. The CLI maps the descriptor's
    /// hue, so this crate names no driver.
    Driver(Option<crate::Role>),
}

/// A design token's entry in the terminal's 16-color palette, for a driver's
/// hue on an unthemed terminal.
fn sixteen(role: crate::Role) -> Option<AnsiColor> {
    use crate::Role;
    match role {
        Role::Accent => Some(AnsiColor::Blue),
        Role::Working => Some(AnsiColor::Green),
        Role::Waiting => Some(AnsiColor::Yellow),
        Role::Blocked => Some(AnsiColor::Red),
        Role::Review => Some(AnsiColor::Magenta),
        Role::Link => Some(AnsiColor::Cyan),
        Role::Text | Role::Muted | Role::Dim | Role::Selection => None,
    }
}

impl Token {
    /// The palette entry, when the token has one. Also the source for
    /// full-screen views (such as a TUI) that cannot use line output.
    pub fn color(self) -> Option<AnsiColor> {
        match self {
            Self::Accent => Some(AnsiColor::Blue),
            Self::Ok => Some(AnsiColor::Green),
            Self::Warn => Some(AnsiColor::Yellow),
            Self::Error => Some(AnsiColor::Red),
            Self::Driver(role) => role.and_then(sixteen),
            Self::Dim | Self::Title | Self::Literal => None,
        }
    }

    /// The design token this shows as in a [`crate::Theme`]; None for the
    /// bold-only tokens.
    pub fn role(self) -> Option<crate::Role> {
        use crate::Role;
        match self {
            Self::Accent => Some(Role::Accent),
            Self::Ok => Some(Role::Working),
            Self::Warn => Some(Role::Waiting),
            Self::Error => Some(Role::Blocked),
            Self::Dim | Self::Driver(None) => Some(Role::Dim),
            Self::Driver(role) => role,
            Self::Title | Self::Literal => None,
        }
    }

    pub fn effects(self) -> Effects {
        match self {
            Self::Dim | Self::Driver(None) => Effects::DIMMED,
            Self::Title | Self::Literal => Effects::BOLD,
            _ => Effects::new(),
        }
    }

    pub fn style(self) -> Style {
        Style::new()
            .fg_color(self.color().map(Into::into))
            .effects(self.effects())
    }

    /// The style under `theme`: its design token's look, or the plain
    /// 16-color style when there is no theme or the token has no design
    /// token (the bold-only ones).
    pub fn themed(self, theme: Option<(Theme, Depth)>) -> Style {
        match (theme, self.role()) {
            (Some((theme, depth)), Some(role)) => theme.style(role, depth),
            _ => self.style(),
        }
    }
}

/// How one output stream renders: decided once, then passed to every renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Terminal {
    pub color: bool,
    /// Columns available. Known only when stdout is a terminal: comfy-table's
    /// `width()` asks the terminal only after its `is_tty` check. Without a
    /// width nothing is truncated, so piped output keeps every value whole.
    /// A terminal that reports 0 columns (a pty nobody sized, such as under
    /// `script` or some CI and SSH sessions) has no known width either.
    pub width: Option<u16>,
    /// The user's theme and this stream's color depth; None renders the
    /// terminal's own 16 colors. Only a colored stream has one.
    pub theme: Option<(Theme, Depth)>,
}

impl Terminal {
    pub const PLAIN: Self = Self {
        color: false,
        width: None,
        theme: None,
    };

    /// Standard output: plain for `--json`, otherwise color unless the stream
    /// is not a terminal or `NO_COLOR`/`CLICOLOR` say otherwise.
    pub fn stdout(json: bool) -> Self {
        if json {
            return Self::PLAIN;
        }
        let color = AutoStream::choice(&io::stdout()) != ColorChoice::Never;
        Self {
            color,
            width: known_width(comfy_table::Table::new().width()),
            theme: themed(color),
        }
    }

    /// Standard error, where errors and hints go; never truncated.
    pub fn stderr() -> Self {
        let color = AutoStream::choice(&io::stderr()) != ColorChoice::Never;
        Self {
            color,
            width: None,
            theme: themed(color),
        }
    }

    pub fn paint(self, token: Token, text: &str) -> String {
        if self.color && !text.is_empty() {
            let style = token.themed(self.theme);
            format!("{style}{text}{style:#}")
        } else {
            text.to_owned()
        }
    }
}

/// Help uses the same tokens as output, so both read as one product.
pub fn help_styles() -> Styles {
    Styles::styled()
        .header(Token::Title.style())
        .usage(Token::Title.style())
        .literal(Token::Literal.style())
        .placeholder(Style::new())
        .error(Token::Error.style().bold())
        .valid(Token::Ok.style())
        .invalid(Token::Warn.style())
        .context(Token::Dim.style())
}

/// The configured theme at this stream's depth, for a colored stream.
fn themed(color: bool) -> Option<(Theme, Depth)> {
    crate::theme::configured()
        .filter(|_| color)
        .map(|theme| (theme, Depth::from_env(true)))
}

fn known_width(reported: Option<u16>) -> Option<u16> {
    reported.filter(|columns| *columns > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_use_only_the_sixteen_color_palette() {
        let tokens = [
            Token::Accent,
            Token::Ok,
            Token::Warn,
            Token::Error,
            Token::Dim,
            Token::Title,
            Token::Literal,
            Token::Driver(Some(crate::Role::Review)),
            Token::Driver(None),
        ];
        for token in tokens {
            let style = token.style().render().to_string();
            assert!(
                !style.contains("38;2") && !style.contains("38;5"),
                "{style}"
            );
        }
        assert_eq!(Token::Driver(None).effects(), Effects::DIMMED);
    }

    #[test]
    fn a_plain_terminal_never_emits_escapes() {
        assert_eq!(Terminal::PLAIN.paint(Token::Error, "failed"), "failed");
        let color = Terminal {
            color: true,
            width: None,
            theme: None,
        };
        assert_eq!(color.paint(Token::Ok, "done"), "\u{1b}[32mdone\u{1b}[0m");
        assert_eq!(color.paint(Token::Ok, ""), "");
        assert_eq!(Terminal::stdout(true), Terminal::PLAIN);
    }

    /// A theme changes only the look of tokens that have a design token;
    /// without one, output is exactly the 16-color output.
    #[test]
    fn a_themed_terminal_paints_design_tokens_and_keeps_bold_tokens() {
        use crate::{Base, Depth, Theme};
        let themed = |base: Base, depth: Depth| Terminal {
            color: true,
            width: None,
            theme: Some((Theme::new(base), depth)),
        };
        let truecolor = themed(Base::Tmt, Depth::TrueColor);
        assert_eq!(
            truecolor.paint(Token::Warn, "wait"),
            "\u{1b}[38;2;255;158;100mwait\u{1b}[0m"
        );
        assert_eq!(
            truecolor.paint(Token::Title, "AGENTS"),
            "\u{1b}[1mAGENTS\u{1b}[0m",
            "bold-only tokens have no design token"
        );
        assert_eq!(
            themed(Base::Tmt, Depth::Ansi16).paint(Token::Ok, "done"),
            "\u{1b}[32mdone\u{1b}[0m",
            "16 colors: the designed terminal column"
        );
        assert_eq!(
            themed(Base::Mono, Depth::TrueColor).paint(Token::Error, "failed"),
            "\u{1b}[1mfailed\u{1b}[0m"
        );
        let plain = Terminal {
            color: true,
            width: None,
            theme: None,
        };
        for token in [
            Token::Accent,
            Token::Ok,
            Token::Warn,
            Token::Error,
            Token::Dim,
        ] {
            assert_eq!(
                plain.paint(token, "x"),
                format!("{}x{:#}", token.style(), token.style()),
                "{token:?}"
            );
        }
        // A plain stream stays plain whatever the theme.
        let no_color = Terminal {
            color: false,
            width: None,
            theme: Some((Theme::default(), Depth::TrueColor)),
        };
        assert_eq!(no_color.paint(Token::Error, "failed"), "failed");
    }

    #[test]
    fn terminal_dim_matches_the_unthemed_effect_without_a_foreground() {
        use crate::Base;
        let dim = Style::new().dimmed();
        for token in [Token::Dim, Token::Driver(None)] {
            assert_eq!(token.style(), dim);
            for base in Base::ALL {
                assert_eq!(token.themed(Some((Theme::new(base), Depth::Ansi16))), dim);
            }
            assert_eq!(
                token.themed(Some((Theme::new(Base::Terminal), Depth::TrueColor))),
                dim,
            );
        }
        let overridden = Theme::parse("theme", [("base", "terminal"), ("dim", "bold")]).unwrap();
        assert_eq!(
            Token::Dim.themed(Some((overridden, Depth::Ansi16))),
            Style::new().bold()
        );
    }

    #[test]
    fn a_terminal_reporting_zero_columns_has_no_known_width() {
        assert_eq!(known_width(Some(0)), None);
        assert_eq!(known_width(None), None);
        assert_eq!(known_width(Some(80)), Some(80));
    }
}

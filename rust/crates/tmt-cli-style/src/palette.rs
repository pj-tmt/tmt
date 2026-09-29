//! Semantic color tokens over the terminal's own 16-color palette, and the
//! single per-stream decision whether to use them.

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
    /// An agent driver's address, in the color its descriptor declares;
    /// dimmed when it declares none. The CLI maps the descriptor's hue, so
    /// this crate names no driver.
    Driver(Option<AnsiColor>),
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
            Self::Driver(color) => color,
            Self::Dim | Self::Title | Self::Literal => None,
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
}

impl Terminal {
    pub const PLAIN: Self = Self {
        color: false,
        width: None,
    };

    /// Standard output: plain for `--json`, otherwise color unless the stream
    /// is not a terminal or `NO_COLOR`/`CLICOLOR` say otherwise.
    pub fn stdout(json: bool) -> Self {
        if json {
            return Self::PLAIN;
        }
        Self {
            color: AutoStream::choice(&io::stdout()) != ColorChoice::Never,
            width: known_width(comfy_table::Table::new().width()),
        }
    }

    /// Standard error, where errors and hints go; never truncated.
    pub fn stderr() -> Self {
        Self {
            color: AutoStream::choice(&io::stderr()) != ColorChoice::Never,
            width: None,
        }
    }

    pub fn paint(self, token: Token, text: &str) -> String {
        if self.color && !text.is_empty() {
            let style = token.style();
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
            Token::Driver(Some(AnsiColor::Magenta)),
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
        };
        assert_eq!(color.paint(Token::Ok, "done"), "\u{1b}[32mdone\u{1b}[0m");
        assert_eq!(color.paint(Token::Ok, ""), "");
        assert_eq!(Terminal::stdout(true), Terminal::PLAIN);
    }

    #[test]
    fn a_terminal_reporting_zero_columns_has_no_known_width() {
        assert_eq!(known_width(Some(0)), None);
        assert_eq!(known_width(None), None);
        assert_eq!(known_width(Some(80)), Some(80));
    }
}

//! Color themes: what each color role looks like, for the command line and
//! full-screen views alike. The roles and the built-in values are the design
//! tokens (`site/src/design/tokens.json`, checked against this file by a
//! test); a theme is a base plus per-role overrides. Nothing outside this
//! crate names a literal color: callers ask a [`Theme`] for a role's style at
//! the [`Depth`] their stream supports.

use anstyle::{AnsiColor, Color, Effects, RgbColor, Style};
use std::fmt;

/// What a color means. Every state also shows as a mark or a word, so a
/// role never carries meaning by color alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Role {
    /// Normal text.
    Text,
    /// Labels and headers.
    Muted,
    /// Secondary text, borders, empty values.
    Dim,
    /// Focus, the selected tab, keys and links in prose.
    Accent,
    /// Waiting on you; anything that needs attention.
    Waiting,
    /// A member at work; success.
    Working,
    /// In review, waiting on someone else.
    Review,
    /// Blocked, failed, errors.
    Blocked,
    /// Links you can open.
    Link,
    /// The selected row, as a background.
    Selection,
}

impl Role {
    pub const ALL: [Self; 10] = [
        Self::Text,
        Self::Muted,
        Self::Dim,
        Self::Accent,
        Self::Waiting,
        Self::Working,
        Self::Review,
        Self::Blocked,
        Self::Link,
        Self::Selection,
    ];

    /// The token's name in configuration and the design tokens.
    pub fn name(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Muted => "muted",
            Self::Dim => "dim",
            Self::Accent => "accent",
            Self::Waiting => "waiting",
            Self::Working => "working",
            Self::Review => "review",
            Self::Blocked => "blocked",
            Self::Link => "link",
            Self::Selection => "selection",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|role| role.name() == name)
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// How much color a stream can show; decided once per stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Depth {
    /// No color at all: `NO_COLOR`, a pipe, a file, `--json`.
    None,
    /// The terminal's 16 palette colors.
    Ansi16,
    /// 24-bit color.
    TrueColor,
}

impl Depth {
    /// `color` is the stream's own decision (a terminal, no `NO_COLOR`);
    /// `colorterm` is `$COLORTERM`, where terminals announce 24-bit color.
    pub fn detect(color: bool, colorterm: Option<&str>) -> Self {
        match (color, colorterm) {
            (false, _) => Self::None,
            (true, Some("truecolor" | "24bit")) => Self::TrueColor,
            (true, _) => Self::Ansi16,
        }
    }

    /// From the process environment.
    pub fn from_env(color: bool) -> Self {
        Self::detect(color, std::env::var("COLORTERM").ok().as_deref())
    }
}

/// One role's appearance before it meets a stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Paint {
    /// Left to the terminal: no color, no effect.
    Plain,
    Rgb(u8, u8, u8),
    Ansi(AnsiColor),
    Bold,
    Dimmed,
    Reverse,
}

/// The ANSI names a user may write, as in the design tokens' terminal column.
const ANSI_NAMES: [(&str, AnsiColor); 16] = [
    ("black", AnsiColor::Black),
    ("red", AnsiColor::Red),
    ("green", AnsiColor::Green),
    ("yellow", AnsiColor::Yellow),
    ("blue", AnsiColor::Blue),
    ("magenta", AnsiColor::Magenta),
    ("cyan", AnsiColor::Cyan),
    ("white", AnsiColor::White),
    ("bright black", AnsiColor::BrightBlack),
    ("bright red", AnsiColor::BrightRed),
    ("bright green", AnsiColor::BrightGreen),
    ("bright yellow", AnsiColor::BrightYellow),
    ("bright blue", AnsiColor::BrightBlue),
    ("bright magenta", AnsiColor::BrightMagenta),
    ("bright cyan", AnsiColor::BrightCyan),
    ("bright white", AnsiColor::BrightWhite),
];

/// Typical renderings of the 16 colors, only to find the nearest one for a
/// hex value on a terminal without 24-bit color.
const ANSI_RGB: [(u8, u8, u8); 16] = [
    (0, 0, 0),
    (205, 49, 49),
    (13, 188, 121),
    (229, 229, 16),
    (36, 114, 200),
    (188, 63, 188),
    (17, 168, 205),
    (229, 229, 229),
    (102, 102, 102),
    (241, 76, 76),
    (35, 209, 139),
    (245, 245, 67),
    (59, 142, 234),
    (214, 112, 214),
    (41, 184, 219),
    (255, 255, 255),
];

impl Paint {
    /// `#rrggbb`, an ANSI name such as `blue` or `bright black`, `default`,
    /// `bold`, `dim` or `reverse`.
    pub fn parse(text: &str) -> Option<Self> {
        if let Some(hex) = text.strip_prefix('#') {
            let channel = |at: usize| u8::from_str_radix(hex.get(at..at + 2)?, 16).ok();
            return (hex.len() == 6 && hex.is_ascii())
                .then(|| Some(Self::Rgb(channel(0)?, channel(2)?, channel(4)?)))
                .flatten();
        }
        match text {
            "default" => Some(Self::Plain),
            "bold" => Some(Self::Bold),
            "dim" => Some(Self::Dimmed),
            "reverse" => Some(Self::Reverse),
            name => ANSI_NAMES
                .iter()
                .find(|(known, _)| *known == name)
                .map(|(_, color)| Self::Ansi(*color)),
        }
    }

    fn nearest(red: u8, green: u8, blue: u8) -> AnsiColor {
        let distance = |(r, g, b): (u8, u8, u8)| {
            let d = |a: u8, b: u8| (i32::from(a) - i32::from(b)).pow(2);
            d(r, red) + d(g, green) + d(b, blue)
        };
        let index = (0..ANSI_RGB.len())
            .min_by_key(|index| distance(ANSI_RGB[*index]))
            .expect("16 colors");
        ANSI_NAMES[index].1
    }

    fn color(self, depth: Depth) -> Option<Color> {
        match (self, depth) {
            (_, Depth::None) => None,
            (Self::Rgb(r, g, b), Depth::TrueColor) => Some(Color::Rgb(RgbColor(r, g, b))),
            (Self::Rgb(r, g, b), Depth::Ansi16) => Some(Self::nearest(r, g, b).into()),
            (Self::Ansi(color), _) => Some(color.into()),
            _ => None,
        }
    }

    fn effects(self, depth: Depth) -> Effects {
        match (self, depth) {
            (_, Depth::None) => Effects::new(),
            (Self::Bold, _) => Effects::BOLD,
            (Self::Dimmed, _) => Effects::DIMMED,
            (Self::Reverse, _) => Effects::INVERT,
            _ => Effects::new(),
        }
    }
}

/// A built-in theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Base {
    /// Soft truecolor for dark terminals; the default.
    Tmt,
    /// The same palette for light backgrounds.
    TmtLight,
    /// The terminal's own 16 colors, so its color scheme decides.
    Terminal,
    /// Bold and dim only.
    Mono,
}

impl Base {
    pub const ALL: [Self; 4] = [Self::Tmt, Self::TmtLight, Self::Terminal, Self::Mono];

    pub fn name(self) -> &'static str {
        match self {
            Self::Tmt => "tmt",
            Self::TmtLight => "tmt-light",
            Self::Terminal => "terminal",
            Self::Mono => "mono",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|base| base.name() == name)
    }

    /// The design tokens' terminal column: what `terminal` shows, and what
    /// `tmt` and `tmt-light` fall back to on a 16-color terminal.
    fn terminal(role: Role) -> Paint {
        match role {
            Role::Text => Paint::Plain,
            Role::Muted => Paint::Ansi(AnsiColor::White),
            Role::Dim => Paint::Dimmed,
            Role::Accent => Paint::Ansi(AnsiColor::Blue),
            Role::Waiting => Paint::Ansi(AnsiColor::Yellow),
            Role::Working => Paint::Ansi(AnsiColor::Green),
            Role::Review => Paint::Ansi(AnsiColor::Magenta),
            Role::Blocked => Paint::Ansi(AnsiColor::Red),
            Role::Link => Paint::Ansi(AnsiColor::Cyan),
            Role::Selection => Paint::Reverse,
        }
    }

    /// The design tokens' dark and light values.
    fn truecolor(role: Role, light: bool) -> Paint {
        let (dark, bright) = match role {
            Role::Text => ((0xC0, 0xCA, 0xF5), (0x34, 0x3B, 0x58)),
            Role::Muted => ((0x9A, 0xA5, 0xCE), (0x5A, 0x63, 0x90)),
            Role::Dim => ((0x7A, 0x83, 0xAE), (0x68, 0x70, 0x9A)),
            Role::Accent => ((0x7A, 0xA2, 0xF7), (0x1F, 0x5F, 0xBF)),
            Role::Waiting => ((0xFF, 0x9E, 0x64), (0x96, 0x50, 0x27)),
            Role::Working => ((0x9E, 0xCE, 0x6A), (0x4F, 0x6A, 0x33)),
            Role::Review => ((0xBB, 0x9A, 0xF7), (0x78, 0x47, 0xBD)),
            Role::Blocked => ((0xF7, 0x76, 0x8E), (0xB6, 0x2C, 0x3B)),
            Role::Link => ((0x7D, 0xCF, 0xFF), (0x00, 0x6B, 0x8F)),
            Role::Selection => ((0x28, 0x34, 0x57), (0xD5, 0xE0, 0xF5)),
        };
        let (r, g, b) = if light { bright } else { dark };
        Paint::Rgb(r, g, b)
    }

    fn mono(role: Role) -> Paint {
        match role {
            Role::Text | Role::Working | Role::Link => Paint::Plain,
            Role::Muted | Role::Dim => Paint::Dimmed,
            Role::Accent | Role::Waiting | Role::Review | Role::Blocked => Paint::Bold,
            Role::Selection => Paint::Reverse,
        }
    }

    fn paint(self, role: Role) -> Paint {
        match self {
            Self::Tmt => Self::truecolor(role, false),
            Self::TmtLight => Self::truecolor(role, true),
            Self::Terminal => Self::terminal(role),
            Self::Mono => Self::mono(role),
        }
    }
}

/// A configuration mistake, with the setting it is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeError {
    pub key: String,
    pub message: String,
}

impl fmt::Display for ThemeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "`{}` {}", self.key, self.message)
    }
}

/// A base with per-role overrides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub base: Base,
    overrides: [Option<Paint>; 10],
}

impl Default for Theme {
    fn default() -> Self {
        Self::new(Base::Tmt)
    }
}

impl Theme {
    pub fn new(base: Base) -> Self {
        Self {
            base,
            overrides: [None; 10],
        }
    }

    /// `base` and `role = paint` settings, as `[theme]` spells them. `place`
    /// prefixes each key in errors, such as `theme` or `squad.product.theme`.
    pub fn parse<'a>(
        place: &str,
        settings: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Result<Self, ThemeError> {
        let mut theme = Self::default();
        let mut overrides = Vec::new();
        for (key, value) in settings {
            let error = |message: String| ThemeError {
                key: format!("{place}.{key}"),
                message,
            };
            if key == "base" {
                theme.base = Base::parse(value)
                    .ok_or_else(|| error("must be tmt, tmt-light, terminal or mono.".to_owned()))?;
            } else {
                let role = Role::parse(key).ok_or_else(|| {
                    error(format!(
                        "is not a theme setting; use base or a token: {}.",
                        Role::ALL.map(Role::name).join(", ")
                    ))
                })?;
                let paint = Paint::parse(value).ok_or_else(|| {
                    error(
                        "must be #rrggbb, a color name such as blue or bright black, \
                         default, bold, dim or reverse."
                            .to_owned(),
                    )
                })?;
                overrides.push((role, paint));
            }
        }
        for (role, paint) in overrides {
            theme = theme.with(role, paint);
        }
        Ok(theme)
    }

    /// This theme with one role changed, as a per-squad override does.
    pub fn with(mut self, role: Role, paint: Paint) -> Self {
        self.overrides[role.index()] = Some(paint);
        self
    }

    fn paint(&self, role: Role, depth: Depth) -> Paint {
        if let Some(paint) = self.overrides[role.index()] {
            return paint;
        }
        match (self.base, depth) {
            // A 16-color terminal gets the designed fallback, not the
            // nearest shade of a truecolor value.
            (Base::Tmt | Base::TmtLight, Depth::Ansi16) => Base::terminal(role),
            (base, _) => base.paint(role),
        }
    }

    /// The role's text style. The selection is a background.
    pub fn style(&self, role: Role, depth: Depth) -> Style {
        let paint = self.paint(role, depth);
        let style = Style::new().effects(paint.effects(depth));
        match (role, paint.color(depth)) {
            (Role::Selection, Some(color)) => style.bg_color(Some(color)),
            (_, color) => style.fg_color(color),
        }
    }
}

/// The theme the user chose for command-line output, set once at startup
/// by the executable that read the configuration. Unset means the
/// terminal's own 16 colors, exactly as without themes.
static CONFIGURED: std::sync::OnceLock<Theme> = std::sync::OnceLock::new();

/// Sets the process's theme. Only the executable's startup calls this, and
/// only once: a second call is a bug. Tests pass a [`Theme`] explicitly
/// instead.
pub fn configure(theme: Theme) {
    let first = CONFIGURED.set(theme).is_ok();
    debug_assert!(
        first,
        "the command-line theme is configured once, at startup"
    );
}

/// The theme set at startup, if any.
pub fn configured() -> Option<Theme> {
    CONFIGURED.get().copied()
}

/// Full-screen views draw with ratatui; they get the same styles.
#[cfg(feature = "ratatui")]
pub mod screen {
    use super::{Depth, Role, Theme};
    use anstyle::{Color, Effects};
    use ratatui::style::{Color as ScreenColor, Modifier, Style};

    fn color(color: Color) -> ScreenColor {
        use anstyle::AnsiColor as Ansi;
        match color {
            Color::Rgb(rgb) => ScreenColor::Rgb(rgb.0, rgb.1, rgb.2),
            Color::Ansi256(index) => ScreenColor::Indexed(index.0),
            Color::Ansi(ansi) => match ansi {
                Ansi::Black => ScreenColor::Black,
                Ansi::Red => ScreenColor::Red,
                Ansi::Green => ScreenColor::Green,
                Ansi::Yellow => ScreenColor::Yellow,
                Ansi::Blue => ScreenColor::Blue,
                Ansi::Magenta => ScreenColor::Magenta,
                Ansi::Cyan => ScreenColor::Cyan,
                Ansi::White => ScreenColor::Gray,
                Ansi::BrightBlack => ScreenColor::DarkGray,
                Ansi::BrightRed => ScreenColor::LightRed,
                Ansi::BrightGreen => ScreenColor::LightGreen,
                Ansi::BrightYellow => ScreenColor::LightYellow,
                Ansi::BrightBlue => ScreenColor::LightBlue,
                Ansi::BrightMagenta => ScreenColor::LightMagenta,
                Ansi::BrightCyan => ScreenColor::LightCyan,
                Ansi::BrightWhite => ScreenColor::White,
            },
        }
    }

    /// The role's style for a ratatui cell.
    pub fn style(theme: &Theme, role: Role, depth: Depth) -> Style {
        let text = theme.style(role, depth);
        let effects = text.get_effects();
        let mut style = Style::new();
        if let Some(fg) = text.get_fg_color() {
            style = style.fg(color(fg));
        }
        if let Some(bg) = text.get_bg_color() {
            style = style.bg(color(bg));
        }
        for (effect, modifier) in [
            (Effects::BOLD, Modifier::BOLD),
            (Effects::DIMMED, Modifier::DIM),
            (Effects::INVERT, Modifier::REVERSED),
        ] {
            if effects.contains(effect) {
                style = style.add_modifier(modifier);
            }
        }
        style
    }
}

#[cfg(test)]
mod tests;

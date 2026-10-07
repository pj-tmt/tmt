use super::*;
use crate::mark::Mark;

fn design_tokens() -> serde_json::Value {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../design/tokens/tokens.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn registered_marks(tokens: &serde_json::Value) -> Result<(), String> {
    let marks = tokens["mark"].as_object().unwrap();
    for mark in Mark::ALL {
        if marks.get(mark.symbol()).and_then(serde_json::Value::as_str) != Some(mark.description())
        {
            return Err(format!(
                "{mark:?} must register {:?}: {:?}",
                mark.symbol(),
                mark.description()
            ));
        }
    }
    for symbol in ["✎", "‹ ›", "▸"] {
        if !marks.contains_key(symbol) {
            return Err(format!("missing board-only mark {symbol}"));
        }
    }
    for (symbol, meaning) in marks {
        if Mark::ALL.into_iter().any(|mark| mark.symbol() == symbol) {
            continue;
        }
        let meaning = meaning.as_str().unwrap_or_default();
        if !meaning.contains("board only") && !meaning.contains("board meter only") {
            return Err(format!("{symbol} must be labelled board only"));
        }
    }
    Ok(())
}

#[test]
fn every_mark_is_registered_with_its_canonical_meaning() {
    registered_marks(&design_tokens()).unwrap();
}

#[test]
fn mark_registry_rejects_missing_or_changed_shared_meanings() {
    let tokens = design_tokens();
    registered_marks(&tokens).unwrap();
    for mark in Mark::ALL {
        let mut missing = tokens.clone();
        missing["mark"]
            .as_object_mut()
            .unwrap()
            .remove(mark.symbol());
        assert!(registered_marks(&missing).is_err(), "missing {mark:?}");
        let mut changed = tokens.clone();
        changed["mark"][mark.symbol()] = "an unrelated meaning".into();
        assert!(registered_marks(&changed).is_err(), "changed {mark:?}");
    }
}

#[test]
fn board_only_marks_are_registered_without_shared_variants() {
    let mut tokens = design_tokens();
    tokens["mark"]["≥"] = "board meter only: a known lower bound".into();
    registered_marks(&tokens).unwrap();
    for symbol in ["✎", "‹ ›", "▸", "≥"] {
        assert!(!Mark::ALL.into_iter().any(|mark| mark.symbol() == symbol));
        let mut changed = tokens.clone();
        changed["mark"][symbol] = "a board mark without its scope label".into();
        assert!(registered_marks(&changed).is_err(), "unlabelled {symbol}");
    }
    for symbol in ["✎", "‹ ›", "▸"] {
        let mut missing = tokens.clone();
        missing["mark"].as_object_mut().unwrap().remove(symbol);
        assert!(registered_marks(&missing).is_err(), "missing {symbol}");
    }
}

#[test]
fn roles_and_bases_are_named_as_the_design_tokens() {
    for role in Role::ALL {
        assert_eq!(Role::parse(role.name()), Some(role));
    }
    for base in Base::ALL {
        assert_eq!(Base::parse(base.name()), Some(base));
    }
    assert_eq!(Role::parse("error"), None);
    assert_eq!(Base::parse("dark"), None);
}

#[test]
fn depth_follows_the_stream_then_colorterm() {
    assert_eq!(Depth::detect(false, Some("truecolor")), Depth::None);
    assert_eq!(Depth::detect(true, Some("truecolor")), Depth::TrueColor);
    assert_eq!(Depth::detect(true, Some("24bit")), Depth::TrueColor);
    assert_eq!(Depth::detect(true, Some("yes")), Depth::Ansi16);
    assert_eq!(Depth::detect(true, None), Depth::Ansi16);
}

#[test]
fn paints_parse_hex_names_and_effects_only() {
    assert_eq!(Paint::parse("#e0a458"), Some(Paint::Rgb(0xE0, 0xA4, 0x58)));
    assert_eq!(Paint::parse("#E0A458"), Some(Paint::Rgb(0xE0, 0xA4, 0x58)));
    assert_eq!(Paint::parse("blue"), Some(Paint::Ansi(AnsiColor::Blue)));
    assert_eq!(
        Paint::parse("bright black"),
        Some(Paint::Ansi(AnsiColor::BrightBlack))
    );
    assert_eq!(Paint::parse("default"), Some(Paint::Plain));
    assert_eq!(Paint::parse("dim"), Some(Paint::Dimmed));
    for bad in [
        "",
        "#e0a45",
        "#e0a4588",
        "#gg0000",
        "#é0a45",
        "Blue",
        "orange",
        "rgb(1,2,3)",
    ] {
        assert_eq!(Paint::parse(bad), None, "{bad}");
    }
}

fn rgb(style: Style) -> Option<(u8, u8, u8)> {
    match style.get_fg_color() {
        Some(Color::Rgb(RgbColor(r, g, b))) => Some((r, g, b)),
        _ => None,
    }
}

#[test]
fn each_theme_renders_a_role_for_the_stream_s_depth() {
    let tmt = Theme::default();
    assert_eq!(tmt.base, Base::Tmt);
    assert_eq!(
        rgb(tmt.style(Role::Waiting, Depth::TrueColor)),
        Some((0xFF, 0x9E, 0x64))
    );
    // A 16-color terminal gets the designed fallback.
    assert_eq!(
        tmt.style(Role::Waiting, Depth::Ansi16).get_fg_color(),
        Some(AnsiColor::Yellow.into())
    );
    let light = Theme::new(Base::TmtLight);
    assert_eq!(
        rgb(light.style(Role::Waiting, Depth::TrueColor)),
        Some((0x96, 0x50, 0x27))
    );
    // terminal keeps the user's 16 colors even where 24-bit color works.
    let terminal = Theme::new(Base::Terminal);
    assert_eq!(
        terminal
            .style(Role::Blocked, Depth::TrueColor)
            .get_fg_color(),
        Some(AnsiColor::Red.into())
    );
    assert_eq!(terminal.style(Role::Text, Depth::TrueColor), Style::new());
    let mono = Theme::new(Base::Mono);
    assert_eq!(
        mono.style(Role::Waiting, Depth::TrueColor),
        Style::new().bold()
    );
    assert_eq!(
        mono.style(Role::Dim, Depth::TrueColor),
        Style::new().dimmed()
    );
    assert_eq!(mono.style(Role::Working, Depth::TrueColor), Style::new());
    // The selection is a background, or reverse video without a color.
    assert_eq!(
        tmt.style(Role::Selection, Depth::TrueColor).get_bg_color(),
        Some(Color::Rgb(RgbColor(0x4A, 0x4A, 0x4A)))
    );
    assert_eq!(
        tmt.style(Role::Selection, Depth::TrueColor).get_fg_color(),
        None
    );
    assert_eq!(
        terminal.style(Role::Selection, Depth::Ansi16),
        Style::new().invert()
    );
    // No color means no styling at all, in every theme.
    for base in Base::ALL {
        for role in Role::ALL {
            assert_eq!(
                Theme::new(base).style(role, Depth::None),
                Style::new(),
                "{base:?} {role:?}"
            );
        }
    }
}

#[test]
fn overrides_replace_one_role_and_hex_falls_back_to_the_nearest_color() {
    let theme = Theme::parse(
        "theme",
        [
            ("base", "terminal"),
            ("waiting", "#e0a458"),
            ("accent", "bright blue"),
        ],
    )
    .unwrap();
    assert_eq!(theme.base, Base::Terminal);
    assert_eq!(
        rgb(theme.style(Role::Waiting, Depth::TrueColor)),
        Some((0xE0, 0xA4, 0x58))
    );
    assert_eq!(
        theme.style(Role::Waiting, Depth::Ansi16).get_fg_color(),
        Some(AnsiColor::BrightYellow.into())
    );
    assert_eq!(
        theme.style(Role::Accent, Depth::TrueColor).get_fg_color(),
        Some(AnsiColor::BrightBlue.into())
    );
    assert_eq!(
        theme.style(Role::Blocked, Depth::TrueColor).get_fg_color(),
        Some(AnsiColor::Red.into()),
        "the rest is the base"
    );
    // An override applies over whichever base is named, in any order.
    let later = Theme::parse("theme", [("waiting", "red"), ("base", "mono")]).unwrap();
    assert_eq!(
        later.style(Role::Waiting, Depth::TrueColor).get_fg_color(),
        Some(AnsiColor::Red.into())
    );
    assert_eq!(
        later.style(Role::Blocked, Depth::TrueColor),
        Style::new().bold()
    );
    let squad = Theme::default().with(Role::Blocked, Paint::Bold);
    assert_eq!(
        squad.style(Role::Blocked, Depth::TrueColor),
        Style::new().bold()
    );
}

#[test]
fn mistakes_name_the_setting() {
    let error = |settings: &[(&str, &str)]| {
        Theme::parse("squad.product.theme", settings.iter().copied())
            .unwrap_err()
            .to_string()
    };
    assert_eq!(
        error(&[("base", "dark")]),
        "`squad.product.theme.base` must be auto, tmt, tmt-light, terminal or mono."
    );
    assert!(error(&[("error", "red")]).starts_with(
        "`squad.product.theme.error` is not a theme setting; use base or a token: text, muted,"
    ));
    assert!(
        error(&[("waiting", "orange")])
            .starts_with("`squad.product.theme.waiting` must be #rrggbb")
    );
}

/// The built-in values are the design tokens: the site and the terminal
/// never drift apart.
#[test]
fn built_in_values_match_the_design_tokens() {
    let tokens = design_tokens();
    let colors = tokens["color"].as_object().unwrap();
    let names: Vec<&str> = colors.keys().map(String::as_str).collect();
    let mut expected: Vec<&str> = Role::ALL.map(Role::name).to_vec();
    expected.sort_unstable();
    let mut names = names;
    names.sort_unstable();
    assert_eq!(names, expected, "one role per color token");
    for role in Role::ALL {
        let token = &colors[role.name()];
        for (light, key) in [(false, "dark"), (true, "light")] {
            let Paint::Rgb(r, g, b) = Base::truecolor(role, light) else {
                panic!("{role:?}");
            };
            assert_eq!(
                format!("#{r:02X}{g:02X}{b:02X}"),
                token[key].as_str().unwrap(),
                "{role:?} {key}"
            );
        }
        assert_eq!(
            Paint::parse(token["terminal"].as_str().unwrap()),
            Some(Base::terminal(role)),
            "{role:?} terminal"
        );
    }
}

#[cfg(feature = "ratatui")]
#[test]
fn screens_get_the_same_style() {
    use ratatui::style::{Color as ScreenColor, Modifier};
    let tmt = Theme::default();
    let waiting = screen::style(&tmt, Role::Waiting, Depth::TrueColor);
    assert_eq!(waiting.fg, Some(ScreenColor::Rgb(0xFF, 0x9E, 0x64)));
    let selected = screen::style(&tmt, Role::Selection, Depth::TrueColor);
    assert_eq!(selected.bg, Some(ScreenColor::Rgb(0x4A, 0x4A, 0x4A)));
    let dim = screen::style(&tmt, Role::Dim, Depth::Ansi16);
    assert_eq!(dim.fg, None);
    assert!(dim.add_modifier.contains(Modifier::DIM));
    let mono = screen::style(&Theme::new(Base::Mono), Role::Blocked, Depth::TrueColor);
    assert!(mono.add_modifier.contains(Modifier::BOLD));
    assert_eq!(mono.fg, None);
    assert_eq!(
        screen::style(&tmt, Role::Blocked, Depth::None),
        ratatui::style::Style::new()
    );
}

/// Today's command-line tokens show as design tokens under a theme.
#[test]
fn command_line_tokens_have_their_design_token() {
    use crate::Token;
    assert_eq!(Token::Accent.role(), Some(Role::Accent));
    assert_eq!(Token::Ok.role(), Some(Role::Working));
    assert_eq!(Token::Warn.role(), Some(Role::Waiting));
    assert_eq!(Token::Error.role(), Some(Role::Blocked));
    assert_eq!(Token::Dim.role(), Some(Role::Dim));
    assert_eq!(Token::Driver(None).role(), Some(Role::Dim));
    assert_eq!(Token::Driver(Some(Role::Review)).role(), Some(Role::Review));
    assert_eq!(Token::Driver(Some(Role::Link)).role(), Some(Role::Link));
    assert_eq!(Token::Title.role(), None);
    assert_eq!(Token::Literal.role(), None);
}

/// Foregrounds must stay readable on the designed paper and representative
/// terminal backgrounds. On the selection background `text` carries words
/// (4.5:1); `muted`, `dim` and state-colored words paint in `text` there, and
/// marks keep their color at the 3:1 floor for non-text.
#[test]
fn design_tokens_keep_text_readable_on_board_backgrounds() {
    let tokens = design_tokens();
    let luminance = |hex: &str| {
        let Paint::Rgb(r, g, b) = Paint::parse(hex).unwrap() else {
            panic!("expected an RGB token: {hex}");
        };
        let linear = |channel: u8| {
            let value = f64::from(channel) / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
    };
    let contrast = |foreground: &str, background: &str| {
        let fg = luminance(foreground);
        let bg = luminance(background);
        (fg.max(bg) + 0.05) / (fg.min(bg) + 0.05)
    };
    let selection_marks = [
        Role::Accent,
        Role::Waiting,
        Role::Working,
        Role::Review,
        Role::Blocked,
        Role::Link,
    ];
    for (mode, terminals) in [
        ("dark", &["#24283B", "#2A2C38"][..]),
        ("light", &["#FFFFFF"][..]),
    ] {
        let color = |role: Role| tokens["color"][role.name()][mode].as_str().unwrap();
        let backgrounds = [tokens["surface"]["paper"][mode].as_str().unwrap()];
        for role in Role::ALL
            .into_iter()
            .filter(|role| *role != Role::Selection)
        {
            let floor = if matches!(role, Role::Muted | Role::Dim) {
                3.0
            } else {
                4.5
            };
            for background in backgrounds.iter().chain(terminals) {
                let ratio = contrast(color(role), background);
                assert!(
                    ratio >= floor,
                    "{mode} {} {} on {background}: {ratio:.2}:1 < {floor}:1",
                    role.name(),
                    color(role),
                );
            }
        }
        let selection = tokens["color"]["selection"][mode].as_str().unwrap();
        let ratio = contrast(color(Role::Text), selection);
        assert!(
            ratio >= 4.5,
            "{mode} text on selection {selection}: {ratio:.2}:1 < 4.5:1"
        );
        for role in selection_marks {
            let ratio = contrast(color(role), selection);
            assert!(
                ratio >= 3.0,
                "{mode} {} mark on selection {selection}: {ratio:.2}:1 < 3:1",
                role.name(),
            );
        }
    }
}

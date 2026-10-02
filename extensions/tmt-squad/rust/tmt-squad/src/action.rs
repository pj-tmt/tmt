//! `[bind]` actions: `event = "verb …"`, parsed once when squad.toml loads.
//! Arguments are split at load time; a `{field}` value later fills (part of)
//! exactly one argument and is never re-split, re-quoted or shell-parsed.

use crate::template::{DEFAULT_COPY, Template};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    Jump,
    Back,
    Open,
    Copy,
    Notes,
    Refresh,
    TokenWindow,
    Theme,
    Run,
    NextPane,
    Toggle,
    /// The row's action menu (the plain host's Enter).
    Menu,
    /// Opens the tab of the row's squad (the `all` tab's Enter).
    Tab,
    Talk,
    Reply,
    Annotate,
}

impl Verb {
    fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "jump" => Self::Jump,
            "back" => Self::Back,
            "open" => Self::Open,
            "copy" => Self::Copy,
            "notes" => Self::Notes,
            "refresh" => Self::Refresh,
            "token-window" => Self::TokenWindow,
            "theme" => Self::Theme,
            "run" => Self::Run,
            "next-pane" => Self::NextPane,
            "toggle" => Self::Toggle,
            "menu" => Self::Menu,
            "tab" => Self::Tab,
            "talk" => Self::Talk,
            "reply" => Self::Reply,
            "annotate" => Self::Annotate,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Jump => "jump",
            Self::Back => "back",
            Self::Open => "open",
            Self::Copy => "copy",
            Self::Notes => "notes",
            Self::Refresh => "refresh",
            Self::TokenWindow => "token-window",
            Self::Theme => "theme",
            Self::Run => "run",
            Self::NextPane => "next-pane",
            Self::Toggle => "toggle",
            Self::Menu => "menu",
            Self::Tab => "tab",
            Self::Talk => "talk",
            Self::Reply => "reply",
            Self::Annotate => "annotate",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    pub verb: Verb,
    /// `run`: the program, then its arguments, one template per argument.
    /// `open`: at most one template. `copy`: one template of free text.
    /// `annotate`: the addressee, `lead` or `member`.
    pub args: Vec<Template>,
    /// The configured line, for help and menus.
    pub text: String,
}

/// Splits at whitespace; double quotes group literal text (with `\"`).
fn tokens(text: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut started = false;
    let mut chars = text.chars();
    while let Some(character) = chars.next() {
        match character {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            '\\' if quoted => match chars.next() {
                Some(escaped @ ('"' | '\\')) => current.push(escaped),
                _ => return Err("only \\\" and \\\\ escapes are allowed in quotes".into()),
            },
            c if c.is_whitespace() && !quoted => {
                if started {
                    out.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            c if c.is_control() => return Err("control characters are not allowed".into()),
            c => {
                current.push(c);
                started = true;
            }
        }
    }
    if quoted {
        return Err("unterminated quote".into());
    }
    if started {
        out.push(current);
    }
    Ok(out)
}

impl Action {
    pub fn parse(line: &str) -> Result<Self, String> {
        let line = line.trim();
        let (verb_name, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        let rest = rest.trim();
        let verb =
            Verb::parse(verb_name).ok_or_else(|| format!("'{verb_name}' is not an action"))?;
        let args = match verb {
            Verb::Toggle => {
                let mut panes = Vec::new();
                for name in rest.split_whitespace() {
                    let pane = crate::config::Pane::parse(name)
                        .ok_or("toggle takes literal panes: rows, notes, detail or replies")?;
                    if panes.contains(&pane) {
                        return Err(format!("toggle repeats the {} pane", pane.title()));
                    }
                    panes.push(pane);
                }
                if panes.is_empty() {
                    return Err(
                        "toggle needs at least one pane: rows, notes, detail or replies".into(),
                    );
                }
                panes
                    .into_iter()
                    .map(|pane| Template::parse(pane.title()))
                    .collect::<Result<Vec<_>, _>>()?
            }
            Verb::Copy => vec![Template::parse(if rest.is_empty() {
                DEFAULT_COPY
            } else {
                rest
            })?],
            Verb::Jump => match rest {
                "" => Vec::new(),
                "lead" => vec![Template::parse("lead")?],
                other => return Err(format!("jump takes nothing or lead, not '{other}'")),
            },
            Verb::Annotate => match rest {
                "" | "lead" => vec![Template::parse("lead")?],
                "member" => vec![Template::parse("member")?],
                other => return Err(format!("annotate takes lead or member, not '{other}'")),
            },
            Verb::Open | Verb::Run => {
                let args = tokens(rest)?
                    .iter()
                    .map(|token| Template::parse(token))
                    .collect::<Result<Vec<_>, _>>()?;
                if verb == Verb::Open && args.len() > 1 {
                    return Err("open takes one link, for example open {pr_link}".into());
                }
                if verb == Verb::Run {
                    let program = args.first().ok_or("run needs a program")?;
                    match program.literal() {
                        Some(name) if name.starts_with('/') || !name.contains('/') => {}
                        _ => {
                            return Err(
                                "run's program must be a literal name on PATH or an absolute path"
                                    .into(),
                            );
                        }
                    }
                }
                args
            }
            _ if !rest.is_empty() => return Err(format!("{} takes no arguments", verb.name())),
            _ => Vec::new(),
        };
        Ok(Self {
            verb,
            args,
            text: line.to_owned(),
        })
    }

    /// `run`: the complete argv. Each template becomes exactly one element,
    /// and no row value can become an option.
    pub fn argv(&self, row: &Value) -> Result<Vec<String>, String> {
        self.args.iter().map(|arg| arg.fill_argument(row)).collect()
    }
}

/// Board keys no binding may take: quit, select, search and help.
const RESERVED: &[&str] = &["q", "j", "k", "/", "?"];

/// Key and mouse events a binding may name. Ctrl-C always quits.
pub fn valid_event(event: &str) -> bool {
    if RESERVED.contains(&event) {
        return false;
    }
    const NAMED: &[&str] = &[
        "enter",
        "backspace",
        "tab",
        "space",
        "delete",
        "home",
        "end",
        "pageup",
        "pagedown",
        "click",
        "double-click",
    ];
    let mut chars = event.chars();
    let single = matches!((chars.next(), chars.next()), (Some(c), None) if c.is_ascii_graphic());
    let function = event
        .strip_prefix('f')
        .and_then(|n| n.parse::<u8>().ok())
        .is_some_and(|n| (1..=12).contains(&n));
    let ctrl = event
        .strip_prefix("ctrl-")
        .is_some_and(|key| key.len() == 1 && key.as_bytes()[0].is_ascii_lowercase() && key != "c");
    NAMED.contains(&event) || single || function || ctrl
}

/// Parsed bindings, by event.
pub type Bindings = BTreeMap<String, Action>;

pub fn parse_bindings<'a>(
    entries: impl Iterator<Item = (&'a str, Option<&'a str>)>,
    place: &str,
) -> Result<Bindings, String> {
    entries
        .map(|(event, line)| {
            if !valid_event(event) {
                return Err(format!(
                    "`{place}.{event}` is not a bindable key or mouse event"
                ));
            }
            let line = line.ok_or_else(|| format!("`{place}.{event}` must be an action string"))?;
            let action =
                Action::parse(line).map_err(|error| format!("`{place}.{event}`: {error}"))?;
            Ok((event.to_owned(), action))
        })
        .collect()
}

/// Host presets. The tmux host jumps; a plain terminal cannot, so Enter and
/// double-click open the row's action menu instead. A single click only
/// selects, so pointing at a row never leaves the board.
pub fn preset(tmux: bool, panes: &[crate::config::Pane]) -> Bindings {
    let detail = match (
        panes.contains(&crate::config::Pane::Detail),
        panes.contains(&crate::config::Pane::Replies),
    ) {
        (true, true) => Some("toggle detail replies"),
        (true, false) => Some("toggle detail"),
        (false, true) => Some("toggle replies"),
        (false, false) => None,
    };
    let enter = if tmux { "jump" } else { "menu" };
    [
        ("enter", enter),
        ("double-click", enter),
        ("backspace", "back"),
        ("t", "talk"),
        ("r", "reply"),
        ("a", "annotate lead"),
        ("o", "open"),
        ("y", "copy"),
        ("n", "notes"),
        ("tab", "next-pane"),
        ("ctrl-r", "refresh"),
        ("w", "token-window"),
        ("T", "theme"),
    ]
    .into_iter()
    // Only a host that can show a pane can jump to the lead.
    .chain(tmux.then_some(("L", "jump lead")))
    .chain(detail.map(|action| ("d", action)))
    .map(|(event, line)| {
        (
            event.to_owned(),
            Action::parse(line).expect("preset action"),
        )
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn row() -> Value {
        json!({
            "name": "auth-fix", "state": "blocked", "pending": null, "note": "needs a call",
            "pane": {"id": "%5", "target": "crew:2.0", "cwd": "/w/app 3"},
            "fields": {
                "task": "rotate; $(rm -rf ~) `id` \"quoted\" *.rs",
                "worktree": "-rf /",
                "pr_link": "https://example.com/pull/412",
            }
        })
    }

    #[test]
    fn run_fills_each_field_into_exactly_one_argument() {
        let action =
            Action::parse(r#"run code --wait "{cwd} (lead)" --task={task} --dir={worktree}"#)
                .unwrap();
        assert_eq!(action.verb, Verb::Run);
        assert_eq!(
            action.argv(&row()).unwrap(),
            [
                "code",
                "--wait",
                "/w/app 3 (lead)",
                "--task=rotate; $(rm -rf ~) `id` \"quoted\" *.rs",
                "--dir=-rf /",
            ]
        );
        // A value that would start an argument with '-' is refused.
        assert_eq!(
            Action::parse("run code --wait {worktree}")
                .unwrap()
                .argv(&row())
                .unwrap_err(),
            "worktree starts with '-' and would be read as an option; refused"
        );
        assert_eq!(
            Action::parse("run code {pending}")
                .unwrap()
                .argv(&row())
                .unwrap_err(),
            "pending is empty for this row"
        );
    }

    #[test]
    fn copy_open_and_annotate_take_their_own_argument_shapes() {
        let copy = Action::parse("copy - [{name}]({pr_link}) {state}").unwrap();
        assert_eq!(
            copy.args[0].fill(&row()).unwrap(),
            "- [auth-fix](https://example.com/pull/412) blocked"
        );
        assert_eq!(
            Action::parse("copy").unwrap().args[0],
            Template::parse(DEFAULT_COPY).unwrap()
        );
        assert_eq!(Action::parse("open {pr_link}").unwrap().args.len(), 1);
        assert!(Action::parse("open").unwrap().args.is_empty());
        assert_eq!(
            Action::parse("annotate member").unwrap().args[0],
            Template::parse("member").unwrap()
        );
        assert_eq!(
            Action::parse("annotate").unwrap().args[0],
            Template::parse("lead").unwrap()
        );
    }

    #[test]
    fn malformed_actions_events_and_fields_are_rejected() {
        for line in [
            "launch",
            "jump now",
            "open {a} {b}",
            "run",
            "run {program}",
            "run ./local",
            "run code {Bad}",
            "run code {unclosed",
            "run code }",
            "run code \"open",
            "annotate everyone",
            "copy {two words}",
            "refresh please",
        ] {
            assert!(Action::parse(line).is_err(), "{line}");
        }
        for event in [
            "enter",
            "o",
            "Y",
            "x",
            "f5",
            "ctrl-r",
            "click",
            "double-click",
        ] {
            assert!(valid_event(event), "{event}");
        }
        for event in [
            "ctrl-c", "ctrl-", "hold", "f13", "", "ab", "é", "q", "j", "k", "/", "?",
        ] {
            assert!(!valid_event(event), "{event}");
        }
    }

    #[test]
    fn both_hosts_default_to_ctrl_r_and_leave_f5_unbound() {
        for tmux in [false, true] {
            let bindings = preset(tmux, &[]);
            assert_eq!(bindings["ctrl-r"].verb, Verb::Refresh);
            assert!(!bindings.contains_key("f5"));
        }
    }

    #[test]
    fn presets_differ_only_where_the_host_cannot_jump() {
        let (tmux, plain) = (preset(true, &[]), preset(false, &[]));
        assert_eq!(tmux["L"].verb, Verb::Jump);
        assert_eq!(tmux["L"].args[0].literal(), Some("lead"));
        assert!(!plain.contains_key("L"), "a plain terminal cannot jump");
        assert_eq!(Action::parse("jump").unwrap().args.len(), 0);
        assert_eq!(
            Action::parse("jump member").unwrap_err(),
            "jump takes nothing or lead, not 'member'"
        );
        assert_eq!(tmux["enter"].verb, Verb::Jump);
        assert_eq!(plain["enter"].verb, Verb::Menu);
        assert_eq!(plain["double-click"].verb, Verb::Menu);
        assert_eq!(tmux["double-click"].verb, Verb::Jump);
        assert!(!tmux.contains_key("click") && !plain.contains_key("click"));
        assert_eq!(tmux["o"], plain["o"]);
    }
    #[test]
    fn toggle_accepts_unique_literal_panes_and_presets_follow_available_panes() {
        for pane in ["rows", "notes", "detail", "replies"] {
            let action = Action::parse(&format!("toggle {pane}")).unwrap();
            assert_eq!(action.verb, Verb::Toggle);
            assert_eq!(action.args[0].literal(), Some(pane));
        }
        for line in [
            "toggle",
            "toggle all",
            "toggle {pane}",
            "toggle detail detail",
            "toggle detail unknown",
            "toggle \"detail\"",
        ] {
            assert!(Action::parse(line).is_err(), "{line}");
        }
        for tmux in [false, true] {
            assert!(!preset(tmux, &[]).contains_key("d"));
            assert_eq!(
                preset(tmux, &[crate::config::Pane::Replies])["d"].text,
                "toggle replies"
            );
            assert_eq!(
                preset(tmux, &[crate::config::Pane::Detail])["d"].text,
                "toggle detail"
            );
            assert_eq!(
                preset(
                    tmux,
                    &[crate::config::Pane::Detail, crate::config::Pane::Replies]
                )["d"]
                    .text,
                "toggle detail replies"
            );
        }
        assert_eq!(
            Action::parse("toggle detail replies")
                .unwrap()
                .args
                .iter()
                .map(|arg| arg.literal().unwrap())
                .collect::<Vec<_>>(),
            ["detail", "replies"]
        );
        assert_eq!(
            parse_bindings([("d", Some("toggle notes"))].into_iter(), "bind").unwrap()["d"].args[0]
                .literal(),
            Some("notes")
        );
    }
}

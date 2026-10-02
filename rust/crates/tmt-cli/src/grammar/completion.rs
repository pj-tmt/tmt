//! Dynamic context is parsed by the public command grammar, not by each shell.

use std::ffi::{OsStr, OsString};
use std::io::{self, Write};

use crate::grammar;

// Completion annotations refer to canonical paths/operand IDs from the grammar;
// aliases, options and operand positions are still resolved exclusively by Clap.
const IDENTITY_OPERANDS: &[(&[&str], &str)] = &[
    (&["name"], "name"),
    (&["add"], "name"),
    (&["marked"], "name"),
    (&["rm"], "name"),
    (&["talk"], "target"),
    (&["check"], "target"),
    (&["focus"], "target"),
    (&["ls"], "target"),
    (&["answer"], "from"),
    (&["identity", "show"], "name"),
    (&["preamble", "show"], "agent"),
    (&["preamble", "set"], "agent"),
    (&["preamble", "rm"], "agent"),
];

pub fn generate(shell: &str, output: &mut impl Write) -> io::Result<()> {
    let (shell, wrapper) = match shell {
        "bash" => (
            clap_complete::Shell::Bash,
            include_str!("../completion.bash"),
        ),
        "zsh" => (clap_complete::Shell::Zsh, include_str!("../completion.zsh")),
        "fish" => (
            clap_complete::Shell::Fish,
            include_str!("../completion.fish"),
        ),
        _ => {
            return writeln!(
                output,
                "Use 'tmt completion bash', 'tmt completion zsh' or 'tmt completion fish' to generate a shell script."
            );
        }
    };
    let mut generated = Vec::new();
    clap_complete::generate(
        shell,
        &mut grammar::public_grammar(&grammar::grammar(), true),
        if shell == clap_complete::Shell::Fish {
            "__tmt_static"
        } else {
            "tmt"
        },
        &mut generated,
    );
    let generated = String::from_utf8(generated).map_err(io::Error::other)?;
    // Do not execute the generated entry point before the dynamic wrapper is
    // defined when zsh autoloads this file. All static helpers stay generated.
    let generated = if shell == clap_complete::Shell::Zsh {
        generated
            .split_once("if [ \"$funcstack[1]\" = \"_tmt\" ]; then")
            .ok_or_else(|| io::Error::other("Unrecognized zsh completion generator footer"))?
            .0
    } else {
        &generated
    };
    if shell == clap_complete::Shell::Fish {
        output.write_all(generated.as_bytes())?;
    } else {
        // Rename the entry point and all generated helper references together.
        output.write_all(generated.replace("_tmt", "_tmt_static").as_bytes())?;
    }
    output.write_all(wrapper.as_bytes())
}

#[derive(Debug, PartialEq, Eq)]
pub enum Context {
    Static,
    Launch {
        prefix: String,
    },
    LaunchCommand {
        first: OsString,
        offset: usize,
    },
    Identities {
        prefix: String,
        remembered: bool,
    },
    /// Zero-based command position in arguments excluding the `tmt` executable.
    Command {
        offset: usize,
    },
}

/// `words` includes the unfinished word, including an empty trailing word after
/// whitespace. Substitution lets Clap identify which operand owns that word
/// without validating an incomplete identity or interpreting command arguments.
pub fn context(words: &[OsString]) -> Context {
    let Some(current) = words.last().and_then(|word| word.to_str()) else {
        return Context::Static;
    };
    let mut marker = String::from("tmt-completion-position");
    while words
        .iter()
        .any(|word| word.to_string_lossy().contains(&marker))
    {
        marker.push('x');
    }
    let inline_identity = current.strip_prefix("--identity=");
    let mut arguments = words.to_vec();
    *arguments.last_mut().expect("nonempty words") = match inline_identity {
        Some(_) => format!("--identity={marker}").into(),
        None => marker.clone().into(),
    };
    let definition = grammar::public_grammar(&grammar::grammar(), true).ignore_errors(true);
    let Ok(matches) =
        definition.try_get_matches_from(std::iter::once(OsString::from("tmt")).chain(arguments))
    else {
        return Context::Static;
    };
    let mut leaf = &matches;
    let mut path = Vec::new();
    while let Some((name, child)) = leaf.subcommand() {
        path.push(name);
        leaf = child;
    }
    let owns_marker = |id: &str| {
        leaf.try_get_raw(id)
            .ok()
            .flatten()
            .is_some_and(|values| values.into_iter().any(|value| value == OsStr::new(&marker)))
    };
    if path == ["run"] {
        let tail = leaf
            .try_get_raw("run-argv")
            .ok()
            .flatten()
            .map(|values| values.collect::<Vec<_>>())
            .unwrap_or_default();
        let remembered = leaf
            .try_get_one::<bool>("resume")
            .ok()
            .flatten()
            .copied()
            .unwrap_or(false);
        if tail.last().is_some_and(|value| {
            *value == OsStr::new(&marker) || *value == OsStr::new(&format!("--identity={marker}"))
        }) {
            if tail.len() > 1 {
                return if remembered {
                    Context::Static
                } else if tail[0].to_str().is_some_and(|first| {
                    tmt_core::driver::ALL
                        .iter()
                        .any(|driver| driver.executables.contains(&first))
                }) {
                    Context::LaunchCommand {
                        first: tail[0].to_owned(),
                        offset: words.len() - tail.len(),
                    }
                } else {
                    Context::Command {
                        offset: words.len() - tail.len() + 1,
                    }
                };
            }
            if !current.starts_with('-') {
                if !remembered {
                    return Context::Launch {
                        prefix: current.into(),
                    };
                }
                return Context::Identities {
                    prefix: current.into(),
                    remembered,
                };
            }
        }
    }
    if path == ["resume"] && owns_marker("name") && !current.starts_with('-') {
        return Context::Identities {
            prefix: current.into(),
            remembered: true,
        };
    }
    let identity_operand = IDENTITY_OPERANDS
        .iter()
        .any(|(candidate, operand)| path.as_slice() == *candidate && owns_marker(operand));
    if owns_marker("identity") || identity_operand && !current.starts_with('-') {
        return Context::Identities {
            prefix: inline_identity.unwrap_or(current).into(),
            remembered: false,
        };
    }
    Context::Static
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_scripts_keep_the_static_grammar_and_dynamic_entry() {
        for shell in ["bash", "zsh", "fish"] {
            let mut bytes = Vec::new();
            generate(shell, &mut bytes).unwrap();
            let script = String::from_utf8(bytes).unwrap();
            assert!(script.contains("_tmt_static"), "{shell}");
            assert!(script.contains("__complete --"), "{shell}");
            assert!(!script.contains("Internal shell completion context"));
        }
    }

    fn inspect(words: &[&str]) -> Context {
        context(&words.iter().map(OsString::from).collect::<Vec<_>>())
    }

    #[test]
    fn grammar_selects_identity_operands_and_aliases() {
        for words in [
            vec!["this", "Al"],
            vec!["rm", "Al"],
            vec!["remove", "Al"],
            vec!["talk", "Al"],
            vec!["add", "%1", "Al"],
            vec!["marked", "Al"],
            vec!["identity", "show", "Al"],
            vec!["preamble", "show", "Al"],
            vec!["preamble", "set", "Al"],
            vec!["preamble", "clear", "Al"],
            vec!["preamble", "rm", "Al"],
            vec!["check", "Al"],
            vec!["ls", "Al"],
            vec!["list", "Al"],
            vec!["talk", "Bob", "message", "--identity", "Al"],
            vec!["notes", "path", "--identity=Al"],
        ] {
            assert_eq!(
                inspect(&words),
                Context::Identities {
                    prefix: "Al".into(),
                    remembered: false
                },
                "{words:?}"
            );
        }
        assert_eq!(
            inspect(&["run", "--resume", ""]),
            Context::Identities {
                prefix: "".into(),
                remembered: true
            }
        );
    }

    #[test]
    fn unnamed_launch_completion_keeps_the_command_at_the_first_operand() {
        for words in [vec!["run", "Al"], vec!["run", "--save", "Al"]] {
            assert_eq!(
                inspect(&words),
                Context::Launch {
                    prefix: "Al".into()
                }
            );
        }
        assert_eq!(
            inspect(&["run", "--channel", "claude", "--model", ""]),
            Context::LaunchCommand {
                first: "claude".into(),
                offset: 2
            }
        );
    }

    #[test]
    fn trailing_exec_arguments_are_never_tmt_options() {
        assert_eq!(
            inspect(&["run", "-s", "Alice", "claude", "--save"]),
            Context::Command { offset: 3 }
        );
        for words in [
            vec!["run", "alice", ""],
            vec!["run", "alice", "claude", "--identity=Bob"],
            vec!["run", "alice", "claude", "--help"],
            vec!["run", "alice", "claude", "", "x"],
        ] {
            assert_eq!(inspect(&words), Context::Command { offset: 2 }, "{words:?}");
        }
        assert_eq!(inspect(&["run", "--resume", "alice", ""]), Context::Static);
        for words in [vec!["resume", "Al"], vec!["resume", "--retry", "Al"]] {
            assert_eq!(
                inspect(&words),
                Context::Identities {
                    prefix: "Al".into(),
                    remembered: true
                },
                "{words:?}"
            );
        }
    }

    #[test]
    fn ordinary_options_and_data_keep_static_completion() {
        for words in [
            vec![""],
            vec!["run", "--he"],
            vec!["talk", "Bob", ""],
            vec!["talk", "--timeout", ""],
            vec!["help", "run", ""],
            vec!["unknown", "run", ""],
            vec!["identity", "create", ""],
            vec!["add", ""],
            vec!["preamble", "set", "Alice", ""],
        ] {
            assert_eq!(inspect(&words), Context::Static, "{words:?}");
        }
    }
}

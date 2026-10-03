//! Read-only, bounded startup-file evidence for shell completion.

use crate::bounded_file::{self, FileReadError};
use std::{
    io,
    path::{Path, PathBuf},
};

const MAXIMUM: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
}

impl Shell {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "bash" => Some(Self::Bash),
            "zsh" => Some(Self::Zsh),
            "fish" => Some(Self::Fish),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Bash => "bash",
            Self::Zsh => "zsh",
            Self::Fish => "fish",
        }
    }
    pub fn line(self) -> &'static str {
        match self {
            Self::Bash => "source <(tmt __completion-script bash)",
            Self::Zsh => "source <(tmt __completion-script zsh)",
            Self::Fish => "tmt __completion-script fish | source",
        }
    }
}

pub fn detected_shell() -> Option<Shell> {
    std::env::var_os("SHELL").and_then(|value| {
        Path::new(&value)
            .file_name()?
            .to_str()
            .and_then(Shell::parse)
    })
}

#[derive(Debug)]
pub struct Plan {
    pub shell: Shell,
    pub file: PathBuf,
    pub configured: bool,
    pub warnings: Vec<String>,
}

impl Plan {
    pub fn inspect(shell: Shell, file: PathBuf) -> io::Result<Self> {
        let bytes = match bounded_file::read(&file, MAXIMUM) {
            Ok(bytes) => bytes,
            Err(FileReadError::Io(error)) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(error) => {
                return Err(io::Error::other(format!(
                    "Cannot inspect {}: {error}",
                    file.display()
                )));
            }
        };
        Self::from_bytes(shell, file, bytes)
    }

    fn from_bytes(shell: Shell, file: PathBuf, before: Vec<u8>) -> io::Result<Self> {
        let text = std::str::from_utf8(&before).map_err(io::Error::other)?;
        let mut configured = 0;
        let mut candidates = 0;
        let mut warnings = Vec::new();
        // Frameworks may initialize completion in sourced files. Literal calls
        // provide ordering evidence only; startup files are never evaluated.
        let compinit = text.lines().position(|raw| {
            let line = raw.split('#').next().unwrap_or("").trim();
            line.split([';', '&']).any(|command| {
                let mut words = command.split_whitespace();
                match words.next() {
                    Some("compinit") => true,
                    Some("command") => words.next() == Some("compinit"),
                    _ => false,
                }
            })
        });
        for (index, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            let words: Vec<_> = line
                .split(|c: char| c.is_whitespace() || "();|<>".contains(c))
                .filter(|word| !word.is_empty())
                .collect();
            if !words.windows(2).any(|pair| {
                let executable = Path::new(pair[0].trim_matches(['\'', '"']))
                    .file_name()
                    .and_then(|name| name.to_str());
                executable == Some("tmt") && pair[1] == "__completion-script"
            }) {
                continue;
            }
            candidates += 1;
            let normalized = line.split_whitespace().collect::<Vec<_>>().join(" ");
            // Recognize documented one-line forms; complex shell constructs are
            // reported for manual inspection, not asserted to be installed.
            let recognized = normalized == shell.line()
                || (shell != Shell::Fish
                    && normalized == shell.line().replacen("source ", ". ", 1));
            if recognized {
                configured += 1;
                if shell == Shell::Zsh && compinit.is_some_and(|init| index < init) {
                    warnings.push(format!("Line {} precedes any recognized compinit call; place completion after compinit.", index + 1));
                }
            } else {
                warnings.push(format!("Line {} mentions completion but is not a recognized setup line for this shell; inspect it manually.", index + 1));
            }
        }
        if candidates > 1 {
            warnings.push("Multiple completion lines found; remove duplicates manually.".into());
        }
        if shell == Shell::Zsh && compinit.is_none() {
            warnings.push("No literal compinit call recognized; your shell framework may initialize completion. Place the displayed line after completion initialization.".into());
        }
        Ok(Self {
            shell,
            file,
            configured: configured > 0,
            warnings,
        })
    }
}

#[cfg(test)]
mod tests;

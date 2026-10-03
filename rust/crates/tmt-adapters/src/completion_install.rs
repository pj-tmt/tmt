//! Bounded startup-file inspection and consented append-only completion setup.

use crate::bounded_file::{self, FileReadError};
use std::{
    fs,
    io::{self, Seek, SeekFrom, Write},
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
    before: Vec<u8>,
    safe_to_append: bool,
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
        let mut compinit = false;
        for (index, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            // This is conservative textual evidence, never shell evaluation.
            if line.split([';', '&']).any(|command| {
                let mut words = command.split_whitespace();
                match words.next() {
                    Some("compinit") => true,
                    Some("command") => words.next() == Some("compinit"),
                    _ => false,
                }
            }) {
                compinit = true;
            }
            let words: Vec<_> = line
                .split(|c: char| c.is_whitespace() || "();|<>".contains(c))
                .filter(|word| !word.is_empty())
                .collect();
            if !words.windows(2).any(|pair| {
                let executable = Path::new(pair[0].trim_matches(['\'', '"']))
                    .file_name()
                    .and_then(|name| name.to_str());
                matches!(executable, Some("tmt" | "tmux-team"))
                    && matches!(pair[1], "completion" | "__completion-script")
            }) {
                continue;
            }
            candidates += 1;
            let normalized = line.split_whitespace().collect::<Vec<_>>().join(" ");
            let legacy = normalized
                .replace("tmux-team", "tmt")
                .replace(" completion ", " __completion-script ");
            // Recognize documented one-line forms; complex shell constructs are
            // reported for manual inspection, not asserted to be installed.
            let recognized = legacy == shell.line()
                || (shell != Shell::Fish && legacy == shell.line().replacen("source ", ". ", 1));
            if recognized {
                configured += 1;
                if shell == Shell::Zsh && !compinit {
                    warnings.push(format!("Line {} precedes any recognized compinit call; place completion after compinit.", index + 1));
                }
            } else {
                warnings.push(format!("Line {} mentions completion but is not a recognized setup line for this shell; inspect it manually.", index + 1));
            }
            if words.contains(&"completion") || words.contains(&"tmux-team") {
                warnings.push(format!("Line {} uses an old completion command; replace it manually with the displayed line.", index + 1));
            }
        }
        if candidates > 1 {
            warnings.push("Multiple completion lines found; remove duplicates manually.".into());
        }
        if shell == Shell::Zsh && !compinit {
            warnings.push("No compinit call recognized. Initialize zsh completion first, then add the displayed line after compinit.".into());
        }
        let safe_to_append = candidates == 0 && (shell != Shell::Zsh || compinit);
        Ok(Self {
            shell,
            file,
            configured: configured > 0,
            warnings,
            before,
            safe_to_append,
        })
    }

    /// Caller has displayed this plan and obtained consent. Cooperating TMT
    /// writers lock the rc inode; changed content refuses instead of rewriting.
    /// No backup, replacement, shell execution, or extra setup line is produced.
    pub fn append(&self) -> io::Result<bool> {
        if self.configured {
            return Ok(false);
        }
        if !self.safe_to_append {
            return Err(io::Error::other(
                "Resolve the startup-file warnings manually before installing completion.",
            ));
        }
        let parent = self
            .file
            .parent()
            .ok_or_else(|| io::Error::other("Startup file has no parent"))?;
        fs::create_dir_all(parent)?;
        let mut file = crate::file_lock::exclusive(&self.file)?;
        let current =
            bounded_file::read_opened(file.try_clone()?, MAXIMUM).map_err(io::Error::other)?;
        if current != self.before {
            return Err(io::Error::other(
                "Startup file changed after inspection; review it and retry.",
            ));
        }
        file.seek(SeekFrom::End(0))?;
        let separator = if current.is_empty() || current.ends_with(b"\n") {
            ""
        } else {
            "\n"
        };
        let addition = format!("{separator}{}\n", self.shell.line());
        file.write_all(addition.as_bytes())
            .and_then(|()| file.sync_all())
            .map_err(|error| {
                io::Error::other(format!(
                    "Append may be incomplete; inspect {} before retrying: {error}",
                    self.file.display()
                ))
            })?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests;

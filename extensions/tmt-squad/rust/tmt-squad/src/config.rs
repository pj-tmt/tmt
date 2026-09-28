//! `squad.toml`: the user's file, beside TMT's global configuration. Squad reads
//! it and writes only `me`, preserving every other byte of the document.

use crate::core::{Core, SquadError};
use std::{
    fs,
    io::{self, Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};
use toml_edit::{DocumentMut, Item, value};

const FILE_LIMIT: u64 = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    Crew,
    PrQueue,
    Minimal,
}

impl Layout {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "crew" => Some(Self::Crew),
            "pr-queue" => Some(Self::PrQueue),
            "minimal" => Some(Self::Minimal),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Crew => "crew",
            Self::PrQueue => "pr-queue",
            Self::Minimal => "minimal",
        }
    }

    /// Ordered state vocabulary; `add` starts members in the first state.
    pub fn states(self) -> &'static [&'static str] {
        match self {
            Self::Crew => &["working", "idle", "blocked", "review", "testing", "hold"],
            Self::PrQueue => &["preparing", "ready", "sent", "merged"],
            Self::Minimal => &[],
        }
    }

    /// Crew sorts rows that owe the user a decision (`pending`) first.
    pub fn pending_first(self) -> bool {
        self == Self::Crew
    }
}

fn invalid(message: impl Into<String>) -> SquadError {
    SquadError::new("SQUAD_CONFIG_INVALID", message)
}

pub struct Config {
    path: PathBuf,
    original: Option<Vec<u8>>,
    document: DocumentMut,
}

impl Config {
    /// The file lives next to the global config that `tmt config show` reports,
    /// so TMT alone owns path discovery. A missing file is an empty document.
    pub fn load(core: &Core) -> Result<Self, SquadError> {
        let shown = core.json(&["config", "show"])?;
        let global = shown["paths"]["global"]
            .as_str()
            .ok_or_else(|| invalid("tmt config show did not report the global config path."))?;
        let path = Path::new(global)
            .parent()
            .ok_or_else(|| invalid("The global config path has no directory."))?
            .join("squad.toml");
        Self::read(path)
    }

    pub fn read(path: PathBuf) -> Result<Self, SquadError> {
        let original = read_bounded(&path)
            .map_err(|error| invalid(format!("Could not read {}: {error}", path.display())))?;
        let text = std::str::from_utf8(original.as_deref().unwrap_or_default())
            .map_err(|_| invalid(format!("{} is not UTF-8.", path.display())))?;
        let document = text
            .parse::<DocumentMut>()
            .map_err(|error| invalid(format!("{}: {error}", path.display())))?;
        let config = Self {
            path,
            original,
            document,
        };
        config.me()?;
        Ok(config)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Top-level `me`: the saved identity that is the user. Never guessed.
    pub fn me(&self) -> Result<Option<&str>, SquadError> {
        match self.document.get("me") {
            None => Ok(None),
            Some(item) => item
                .as_str()
                .filter(|name| !name.trim().is_empty())
                .map(Some)
                .ok_or_else(|| invalid("`me` must be a non-empty identity name.")),
        }
    }

    /// `[squad.<name>] layout` selects the preset; crew is the default.
    pub fn layout(&self, squad: &str) -> Result<Layout, SquadError> {
        let Some(section) = self.document.get("squad") else {
            return Ok(Layout::Crew);
        };
        let table = section
            .as_table_like()
            .ok_or_else(|| invalid("`squad` must be a table of squads."))?;
        let Some(entry) = table.get(squad) else {
            return Ok(Layout::Crew);
        };
        match entry
            .as_table_like()
            .ok_or_else(|| invalid(format!("`squad.{squad}` must be a table.")))?
            .get("layout")
        {
            None => Ok(Layout::Crew),
            Some(item) => item.as_str().and_then(Layout::parse).ok_or_else(|| {
                invalid(format!(
                    "`squad.{squad}.layout` must be crew, pr-queue or minimal."
                ))
            }),
        }
    }

    /// Writes `me` by replacing the file atomically. Refuses if another editor
    /// changed the file since it was read, rather than overwriting their edit.
    pub fn set_me(&mut self, name: &str) -> Result<(), SquadError> {
        let current = read_bounded(&self.path).map_err(|error| write_failed(&self.path, error))?;
        if current != self.original {
            return Err(SquadError::new(
                "SQUAD_CONFIG_CHANGED",
                format!(
                    "{} changed while squad was running; retry.",
                    self.path.display()
                ),
            ));
        }
        self.document.insert(
            "me",
            Item::Value(value(name).into_value().expect("string value")),
        );
        let bytes = self.document.to_string().into_bytes();
        publish(&self.path, &bytes).map_err(|error| write_failed(&self.path, error))?;
        self.original = Some(bytes);
        Ok(())
    }
}

fn write_failed(path: &Path, error: io::Error) -> SquadError {
    SquadError::new(
        "SQUAD_CONFIG_WRITE_FAILED",
        format!("Could not write {}: {error}", path.display()),
    )
}

fn read_bounded(path: &Path) -> io::Result<Option<Vec<u8>>> {
    let file = match fs::File::open(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        result => result?,
    };
    if !file.metadata()?.is_file() {
        return Err(io::Error::other("not a regular file"));
    }
    let mut bytes = Vec::new();
    file.take(FILE_LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > FILE_LIMIT {
        return Err(io::Error::other("larger than 1 MiB"));
    }
    Ok(Some(bytes))
}

fn publish(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let directory = path
        .parent()
        .ok_or_else(|| io::Error::other("no parent directory"))?;
    fs::create_dir_all(directory)?;
    let staged = directory.join(format!(".squad.toml.{}", std::process::id()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&staged)?;
    let written = file
        .write_all(bytes)
        .and_then(|()| file.sync_all())
        .and_then(|()| fs::rename(&staged, path));
    if written.is_err() {
        let _ = fs::remove_file(&staged);
    }
    written?;
    fs::File::open(directory)?.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("tmt-squad-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        directory.join("squad.toml")
    }

    #[test]
    fn me_is_written_without_disturbing_the_users_document() {
        let path = temp("me");
        let original = "# my board\n[squad.product]\nlayout = \"pr-queue\" # queue\n\n[bind]\no = \"open {pr_link}\"\n";
        fs::write(&path, original).unwrap();
        let mut config = Config::read(path.clone()).unwrap();
        assert_eq!(config.me().unwrap(), None);
        assert_eq!(config.layout("product").unwrap(), Layout::PrQueue);
        assert_eq!(config.layout("other").unwrap(), Layout::Crew);
        config.set_me("Ben").unwrap();
        let written = fs::read_to_string(&path).unwrap();
        assert!(
            written.contains(original),
            "user bytes are preserved: {written}"
        );
        assert_eq!(
            Config::read(path.clone()).unwrap().me().unwrap(),
            Some("Ben")
        );
        fs::write(&path, "me = \"Someone\"\n").unwrap();
        assert_eq!(
            config.set_me("Ben").unwrap_err().code,
            "SQUAD_CONFIG_CHANGED"
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "me = \"Someone\"\n");
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn invalid_documents_and_values_are_rejected_before_use() {
        let path = temp("invalid");
        assert_eq!(Config::read(path.clone()).unwrap().me().unwrap(), None);
        for text in ["me = 3\n", "me = \"\"\n", "[squad\n", "squad = 1\n"] {
            fs::write(&path, text).unwrap();
            let result = Config::read(path.clone()).and_then(|config| config.layout("x"));
            assert_eq!(result.unwrap_err().code, "SQUAD_CONFIG_INVALID", "{text}");
        }
        fs::write(&path, "[squad.x]\nlayout = \"kanban\"\n").unwrap();
        let config = Config::read(path.clone()).unwrap();
        assert_eq!(config.layout("x").unwrap_err().code, "SQUAD_CONFIG_INVALID");
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }
}

//! `squad.toml`: the user's file, beside TMT's global configuration. Squad reads
//! it and writes only `me`, preserving every other byte of the document.

use crate::core::{Core, SquadError};
use crate::filter::{Filter, Row};
use std::{
    fs,
    io::{self, Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};
use toml_edit::{DocumentMut, Item, Table, TableLike, value};

const FILE_LIMIT: u64 = 1024 * 1024;
const MAX_SECTIONS: usize = 16;

/// One sort key; `-field` sorts descending. `state` follows the layout's order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SortKey {
    pub field: String,
    pub descending: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub title: String,
    /// None shows every row.
    pub filter: Option<Filter>,
    pub sort: Vec<SortKey>,
}

impl Section {
    fn read(table: &Table, place: &str) -> Result<Self, SquadError> {
        let text = |key: &str| -> Result<Option<&str>, SquadError> {
            table
                .get(key)
                .map(|item| {
                    item.as_str()
                        .ok_or_else(|| invalid(format!("`{place}.{key}` must be a string.")))
                })
                .transpose()
        };
        if let Some(key) = table
            .iter()
            .map(|(key, _)| key)
            .find(|key| !["title", "filter", "sort", "bind"].contains(key))
        {
            return Err(invalid(format!(
                "`{place}.{key}` is not a section setting."
            )));
        }
        let title = text("title")?
            .filter(|title| {
                !title.trim().is_empty()
                    && title.len() <= 80
                    && !title.chars().any(char::is_control)
            })
            .ok_or_else(|| invalid(format!("`{place}.title` must be one line of 1-80 bytes.")))?;
        let filter = text("filter")?
            .map(|filter| {
                Filter::parse(filter)
                    .map_err(|error| invalid(format!("`{place}.filter`: {error}.")))
            })
            .transpose()?;
        let sort = match table.get("sort") {
            None => Vec::new(),
            Some(item) => item
                .as_array()
                .ok_or_else(|| invalid(format!("`{place}.sort` must be an array of field names.")))?
                .iter()
                .map(|value| {
                    let key = value.as_str().unwrap_or_default();
                    let (descending, field) = key
                        .strip_prefix('-')
                        .map_or((false, key), |field| (true, field));
                    (!field.is_empty()
                        && field.bytes().all(|b| {
                            b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-'
                        }))
                    .then(|| SortKey {
                        field: field.into(),
                        descending,
                    })
                    .ok_or_else(|| {
                        invalid(format!(
                            "`{place}.sort` entries are field names, optionally prefixed with '-'."
                        ))
                    })
                })
                .collect::<Result<_, _>>()?,
        };
        if let Some(bind) = table.get("bind") {
            let entries = bind
                .as_table_like()
                .ok_or_else(|| invalid(format!("`{place}.bind` must be a table.")))?;
            if entries.iter().any(|(_, action)| action.as_str().is_none()) {
                return Err(invalid(format!(
                    "`{place}.bind` values must be action strings."
                )));
            }
        }
        Ok(Self {
            title: title.into(),
            filter,
            sort,
        })
    }

    pub fn includes(&self, row: &impl Row) -> bool {
        self.filter
            .as_ref()
            .is_none_or(|filter| filter.matches(row))
    }
}

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

    /// The `[squad.<name>]` table, when the user configured one.
    fn squad_table(&self, squad: &str) -> Result<Option<&dyn TableLike>, SquadError> {
        let Some(section) = self.document.get("squad") else {
            return Ok(None);
        };
        let table = section
            .as_table_like()
            .ok_or_else(|| invalid("`squad` must be a table of squads."))?;
        table
            .get(squad)
            .map(|entry| {
                entry
                    .as_table_like()
                    .ok_or_else(|| invalid(format!("`squad.{squad}` must be a table.")))
            })
            .transpose()
    }

    /// `[squad.<name>] layout` selects the preset; crew is the default.
    pub fn layout(&self, squad: &str) -> Result<Layout, SquadError> {
        match self
            .squad_table(squad)?
            .and_then(|table| table.get("layout"))
        {
            None => Ok(Layout::Crew),
            Some(item) => item.as_str().and_then(Layout::parse).ok_or_else(|| {
                invalid(format!(
                    "`squad.{squad}.layout` must be crew, pr-queue or minimal."
                ))
            }),
        }
    }

    /// User-defined `[[squad.<name>.section]]` entries, in order. None means
    /// the single default list. `bind` is shape-checked here and used by the
    /// board's actions; it never comes from row data.
    pub fn sections(&self, squad: &str) -> Result<Vec<Section>, SquadError> {
        let Some(item) = self
            .squad_table(squad)?
            .and_then(|table| table.get("section"))
        else {
            return Ok(Vec::new());
        };
        let place = format!("squad.{squad}.section");
        let tables = item
            .as_array_of_tables()
            .ok_or_else(|| invalid(format!("`{place}` must be [[{place}]] tables.")))?;
        if tables.len() > MAX_SECTIONS {
            return Err(invalid(format!(
                "`{place}` allows at most {MAX_SECTIONS} sections."
            )));
        }
        tables
            .iter()
            .enumerate()
            .map(|(index, table)| Section::read(table, &format!("{place}[{index}]")))
            .collect()
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

    #[test]
    fn sections_are_optional_ordered_and_strictly_validated() {
        let path = temp("sections");
        fs::write(
            &path,
            r#"[squad.product]
layout = "crew"
[[squad.product.section]]
title = "Needs me"
filter = "pending or state = blocked"
[squad.product.section.bind]
enter = "reply"
[[squad.product.section]]
title = "Everyone"
sort = ["state", "-name"]
"#,
        )
        .unwrap();
        let config = Config::read(path.clone()).unwrap();
        assert!(config.sections("other").unwrap().is_empty());
        let sections = config.sections("product").unwrap();
        assert_eq!(
            sections
                .iter()
                .map(|s| s.title.as_str())
                .collect::<Vec<_>>(),
            ["Needs me", "Everyone"]
        );
        assert!(sections[0].filter.is_some() && sections[1].filter.is_none());
        assert_eq!(
            sections[1].sort,
            [
                SortKey {
                    field: "state".into(),
                    descending: false
                },
                SortKey {
                    field: "name".into(),
                    descending: true
                }
            ]
        );
        for body in [
            "[squad.x]\nsection = 1\n",
            "[[squad.x.section]]\nfilter = \"a\"\n",
            "[[squad.x.section]]\ntitle = \"\"\n",
            "[[squad.x.section]]\ntitle = \"T\"\nfilter = \"a and\"\n",
            "[[squad.x.section]]\ntitle = \"T\"\nsort = [\"Bad\"]\n",
            "[[squad.x.section]]\ntitle = \"T\"\ncolour = \"red\"\n",
            "[[squad.x.section]]\ntitle = \"T\"\n[squad.x.section.bind]\no = 3\n",
        ] {
            fs::write(&path, body).unwrap();
            let code = Config::read(path.clone())
                .and_then(|config| config.sections("x"))
                .err()
                .map(|error| error.code);
            assert_eq!(code.as_deref(), Some("SQUAD_CONFIG_INVALID"), "{body}");
        }
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }
}

//! Digest owns its TOML; Core supplies only the configuration directory and identities.

use crate::core::Error;
use nix::fcntl::{Flock, FlockArg};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};
use toml_edit::{DocumentMut, Item, Table};

const FILE_LIMIT: u64 = 1024 * 1024;
const MAX_MS: u64 = 9_007_199_254_740_991;
pub const DEFAULT_FLUSH_COUNT: i64 = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    Auto,
    Off,
    Interval { text: String, milliseconds: u64 },
}
impl Mode {
    pub fn parse(text: &str) -> Result<Self, Error> {
        match text {
            "auto" => Ok(Self::Auto),
            "off" => Ok(Self::Off),
            _ => duration(text).map(|milliseconds| Self::Interval {
                text: if text.bytes().last().is_some_and(|b| b.is_ascii_digit()) {
                    format!("{text}s")
                } else {
                    text.into()
                },
                milliseconds,
            }),
        }
    }
    pub fn text(&self) -> &str {
        match self {
            Self::Auto => "auto",
            Self::Off => "off",
            Self::Interval { text, .. } => text,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemberSetting {
    Default,
    Override(Mode),
}
impl MemberSetting {
    pub fn parse(text: &str) -> Result<Self, Error> {
        if text == "default" {
            Ok(Self::Default)
        } else {
            Mode::parse(text).map(Self::Override)
        }
    }
}
fn invalid(message: impl Into<String>) -> Error {
    Error::new("DIGEST_SETTINGS_INVALID", message)
}

/// Decimal durations are exact milliseconds; no float rounding or sub-millisecond zero.
fn duration(text: &str) -> Result<u64, Error> {
    let fail = || invalid("Use a positive duration in ms/s/m/h/d, auto, off or default");
    let (number, unit) = [
        ("ms", 1),
        ("s", 1000),
        ("m", 60_000),
        ("h", 3_600_000),
        ("d", 86_400_000),
    ]
    .into_iter()
    .find_map(|(suffix, unit)| text.strip_suffix(suffix).map(|number| (number, unit)))
    .unwrap_or((text, 1000));
    let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
    if whole.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || !fraction.bytes().all(|b| b.is_ascii_digit())
        || (number.contains('.') && fraction.is_empty())
        || fraction.len() > 9
    {
        return Err(fail());
    }
    let scale = 10_u64.checked_pow(fraction.len() as u32).ok_or_else(fail)?;
    let whole: u64 = whole.parse().map_err(|_| fail())?;
    let fraction: u64 = if fraction.is_empty() {
        0
    } else {
        fraction.parse().map_err(|_| fail())?
    };
    let fraction = fraction.checked_mul(unit).ok_or_else(fail)?;
    if fraction % scale != 0 {
        return Err(fail());
    }
    let ms = whole
        .checked_mul(unit)
        .and_then(|n| n.checked_add(fraction / scale))
        .ok_or_else(fail)?;
    if ms == 0 || ms > MAX_MS {
        return Err(fail());
    }
    Ok(ms)
}

pub fn canonical_uuid(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(index, b)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                b == b'-'
            } else {
                b.is_ascii_digit() || (b'a'..=b'f').contains(&b)
            }
        })
}

pub struct DigestConfig {
    pub default: Mode,
    pub flush_count: i64,
    document: DocumentMut,
}
impl DigestConfig {
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        let text = std::str::from_utf8(bytes).map_err(|_| invalid("digest.toml must be UTF-8"))?;
        let document = text
            .parse::<DocumentMut>()
            .map_err(|error| invalid(format!("Invalid digest.toml: {error}")))?;
        let default = match document.get("default") {
            None => Mode::Auto,
            Some(item) => Mode::parse(
                item.as_str()
                    .ok_or_else(|| invalid("default must be a duration, auto or off"))?,
            )?,
        };
        let flush_count = match document.get("flushCount") {
            None => DEFAULT_FLUSH_COUNT,
            Some(item) => item
                .as_integer()
                .filter(|count| *count > 0)
                .ok_or_else(|| invalid("flushCount must be a positive integer"))?,
        };
        if let Some(item) = document.get("members") {
            let members = item
                .as_table_like()
                .ok_or_else(|| invalid("members must be a UUID-keyed table"))?;
            for (id, item) in members.iter() {
                if !canonical_uuid(id) {
                    return Err(invalid("Member keys must be canonical identity UUIDs"));
                }
                let row = item
                    .as_table_like()
                    .ok_or_else(|| invalid("Each member must be a table"))?;
                Mode::parse(
                    row.get("mode")
                        .and_then(Item::as_str)
                        .ok_or_else(|| invalid("Member mode must be a duration, auto or off"))?,
                )?;
                if let Some(setter) = row.get("setByIdentityId")
                    && !setter.as_str().is_some_and(canonical_uuid)
                {
                    return Err(invalid("setByIdentityId must be a canonical identity UUID"));
                }
                if let Some(stamp) = row.get("setAtMs")
                    && !stamp
                        .as_integer()
                        .is_some_and(|n| n >= 0 && n as u64 <= MAX_MS)
                {
                    return Err(invalid("setAtMs must be a nonnegative JS-safe integer"));
                }
            }
        }
        Ok(Self {
            default,
            flush_count,
            document,
        })
    }
    pub fn effective(&self, id: &str) -> Result<Mode, Error> {
        match self
            .document
            .get("members")
            .and_then(|item| item.get(id))
            .and_then(|row| row.get("mode"))
            .and_then(Item::as_str)
        {
            Some(text) => Mode::parse(text),
            None => Ok(self.default.clone()),
        }
    }
    fn edit(
        &mut self,
        id: &str,
        setting: &MemberSetting,
        setter: Option<&str>,
        now: i64,
    ) -> Result<(), Error> {
        if !canonical_uuid(id)
            || setter.is_some_and(|id| !canonical_uuid(id))
            || now < 0
            || now as u64 > MAX_MS
        {
            return Err(invalid("Invalid identity or write timestamp"));
        }
        match setting {
            MemberSetting::Default => {
                if let Some(members) = self
                    .document
                    .get_mut("members")
                    .and_then(Item::as_table_like_mut)
                {
                    members.remove(id);
                }
            }
            MemberSetting::Override(mode) => {
                if !self.document.contains_key("members") {
                    self.document["members"] = Item::Table(Table::new());
                }
                let inline = self.document["members"].is_inline_table();
                let members = self.document["members"]
                    .as_table_like_mut()
                    .expect("validated table");
                if members.get(id).is_none() {
                    let row = if inline {
                        Item::Value(toml_edit::InlineTable::new().into())
                    } else {
                        Item::Table(Table::new())
                    };
                    members.insert(id, row);
                }
                let row = members
                    .get_mut(id)
                    .and_then(Item::as_table_like_mut)
                    .expect("validated member table");
                row.insert("mode", toml_edit::value(mode.text()));
                row.insert("setAtMs", toml_edit::value(now));
                if let Some(setter) = setter {
                    row.insert("setByIdentityId", toml_edit::value(setter));
                } else {
                    row.remove("setByIdentityId");
                }
            }
        }
        Ok(())
    }
}

fn read(path: &Path) -> Result<Vec<u8>, Error> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    if !file.metadata()?.is_file() {
        return Err(invalid("digest.toml must be a regular file"));
    }
    let mut bytes = Vec::new();
    file.take(FILE_LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > FILE_LIMIT {
        return Err(invalid("digest.toml exceeds 1 MiB"));
    }
    Ok(bytes)
}

/// Serialize extension writers, preserve comments/unknown fields and publish one atomic file.
pub fn set(
    path: &Path,
    id: &str,
    setting: &MemberSetting,
    setter: Option<&str>,
    now: i64,
) -> Result<(Mode, i64), Error> {
    // Validate the existing file before creating a lock or staging anything.
    DigestConfig::parse(&read(path)?)?;
    let directory = path
        .parent()
        .ok_or_else(|| invalid("Settings path has no directory"))?;
    fs::create_dir_all(directory)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(directory.join(".digest.toml.lock"))?;
    let _guard = Flock::lock(lock, FlockArg::LockExclusive)
        .map_err(|(_, error)| Error::new("DIGEST_SETTINGS_IO", error.to_string()))?;
    let original = read(path)?;
    let mut settings = DigestConfig::parse(&original)?;
    settings.edit(id, setting, setter, now)?;
    let effective = settings.effective(id)?;
    let bytes = settings.document.to_string().into_bytes();
    if bytes.len() as u64 > FILE_LIMIT {
        return Err(invalid("Updated digest.toml exceeds 1 MiB"));
    }
    if bytes == original {
        return Ok((effective, settings.flush_count));
    }
    let staged: PathBuf = directory.join(format!(".digest.toml.{}", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&staged)?;
    let result = (|| {
        file.write_all(&bytes)?;
        file.sync_all()?;
        if read(path).map_err(|e| std::io::Error::other(e.message))? != original {
            return Err(std::io::Error::other(
                "digest.toml changed during the edit; retry",
            ));
        }
        fs::rename(&staged, path)?;
        File::open(directory)?.sync_all()
    })();
    if result.is_err() {
        let _ = fs::remove_file(&staged);
    }
    result?;
    Ok((effective, settings.flush_count))
}

#[cfg(test)]
#[path = "settings/tests.rs"]
mod tests;

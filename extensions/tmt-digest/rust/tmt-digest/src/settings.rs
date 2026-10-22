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
/// An `auto` member's hold ends with this many messages held, or when the oldest has waited
/// this long. Both are settable ([`Caps`]); the re-offer interval is not.
pub const DEFAULT_MAX_COUNT: u64 = 5;
pub const DEFAULT_MAX_WAIT_MS: u64 = 300_000;
const MIN_MAX_WAIT_MS: u64 = 30_000;
const MAX_MAX_WAIT_MS: u64 = 3_600_000;
const MAX_MAX_COUNT: u64 = 100;

/// The caps an `auto` member runs under, after the member's value, the global one and the defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AutoCaps {
    pub max_wait_ms: u64,
    pub max_count: u64,
}
/// Caps a command supplies; a field left out keeps what is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Caps {
    pub max_wait_ms: Option<u64>,
    pub max_count: Option<u64>,
}
impl Default for AutoCaps {
    fn default() -> Self {
        Self {
            max_wait_ms: DEFAULT_MAX_WAIT_MS,
            max_count: DEFAULT_MAX_COUNT,
        }
    }
}
impl Caps {
    pub fn is_empty(&self) -> bool {
        self.max_wait_ms.is_none() && self.max_count.is_none()
    }
}
/// The longest an `auto` hold may keep a message: 30 seconds to one hour.
pub fn max_wait(text: &str) -> Result<u64, Error> {
    duration(text)
        .ok()
        .filter(|ms| (MIN_MAX_WAIT_MS..=MAX_MAX_WAIT_MS).contains(ms))
        .ok_or_else(|| invalid("The longest wait must be a duration from 30s to 1h (3600s)"))
}
/// The most messages one `auto` checklist waits for: 1 to 100.
pub fn max_count(count: u64) -> Result<u64, Error> {
    (1..=MAX_MAX_COUNT)
        .contains(&count)
        .then_some(count)
        .ok_or_else(|| invalid("The most messages held must be a whole number from 1 to 100"))
}
fn wait_text(ms: u64) -> String {
    if ms.is_multiple_of(1000) {
        format!("{}s", ms / 1000)
    } else {
        format!("{ms}ms")
    }
}
fn read_caps(row: &dyn toml_edit::TableLike) -> Result<Caps, Error> {
    let max_wait_ms = match row.get("maxWait") {
        None => None,
        Some(item) => Some(max_wait(item.as_str().ok_or_else(|| {
            invalid("maxWait must be a duration string such as '90s'")
        })?)?),
    };
    let max_count = match row.get("maxCount") {
        None => None,
        Some(item) => Some(max_count(
            item.as_integer()
                .and_then(|count| u64::try_from(count).ok())
                .ok_or_else(|| invalid("maxCount must be a whole number from 1 to 100"))?,
        )?),
    };
    Ok(Caps {
        max_wait_ms,
        max_count,
    })
}

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
    /// The global switch (`enabled = false`; absent means on). While off every member is
    /// handled as `off` and no stored setting is read or changed.
    enabled: bool,
    global_caps: Caps,
    pub flush_count: i64,
    document: DocumentMut,
}
impl DigestConfig {
    pub fn load(path: &Path) -> Result<Self, Error> {
        Self::parse(&read(path)?)
    }
    pub fn setter(&self, id: &str) -> Option<&str> {
        self.document
            .get("members")
            .and_then(|members| members.get(id))
            .and_then(|row| row.get("setByIdentityId"))
            .and_then(Item::as_str)
    }
    /// Who set the global default, when it was written with `tmt digest global`.
    pub fn default_setter(&self) -> Option<&str> {
        self.document
            .get("defaultSetByIdentityId")
            .and_then(Item::as_str)
    }
    /// The recorded setter behind a member's effective setting: its own override's,
    /// or for a member that inherits, the global default's.
    pub fn recorded_setter(&self, id: &str) -> Option<&str> {
        match self.member_setting(id) {
            Ok(MemberSetting::Override(_)) => self.setter(id),
            _ => self.default_setter(),
        }
    }
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
        let enabled = match document.get("enabled") {
            None => true,
            Some(item) => item
                .as_bool()
                .ok_or_else(|| invalid("enabled must be true or false"))?,
        };
        let global_caps = read_caps(document.as_table())?;
        let flush_count = match document.get("flushCount") {
            None => DEFAULT_FLUSH_COUNT,
            Some(item) => item
                .as_integer()
                .filter(|count| *count > 0)
                .ok_or_else(|| invalid("flushCount must be a positive integer"))?,
        };
        if let Some(setter) = document.get("defaultSetByIdentityId")
            && !setter.as_str().is_some_and(canonical_uuid)
        {
            return Err(invalid(
                "defaultSetByIdentityId must be a canonical identity UUID",
            ));
        }
        if let Some(stamp) = document.get("defaultSetAtMs")
            && !stamp
                .as_integer()
                .is_some_and(|n| n >= 0 && n as u64 <= MAX_MS)
        {
            return Err(invalid(
                "defaultSetAtMs must be a nonnegative JS-safe integer",
            ));
        }
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
                let mode = Mode::parse(
                    row.get("mode")
                        .and_then(Item::as_str)
                        .ok_or_else(|| invalid("Member mode must be a duration, auto or off"))?,
                )?;
                if !read_caps(row)?.is_empty() && mode != Mode::Auto {
                    return Err(invalid("maxWait and maxCount apply to an auto member only"));
                }
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
            global_caps,
            enabled,
            flush_count,
            document,
        })
    }
    pub fn member_setting(&self, id: &str) -> Result<MemberSetting, Error> {
        match self
            .document
            .get("members")
            .and_then(|item| item.get(id))
            .and_then(|row| row.get("mode"))
            .and_then(Item::as_str)
        {
            Some(text) => Mode::parse(text).map(MemberSetting::Override),
            None => Ok(MemberSetting::Default),
        }
    }
    /// The caps an `auto` member runs under: its own value, else the global one, else the default.
    pub fn auto_caps(&self, id: &str) -> Result<AutoCaps, Error> {
        let own = match self
            .document
            .get("members")
            .and_then(|members| members.get(id))
            .and_then(Item::as_table_like)
        {
            Some(row) => read_caps(row)?,
            None => Caps::default(),
        };
        Ok(AutoCaps {
            max_wait_ms: own
                .max_wait_ms
                .or(self.global_caps.max_wait_ms)
                .unwrap_or(DEFAULT_MAX_WAIT_MS),
            max_count: own
                .max_count
                .or(self.global_caps.max_count)
                .unwrap_or(DEFAULT_MAX_COUNT),
        })
    }
    /// Whether Digest batches at all. A legacy `default = "off"` is the same as off.
    pub fn enabled(&self) -> bool {
        self.enabled && self.default != Mode::Off
    }
    /// What the member would get with the switch on.
    pub fn stored_effective(&self, id: &str) -> Result<Mode, Error> {
        match self.member_setting(id)? {
            MemberSetting::Default => Ok(self.default.clone()),
            MemberSetting::Override(mode) => Ok(mode),
        }
    }
    /// What the tick applies: the switch masks every stored setting.
    pub fn effective(&self, id: &str) -> Result<Mode, Error> {
        if !self.enabled {
            return Ok(Mode::Off);
        }
        match self.member_setting(id)? {
            MemberSetting::Default => Ok(self.default.clone()),
            MemberSetting::Override(mode) => Ok(mode),
        }
    }
    pub fn set_at_ms(&self, id: &str) -> Option<i64> {
        self.document
            .get("members")
            .and_then(|members| members.get(id))
            .and_then(|row| row.get("setAtMs"))
            .and_then(Item::as_integer)
    }
    /// Flip the switch only; every stored setting stays. Turning it on removes the key and
    /// gives a legacy `default = "off"`, which has nothing to restore, the shipped `auto`.
    fn edit_enabled(&mut self, enabled: bool) -> Result<(), Error> {
        if enabled {
            self.document.remove("enabled");
            if self.default == Mode::Off {
                self.document["default"] = toml_edit::value(Mode::Auto.text());
                self.default = Mode::Auto;
            }
        } else {
            self.document["enabled"] = toml_edit::value(false);
        }
        self.enabled = enabled;
        Ok(())
    }
    /// The global caps belong to every `auto` member, whatever the default is, so a new
    /// default keeps them; supplying caps with a mode that is not `auto` is refused.
    fn edit_default(
        &mut self,
        mode: &Mode,
        caps: &Caps,
        setter: Option<&str>,
        now: i64,
    ) -> Result<(), Error> {
        if !caps.is_empty() && *mode != Mode::Auto {
            return Err(invalid("--max-wait and --max-count apply to auto only"));
        }
        if setter.is_some_and(|id| !canonical_uuid(id)) || now < 0 || now as u64 > MAX_MS {
            return Err(invalid("Invalid identity or write timestamp"));
        }
        self.document["default"] = toml_edit::value(mode.text());
        self.document["defaultSetAtMs"] = toml_edit::value(now);
        match setter {
            Some(setter) => {
                self.document["defaultSetByIdentityId"] = toml_edit::value(setter);
            }
            None => {
                self.document.remove("defaultSetByIdentityId");
            }
        }
        self.default = mode.clone();
        write_caps(self.document.as_table_mut(), caps);
        if let Some(ms) = caps.max_wait_ms {
            self.global_caps.max_wait_ms = Some(ms);
        }
        if let Some(count) = caps.max_count {
            self.global_caps.max_count = Some(count);
        }
        // Choosing a global mode is choosing to batch with it.
        self.edit_enabled(true)
    }
    /// Caps ride with an `auto` row: an `auto` save without caps keeps the stored ones, any
    /// other mode drops them, and caps with a mode that is not `auto` are refused.
    fn edit(
        &mut self,
        id: &str,
        setting: &MemberSetting,
        caps: &Caps,
        setter: Option<&str>,
        now: i64,
    ) -> Result<(), Error> {
        if !caps.is_empty() && *setting != MemberSetting::Override(Mode::Auto) {
            return Err(invalid("--max-wait and --max-count apply to auto only"));
        }
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
                if *mode == Mode::Auto {
                    write_caps(row, caps);
                } else {
                    row.remove("maxWait");
                    row.remove("maxCount");
                }
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

fn write_caps(table: &mut dyn toml_edit::TableLike, caps: &Caps) {
    if let Some(ms) = caps.max_wait_ms {
        table.insert("maxWait", toml_edit::value(wait_text(ms)));
    }
    if let Some(count) = caps.max_count {
        table.insert("maxCount", toml_edit::value(count as i64));
    }
}

/// Save one member's setting with the auto caps the command supplied.
pub fn set_caps(
    path: &Path,
    id: &str,
    setting: &MemberSetting,
    caps: &Caps,
    setter: Option<&str>,
    now: i64,
) -> Result<(Mode, i64), Error> {
    let settings = update(path, |settings| {
        settings.edit(id, setting, caps, setter, now)
    })?;
    Ok((settings.stored_effective(id)?, settings.flush_count))
}

/// Save the global default with the auto caps the command supplied.
pub fn set_default_caps(
    path: &Path,
    mode: &Mode,
    caps: &Caps,
    setter: Option<&str>,
    now: i64,
) -> Result<(), Error> {
    update(path, |settings| {
        settings.edit_default(mode, caps, setter, now)
    })
    .map(drop)
}

/// Turn Digest on or off for everyone without touching any stored setting.
pub fn set_enabled(path: &Path, enabled: bool) -> Result<(), Error> {
    update(path, |settings| settings.edit_enabled(enabled)).map(drop)
}

/// Serialize extension writers, preserve comments/unknown fields and publish one atomic file.
fn update(
    path: &Path,
    edit: impl FnOnce(&mut DigestConfig) -> Result<(), Error>,
) -> Result<DigestConfig, Error> {
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
    edit(&mut settings)?;
    let bytes = settings.document.to_string().into_bytes();
    if bytes.len() as u64 > FILE_LIMIT {
        return Err(invalid("Updated digest.toml exceeds 1 MiB"));
    }
    if bytes == original {
        return Ok(settings);
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
    Ok(settings)
}

#[cfg(test)]
#[path = "settings/tests.rs"]
mod tests;

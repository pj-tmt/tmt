//! File drops: a terminal never receives file bytes, it pastes the dropped files' paths.
//! A paste that is only existing regular files becomes one chip per file. The chip is an
//! Editor fold over the absolute path, so the editor stays the only source of send bytes;
//! sizes come from `stat` at once, image dimensions and the popup's text come from a worker.
use std::{
    collections::BTreeMap,
    io::Read,
    sync::mpsc::{self, Sender},
};

/// A drop is at most this many files and bytes of pasted text.
pub(super) const MAX_FILES: usize = 16;
pub(super) const MAX_PASTE: usize = 16 * 1024;
/// The worker reads at most this much of a file: enough for any image header and a text head.
const HEAD_BYTES: u64 = 64 * 1024;
const TEXT_BYTES: usize = 8 * 1024;
const TEXT_LINES: usize = 10;
const LINE_CHARS: usize = 200;
/// The table keeps this many files; the oldest probe goes first.
const TABLE_LIMIT: usize = 128;

/// One dropped file as `stat` sees it when the paste arrives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::board) struct Dropped {
    pub path: String,
    pub size: u64,
}

/// What the worker learned from the file's first bytes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::board) struct Head {
    pub dims: Option<(u32, u32)>,
    pub kind: Option<&'static str>,
    /// The first lines of a text file; none for images, binary files and unreadable files.
    pub lines: Option<Vec<String>>,
    pub failed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::board) struct Info {
    pub size: u64,
    pub head: Option<Head>,
    seq: u64,
}

/// The file facts of this session, by absolute path.
#[derive(Default)]
pub(in crate::board) struct Table {
    files: BTreeMap<String, Info>,
    next: u64,
}

impl Table {
    pub(super) fn get(&self, path: &str) -> Option<&Info> {
        self.files.get(path)
    }
    /// Returns whether the path still needs its worker probe.
    pub(super) fn note(&mut self, file: &Dropped) -> bool {
        if let Some(info) = self.files.get_mut(&file.path) {
            info.size = file.size;
            return info.head.is_none();
        }
        if self.files.len() >= TABLE_LIMIT
            && let Some(oldest) = self
                .files
                .iter()
                .min_by_key(|(_, info)| info.seq)
                .map(|(path, _)| path.clone())
        {
            self.files.remove(&oldest);
        }
        self.next += 1;
        self.files.insert(
            file.path.clone(),
            Info {
                size: file.size,
                head: None,
                seq: self.next,
            },
        );
        true
    }
    pub(super) fn apply(&mut self, delivery: Delivery) -> Option<&Info> {
        let info = self.files.get_mut(&delivery.path)?;
        info.head = Some(delivery.head);
        Some(info)
    }
}

/// One finished probe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::board) struct Delivery {
    pub path: String,
    pub head: Head,
}

/// The worker: one thread, one path at a time, never on the paste or draw path.
#[derive(Clone)]
pub(in crate::board) struct Probe {
    paths: Sender<String>,
}

impl Probe {
    pub(in crate::board) fn spawn(deliver: impl Fn(Delivery) -> bool + Send + 'static) -> Self {
        let (paths, requests) = mpsc::channel::<String>();
        std::thread::spawn(move || {
            while let Ok(path) = requests.recv() {
                let head = read_head(&path);
                if !deliver(Delivery { path, head }) {
                    break;
                }
            }
        });
        Self { paths }
    }
    pub(super) fn request(&self, path: &str) {
        let _ = self.paths.send(path.to_owned());
    }
}

/// What a chip puts in the message for `path`: the path itself, shell-quoted when whitespace, a
/// quote or a backslash would blur where it ends next to the other chips.
pub(super) fn send_text(path: &str) -> String {
    if path
        .chars()
        .any(|ch| ch.is_whitespace() || matches!(ch, '\'' | '"' | '\\'))
    {
        format!("'{}'", path.replace('\'', "'\\''"))
    } else {
        path.to_owned()
    }
}

/// The path a chip's message text stands for; None unless the text is exactly one path.
pub(super) fn path_of(source: &str) -> Option<String> {
    let mut tokens = tokens(source)?;
    (tokens.len() == 1).then(|| tokens.remove(0))
}

/// Splits a paste with the shell's quoting: `'single'`, `"double"` and backslash escapes, so
/// every way a terminal writes a path with spaces reads back as one token. None when a quote
/// is left open.
fn tokens(text: &str) -> Option<Vec<String>> {
    let mut out = Vec::new();
    let mut token = String::new();
    let mut started = false;
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        match ch {
            c if c.is_whitespace() => {
                if started {
                    out.push(std::mem::take(&mut token));
                    started = false;
                }
            }
            '\\' => {
                token.push(chars.next()?);
                started = true;
            }
            '\'' => {
                started = true;
                loop {
                    match chars.next()? {
                        '\'' => break,
                        c => token.push(c),
                    }
                }
            }
            '"' => {
                started = true;
                loop {
                    match chars.next()? {
                        '"' => break,
                        '\\' => match chars.next()? {
                            c @ ('"' | '\\' | '$' | '`') => token.push(c),
                            c => {
                                token.push('\\');
                                token.push(c);
                            }
                        },
                        c => token.push(c),
                    }
                }
            }
            c => {
                token.push(c);
                started = true;
            }
        }
    }
    if started {
        out.push(token);
    }
    Some(out)
}

/// `file:///a%20b` is `/a b`; `localhost` is the only host that names this machine.
fn uri_path(token: &str) -> Option<String> {
    let rest = token.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    if !rest.starts_with('/') {
        return None;
    }
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = rest.get(index + 1..index + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The dropped files, when the whole paste is existing regular files. Anything else, a missing
/// file, a directory, a relative path or any other text, is an ordinary paste.
pub(super) fn parse(text: &str) -> Option<Vec<Dropped>> {
    if text.trim().is_empty() || text.len() > MAX_PASTE {
        return None;
    }
    let tokens = tokens(text)?;
    if tokens.is_empty() || tokens.len() > MAX_FILES {
        return None;
    }
    tokens
        .into_iter()
        .map(|token| {
            let path = if token.starts_with("file://") {
                uri_path(&token)?
            } else {
                token
            };
            if !path.starts_with('/') || path.chars().any(char::is_control) {
                return None;
            }
            let meta = std::fs::metadata(&path).ok()?;
            meta.is_file().then_some(Dropped {
                size: meta.len(),
                path,
            })
        })
        .collect()
}

pub(super) fn name(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    crate::board::notes::sanitize(name)
}

pub(super) fn size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let value = bytes as f64;
    if bytes < 1024 {
        format!("{bytes} B")
    } else if value < KB * KB {
        format!("{} KB", (value / KB).round() as u64)
    } else if value < KB * KB * KB {
        format!("{:.1} MB", value / (KB * KB))
    } else {
        format!("{:.1} GB", value / (KB * KB * KB))
    }
}

/// The chip's text without its remove mark: `name · 1200×800 · 312 KB`, or in ASCII
/// `name 1200x800 312 KB`.
pub(super) fn content(path: &str, info: &Info, ascii: bool) -> String {
    let name = name(path);
    let dims = info
        .head
        .as_ref()
        .and_then(|head| head.dims)
        .map(|(w, h)| format!("{w}{}{h}", if ascii { "x" } else { "×" }));
    let mut parts = vec![name];
    parts.extend(dims);
    parts.push(size(info.size));
    parts.join(if ascii { " " } else { " · " })
}

/// The text the editor reserves the chip's cells from.
pub(super) fn label(path: &str, info: &Info) -> String {
    format!("{} ×", content(path, info, false))
}

/// The popup: the path, then what is known of the file.
pub(super) fn popup(path: &str, info: &Info) -> String {
    let mut text = format!("{path}\n\nSize: {} ({} bytes)", size(info.size), info.size);
    match &info.head {
        None => text.push_str("\nReading…"),
        Some(head) if head.failed => text.push_str("\nCannot read this file."),
        Some(head) => {
            if let Some(kind) = head.kind {
                text.push_str(&format!("\nType: {kind}"));
            }
            if let Some((w, h)) = head.dims {
                text.push_str(&format!("\nDimensions: {w}×{h}"));
            }
            if let Some(lines) = &head.lines {
                text.push_str("\n\n");
                text.push_str(&lines.join("\n"));
            }
        }
    }
    text
}

fn read_head(path: &str) -> Head {
    let mut bytes = Vec::new();
    let read =
        std::fs::File::open(path).and_then(|file| file.take(HEAD_BYTES).read_to_end(&mut bytes));
    if read.is_err() {
        return Head {
            failed: true,
            ..Head::default()
        };
    }
    head_of(&bytes)
}

pub(super) fn head_of(bytes: &[u8]) -> Head {
    if let Some((kind, dims)) = image(bytes) {
        return Head {
            dims,
            kind: Some(kind),
            ..Head::default()
        };
    }
    let sample = &bytes[..bytes.len().min(TEXT_BYTES)];
    let text = match std::str::from_utf8(sample) {
        Ok(text) => Some(text),
        // A multi-byte character cut by the sample is still text.
        Err(error) if error.error_len().is_none() => {
            std::str::from_utf8(&sample[..error.valid_up_to()]).ok()
        }
        Err(_) => None,
    };
    match text.filter(|text| !text.contains('\0')) {
        Some(text) => Head {
            kind: Some("text"),
            lines: Some(
                text.lines()
                    .take(TEXT_LINES)
                    .map(|line| {
                        let line: String = line.chars().take(LINE_CHARS).collect();
                        crate::board::notes::sanitize(&line)
                    })
                    .collect(),
            ),
            ..Head::default()
        },
        None => Head::default(),
    }
}

/// The image's type and the size its header states, without decoding a pixel.
fn image(b: &[u8]) -> Option<(&'static str, Option<(u32, u32)>)> {
    let be32 = |at: usize| {
        b.get(at..at + 4)
            .map(|v| u32::from_be_bytes(v.try_into().unwrap()))
    };
    let le16 = |at: usize| {
        b.get(at..at + 2)
            .map(|v| u32::from(u16::from_le_bytes(v.try_into().unwrap())))
    };
    let nonzero = |dims: Option<(u32, u32)>| dims.filter(|(w, h)| *w > 0 && *h > 0);
    if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some(("image/png", nonzero(be32(16).zip(be32(20)))));
    }
    if b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a") {
        return Some(("image/gif", nonzero(le16(6).zip(le16(8)))));
    }
    if b.starts_with(&[0xFF, 0xD8]) {
        return Some(("image/jpeg", nonzero(jpeg(b))));
    }
    if b.len() >= 16 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        return Some(("image/webp", nonzero(webp(b))));
    }
    None
}

fn jpeg(b: &[u8]) -> Option<(u32, u32)> {
    let mut at = 2;
    while at + 4 <= b.len() {
        if b[at] != 0xFF {
            return None;
        }
        let marker = b[at + 1];
        match marker {
            0xFF => at += 1,
            // Standalone markers carry no length.
            0x01 | 0xD0..=0xD8 => at += 2,
            0xC0..=0xCF if !matches!(marker, 0xC4 | 0xC8 | 0xCC) => {
                let h = u32::from(u16::from_be_bytes(b.get(at + 5..at + 7)?.try_into().ok()?));
                let w = u32::from(u16::from_be_bytes(b.get(at + 7..at + 9)?.try_into().ok()?));
                return Some((w, h));
            }
            _ => {
                let len = usize::from(u16::from_be_bytes(b.get(at + 2..at + 4)?.try_into().ok()?));
                at += 2 + len;
            }
        }
    }
    None
}

fn webp(b: &[u8]) -> Option<(u32, u32)> {
    let le24 = |at: usize| {
        let v = b.get(at..at + 3)?;
        Some(u32::from(v[0]) | u32::from(v[1]) << 8 | u32::from(v[2]) << 16)
    };
    match b.get(12..16)? {
        b"VP8X" => Some((le24(24)? + 1, le24(27)? + 1)),
        b"VP8L" if *b.get(20)? == 0x2F => {
            let v = b.get(21..25)?;
            let bits = u32::from_le_bytes(v.try_into().ok()?);
            Some(((bits & 0x3FFF) + 1, ((bits >> 14) & 0x3FFF) + 1))
        }
        b"VP8 " if b.get(23..26)? == [0x9D, 0x01, 0x2A] => {
            let w = u32::from(u16::from_le_bytes(b.get(26..28)?.try_into().ok()?) & 0x3FFF);
            let h = u32::from(u16::from_le_bytes(b.get(28..30)?.try_into().ok()?) & 0x3FFF);
            Some((w, h))
        }
        _ => None,
    }
}

#[cfg(test)]
pub(in crate::board) mod tests;

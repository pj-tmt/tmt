//! The tiled tmux v2 producer grammar, not an arbitrary JSON document reader.
//! Metadata is validated here; geometry and recovery admission have one owner.

use super::*;
use WorkspaceLayoutError::{DuplicatePane, Geometry, Limit, Syntax};

pub(super) fn parse(value: &str) -> Result<WorkspaceLayoutCell, WorkspaceLayoutError> {
    let mut p = Parser {
        bytes: value.as_bytes(),
        offset: 0,
        panes: HashSet::new(),
        indexes: HashSet::new(),
        last: HashSet::new(),
        active: false,
    };
    p.take(b'{')?;
    let mut version = None;
    let mut root = None;
    loop {
        match p.string()? {
            "V" if version.is_none() => {
                p.take(b':')?;
                version = Some(p.number()?);
            }
            "L" if root.is_none() => {
                p.take(b':')?;
                root = Some(p.cell(0)?);
            }
            _ => return Err(Syntax),
        }
        if p.end(b'}')? {
            break;
        }
    }
    p.whitespace();
    if version != Some(2) || p.offset != p.bytes.len() {
        return Err(Syntax);
    }
    root.ok_or(Syntax)
}

struct Parser<'a> {
    bytes: &'a [u8],
    offset: usize,
    panes: HashSet<u32>,
    indexes: HashSet<u32>,
    last: HashSet<u32>,
    active: bool,
}

impl<'a> Parser<'a> {
    fn whitespace(&mut self) {
        while self
            .bytes
            .get(self.offset)
            .is_some_and(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r'))
        {
            self.offset += 1;
        }
    }
    fn take(&mut self, byte: u8) -> Result<(), WorkspaceLayoutError> {
        self.whitespace();
        if self.bytes.get(self.offset) != Some(&byte) {
            return Err(Syntax);
        }
        self.offset += 1;
        Ok(())
    }
    fn end(&mut self, close: u8) -> Result<bool, WorkspaceLayoutError> {
        self.whitespace();
        if self.bytes.get(self.offset) == Some(&close) {
            self.offset += 1;
            Ok(true)
        } else {
            self.take(b',')?;
            Ok(false)
        }
    }
    fn string(&mut self) -> Result<&'a str, WorkspaceLayoutError> {
        self.take(b'"')?;
        let start = self.offset;
        while let Some(byte) = self.bytes.get(self.offset) {
            if *byte == b'"' {
                let value =
                    std::str::from_utf8(&self.bytes[start..self.offset]).map_err(|_| Syntax)?;
                self.offset += 1;
                return Ok(value);
            }
            // Producer keys, types and pane IDs are literal ASCII only.
            if !byte.is_ascii_graphic() || *byte == b'\\' {
                return Err(Syntax);
            }
            self.offset += 1;
        }
        Err(Syntax)
    }
    fn number(&mut self) -> Result<u32, WorkspaceLayoutError> {
        self.whitespace();
        let start = self.offset;
        let mut value = 0u32;
        while let Some(b) = self.bytes.get(self.offset).filter(|b| b.is_ascii_digit()) {
            value = value
                .checked_mul(10)
                .and_then(|n| n.checked_add(u32::from(*b - b'0')))
                .ok_or(Syntax)?;
            self.offset += 1;
        }
        if self.offset == start || (self.offset > start + 1 && self.bytes[start] == b'0') {
            return Err(Syntax);
        }
        Ok(value)
    }
    fn boolean(&mut self) -> Result<bool, WorkspaceLayoutError> {
        self.whitespace();
        for (literal, value) in [(b"true".as_slice(), true), (b"false".as_slice(), false)] {
            if self.bytes[self.offset..].starts_with(literal) {
                self.offset += literal.len();
                return Ok(value);
            }
        }
        Err(Syntax)
    }
    fn cell(&mut self, depth: usize) -> Result<WorkspaceLayoutCell, WorkspaceLayoutError> {
        if depth >= MAX_DEPTH {
            return Err(Limit);
        }
        self.take(b'{')?;
        let (mut kind, mut width, mut height, mut left, mut top) = (None, None, None, None, None);
        let (mut children, mut index, mut id, mut active, mut last) =
            (None, None, None, None, None);
        let mut seen = 0u16;
        loop {
            let key = self.string()?;
            let bit = match key {
                "t" => 0,
                "w" => 1,
                "h" => 2,
                "x" => 3,
                "y" => 4,
                "c" => 5,
                "i" => 6,
                "I" => 7,
                "a" => 8,
                "l" => 9,
                _ => return Err(Syntax),
            };
            if seen & (1 << bit) != 0 {
                return Err(Syntax);
            }
            seen |= 1 << bit;
            self.take(b':')?;
            match key {
                "t" => kind = Some(self.string()?),
                "w" => width = Some(self.dimension()?),
                "h" => height = Some(self.dimension()?),
                "x" => left = Some(self.dimension()?),
                "y" => top = Some(self.dimension()?),
                "i" => index = Some(self.index()?),
                "I" => {
                    let value = self.string()?.strip_prefix('%').ok_or(Syntax)?;
                    let number = value.parse::<u32>().map_err(|_| Syntax)?;
                    if number.to_string() != value {
                        return Err(Syntax);
                    }
                    id = Some(number);
                }
                "a" => active = Some(self.boolean()?),
                "l" => last = Some(self.index()?),
                "c" => {
                    self.take(b'[')?;
                    let mut values = Vec::new();
                    loop {
                        values.push(self.cell(depth + 1)?);
                        if values.len() > MAX_PANES {
                            return Err(Limit);
                        }
                        if self.end(b']')? {
                            break;
                        }
                    }
                    children = Some(values);
                }
                _ => unreachable!(),
            }
            if self.end(b'}')? {
                break;
            }
        }
        let kind = match kind.ok_or(Syntax)? {
            "p" => {
                if children.is_some() || (active.is_some() && last.is_some()) {
                    return Err(Syntax);
                }
                let id = id.ok_or(Syntax)?;
                if !self.panes.insert(id) {
                    return Err(DuplicatePane);
                }
                if self.panes.len() > MAX_PANES {
                    return Err(Limit);
                }
                if !self.indexes.insert(index.ok_or(Syntax)?) {
                    return Err(Syntax);
                }
                if let Some(last) = last
                    && !self.last.insert(last)
                {
                    return Err(Syntax);
                }
                if active == Some(true) {
                    if self.active {
                        return Err(Syntax);
                    }
                    self.active = true;
                }
                WorkspaceLayoutKind::Pane(id)
            }
            kind @ ("h" | "v") => {
                if index.is_some() || id.is_some() || active.is_some() || last.is_some() {
                    return Err(Syntax);
                }
                WorkspaceLayoutKind::Split(
                    if kind == "h" {
                        WorkspaceSplitAxis::Horizontal
                    } else {
                        WorkspaceSplitAxis::Vertical
                    },
                    children.ok_or(Syntax)?,
                )
            }
            _ => return Err(Syntax),
        };
        Ok(WorkspaceLayoutCell {
            width: width.ok_or(Syntax)?,
            height: height.ok_or(Syntax)?,
            left: left.ok_or(Syntax)?,
            top: top.ok_or(Syntax)?,
            kind,
        })
    }
    fn dimension(&mut self) -> Result<u16, WorkspaceLayoutError> {
        u16::try_from(self.number()?).map_err(|_| Geometry)
    }
    fn index(&mut self) -> Result<u32, WorkspaceLayoutError> {
        self.number()?
            .try_into()
            .map(|n: i32| n as u32)
            .map_err(|_| Syntax)
    }
}

pub(super) fn serialize(root: &WorkspaceLayoutCell) -> String {
    let mut out = String::from("{\"V\":2,\"L\":");
    serialize_cell(root, &mut 0, &mut out);
    out.push('}');
    out
}

fn serialize_cell(cell: &WorkspaceLayoutCell, index: &mut u32, out: &mut String) {
    use std::fmt::Write;
    let kind = match &cell.kind {
        WorkspaceLayoutKind::Pane(_) => "p",
        WorkspaceLayoutKind::Split(WorkspaceSplitAxis::Horizontal, _) => "h",
        WorkspaceLayoutKind::Split(WorkspaceSplitAxis::Vertical, _) => "v",
    };
    write!(
        out,
        "{{\"t\":\"{kind}\",\"w\":{},\"h\":{},\"x\":{},\"y\":{}",
        cell.width, cell.height, cell.left, cell.top
    )
    .unwrap();
    match &cell.kind {
        WorkspaceLayoutKind::Pane(id) => {
            write!(out, ",\"i\":{index},\"I\":\"%{id}\"").unwrap();
            *index += 1;
        }
        WorkspaceLayoutKind::Split(_, children) => {
            out.push_str(",\"c\":[");
            for (i, child) in children.iter().enumerate() {
                if i != 0 {
                    out.push(',');
                }
                serialize_cell(child, index, out);
            }
            out.push(']');
        }
    }
    out.push('}');
}

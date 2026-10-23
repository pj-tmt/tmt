//! Validate saved tmux geometry before any restore effect is permitted.

use std::collections::HashSet;

mod json;

use super::{MAX_PANES, WorkspacePane, WorkspaceSession, WorkspaceSnapshot, WorkspaceWindow};

// tmux's serialized layout buffer is bounded. Limit recursion independently of
// the snapshot byte bound so malformed recovery input cannot exhaust the stack.
const MAX_LAYOUT_BYTES: usize = 8196; // 8191 body bytes plus checksum and comma.
const MAX_DEPTH: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceSplitAxis {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceLayoutKind {
    Pane(u32),
    Split(WorkspaceSplitAxis, Vec<WorkspaceLayoutCell>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceLayoutCell {
    pub width: u16,
    pub height: u16,
    pub left: u16,
    pub top: u16,
    pub kind: WorkspaceLayoutKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceLayoutError {
    Limit,
    Checksum,
    Syntax,
    Geometry,
    DuplicatePane,
    Topology,
}

/// One split of an invocation-owned pane. Historical IDs are correspondence
/// keys; the host substitutes only IDs returned by this restore invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkspaceLayoutSplit {
    pub target_leaf: u32,
    pub new_leaf: u32,
    pub axis: WorkspaceSplitAxis,
    pub remaining_extent: u16,
}

/// Geometry and leaf correspondence checked for every window before creation.
/// References keep the snapshot as the sole owner of names and directories.
pub struct WorkspaceLayoutWindow<'a> {
    pub window: &'a WorkspaceWindow,
    /// The tiled tree only; floating panes are not part of it.
    pub root: WorkspaceLayoutCell,
    pub panes: Vec<&'a WorkspacePane>,
    /// Panes that floated above the tiled tree (tmux 3.8 `new-pane`), frontmost first.
    /// Restore creates none of them and targets none of their historical IDs.
    pub floating: Vec<&'a WorkspacePane>,
    pub zoomed: bool,
    pub border_status: &'static str,
    pub splits: Vec<WorkspaceLayoutSplit>,
}

/// Pure creation order. It does not authorize effects on an observed server.
pub struct WorkspaceLayoutPlan<'a> {
    pub windows: Vec<WorkspaceLayoutWindow<'a>>,
    pub sessions: Vec<WorkspaceLayoutSession<'a>>,
}

pub struct WorkspaceLayoutSession<'a> {
    pub session: &'a WorkspaceSession,
    pub action: WorkspaceLayoutSessionAction,
}

pub enum WorkspaceLayoutSessionAction {
    /// This snapshot's restore already made the session; a repeat changes nothing.
    SkipExisting,
    Create {
        /// The new session's name: the saved one, or `<name>-restored` beside a session in use.
        name: String,
        /// Use a real, not-yet-created window for new-session whenever possible.
        seed_window: Option<usize>,
        /// Only an all-shared session needs an invocation-owned bootstrap pane.
        bootstrap_index: Option<u64>,
        links: Vec<WorkspaceLayoutLink>,
    },
    /// The saved windows are added to the server's fresh default session, after its own window.
    /// The indexes are assigned here; that window is never moved, renumbered or killed.
    Merge {
        name: String,
        links: Vec<WorkspaceLayoutLink>,
    },
}

/// What a restore saw of a live session, so a name in use is never skipped silently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveSession {
    /// A restore made it earlier and recorded `marker` on it. Under any name: a session restored
    /// beside one in use keeps its `-restored` name, and a repeat still recognises it.
    Restored { name: String, marker: String },
    /// The server's own fresh default: one window of one pane with no agent bound to it.
    Fresh { name: String, window_index: u64 },
    /// Any other session.
    InUse(String),
}

impl LiveSession {
    pub fn name(&self) -> &str {
        match self {
            Self::Restored { name, .. } | Self::InUse(name) | Self::Fresh { name, .. } => name,
        }
    }
}

/// The value a restore records on a session it made, so the same snapshot restored again
/// recognises it. Another snapshot's session with the same name is not this one.
pub fn restored_marker(snapshot: &WorkspaceSnapshot, session: &WorkspaceSession) -> String {
    format!("{}/{}", snapshot.captured_at_ms, session.id)
}

/// The first of `<name>-restored`, `<name>-restored-2`, … that no session uses or will use.
fn beside(name: &str, taken: &HashSet<String>) -> String {
    let mut candidate = format!("{name}-restored");
    let mut number = 2;
    while taken.contains(&candidate) {
        candidate = format!("{name}-restored-{number}");
        number += 1;
    }
    candidate
}

pub struct WorkspaceLayoutLink {
    pub index: u64,
    pub window: usize,
    pub create: bool,
    pub active: bool,
}

/// A session this snapshot's restore already made is kept whole. A fresh default session with
/// a saved session's name takes that session's windows after its own; any other session in
/// use leaves the saved one to be created beside it. Windows planned for an existing session
/// are never reused: only windows planned for a created or merged session enter this map.
pub fn layout_creation<'a>(
    snapshot: &'a WorkspaceSnapshot,
    live: &[LiveSession],
) -> Result<WorkspaceLayoutPlan<'a>, WorkspaceLayoutError> {
    let windows = prepare_layout(snapshot)?;
    let mut created = HashSet::new();
    let mut taken: HashSet<String> = live
        .iter()
        .map(|session| session.name().to_owned())
        .chain(snapshot.sessions.iter().map(|session| session.name.clone()))
        .collect();
    let mut sessions = Vec::with_capacity(snapshot.sessions.len());
    for session in &snapshot.sessions {
        let found = live.iter().find(|found| found.name() == session.name);
        let marker = restored_marker(snapshot, session);
        let action = if live.iter().any(
            |found| matches!(found, LiveSession::Restored { marker: made, .. } if *made == marker),
        ) {
            WorkspaceLayoutSessionAction::SkipExisting
        } else if let Some(LiveSession::Fresh { name, window_index }) = found {
            let links = session
                .windows
                .iter()
                .enumerate()
                .map(|(position, link)| {
                    let window = windows
                        .iter()
                        .position(|window| window.window.id == link.window)
                        .ok_or(WorkspaceLayoutError::Topology)?;
                    Ok(WorkspaceLayoutLink {
                        // After the live window, in the saved order; never its index.
                        index: window_index
                            .checked_add(1 + position as u64)
                            .filter(|index| *index <= i32::MAX as u64)
                            .ok_or(WorkspaceLayoutError::Limit)?,
                        window,
                        create: created.insert(&link.window),
                        active: link.active,
                    })
                })
                .collect::<Result<Vec<_>, WorkspaceLayoutError>>()?;
            WorkspaceLayoutSessionAction::Merge {
                name: name.clone(),
                links,
            }
        } else {
            let name = if found.is_some() {
                let name = beside(&session.name, &taken);
                taken.insert(name.clone());
                name
            } else {
                session.name.clone()
            };
            let seed_window = session
                .windows
                .iter()
                .find(|link| !created.contains(&link.window))
                .map(|link| {
                    windows
                        .iter()
                        .position(|window| window.window.id == link.window)
                        .ok_or(WorkspaceLayoutError::Topology)
                })
                .transpose()?;
            let bootstrap_index = if seed_window.is_none() {
                Some(
                    (0..=MAX_PANES as u64)
                        .find(|index| !session.windows.iter().any(|link| link.index == *index))
                        .ok_or(WorkspaceLayoutError::Limit)?,
                )
            } else {
                None
            };
            let links = session
                .windows
                .iter()
                .map(|link| {
                    let window = windows
                        .iter()
                        .position(|window| window.window.id == link.window)
                        .ok_or(WorkspaceLayoutError::Topology)?;
                    Ok(WorkspaceLayoutLink {
                        index: link.index,
                        window,
                        create: created.insert(&link.window),
                        active: link.active,
                    })
                })
                .collect::<Result<Vec<_>, WorkspaceLayoutError>>()?;
            WorkspaceLayoutSessionAction::Create {
                name,
                seed_window,
                bootstrap_index,
                links,
            }
        };
        sessions.push(WorkspaceLayoutSession { session, action });
    }
    Ok(WorkspaceLayoutPlan { windows, sessions })
}

impl WorkspaceLayoutCell {
    pub fn leaf_ids(&self) -> Vec<u32> {
        let mut ids = Vec::new();
        self.append_leaf_ids(&mut ids);
        ids
    }

    fn append_leaf_ids(&self, ids: &mut Vec<u32>) {
        match &self.kind {
            WorkspaceLayoutKind::Pane(id) => ids.push(*id),
            WorkspaceLayoutKind::Split(_, children) => {
                for child in children {
                    child.append_leaf_ids(ids);
                }
            }
        }
    }
}

/// Old IDs of the panes the snapshot's layouts say floated. A layout that does not
/// parse names none here; restore refuses it on its own, so a preview never guesses.
pub fn floating_pane_ids(snapshot: &WorkspaceSnapshot) -> HashSet<String> {
    snapshot
        .windows
        .iter()
        .filter_map(|window| parse_saved_layout(&window.layout).ok())
        .flat_map(|layout| layout.floating)
        .map(|id| format!("%{id}"))
        .collect()
}

/// A checksum-valid layout alone is insufficient: it must describe exactly the
/// recorded panes. Zoomed pane geometry belongs to the visible layout, not the
/// saved full tree, so correspondence uses old IDs rather than screen positions.
pub fn prepare_layout(
    snapshot: &WorkspaceSnapshot,
) -> Result<Vec<WorkspaceLayoutWindow<'_>>, WorkspaceLayoutError> {
    use WorkspaceLayoutError::Topology;
    if !snapshot.is_consistent()
        || snapshot.panes.len() > MAX_PANES
        || snapshot.windows.len() > MAX_PANES
        || snapshot.sessions.len() > MAX_PANES
        || snapshot.sessions.iter().any(|session| {
            session.name.is_empty()
                || session.name.contains(['.', ':'])
                || session.name.chars().any(char::is_control)
                || session
                    .windows
                    .iter()
                    .any(|link| link.index > i32::MAX as u64)
                || session
                    .windows
                    .iter()
                    .map(|link| &link.window)
                    .collect::<HashSet<_>>()
                    .len()
                    != session.windows.len()
        })
        || snapshot
            .sessions
            .iter()
            .map(|value| &value.name)
            .collect::<HashSet<_>>()
            .len()
            != snapshot.sessions.len()
        || snapshot
            .panes
            .iter()
            .any(|pane| !pane.cwd.starts_with('/') || pane.cwd.contains('\0'))
    {
        return Err(Topology);
    }
    snapshot
        .windows
        .iter()
        .map(|window| {
            if window.name.contains('\0') {
                return Err(Topology);
            }
            if !snapshot
                .sessions
                .iter()
                .any(|session| session.windows.iter().any(|link| link.window == window.id))
            {
                return Err(Topology);
            }
            let SavedLayout {
                tiled: root,
                floating,
            } = parse_saved_layout(&window.layout)?;
            if u64::from(root.width) != window.width || u64::from(root.height) != window.height {
                return Err(WorkspaceLayoutError::Geometry);
            }
            let in_window = |id: &u32| {
                snapshot
                    .panes
                    .iter()
                    .find(|pane| pane.window == window.id && pane.id == format!("%{id}"))
                    .ok_or(Topology)
            };
            let panes = root
                .leaf_ids()
                .iter()
                .map(in_window)
                .collect::<Result<Vec<_>, _>>()?;
            let floating_panes = floating
                .iter()
                .map(in_window)
                .collect::<Result<Vec<_>, _>>()?;
            if snapshot
                .panes
                .iter()
                .filter(|pane| pane.window == window.id)
                .count()
                != panes.len() + floating_panes.len()
            {
                return Err(Topology);
            }
            let SavedLayout {
                tiled: visible,
                floating: visible_floating,
            } = parse_saved_layout(&window.visible_layout)?;
            // A zoom hides every floating pane except the ones made to stay above it.
            if visible_floating.iter().any(|id| !floating.contains(id)) {
                return Err(Topology);
            }
            let zoomed = window.visible_layout != window.layout;
            if zoomed
                && (visible.leaf_ids().len() != 1
                    || format!("%{}", visible.leaf_ids()[0]) != window.active_pane
                    || visible.width != root.width
                    || visible.height != root.height)
            {
                return Err(Topology);
            }
            // Physical pane geometry omits the top/bottom status row. Recover
            // only the window-local option needed to preserve recorded sizes.
            let border_status = ["off", "top", "bottom"]
                .into_iter()
                .find(|status| {
                    panes.iter().all(|pane| {
                        let tree = if zoomed && pane.id == window.active_pane {
                            &visible
                        } else {
                            &root
                        };
                        let Some(cell) = leaf_cell(
                            tree,
                            pane.id
                                .strip_prefix('%')
                                .and_then(|id| id.parse().ok())
                                .unwrap_or(u32::MAX),
                        ) else {
                            return false;
                        };
                        let top = *status == "top" && cell.top == 0;
                        let bottom = *status == "bottom"
                            && u32::from(cell.top) + u32::from(cell.height)
                                == u32::from(tree.height);
                        pane.left == u64::from(cell.left)
                            && pane.width == u64::from(cell.width)
                            && pane.top == u64::from(cell.top) + u64::from(top)
                            && pane.height.checked_add(u64::from(top || bottom))
                                == Some(u64::from(cell.height))
                    })
                })
                .ok_or(WorkspaceLayoutError::Geometry)?;
            let mut splits = Vec::with_capacity(panes.len().saturating_sub(1));
            append_splits(&root, &mut splits)?;
            Ok(WorkspaceLayoutWindow {
                window,
                root,
                panes,
                floating: floating_panes,
                zoomed,
                border_status,
                splits,
            })
        })
        .collect()
}

/// A saved layout as restore sees it: the tiled tree it can recreate, and the
/// panes that floated above it, which it cannot.
pub struct SavedLayout {
    pub tiled: WorkspaceLayoutCell,
    /// Old pane IDs, frontmost first.
    pub floating: Vec<u32>,
}

/// A live layout: tiled only. A floating cell is refused here.
/// Saved pane IDs describe the old layout, never select a current tmux target.
pub fn parse_layout(value: &str) -> Result<WorkspaceLayoutCell, WorkspaceLayoutError> {
    parse_with(value, false).map(|layout| layout.tiled)
}

/// A saved layout (#4405): the same strict grammar, plus tmux 3.8 floating cells
/// directly under the root, which are set aside and reported, never admitted into
/// the tiled model.
pub fn parse_saved_layout(value: &str) -> Result<SavedLayout, WorkspaceLayoutError> {
    parse_with(value, true)
}

fn parse_with(value: &str, floating: bool) -> Result<SavedLayout, WorkspaceLayoutError> {
    use WorkspaceLayoutError::{Checksum, Limit, Syntax};
    let bytes = value.as_bytes();
    if bytes.len() > MAX_LAYOUT_BYTES {
        return Err(Limit);
    }
    if value
        .trim_start_matches([' ', '\t', '\r', '\n'])
        .starts_with('{')
    {
        let (root, floating) = json::parse(value, floating)?;
        validate_root(&root)?;
        return Ok(SavedLayout {
            tiled: root,
            floating,
        });
    }
    if bytes.len() < 6 || bytes[4] != b',' || !bytes[..4].iter().all(u8::is_ascii_hexdigit) {
        return Err(Syntax);
    }
    let expected = u16::from_str_radix(&value[..4], 16).map_err(|_| Syntax)?;
    let body = &bytes[5..];
    let checksum = body.iter().fold(0u16, |sum, byte| {
        sum.rotate_right(1).wrapping_add(u16::from(*byte))
    });
    if expected != checksum {
        return Err(Checksum);
    }
    let mut parser = LayoutParser {
        bytes: body,
        offset: 0,
        panes: HashSet::new(),
    };
    let root = parser.cell(0)?;
    if parser.offset != body.len() {
        return Err(Syntax);
    }
    validate_root(&root)?;
    Ok(SavedLayout {
        tiled: root,
        floating: Vec::new(),
    })
}

fn validate_root(root: &WorkspaceLayoutCell) -> Result<(), WorkspaceLayoutError> {
    if root.left != 0 || root.top != 0 {
        return Err(WorkspaceLayoutError::Geometry);
    }
    validate_cell(root)
}

fn validate_cell(cell: &WorkspaceLayoutCell) -> Result<(), WorkspaceLayoutError> {
    use WorkspaceLayoutError::Geometry;
    if cell.width == 0 || cell.height == 0 {
        return Err(Geometry);
    }
    let WorkspaceLayoutKind::Split(axis, children) = &cell.kind else {
        return Ok(());
    };
    if children.len() < 2 {
        return Err(Geometry);
    }
    let mut position = match axis {
        WorkspaceSplitAxis::Horizontal => u32::from(cell.left),
        WorkspaceSplitAxis::Vertical => u32::from(cell.top),
    };
    for child in children {
        validate_cell(child)?;
        let extent = match axis {
            WorkspaceSplitAxis::Horizontal => {
                if child.height != cell.height
                    || child.top != cell.top
                    || u32::from(child.left) != position
                {
                    return Err(Geometry);
                }
                child.width
            }
            WorkspaceSplitAxis::Vertical => {
                if child.width != cell.width
                    || child.left != cell.left
                    || u32::from(child.top) != position
                {
                    return Err(Geometry);
                }
                child.height
            }
        };
        position += u32::from(extent) + 1;
    }
    let expected = match axis {
        WorkspaceSplitAxis::Horizontal => u32::from(cell.left) + u32::from(cell.width),
        WorkspaceSplitAxis::Vertical => u32::from(cell.top) + u32::from(cell.height),
    };
    if position - 1 != expected {
        return Err(Geometry);
    }
    Ok(())
}

impl WorkspaceLayoutWindow<'_> {
    /// Adapt admitted saved geometry to a fenced, invocation-owned window's
    /// observed encoding. Preserve original bytes only when tmux's assignment
    /// order agrees with the invocation's tree-ordered native creation.
    pub fn layout_for_server(&self, live: &str) -> Result<String, WorkspaceLayoutError> {
        parse_layout(live)?;
        let is_json = |value: &str| {
            value
                .trim_start_matches([' ', '\t', '\r', '\n'])
                .starts_with('{')
        };
        let live_json = is_json(live);
        // The saved bytes name floating cells the live window does not have.
        if live_json == is_json(&self.window.layout)
            && self.floating.is_empty()
            && (!live_json || json::creation_ordered(&self.window.layout)?)
        {
            return Ok(self.window.layout.clone());
        }
        let result = if live_json {
            json::serialize(&self.root)
        } else {
            let mut body = String::new();
            legacy_body(&self.root, &mut body);
            let checksum = body.bytes().fold(0u16, |sum, byte| {
                sum.rotate_right(1).wrapping_add(u16::from(byte))
            });
            format!("{checksum:04x},{body}")
        };
        // Conversion never widens the admitted wire bound.
        parse_layout(&result)?;
        Ok(result)
    }
}

fn legacy_body(cell: &WorkspaceLayoutCell, out: &mut String) {
    use std::fmt::Write;
    write!(
        out,
        "{}x{},{},{}",
        cell.width, cell.height, cell.left, cell.top
    )
    .unwrap();
    match &cell.kind {
        WorkspaceLayoutKind::Pane(id) => write!(out, ",{id}").unwrap(),
        WorkspaceLayoutKind::Split(axis, children) => {
            let (open, close) = match axis {
                WorkspaceSplitAxis::Horizontal => ('{', '}'),
                WorkspaceSplitAxis::Vertical => ('[', ']'),
            };
            out.push(open);
            for (i, child) in children.iter().enumerate() {
                if i != 0 {
                    out.push(',');
                }
                legacy_body(child, out);
            }
            out.push(close);
        }
    }
}

struct LayoutParser<'a> {
    bytes: &'a [u8],
    offset: usize,
    panes: HashSet<u32>,
}

impl LayoutParser<'_> {
    fn number(&mut self) -> Result<u32, WorkspaceLayoutError> {
        let start = self.offset;
        let mut value = 0u32;
        while let Some(byte) = self
            .bytes
            .get(self.offset)
            .filter(|byte| byte.is_ascii_digit())
        {
            value = value
                .checked_mul(10)
                .and_then(|value| value.checked_add(u32::from(*byte - b'0')))
                .ok_or(WorkspaceLayoutError::Syntax)?;
            self.offset += 1;
        }
        if self.offset == start {
            return Err(WorkspaceLayoutError::Syntax);
        }
        Ok(value)
    }

    fn take(&mut self, byte: u8) -> Result<(), WorkspaceLayoutError> {
        if self.bytes.get(self.offset) != Some(&byte) {
            return Err(WorkspaceLayoutError::Syntax);
        }
        self.offset += 1;
        Ok(())
    }

    fn dimension(&mut self) -> Result<u16, WorkspaceLayoutError> {
        u16::try_from(self.number()?).map_err(|_| WorkspaceLayoutError::Geometry)
    }

    fn cell(&mut self, depth: usize) -> Result<WorkspaceLayoutCell, WorkspaceLayoutError> {
        use WorkspaceLayoutError::{DuplicatePane, Geometry, Limit, Syntax};
        if depth >= MAX_DEPTH {
            return Err(Limit);
        }
        let width = self.dimension()?;
        self.take(b'x')?;
        let height = self.dimension()?;
        self.take(b',')?;
        let left = self.dimension()?;
        self.take(b',')?;
        let top = self.dimension()?;
        if width == 0 || height == 0 {
            return Err(Geometry);
        }
        let kind = match self.bytes.get(self.offset) {
            Some(b',') => {
                self.offset += 1;
                let pane = self.number()?;
                if !self.panes.insert(pane) {
                    return Err(DuplicatePane);
                }
                if self.panes.len() > MAX_PANES {
                    return Err(Limit);
                }
                WorkspaceLayoutKind::Pane(pane)
            }
            Some(open @ (b'{' | b'[')) => {
                let (axis, close) = if *open == b'{' {
                    (WorkspaceSplitAxis::Horizontal, b'}')
                } else {
                    (WorkspaceSplitAxis::Vertical, b']')
                };
                self.offset += 1;
                let mut children = vec![self.cell(depth + 1)?];
                while self.bytes.get(self.offset) == Some(&b',') {
                    self.offset += 1;
                    children.push(self.cell(depth + 1)?);
                }
                self.take(close)?;
                WorkspaceLayoutKind::Split(axis, children)
            }
            _ => return Err(Syntax),
        };
        Ok(WorkspaceLayoutCell {
            width,
            height,
            left,
            top,
            kind,
        })
    }
}

/// Carve the next sibling from the remaining region, then finish the previous
/// subtree. tmux inserts each new pane after its target; this ordering leaves
/// the final native pane list in the saved tree's depth-first leaf order.
fn append_splits(
    cell: &WorkspaceLayoutCell,
    splits: &mut Vec<WorkspaceLayoutSplit>,
) -> Result<u32, WorkspaceLayoutError> {
    match &cell.kind {
        WorkspaceLayoutKind::Pane(id) => Ok(*id),
        WorkspaceLayoutKind::Split(axis, children) => {
            let first_id = children
                .first()
                .and_then(first_leaf)
                .ok_or(WorkspaceLayoutError::Geometry)?;
            let mut remaining = match axis {
                WorkspaceSplitAxis::Horizontal => cell.width,
                WorkspaceSplitAxis::Vertical => cell.height,
            };
            for pair in children.windows(2) {
                let completed = &pair[0];
                let next = &pair[1];
                let extent = match axis {
                    WorkspaceSplitAxis::Horizontal => completed.width,
                    WorkspaceSplitAxis::Vertical => completed.height,
                };
                remaining = remaining
                    .checked_sub(extent)
                    .and_then(|value| value.checked_sub(1))
                    .filter(|value| *value > 0)
                    .ok_or(WorkspaceLayoutError::Geometry)?;
                splits.push(WorkspaceLayoutSplit {
                    target_leaf: first_leaf(completed).ok_or(WorkspaceLayoutError::Geometry)?,
                    new_leaf: first_leaf(next).ok_or(WorkspaceLayoutError::Geometry)?,
                    axis: *axis,
                    remaining_extent: remaining,
                });
                append_splits(completed, splits)?;
            }
            append_splits(
                children.last().ok_or(WorkspaceLayoutError::Geometry)?,
                splits,
            )?;
            Ok(first_id)
        }
    }
}

fn first_leaf(cell: &WorkspaceLayoutCell) -> Option<u32> {
    match &cell.kind {
        WorkspaceLayoutKind::Pane(id) => Some(*id),
        WorkspaceLayoutKind::Split(_, children) => children.first().and_then(first_leaf),
    }
}

#[cfg(test)]
mod tests;

fn leaf_cell(cell: &WorkspaceLayoutCell, id: u32) -> Option<&WorkspaceLayoutCell> {
    match &cell.kind {
        WorkspaceLayoutKind::Pane(found) if *found == id => Some(cell),
        WorkspaceLayoutKind::Pane(_) => None,
        WorkspaceLayoutKind::Split(_, children) => {
            children.iter().find_map(|child| leaf_cell(child, id))
        }
    }
}

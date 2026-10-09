//! Validate saved tmux geometry before any restore effect is permitted.

use std::collections::HashSet;

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
    pub root: WorkspaceLayoutCell,
    pub panes: Vec<&'a WorkspacePane>,
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
    SkipExisting,
    Create {
        /// Use a real, not-yet-created window for new-session whenever possible.
        seed_window: Option<usize>,
        /// Only an all-shared session needs an invocation-owned bootstrap pane.
        bootstrap_index: Option<u64>,
        links: Vec<WorkspaceLayoutLink>,
    },
}

pub struct WorkspaceLayoutLink {
    pub index: u64,
    pub window: usize,
    pub create: bool,
    pub active: bool,
}

/// Existing names are skipped as whole sessions. Their saved windows are never
/// reused: only windows planned for a newly created session enter this map.
pub fn layout_creation<'a>(
    snapshot: &'a WorkspaceSnapshot,
    current_sessions: &[String],
) -> Result<WorkspaceLayoutPlan<'a>, WorkspaceLayoutError> {
    let windows = prepare_layout(snapshot)?;
    let mut created = HashSet::new();
    let mut sessions = Vec::with_capacity(snapshot.sessions.len());
    for session in &snapshot.sessions {
        let action = if current_sessions.contains(&session.name) {
            WorkspaceLayoutSessionAction::SkipExisting
        } else {
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
            let root = parse_layout(&window.layout)?;
            if u64::from(root.width) != window.width || u64::from(root.height) != window.height {
                return Err(WorkspaceLayoutError::Geometry);
            }
            let ids = root.leaf_ids();
            let panes = ids
                .iter()
                .map(|id| {
                    snapshot
                        .panes
                        .iter()
                        .find(|pane| pane.window == window.id && pane.id == format!("%{id}"))
                        .ok_or(Topology)
                })
                .collect::<Result<Vec<_>, _>>()?;
            if snapshot
                .panes
                .iter()
                .filter(|pane| pane.window == window.id)
                .count()
                != panes.len()
            {
                return Err(Topology);
            }
            let visible = parse_layout(&window.visible_layout)?;
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
                zoomed,
                border_status,
                splits,
            })
        })
        .collect()
}

/// Saved pane IDs describe the old layout, never select a current tmux target.
pub fn parse_layout(value: &str) -> Result<WorkspaceLayoutCell, WorkspaceLayoutError> {
    use WorkspaceLayoutError::{Checksum, Geometry, Limit, Syntax};
    let bytes = value.as_bytes();
    if bytes.len() > MAX_LAYOUT_BYTES {
        return Err(Limit);
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
    if root.left != 0 || root.top != 0 {
        return Err(Geometry);
    }
    Ok(root)
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
                if children.len() < 2 {
                    return Err(Geometry);
                }
                let mut position = match axis {
                    WorkspaceSplitAxis::Horizontal => u32::from(left),
                    WorkspaceSplitAxis::Vertical => u32::from(top),
                };
                for child in &children {
                    let extent = match axis {
                        WorkspaceSplitAxis::Horizontal => {
                            if child.height != height
                                || child.top != top
                                || u32::from(child.left) != position
                            {
                                return Err(Geometry);
                            }
                            child.width
                        }
                        WorkspaceSplitAxis::Vertical => {
                            if child.width != width
                                || child.left != left
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
                    WorkspaceSplitAxis::Horizontal => u32::from(left) + u32::from(width),
                    WorkspaceSplitAxis::Vertical => u32::from(top) + u32::from(height),
                };
                if position - 1 != expected {
                    return Err(Geometry);
                }
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

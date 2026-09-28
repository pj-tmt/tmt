//! Fixed room slots project into the existing topology/admission boundary.
//! Material and resource bindings never change generated circulation.

use super::{
    Area, AreaKind, Axis, COORDINATE_LIMIT, Edge, FloorSpan, MAX_AREAS, MapDraft, MapError,
};
use std::collections::{BTreeMap, BTreeSet};

mod central_grid;

pub const ROOM_WIDTH: i32 = 48;
pub const ROOM_HEIGHT: i32 = 40;
pub const PASSAGE_WIDTH: i32 = 8;
pub const COLUMN_STEP: i32 = ROOM_WIDTH + PASSAGE_WIDTH;
pub const ROW_STEP: i32 = ROOM_HEIGHT + PASSAGE_WIDTH;
pub const LOBBY_WIDTH: i32 = ROOM_WIDTH * 2 + PASSAGE_WIDTH;
pub const LOBBY_HEIGHT: i32 = ROOM_HEIGHT * 2 + PASSAGE_WIDTH;
pub const MEETING_X: i32 = LOBBY_WIDTH + PASSAGE_WIDTH * 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Material {
    Workshop,
    Moonlight,
    Copper,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Lobby,
    Office { column: i32, row: i32 },
    Meeting { index: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Module {
    pub area: Area,
    pub slot: Slot,
    pub material: Material,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleDraft {
    pub primary_lobby_id: String,
    pub modules: Vec<Module>,
    pub layout: ModuleLayout,
}

/// Persisted projection semantics: existing saved wall mounts must remain readable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleLayout {
    ShortLinks,
    Grid,
    CentralGrid,
    CompactGrid,
    Skybridges,
    IndependentMeetings,
    UnifiedAreas,
}

impl ModuleLayout {
    fn central(self) -> bool {
        matches!(
            self,
            Self::CentralGrid
                | Self::CompactGrid
                | Self::Skybridges
                | Self::IndependentMeetings
                | Self::UnifiedAreas
        )
    }

    fn platforms(self) -> bool {
        matches!(
            self,
            Self::Skybridges | Self::IndependentMeetings | Self::UnifiedAreas
        )
    }
}

/// Physical floor extents, shared by projection and prospective-slot validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModuleRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl ModuleRect {
    fn right(self) -> i32 {
        self.x + self.width
    }
    fn bottom(self) -> i32 {
        self.y + self.height
    }
    fn overlaps(self, other: Self) -> bool {
        self.x < other.right()
            && self.right() > other.x
            && self.y < other.bottom()
            && self.bottom() > other.y
    }
    fn checked(self) -> Result<Self, MapError> {
        if self.x < -COORDINATE_LIMIT
            || self.y < -COORDINATE_LIMIT
            || self.right() > COORDINATE_LIMIT
            || self.bottom() > COORDINATE_LIMIT
        {
            return Err(MapError::InvalidModule);
        }
        Ok(self)
    }
}

impl Module {
    pub fn bounds(&self, layout: ModuleLayout) -> Result<ModuleRect, MapError> {
        let invalid = || MapError::InvalidModule;
        if layout == ModuleLayout::UnifiedAreas && matches!(self.slot, Slot::Meeting { .. }) {
            return Err(MapError::InvalidModule);
        }
        let (x, y, width) = match (&self.area.kind, self.slot) {
            (AreaKind::Lobby, Slot::Lobby) => (0, 0, LOBBY_WIDTH),
            (AreaKind::Personal { .. }, Slot::Office { column, row }) => (
                column.checked_mul(COLUMN_STEP).ok_or_else(invalid)?,
                row.checked_mul(ROW_STEP).ok_or_else(invalid)?,
                ROOM_WIDTH,
            ),
            (AreaKind::Meeting { .. }, Slot::Office { column, row })
                if layout == ModuleLayout::UnifiedAreas =>
            {
                (
                    column.checked_mul(COLUMN_STEP).ok_or_else(invalid)?,
                    row.checked_mul(ROW_STEP).ok_or_else(invalid)?,
                    ROOM_WIDTH,
                )
            }
            (AreaKind::Meeting { .. }, Slot::Meeting { index }) => (
                MEETING_X
                    + if layout.platforms() {
                        2 * PASSAGE_WIDTH
                    } else {
                        0
                    },
                i32::try_from(index)
                    .ok()
                    .and_then(|index| {
                        index.checked_mul(if layout == ModuleLayout::CompactGrid {
                            ROOM_HEIGHT
                        } else {
                            ROW_STEP
                        })
                    })
                    .ok_or_else(invalid)?,
                ROOM_WIDTH,
            ),
            _ => return Err(MapError::InvalidModule),
        };
        // Check before addition so even hostile extreme slot indices cannot overflow.
        if !(-COORDINATE_LIMIT..COORDINATE_LIMIT).contains(&x)
            || !(-COORDINATE_LIMIT..COORDINATE_LIMIT).contains(&y)
        {
            return Err(MapError::InvalidModule);
        }
        ModuleRect {
            x,
            y,
            width,
            height: if self.slot == Slot::Lobby && layout.central() {
                LOBBY_HEIGHT
            } else {
                ROOM_HEIGHT
            },
        }
        .checked()
    }
}

impl ModuleDraft {
    pub(super) fn project(&self) -> Result<MapDraft, MapError> {
        if self.modules.len() > MAX_AREAS {
            return Err(MapError::LimitExceeded);
        }
        let bounds = self
            .modules
            .iter()
            .map(|module| module.bounds(self.layout))
            .collect::<Result<Vec<_>, _>>()?;
        let lobby = self
            .modules
            .iter()
            .filter(|module| matches!(module.slot, Slot::Lobby))
            .collect::<Vec<_>>();
        if lobby.len() != 1 || lobby[0].area.id != self.primary_lobby_id {
            return Err(MapError::PrimaryLobbyRequired);
        }
        let reserved = ModuleRect {
            x: MEETING_X - PASSAGE_WIDTH,
            y: 0,
            width: ROOM_WIDTH
                + PASSAGE_WIDTH
                + if self.layout.platforms() {
                    2 * PASSAGE_WIDTH
                } else {
                    0
                },
            height: COORDINATE_LIMIT,
        };
        for (index, module) in self.modules.iter().enumerate() {
            if self.layout != ModuleLayout::UnifiedAreas
                && matches!(module.slot, Slot::Office { .. })
                && bounds[index].overlaps(reserved)
            {
                return Err(MapError::ModuleCollision);
            }
            if bounds[..index]
                .iter()
                .any(|other| bounds[index].overlaps(*other))
            {
                return Err(MapError::ModuleCollision);
            }
        }
        let mut floor = Vec::new();
        let mut circulation = Circulation::default();
        for (index, module) in self.modules.iter().enumerate() {
            let rect = bounds[index];
            for y in rect.y..rect.bottom() {
                floor.push(FloorSpan {
                    y,
                    start: rect.x,
                    end: rect.right(),
                    area_id: Some(module.area.id.clone()),
                });
            }
            if matches!(module.slot, Slot::Meeting { .. }) || self.layout.central() {
                continue;
            }
            for (other_index, other) in self.modules[..index].iter().enumerate() {
                if !matches!(other.slot, Slot::Meeting { .. }) {
                    circulation.connect(rect, bounds[other_index], self.layout);
                }
            }
        }
        if self.layout == ModuleLayout::UnifiedAreas {
            for (index, rect) in bounds.iter().enumerate() {
                for other in &bounds[..index] {
                    circulation.connect(*rect, *other, self.layout);
                }
            }
        } else if self.layout.central() {
            let main = self
                .modules
                .iter()
                .zip(&bounds)
                .filter(|(module, _)| !matches!(module.slot, Slot::Meeting { .. }))
                .map(|(_, rect)| *rect)
                .collect::<Vec<_>>();
            central_grid::project(
                &mut circulation,
                &main,
                reserved,
                matches!(
                    self.layout,
                    ModuleLayout::CompactGrid
                        | ModuleLayout::Skybridges
                        | ModuleLayout::IndependentMeetings
                ),
                self.layout.platforms(),
            )?;
        }
        if self.layout == ModuleLayout::Grid {
            let main = self
                .modules
                .iter()
                .zip(&bounds)
                .filter(|(module, _)| !matches!(module.slot, Slot::Meeting { .. }))
                .map(|(_, rect)| *rect)
                .collect::<Vec<_>>();
            circulation.junctions(&main);
        }
        let meetings = self
            .modules
            .iter()
            .zip(&bounds)
            .filter(|(module, _)| matches!(module.slot, Slot::Meeting { .. }))
            .map(|(_, rect)| *rect)
            .collect::<Vec<_>>();
        if let Some(last) = meetings.iter().map(|rect| rect.y).max()
            && self.layout != ModuleLayout::IndependentMeetings
        {
            let room_door_y = (ROOM_HEIGHT - PASSAGE_WIDTH) / 2;
            let start_y = if self.layout.central() {
                (LOBBY_HEIGHT - PASSAGE_WIDTH) / 2
            } else {
                room_door_y
            };
            let first_y = meetings
                .iter()
                .map(|rect| rect.y + room_door_y)
                .min()
                .unwrap();
            let spine_y = start_y.min(first_y);
            // A stable common spine reaches every occupied slot, including gaps.
            // Removing a meeting never repacks another meeting or moves its floor.
            circulation.rect(ModuleRect {
                x: LOBBY_WIDTH,
                y: start_y,
                width: MEETING_X - LOBBY_WIDTH,
                height: PASSAGE_WIDTH,
            });
            circulation.rect(ModuleRect {
                x: MEETING_X - PASSAGE_WIDTH,
                y: spine_y,
                width: PASSAGE_WIDTH,
                height: start_y.max(last + room_door_y) + PASSAGE_WIDTH - spine_y,
            });
            circulation.door(LOBBY_WIDTH, start_y, Axis::Vertical);
            for rect in meetings {
                if self.layout == ModuleLayout::Skybridges {
                    circulation.rect(ModuleRect {
                        x: MEETING_X,
                        y: rect.y + room_door_y,
                        width: rect.x - MEETING_X,
                        height: PASSAGE_WIDTH,
                    });
                }
                circulation.door(rect.x, rect.y + room_door_y, Axis::Vertical);
            }
        }
        floor.extend(circulation.floor());
        Ok(MapDraft {
            primary_lobby_id: self.primary_lobby_id.clone(),
            areas: self
                .modules
                .iter()
                .map(|module| module.area.clone())
                .collect(),
            floor,
            doors: circulation.doors.into_iter().collect(),
        })
    }
}

/// Union only derived common floor, never module interiors or arbitrary tile input.
#[derive(Default)]
struct Circulation {
    rows: BTreeMap<i32, Vec<(i32, i32)>>,
    doors: BTreeSet<Edge>,
}

impl Circulation {
    fn rect(&mut self, rect: ModuleRect) {
        for y in rect.y..rect.bottom() {
            self.rows.entry(y).or_default().push((rect.x, rect.right()));
        }
    }
    fn door(&mut self, x: i32, y: i32, axis: Axis) {
        for offset in 0..PASSAGE_WIDTH {
            self.doors.insert(Edge {
                x: x + if axis == Axis::Horizontal { offset } else { 0 },
                y: y + if axis == Axis::Vertical { offset } else { 0 },
                axis,
            });
        }
    }
    fn connect(&mut self, a: ModuleRect, b: ModuleRect, layout: ModuleLayout) {
        let (left, right) = if a.x < b.x { (a, b) } else { (b, a) };
        let overlap_y = left.bottom().min(right.bottom()) - left.y.max(right.y);
        if right.x - left.right() == PASSAGE_WIDTH && overlap_y >= PASSAGE_WIDTH {
            let y = left.y.max(right.y) + (overlap_y - PASSAGE_WIDTH) / 2;
            self.rect(ModuleRect {
                x: left.right(),
                y: if layout == ModuleLayout::Grid {
                    left.y.max(right.y)
                } else {
                    y
                },
                width: PASSAGE_WIDTH,
                height: if layout == ModuleLayout::Grid {
                    overlap_y
                } else {
                    PASSAGE_WIDTH
                },
            });
            self.door(left.right(), y, Axis::Vertical);
            self.door(right.x, y, Axis::Vertical);
        }
        let (above, below) = if a.y < b.y { (a, b) } else { (b, a) };
        let overlap_x = above.right().min(below.right()) - above.x.max(below.x);
        if below.y - above.bottom() == PASSAGE_WIDTH && overlap_x >= PASSAGE_WIDTH {
            let x = above.x.max(below.x) + (overlap_x - PASSAGE_WIDTH) / 2;
            self.rect(ModuleRect {
                x: if layout == ModuleLayout::Grid {
                    above.x.max(below.x)
                } else {
                    x
                },
                y: above.bottom(),
                width: if layout == ModuleLayout::Grid {
                    overlap_x
                } else {
                    PASSAGE_WIDTH
                },
                height: PASSAGE_WIDTH,
            });
            self.door(x, above.bottom(), Axis::Horizontal);
            self.door(x, below.y, Axis::Horizontal);
        }
    }
    fn contains(&self, x: i32, y: i32) -> bool {
        self.rows
            .get(&y)
            .is_some_and(|ranges| ranges.iter().any(|&(start, end)| start <= x && x < end))
    }
    fn junctions(&mut self, rooms: &[ModuleRect]) {
        let mut candidates = BTreeSet::new();
        for room in rooms {
            for x in [room.x - PASSAGE_WIDTH, room.right()] {
                for y in [room.y - PASSAGE_WIDTH, room.bottom()] {
                    candidates.insert((x, y));
                }
            }
        }
        // Inspect existing lane arms before adding any junction: corner fill
        // cannot recursively invent access across empty or diagonal-only cells.
        let joins = candidates
            .into_iter()
            .filter(|&(x, y)| {
                let arms = [
                    (0..PASSAGE_WIDTH).all(|i| self.contains(x + i, y - 1)),
                    (0..PASSAGE_WIDTH).all(|i| self.contains(x + i, y + PASSAGE_WIDTH)),
                    (0..PASSAGE_WIDTH).all(|i| self.contains(x - 1, y + i)),
                    (0..PASSAGE_WIDTH).all(|i| self.contains(x + PASSAGE_WIDTH, y + i)),
                ];
                arms.into_iter().filter(|present| *present).count() >= 2
            })
            .collect::<Vec<_>>();
        for (x, y) in joins {
            self.rect(ModuleRect {
                x,
                y,
                width: PASSAGE_WIDTH,
                height: PASSAGE_WIDTH,
            });
        }
    }
    fn floor(&mut self) -> Vec<FloorSpan> {
        let mut floor: Vec<FloorSpan> = Vec::new();
        for (&y, ranges) in &mut self.rows {
            ranges.sort_unstable();
            for &(start, end) in ranges.iter() {
                if let Some(last) = floor.last_mut()
                    && last.y == y
                    && start <= last.end
                {
                    last.end = last.end.max(end);
                } else {
                    floor.push(FloorSpan {
                        y,
                        start,
                        end,
                        area_id: None,
                    });
                }
            }
        }
        floor
    }
}

#[cfg(test)]
mod tests;

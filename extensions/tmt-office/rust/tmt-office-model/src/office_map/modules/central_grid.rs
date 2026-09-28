//! V6 cardinal neighbor links; retained layouts use the public perimeter lattice.

use super::*;
use crate::office_map::{MAX_SPANS, MAX_TILES};

pub(super) fn project(
    circulation: &mut Circulation,
    rooms: &[ModuleRect],
    reserved: ModuleRect,
    compact: bool,
    bridges: bool,
) -> Result<(), MapError> {
    if bridges {
        neighbor_bridges(circulation, rooms);
        return Ok(());
    }
    let left = rooms.iter().map(|rect| rect.x).min().unwrap();
    let right = rooms.iter().map(|rect| rect.right()).max().unwrap();
    let top = rooms.iter().map(|rect| rect.y).min().unwrap();
    let bottom = rooms.iter().map(|rect| rect.bottom()).max().unwrap();
    let lobby = ModuleRect {
        x: 0,
        y: 0,
        width: LOBBY_WIDTH,
        height: LOBBY_HEIGHT,
    };
    let mut tiles = rooms
        .iter()
        .map(|rect| (rect.width * rect.height) as usize)
        .sum::<usize>();
    let mut spans = rooms.iter().map(|rect| rect.height as usize).sum::<usize>();
    let mut required = Vec::new();
    if compact {
        for room in rooms.iter().filter(|room| **room != lobby) {
            let branch_y = if room.y < 0 {
                room.bottom()
            } else if room.y >= LOBBY_HEIGHT {
                room.y - PASSAGE_WIDTH
            } else {
                ROOM_HEIGHT
            };
            let entrance_x = room.x + (room.width - PASSAGE_WIDTH) / 2;
            required.push(ModuleRect {
                x: entrance_x.min(ROOM_WIDTH),
                y: branch_y,
                width: entrance_x.max(ROOM_WIDTH) + PASSAGE_WIDTH - entrance_x.min(ROOM_WIDTH),
                height: PASSAGE_WIDTH,
            });
            required.push(ModuleRect {
                x: ROOM_WIDTH,
                y: branch_y.min(ROOM_HEIGHT),
                width: PASSAGE_WIDTH,
                height: branch_y.max(ROOM_HEIGHT) + PASSAGE_WIDTH - branch_y.min(ROOM_HEIGHT),
            });
        }
    }
    // Generate disjoint row runs, not a dense bounding-box tile allocation. Check
    // the ordinary map budgets while expanding hostile, widely separated slots.
    for y in top..bottom {
        let runs = if compact {
            let mut intervals = required
                .iter()
                .filter(|rect| y >= rect.y && y < rect.bottom())
                .map(|rect| (rect.x, rect.right()))
                .collect::<Vec<_>>();
            intervals.sort_unstable();
            let mut merged: Vec<(i32, i32)> = Vec::new();
            for (start, end) in intervals {
                if let Some(last) = merged.last_mut().filter(|last| start <= last.1) {
                    last.1 = last.1.max(end);
                } else {
                    merged.push((start, end));
                }
            }
            merged
        } else if y.rem_euclid(ROW_STEP) >= ROOM_HEIGHT {
            vec![(left, right)]
        } else {
            (left.div_euclid(COLUMN_STEP)..=right.div_euclid(COLUMN_STEP))
                .filter_map(|column| {
                    let start = (column * COLUMN_STEP + ROOM_WIDTH).max(left);
                    let end = ((column + 1) * COLUMN_STEP).min(right);
                    (start < end).then_some((start, end))
                })
                .collect()
        };
        let mut runs = runs;
        for exclusion in [lobby, reserved] {
            if y < exclusion.y || y >= exclusion.bottom() {
                continue;
            }
            runs = runs
                .into_iter()
                .flat_map(|(start, end)| {
                    let mut remaining = Vec::with_capacity(2);
                    if start < exclusion.x {
                        remaining.push((start, end.min(exclusion.x)));
                    }
                    if end > exclusion.right() {
                        remaining.push((start.max(exclusion.right()), end));
                    }
                    remaining
                })
                .collect();
        }
        for (start, end) in runs {
            tiles += (end - start) as usize;
            spans += 1;
            if tiles > MAX_TILES || spans > MAX_SPANS {
                return Err(MapError::LimitExceeded);
            }
            circulation.rect(ModuleRect {
                x: start,
                y,
                width: end - start,
                height: 1,
            });
        }
    }
    // An opening depends only on adjacent public floor. No opposite room is
    // required, including the four entrances on the Lobby's central axes.
    for room in rooms {
        let x = room.x + (room.width - PASSAGE_WIDTH) / 2;
        let y = room.y + (room.height - PASSAGE_WIDTH) / 2;
        for (edge_x, outside_x) in [(room.x, room.x - 1), (room.right(), room.right())] {
            if (0..PASSAGE_WIDTH).all(|i| circulation.contains(outside_x, y + i)) {
                circulation.door(edge_x, y, Axis::Vertical);
            }
        }
        for (edge_y, outside_y) in [(room.y, room.y - 1), (room.bottom(), room.bottom())] {
            if (0..PASSAGE_WIDTH).all(|i| circulation.contains(x + i, outside_y)) {
                circulation.door(x, edge_y, Axis::Horizontal);
            }
        }
    }
    Ok(())
}

/// Connect each immediate pair once; platform traversal replaces perimeter detours.
fn neighbor_bridges(circulation: &mut Circulation, rooms: &[ModuleRect]) {
    let mut ordered = rooms.to_vec();
    ordered.sort_by_key(|room| (room.y, room.x));
    for (index, a) in ordered.iter().enumerate() {
        for b in &ordered[index + 1..] {
            let (west, east) = if a.x < b.x { (a, b) } else { (b, a) };
            let top = a.y.max(b.y);
            let bottom = a.bottom().min(b.bottom());
            if east.x - west.right() == PASSAGE_WIDTH && bottom - top >= PASSAGE_WIDTH {
                let x = west.right();
                let y = top + (bottom - top - PASSAGE_WIDTH) / 2;
                circulation.rect(ModuleRect {
                    x,
                    y,
                    width: PASSAGE_WIDTH,
                    height: PASSAGE_WIDTH,
                });
                circulation.door(x, y, Axis::Vertical);
                circulation.door(x + PASSAGE_WIDTH, y, Axis::Vertical);
            }
            let left = a.x.max(b.x);
            let right = a.right().min(b.right());
            if b.y - a.bottom() == PASSAGE_WIDTH && right - left >= PASSAGE_WIDTH {
                let x = left + (right - left - PASSAGE_WIDTH) / 2;
                let y = a.bottom();
                circulation.rect(ModuleRect {
                    x,
                    y,
                    width: PASSAGE_WIDTH,
                    height: PASSAGE_WIDTH,
                });
                circulation.door(x, y, Axis::Horizontal);
                circulation.door(x, y + PASSAGE_WIDTH, Axis::Horizontal);
            }
        }
    }
}

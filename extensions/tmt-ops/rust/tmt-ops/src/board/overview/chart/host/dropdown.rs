//! Chart choices use the same anchored menu surface as other board menus.
use super::{Each, Focus, Selection, State, Target};
use crate::board::{
    app::{Choice, MenuEntry},
    menu_surface::{MenuSurface, Pointer},
};
use ratatui::{
    Frame,
    crossterm::event::{KeyCode, MouseEvent},
    layout::Rect,
};
use unicode_width::UnicodeWidthStr;

pub(super) struct Dropdown {
    pub selected: usize,
    targets: Vec<Target>,
    entries: Vec<MenuEntry>,
    surface: MenuSurface,
    anchor: Rect,
    trigger: Target,
    hint: Option<String>,
}
impl Dropdown {
    pub fn new(state: &State, focus: Focus) -> Self {
        let targets = if focus == Focus::Range {
            Selection::RANGE
                .iter()
                .copied()
                .map(Target::Range)
                .collect::<Vec<_>>()
        } else {
            std::iter::once(Target::Each(Each::Auto))
                .chain(
                    Selection::each_choices(state.selection.range_minutes)
                        .iter()
                        .copied()
                        .map(|n| Target::Each(Each::Fixed(n))),
                )
                .collect()
        };
        let current = if focus == Focus::Range {
            Target::Range(state.selection.range_minutes)
        } else {
            Target::Each(state.each)
        };
        let selected = targets.iter().position(|t| *t == current).unwrap_or(0);
        let entries = targets
            .iter()
            .map(|target| {
                let label = match target {
                    Target::Each(Each::Auto) => format!(
                        "Automatic ({} per bar)",
                        super::super::format::interval(
                            Each::Auto
                                .resolve(state.selection.range_minutes, state.viewport.width)
                                .each_minutes
                        )
                    ),
                    Target::Each(Each::Fixed(n)) => {
                        format!("{} per bar", super::super::format::interval(*n))
                    }
                    Target::Range(n) => super::super::format::range(*n),
                    _ => unreachable!(),
                };
                let reason = match target {
                    Target::Each(Each::Fixed(n)) => Selection {
                        each_minutes: *n,
                        range_minutes: state.selection.range_minutes,
                    }
                    .validate()
                    .err()
                    .map(|_| {
                        let minimum = Selection::each_choices(state.selection.range_minutes)
                            .iter()
                            .copied()
                            .find(|n| state.selection.range_minutes / *n <= Selection::MAX_BARS)
                            .unwrap();
                        format!(
                            "{} needs at least {} per bar",
                            super::super::format::range(state.selection.range_minutes),
                            super::super::format::interval(minimum)
                        )
                    }),
                    _ => None,
                };
                MenuEntry {
                    key: if *target == current {
                        "✓".into()
                    } else {
                        String::new()
                    },
                    label,
                    choice: reason.map_or(Choice::Dismiss, Choice::Unavailable),
                }
            })
            .collect::<Vec<_>>();
        let anchor = state
            .hits
            .iter()
            .find(|hit| {
                (hit.target
                    == if focus == Focus::Range {
                        Target::OpenRange
                    } else {
                        Target::OpenEach
                    })
                    || hit.target == Target::Controls
            })
            .map(|hit| hit.area)
            .unwrap_or_default();
        let title = if focus == Focus::Range {
            "Range"
        } else {
            "Time per bar"
        };
        let surface = MenuSurface::dropdown(title, &entries);
        Self {
            selected,
            targets,
            entries,
            surface,
            anchor,
            hint: None,
            trigger: if focus == Focus::Range {
                Target::OpenRange
            } else {
                Target::OpenEach
            },
        }
    }
    pub fn resolve_width(&mut self, range: u32, width: usize) {
        if let Some(index) = self
            .targets
            .iter()
            .position(|t| *t == Target::Each(Each::Auto))
        {
            self.entries[index].label = format!(
                "Automatic ({} per bar)",
                super::super::format::interval(Each::Auto.resolve(range, width).each_minutes)
            );
        }
    }
    pub fn invalidate(&mut self) {
        self.surface.invalidate();
    }
    pub fn reanchor(&mut self, hits: &[super::Hit]) {
        if let Some(hit) = hits
            .iter()
            .find(|hit| hit.target == self.trigger || hit.target == Target::Controls)
        {
            self.anchor = hit.area;
        }
    }
    pub fn choose(&self) -> Result<Target, String> {
        if let Choice::Unavailable(reason) = &self.entries[self.selected].choice {
            Err(reason.clone())
        } else {
            Ok(self.targets[self.selected])
        }
    }
    pub fn hint_for(&mut self, index: usize) {
        self.hint = match self.entries.get(index).map(|entry| &entry.choice) {
            Some(Choice::Unavailable(reason)) => Some(reason.clone()),
            _ => None,
        };
    }
    fn move_by(&mut self, delta: isize) {
        self.hint = None;
        let mut next = self.selected;
        loop {
            let moved = next
                .saturating_add_signed(delta)
                .min(self.entries.len() - 1);
            if moved == next {
                return;
            }
            next = moved;
            if !matches!(self.entries[next].choice, Choice::Unavailable(_)) {
                self.selected = next;
                return;
            }
            self.hint_for(next);
        }
    }
    pub fn key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Up => self.move_by(-1),
            KeyCode::Down => self.move_by(1),
            KeyCode::Home => self.selected = 0, // Auto/first range is always admitted.
            KeyCode::End => self.selected = self.entries.len() - 1,
            _ => (),
        }
    }
    pub fn pointer(&mut self, event: MouseEvent) -> bool {
        let before = self.hint.clone();
        if let Some(index) = self.surface.row_at(event) {
            self.hint_for(index);
        } else {
            self.hint = None;
        }
        self.hint != before
    }
    pub fn mouse(&mut self, event: MouseEvent) -> bool {
        match self.surface.pointer(event) {
            Some(Pointer::Choose(index)) => {
                self.hint_for(index);
                if self.hint.is_some() {
                    return false;
                }
                self.selected = index;
                true
            }
            Some(Pointer::Wheel(delta)) => {
                self.move_by(delta);
                false
            }
            None => false,
        }
    }
    pub fn render(&mut self, frame: &mut Frame, look: crate::look::Look, screen: Rect) {
        let usable = Rect {
            height: screen.height.saturating_sub(1),
            ..screen
        };
        // Keep the complete time-per-bar labels beside the three-cell checkmark
        // track; the existing menu owner still clips to the available screen.
        let width = self
            .entries
            .iter()
            .map(|entry| entry.label.width())
            .max()
            .unwrap_or_default()
            .saturating_add(3)
            .max(18);
        let width = u16::try_from(width).unwrap_or(u16::MAX);
        let mut area =
            crate::board::member_links::menu_area(usable, self.anchor, self.entries.len() as u16)
                .unwrap_or_else(|| {
                    Rect::new(
                        self.anchor.x,
                        usable.y,
                        width,
                        usable.height.min(self.entries.len() as u16),
                    )
                });
        area.width = screen.width.saturating_sub(2).min(width);
        area.x = self
            .anchor
            .x
            .min(screen.right().saturating_sub(area.width + 1));
        let label = if self.trigger == Target::OpenEach {
            "Time per bar"
        } else {
            "Range"
        };
        let footer = self
            .hint
            .clone()
            .unwrap_or_else(|| format!("{label} · ↑↓ choose · Enter apply · Esc close"));
        tmt_tui::components::strip::paint_left(
            frame.buffer_mut(),
            Rect::new(screen.x, screen.bottom().saturating_sub(1), screen.width, 1),
            ratatui::text::Line::styled(
                crate::board::view::fit(&footer, usize::from(screen.width)),
                look.role(tmt_cli_style::Role::Muted),
            ),
        );
        self.surface
            .render(&self.entries, self.selected, frame, look, area);
    }
}

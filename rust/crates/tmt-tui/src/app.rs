//! Focus and event routing from `design/cli-style.md#layers-and-focus`.
//! The caller owns terminal acquisition, data, cursors and application effects.
use ratatui::crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};

pub type ComponentId = Vec<String>;

#[derive(Debug, Clone)]
struct Layer {
    id: ComponentId,
    fields: Vec<ComponentId>,
    focused: usize,
}

/// One base focus and one active modal. Replacing the modal preserves the base
/// return target; item selection and scroll state stay with their component.
#[derive(Debug, Default)]
pub struct FocusStack {
    base: Vec<ComponentId>,
    focused: usize,
    overlay: Option<Layer>,
}

impl FocusStack {
    pub fn new(base: Vec<ComponentId>) -> Self {
        Self {
            base,
            ..Self::default()
        }
    }

    pub fn focused(&self) -> Option<&ComponentId> {
        self.overlay.as_ref().map_or_else(
            || self.base.get(self.focused),
            |layer| Some(layer.fields.get(layer.focused).unwrap_or(&layer.id)),
        )
    }

    pub fn base_focus(&self) -> Option<&ComponentId> {
        self.base.get(self.focused)
    }

    pub fn focus_base(&mut self, id: &[String]) -> bool {
        if let Some(index) = self.base.iter().position(|candidate| candidate == id) {
            self.focused = index;
            true
        } else {
            false
        }
    }

    /// Reconcile reading order by identity, or the nearest surviving position.
    pub fn reconcile_base(&mut self, base: Vec<ComponentId>) {
        self.focused = self
            .base_focus()
            .and_then(|id| base.iter().position(|candidate| candidate == id))
            .unwrap_or(self.focused.min(base.len().saturating_sub(1)));
        self.base = base;
    }

    pub fn open(&mut self, id: ComponentId, fields: Vec<ComponentId>) {
        self.overlay = Some(Layer {
            id,
            fields,
            focused: 0,
        });
    }

    /// Closing never dispatches the closing event to the restored base.
    pub fn close(&mut self) -> Option<ComponentId> {
        self.overlay.take().map(|layer| layer.id)
    }

    pub fn overlay(&self) -> Option<&ComponentId> {
        self.overlay.as_ref().map(|layer| &layer.id)
    }

    fn advance(&mut self, backwards: bool) {
        let (index, count) = match &mut self.overlay {
            Some(layer) => (&mut layer.focused, layer.fields.len()),
            None => (&mut self.focused, self.base.len()),
        };
        if count > 0 {
            *index = if backwards {
                (*index + count - 1) % count
            } else {
                (*index + 1) % count
            };
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Routed<E> {
    Handled(E),
    Captured,
    Unhandled,
    /// Ctrl-C is the only global escape from modal routing. The caller performs
    /// cleanup and discards unsaved input before exiting.
    Quit,
}

/// A focused field gets the event before its containing modal. An unhandled
/// modal event, including mouse input outside its box, never reaches the base.
/// A handler may close/replace layers after this call returns its effect.
pub fn route<E>(
    stack: &mut FocusStack,
    event: &Event,
    mut handler: impl FnMut(&ComponentId, &Event) -> Option<E>,
) -> Routed<E> {
    if let Event::Key(key) = event {
        if key.kind == KeyEventKind::Release {
            return if stack.overlay.is_some() {
                Routed::Captured
            } else {
                Routed::Unhandled
            };
        }
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return Routed::Quit;
        }
    }
    if let Some(id) = stack.focused()
        && let Some(effect) = handler(id, event)
    {
        return Routed::Handled(effect);
    }
    if let Some(layer) = &stack.overlay
        && stack.focused() != Some(&layer.id)
        && let Some(effect) = handler(&layer.id, event)
    {
        return Routed::Handled(effect);
    }
    if let Event::Key(key) = event
        && matches!(key.code, KeyCode::Tab | KeyCode::BackTab)
        && !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    {
        stack.advance(key.code == KeyCode::BackTab || key.modifiers.contains(KeyModifiers::SHIFT));
        return Routed::Captured;
    }
    if stack.overlay.is_some() {
        Routed::Captured
    } else {
        Routed::Unhandled
    }
}

#[cfg(test)]
mod tests;

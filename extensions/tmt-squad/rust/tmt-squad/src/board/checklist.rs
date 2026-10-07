//! Checklist modal state. Domain admission/storage remain in crate::checklist.
mod controller;
mod forms;
pub(super) mod load;
mod surface;
#[cfg(test)]
mod tests;

use super::picker_surface;
use crate::checklist::model::{Completion, Id, Request};
use crate::checklist::{Error, Preview};
use std::cell::{Cell, RefCell};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Assignment {
    All,
    Unassigned,
    Member(Id),
}
#[derive(Clone, Debug, PartialEq, Eq)]
enum Screen {
    Unknown,
    Rooms,
    List,
    Details(Id),
    Actions,
    Filters,
    Members(Option<Id>),
    Form,
    Confirm,
    Reorder,
    OrderActions(Id),
}
#[derive(Clone)]
struct Row {
    id: String,
    label: String,
    disabled: bool,
}
impl Row {
    fn choice(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            disabled: false,
        }
    }
    fn text(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            disabled: true,
            ..Self::choice(id, label)
        }
    }
}
pub(in crate::board) struct Controller {
    pub active: bool,
    owner: u64,
    serial: u64,
    pub room: Option<Id>,
    rooms: Vec<(Id, String)>,
    screen: Screen,
    preview: Option<Preview>,
    completion: Option<Completion>,
    assignment: Assignment,
    archived: bool,
    focus: usize,
    pending: Option<load::Key>,
    draft: Option<forms::Draft>,
    submit: Option<Request>,
    reviewed: bool,
    target_label: Option<String>,
    unknown: Vec<String>,
    notice: Option<String>,
    failure: Option<Error>,
    template: RefCell<Option<(String, tmt_tui::components::surface::Template<()>)>>,
    controls: Cell<[ratatui::layout::Rect; 2]>,
    list: RefCell<picker_surface::State>,
    panel: RefCell<picker_surface::State>,
    reading: RefCell<picker_surface::State>,
    pub(super) readable: Cell<bool>,
    return_to: Option<Return>,
}
struct Return {
    tab: Option<String>,
    room: Option<String>,
    pane: crate::config::Pane,
    jobs: bool,
    row: Option<super::app::RowTarget>,
}
impl Controller {
    pub fn new(owner: u64, room: Option<Id>) -> Self {
        Self {
            active: true,
            owner,
            serial: 0,
            screen: if room.is_some() {
                Screen::List
            } else {
                Screen::Rooms
            },
            room,
            rooms: vec![],
            preview: None,
            completion: Some(Completion::Open),
            assignment: Assignment::All,
            archived: false,
            focus: 0,
            pending: None,
            draft: None,
            submit: None,
            reviewed: false,
            target_label: None,
            unknown: vec![],
            notice: None,
            failure: None,
            template: RefCell::new(None),
            controls: Cell::new([ratatui::layout::Rect::default(); 2]),
            list: RefCell::new(picker_surface::State::new(None, vec![], None)),
            panel: RefCell::new(picker_surface::State::new(None, vec![], None)),
            reading: RefCell::new(picker_surface::State::new(None, vec![], None)),
            readable: Cell::new(false),
            return_to: None,
        }
    }
    pub fn invalidate(&self) {
        self.list.borrow_mut().invalidate();
        self.panel.borrow_mut().invalidate();
        self.reading.borrow_mut().invalidate();
        self.readable.set(false);
        self.controls.set([ratatui::layout::Rect::default(); 2]);
    }
    fn task(&mut self, job: load::Job) -> super::app::Effect {
        self.serial += 1;
        let key = load::Key {
            controller: self.owner,
            serial: self.serial,
            room: self.room.clone(),
        };
        self.pending = Some(key.clone());
        self.invalidate();
        super::app::Effect::Checklist(load::Task { key, job })
    }
    fn read(&mut self) -> super::app::Effect {
        self.task(if self.room.is_some() {
            load::Job::Read
        } else {
            load::Job::Rooms
        })
    }
    fn current(&self) -> Option<&crate::checklist::Current> {
        self.preview.as_ref().map(|p| &p.current)
    }
    fn manager(&self) -> bool {
        self.current()
            .is_some_and(|c| c.room.available && c.room.manager)
    }
    fn writable(&self) -> bool {
        self.current().is_some_and(|c| c.room.available)
            && self.failure.is_none()
            && self.pending.is_none()
    }
    fn item(&self, id: &Id) -> Option<&crate::checklist::ItemView> {
        self.current()?
            .items
            .iter()
            .find(|view| &view.item.id == id)
    }
    fn matches(&self, view: &crate::checklist::ItemView) -> bool {
        (self.archived || !view.item.archived)
            && self.completion.is_none_or(|c| c == view.item.completion)
            && match &self.assignment {
                Assignment::All => true,
                Assignment::Unassigned => view.item.assignee.is_none(),
                Assignment::Member(id) => view.item.assignee.as_ref().is_some_and(|a| &a.id == id),
            }
    }
}

//! Form input and current painted-hit admission; effects leave on the lane.
use super::model::Slot;
use super::*;
impl App {
    pub(in crate::board) fn new_member_event(&mut self, event: &Event) -> Option<Effect> {
        let c = self.new_member.as_mut().filter(|c| c.shown)?;
        if matches!(event, Event::Resize(..)) {
            c.invalidate();
            return Some(Effect::None);
        }
        let mut activate = None;
        if let Event::Mouse(mouse) = event
            && mouse.kind == MouseEventKind::Down(MouseButton::Left)
        {
            let position = ratatui::layout::Position::new(mouse.column, mouse.row);
            let field = c
                .fields
                .borrow()
                .iter()
                .find(|(_, area)| area.contains(position))
                .copied();
            if let Some((field, area)) = field {
                match field {
                    Field::Slot(slot) => {
                        c.at = Focus::Slot(slot);
                        if slot.listed() {
                            c.open_list(slot);
                        } else {
                            c.draft.fields[slot.index()].place(
                                area.width,
                                0,
                                usize::from(mouse.column.saturating_sub(area.x)),
                                false,
                            );
                        }
                    }
                    Field::Keep(saved) => {
                        c.at = Focus::Slot(Slot::Keep);
                        c.draft.saved = saved;
                    }
                    Field::Choice(row) => c.choose(row),
                }
                c.invalidate();
                return Some(Effect::None);
            }
            activate = c
                .controls
                .get()
                .iter()
                .position(|area| area.is_some_and(|a| a.contains(position)));
        }
        if let Event::Paste(text) = event {
            if !text.chars().any(char::is_control) {
                if c.stage == Stage::Template {
                    c.template.insert(text, 16384);
                } else if let Some(list) = &mut c.list {
                    list.filter.insert(text, 1024);
                    list.row = 0;
                } else if c.stage == Stage::Form
                    && let Focus::Slot(slot) = c.at
                    && slot.typed()
                {
                    c.draft.fields[slot.index()].insert(text, 1024);
                }
            }
            c.invalidate();
            return Some(Effect::None);
        }
        if let Event::Key(key) = event {
            if key.kind == KeyEventKind::Release {
                return Some(Effect::None);
            }
            if key.code == KeyCode::Esc {
                match c.stage {
                    Stage::Template => {
                        c.stage = if c.destination == Destination::Settings {
                            Stage::Hosts
                        } else {
                            Stage::Form
                        };
                        c.focus = 0;
                        c.notice = None;
                        c.invalidate();
                    }
                    Stage::Form if c.list.is_some() => {
                        c.list = None;
                        c.invalidate();
                    }
                    _ => return Some(self.new_member_close()),
                }
                return Some(Effect::None);
            }
            if matches!(c.stage, Stage::Launching | Stage::Saving) {
                return Some(Effect::None);
            }
            if c.list.is_some() {
                let count = c.rows().len().max(1);
                let width = c.editor_width.get();
                let Some(list) = c.list.as_mut() else {
                    return Some(Effect::None);
                };
                match key.code {
                    KeyCode::Up | KeyCode::BackTab => list.row = (list.row + count - 1) % count,
                    KeyCode::Down | KeyCode::Tab => list.row = (list.row + 1) % count,
                    KeyCode::Enter => {
                        let row = list.row;
                        c.choose(row);
                    }
                    _ => {
                        let before = list.filter.as_str().to_owned();
                        list.filter.key(*key, width, 1024);
                        if list.filter.as_str() != before {
                            list.row = 0;
                        }
                    }
                }
                c.invalidate();
                return Some(Effect::None);
            }
            let plain = !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
            if key.code == KeyCode::Char('e')
                && plain
                && c.stage == Stage::Form
                && matches!(
                    c.at,
                    Focus::Slot(Slot::Host) | Focus::Cancel | Focus::Create
                )
            {
                c.begin_template();
                return Some(Effect::None);
            }
            if key.code == KeyCode::Char('r')
                && key.modifiers == KeyModifiers::CONTROL
                && c.stage == Stage::Template
            {
                let mut task = c.task(Work::Save {
                    host: c.draft.text(Slot::Host).to_owned(),
                    argv: None,
                });
                task.config = c.loaded.as_ref().map(|l| Box::new(l.config.clone()));
                c.stage = Stage::Saving;
                c.invalidate();
                return Some(Effect::NewMember(task));
            }
            if c.stage == Stage::Hosts {
                let hosts = c.loaded.as_ref().map(|l| &l.hosts);
                let available = hosts
                    .into_iter()
                    .flatten()
                    .filter(|h| matches!(h.state.as_str(), "builtin" | "ok"))
                    .map(|h| h.name.clone())
                    .collect::<Vec<_>>();
                if matches!(
                    key.code,
                    KeyCode::Down | KeyCode::Up | KeyCode::Tab | KeyCode::BackTab
                ) && !available.is_empty()
                {
                    let current = available
                        .iter()
                        .position(|h| h == c.draft.text(Slot::Host))
                        .unwrap_or(0);
                    let back = matches!(key.code, KeyCode::Up | KeyCode::BackTab);
                    c.draft.set(
                        Slot::Host,
                        &available[(current + if back { available.len() - 1 } else { 1 })
                            % available.len()]
                        .clone(),
                    );
                    c.invalidate();
                    return Some(Effect::None);
                }
                if key.code == KeyCode::Enter {
                    c.begin_template();
                    return Some(Effect::None);
                }
            }
            if c.stage == Stage::Form {
                match (key.code, c.at) {
                    (KeyCode::Up | KeyCode::BackTab, _) => c.step_focus(true),
                    (KeyCode::Down | KeyCode::Tab, _) => c.step_focus(false),
                    (
                        KeyCode::Left | KeyCode::Right | KeyCode::Char(' '),
                        Focus::Slot(Slot::Keep),
                    ) => c.draft.saved = !c.draft.saved,
                    (KeyCode::Left | KeyCode::Right, Focus::Slot(slot)) if slot.listed() => {
                        c.cycle(slot, key.code == KeyCode::Left);
                    }
                    (KeyCode::Char(' '), Focus::Slot(slot)) if slot.listed() && !slot.typed() => {
                        c.cycle(slot, false);
                    }
                    (KeyCode::Enter, Focus::Slot(Slot::Keep)) => c.draft.saved = !c.draft.saved,
                    (KeyCode::Enter, Focus::Slot(slot)) if slot.listed() => c.open_list(slot),
                    (KeyCode::Enter, Focus::Slot(_)) => c.step_focus(false),
                    (KeyCode::Enter, Focus::Cancel) => activate = Some(0),
                    (KeyCode::Enter, Focus::Create) => activate = Some(1),
                    (_, Focus::Slot(slot)) if slot.typed() => {
                        c.draft.fields[slot.index()].key(*key, c.editor_width.get(), 1024);
                    }
                    _ => {}
                }
                // A pressed button keeps the painted admission it is checked against.
                if activate.is_none() {
                    c.invalidate();
                }
            } else {
                match key.code {
                    KeyCode::Tab | KeyCode::BackTab if c.stage == Stage::Template => {
                        c.focus = (c.focus + if key.code == KeyCode::Tab { 1 } else { 2 }) % 3;
                        c.invalidate();
                    }
                    KeyCode::Enter => {
                        if c.stage == Stage::Template && c.focus == 0 {
                            c.focus = 1;
                            c.invalidate();
                        } else {
                            activate = Some(if c.stage == Stage::Template {
                                c.focus - 1
                            } else {
                                c.focus
                            });
                        }
                    }
                    _ => {
                        if c.stage == Stage::Template && c.focus == 0 {
                            c.template.key(*key, c.editor_width.get(), 16384);
                        }
                        c.invalidate();
                    }
                }
            }
        }
        if matches!(c.stage, Stage::Result(_)) && c.scroll.borrow_mut().input(event) {
            c.readable.set(false);
        }
        if let Some(index) = activate {
            if index == 0 {
                match c.stage {
                    Stage::Template => {
                        c.stage = if c.destination == Destination::Settings {
                            Stage::Hosts
                        } else {
                            Stage::Form
                        };
                        c.focus = 0;
                        c.invalidate();
                    }
                    Stage::Form if c.list.is_some() => {
                        c.list = None;
                        c.invalidate();
                    }
                    _ => return Some(self.new_member_close()),
                };
                return Some(Effect::None);
            }
            if !c.readable.get() {
                c.notice = Some(
                    "Enlarge the terminal to review every field and command before continuing."
                        .into(),
                );
                return Some(Effect::None);
            }
            match c.stage {
                Stage::Form if c.list.is_some() => {
                    let row = c.list.as_ref().map_or(0, |l| l.row);
                    c.choose(row);
                }
                Stage::Form => {
                    let name = c.draft.text(Slot::Name);
                    if self
                        .references
                        .agents
                        .values()
                        .any(|agent| agent.name == name || agent.canonical == name)
                    {
                        c.notice = Some("That identity already exists; choose a new name.".into());
                    } else if let Some(loaded) = &c.loaded {
                        match c.draft.review(loaded) {
                            // No template: Create copies the commands instead.
                            Ok(review) if review.outer.is_none() => {
                                let text = review.copy();
                                c.stage = Stage::Result(review.copy_notice());
                                c.invalidate();
                                return Some(Effect::Act(Request::Copy {
                                    text,
                                    program: self.view.as_ref().and_then(|v| v.clipboard.clone()),
                                    feedback: CopyFeedback::Destination,
                                }));
                            }
                            Ok(review) => {
                                let task = c.task(Work::Launch(review.clone()));
                                c.review = Some(review);
                                c.stage = Stage::Launching;
                                c.notice = None;
                                c.invalidate();
                                return Some(Effect::NewMember(task));
                            }
                            Err(error) => c.notice = Some(error),
                        }
                    }
                }
                Stage::Hosts => {
                    c.begin_template();
                }
                Stage::Template => {
                    let argv = serde_json::from_str::<serde_json::Value>(&c.template)
                        .map_err(|e| e.to_string())
                        .and_then(|v| crate::launch::words(&v))
                        .and_then(|argv| crate::launch::validate(&argv).map(|()| argv));
                    match argv {
                        Ok(argv) => {
                            let mut task = c.task(Work::Save {
                                host: c.draft.text(Slot::Host).to_owned(),
                                argv: Some(argv),
                            });
                            task.config = c.loaded.as_ref().map(|l| Box::new(l.config.clone()));
                            c.stage = Stage::Saving;
                            c.invalidate();
                            return Some(Effect::NewMember(task));
                        }
                        Err(error) => c.notice = Some(error),
                    }
                }
                _ => {}
            }
            c.invalidate();
        }
        Some(Effect::None)
    }
}

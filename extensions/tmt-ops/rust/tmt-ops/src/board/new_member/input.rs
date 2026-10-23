//! Form input and current painted-hit admission; effects leave on the lane.
use super::*;
impl App {
    pub(in crate::board) fn new_member_event(&mut self, event: &Event) -> Option<Effect> {
        let c = self.new_member.as_mut()?;
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
                    Field::Text(index) => {
                        c.focus = index;
                        c.draft.fields[index].place(
                            area.width,
                            0,
                            usize::from(mouse.column.saturating_sub(area.x)),
                            false,
                        );
                    }
                    Field::Host => {
                        c.focus = 4;
                        c.cycle(false);
                    }
                    Field::Identity(saved) => {
                        c.focus = 6;
                        c.draft.saved = saved;
                    }
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
                } else if c.stage == Stage::Form
                    && c.focus < 6
                    && c.focus != 4
                    && (c.focus != 3 || c.draft.saved)
                {
                    c.draft.fields[c.focus].insert(text, 1024);
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
                    Stage::Launching | Stage::Saving => {
                        c.notice = Some(if c.stage == Stage::Saving {
                            "Template save is in progress; the member draft is kept.".into()
                        } else {
                            "Launch is in progress; its outcome will be reported. No retry or window close.".into()
                        });
                    }
                    Stage::Review | Stage::Template => {
                        c.stage = if c.destination == Destination::Settings {
                            Stage::Hosts
                        } else {
                            Stage::Form
                        };
                        c.focus = 0;
                        c.notice = None;
                        c.invalidate();
                    }
                    _ => self.new_member = None,
                }
                return Some(Effect::None);
            }
            if matches!(c.stage, Stage::Launching | Stage::Saving) {
                return Some(Effect::None);
            }
            let plain = !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
            if key.code == KeyCode::Char('e')
                && plain
                && (c.stage == Stage::Review || c.stage == Stage::Form && c.focus == 4)
            {
                c.begin_template();
                return Some(Effect::None);
            }
            if key.code == KeyCode::Char('c')
                && plain
                && c.stage == Stage::Review
                && let Some(review) = &c.review
            {
                let text = review.copy();
                if review.outer.is_none() {
                    c.stage = Stage::Result(review.copy_notice());
                }
                return Some(Effect::Act(Request::Copy {
                    text,
                    program: self.view.as_ref().and_then(|v| v.clipboard.clone()),
                    feedback: CopyFeedback::Destination,
                }));
            }
            if key.code == KeyCode::Char('r')
                && key.modifiers == KeyModifiers::CONTROL
                && c.stage == Stage::Template
            {
                let mut task = c.task(Work::Save {
                    host: c.draft.fields[4].to_string(),
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
                        .position(|h| h == c.draft.fields[4].as_str())
                        .unwrap_or(0);
                    let back = matches!(key.code, KeyCode::Up | KeyCode::BackTab);
                    c.draft.fields[4] = Editor::from(
                        available[(current + if back { available.len() - 1 } else { 1 })
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
            match key.code {
                KeyCode::Tab | KeyCode::BackTab => {
                    let count = if c.stage == Stage::Form {
                        9
                    } else if c.stage == Stage::Template {
                        3
                    } else {
                        2
                    };
                    c.focus = (c.focus
                        + if key.code == KeyCode::Tab {
                            1
                        } else {
                            count - 1
                        })
                        % count;
                    if c.stage == Stage::Form && c.focus == 3 && !c.draft.saved {
                        c.focus = if key.code == KeyCode::Tab { 4 } else { 2 };
                    }
                    c.invalidate();
                }
                KeyCode::Char(' ')
                    if c.stage == Stage::Form && (1..=4).contains(&c.focus)
                        || key.code == KeyCode::Char(' ')
                            && c.stage == Stage::Form
                            && c.focus == 6 =>
                {
                    c.cycle(false);
                    c.invalidate();
                }
                KeyCode::Enter => {
                    if c.stage == Stage::Form && c.focus < 7
                        || c.stage == Stage::Template && c.focus == 0
                    {
                        c.focus += 1;
                        if c.stage == Stage::Form && c.focus == 3 && !c.draft.saved {
                            c.focus = 4;
                        }
                        c.invalidate();
                    } else {
                        activate = Some(if c.stage == Stage::Form {
                            c.focus - 7
                        } else if c.stage == Stage::Template {
                            c.focus.saturating_sub(1)
                        } else {
                            c.focus
                        });
                    }
                }
                _ => {
                    if c.stage == Stage::Template && c.focus == 0 {
                        c.template.key(*key, c.editor_width.get(), 16384);
                    } else if c.stage == Stage::Form
                        && c.focus < 6
                        && c.focus != 4
                        && (c.focus != 3 || c.draft.saved)
                    {
                        c.draft.fields[c.focus].key(*key, c.editor_width.get(), 1024);
                    }
                    c.invalidate();
                }
            }
        }
        if matches!(c.stage, Stage::Review | Stage::Result(_)) && c.scroll.borrow_mut().input(event)
        {
            c.readable.set(false);
        }
        if let Some(index) = activate {
            if index == 0 {
                match c.stage {
                    Stage::Review | Stage::Template => {
                        c.stage = if c.destination == Destination::Settings {
                            Stage::Hosts
                        } else {
                            Stage::Form
                        };
                        c.focus = 0;
                        c.invalidate();
                    }
                    _ => self.new_member = None,
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
                Stage::Form => {
                    if self.references.agents.values().any(|agent| {
                        agent.name == c.draft.fields[0].as_str()
                            || agent.canonical == c.draft.fields[0].as_str()
                    }) {
                        c.notice = Some("That identity already exists; choose a new name.".into());
                    } else if let Some(loaded) = &c.loaded {
                        match c.draft.review(loaded) {
                            Ok(review) => {
                                c.review = Some(review);
                                c.stage = Stage::Review;
                                c.focus = 0;
                                c.notice = None;
                            }
                            Err(error) => c.notice = Some(error),
                        }
                    }
                }
                Stage::Review => {
                    if let Some(review) = &c.review {
                        if review.outer.is_none() {
                            let text = review.copy();
                            c.stage = Stage::Result(review.copy_notice());
                            return Some(Effect::Act(Request::Copy {
                                text,
                                program: self.view.as_ref().and_then(|v| v.clipboard.clone()),
                                feedback: CopyFeedback::Destination,
                            }));
                        }
                        let task = c.task(Work::Launch(review.clone()));
                        c.stage = Stage::Launching;
                        c.invalidate();
                        return Some(Effect::NewMember(task));
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
                                host: c.draft.fields[4].to_string(),
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

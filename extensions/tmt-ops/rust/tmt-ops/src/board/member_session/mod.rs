//! Selected-member session observation and explicit foreground handoff.
mod model;
mod surface;
#[cfg(test)]
pub(in crate::board) mod tests;

use super::{
    app::{App, CopyFeedback, Effect, Request},
    picker_surface,
    workspace::OpsTarget,
};
use crate::core::{Core, SquadError};
use model::Evidence;
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::board) struct Task {
    owner: u64,
    target: OpsTarget,
    evidence: Evidence,
}
pub(in crate::board) struct Completed {
    task: Task,
    result: Result<Observed, SquadError>,
}
struct Observed {
    executable: PathBuf,
    launcher: String,
    preset: Option<String>,
}
pub(in crate::board) fn complete(core: &Core, task: Task) -> Completed {
    let result = (|| {
        let OpsTarget::Member { identity_id, .. } = &task.target else {
            unreachable!("selected member task");
        };
        let value = core.resume_show(identity_id)?;
        if !value["identity"].as_str().is_some_and(|id| !id.is_empty())
            || !value.get("launch").is_some_and(|v| {
                v.is_null()
                    || v["executable"]
                        .as_str()
                        .is_some_and(|executable| !executable.is_empty())
            })
        {
            return Err(SquadError::new(
                "SQUAD_CORE_INVALID",
                "Session preset observation unavailable.",
            ));
        }
        if core
            .executable()
            .to_str()
            .is_none_or(|path| path.chars().any(char::is_control))
        {
            return Err(SquadError::new(
                "SQUAD_CORE_INVALID",
                "The Core executable path cannot be represented as one copyable command line.",
            ));
        }
        let preset = value["launch"]["executable"].as_str().map(str::to_owned);
        Ok(Observed {
            executable: core.executable().to_owned(),
            launcher: super::command_line::executable_word(
                core.executable(),
                std::env::var_os("PATH").as_deref(),
            ),
            preset,
        })
    })();
    Completed { task, result }
}

/// One literal argv owner shared by display, copy and the final exec.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::board) struct Foreground {
    executable: PathBuf,
    launcher: String,
    args: Vec<String>,
}
impl Foreground {
    fn new(executable: PathBuf, launcher: &str, id: &str, move_here: bool) -> Self {
        let mut args = vec!["resume".into()];
        if move_here {
            args.push("--move".into());
        }
        args.extend(["--".into(), id.into()]);
        Self {
            executable,
            launcher: launcher.into(),
            args,
        }
    }
    fn line(&self) -> String {
        // Display and copy share literal argv formatting; execution never parses it.
        super::command_line::command_line(
            &std::iter::once(self.launcher.as_str())
                .chain(self.args.iter().map(String::as_str))
                .collect::<Vec<_>>(),
        )
    }
    pub fn exec(self) -> std::io::Error {
        use std::os::unix::process::CommandExt;
        std::process::Command::new(self.executable)
            .args(self.args)
            .exec()
    }
}

pub(in crate::board) struct Controller {
    task: Task,
    loaded: Option<Result<Observed, SquadError>>,
    confirmation: Option<Foreground>,
    status: Option<String>,
    readable: Cell<bool>,
    ui: RefCell<picker_surface::State>,
}
impl Controller {
    fn recovery_command(&self) -> Option<String> {
        if !self.task.evidence.is_unbound() || !self.task.evidence.remembered() {
            return None;
        }
        let Some(Ok(observed)) = &self.loaded else {
            return None;
        };
        let OpsTarget::Member { identity_id, .. } = &self.task.target else {
            unreachable!()
        };
        // TODO(#4663): Reuse the UUID handoff after Core supports UUID launch.
        Some(
            Foreground::new(
                observed.executable.clone(),
                &observed.launcher,
                identity_id,
                false,
            )
            .line(),
        )
    }
    pub fn invalidate(&self) {
        self.readable.set(false);
        self.ui.borrow_mut().invalidate();
    }
}
impl App {
    pub(in crate::board) fn open_member_session(&mut self) -> Effect {
        if self.input.is_some() || self.searching || self.modal_open() && self.menu.is_none() {
            return self.say("Close the current input or overlay before opening Session.");
        }
        let Some(target @ OpsTarget::Member { .. }) = self.workspace_selected_member().cloned()
        else {
            return self.say("Select a visible agent before opening Session.");
        };
        let OpsTarget::Member { identity_id, .. } = &target else {
            unreachable!()
        };
        let evidence = self.member_session_evidence(identity_id);
        self.member_session_generation += 1;
        let task = Task {
            owner: self.member_session_generation,
            target,
            evidence,
        };
        self.menu = None;
        self.member_session = Some(Controller {
            task: task.clone(),
            loaded: None,
            confirmation: None,
            status: None,
            readable: Cell::new(false),
            ui: RefCell::new(picker_surface::State::new(None, vec![], None)),
        });
        Effect::MemberSession(task)
    }
    pub(in crate::board) fn member_session_changed(&mut self) {
        let valid = self
            .member_session
            .as_ref()
            .is_none_or(|c| self.session_target_matches(&c.task.target, &c.task.evidence));
        if !valid && let Some(c) = &mut self.member_session {
            c.confirmation = None;
            c.loaded = Some(Err(SquadError::new(
                "SQUAD_SESSION_CHANGED",
                "Selected member or session observation changed. Close and reopen Session.",
            )));
            c.invalidate();
        }
    }
    pub(in crate::board) fn finished_member_session(&mut self, completed: Completed) -> Effect {
        self.member_session_changed();
        if let Some(c) = &mut self.member_session
            && c.task == completed.task
            && c.loaded.is_none()
        {
            c.loaded = Some(completed.result);
            c.invalidate();
        }
        Effect::None
    }
    pub(in crate::board) fn member_session_event(
        &mut self,
        event: &ratatui::crossterm::event::Event,
    ) -> Option<Effect> {
        use ratatui::crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
        self.member_session_changed();
        let c = self.member_session.as_mut()?;
        if let Event::Resize(..) = event {
            c.invalidate();
            return Some(Effect::None);
        }
        if let Event::Key(key) = event {
            if key.kind == KeyEventKind::Release {
                return Some(Effect::None);
            }
            if key.code == KeyCode::Esc {
                if c.confirmation.take().is_some() {
                    c.invalidate();
                } else {
                    self.member_session = None;
                }
                return Some(Effect::None);
            }
            if key.code == KeyCode::Char('c')
                && !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                && let Some(command) = &c.confirmation
            {
                return Some(Effect::Act(Request::Copy {
                    text: command.line(),
                    program: self.view.as_ref().and_then(|v| v.clipboard.clone()),
                    feedback: CopyFeedback::Destination,
                }));
            }
        }
        let choice = c.choice(event);
        match choice.as_deref() {
            Some("close") => self.member_session = None,
            Some("cancel") => {
                let c = self.member_session.as_mut()?;
                c.confirmation = None;
                c.invalidate();
            }
            Some("copy") => {
                if let Some(command) = self.member_session.as_ref()?.recovery_command() {
                    return Some(Effect::Act(Request::Copy {
                        text: command,
                        program: self.view.as_ref().and_then(|view| view.clipboard.clone()),
                        feedback: CopyFeedback::Destination,
                    }));
                }
            }
            Some("resume" | "move") => {
                let c = self.member_session.as_mut()?;
                if !c.task.evidence.is_unbound()
                    && c.task.evidence.remembered()
                    && let Some(Ok(observed)) = &c.loaded
                {
                    let OpsTarget::Member { identity_id, .. } = &c.task.target else {
                        unreachable!()
                    };
                    c.confirmation = Some(Foreground::new(
                        observed.executable.clone(),
                        &observed.launcher,
                        identity_id,
                        choice.as_deref() == Some("move"),
                    ));
                    c.status = None;
                    let rows = c.list_rows();
                    let mut ui = c.ui.borrow_mut();
                    ui.reconcile(rows);
                    ui.select("cancel");
                    drop(ui);
                    c.invalidate();
                }
            }
            Some("confirm") => {
                let blocker = self.member_session_blocker();
                let c = self.member_session.as_mut()?;
                if let Some(reason) = blocker {
                    c.status = Some(reason.into());
                    c.invalidate();
                } else if c.readable.get()
                    && let Some(command) = c.confirmation.take()
                {
                    return Some(Effect::Foreground(command));
                } else {
                    c.status = Some(
                        "Enlarge the terminal to review the full confirmation before continuing."
                            .into(),
                    );
                }
            }
            _ => {}
        }
        Some(Effect::None)
    }
}

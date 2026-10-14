//! Bounded scheduled-invocation preview on the existing action lane, never paint.
use super::CronRequest;
use crate::board::app::{App, Effect, Request};
use ratatui::{
    Frame,
    crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind},
    layout::Rect,
    text::Line,
};
use std::cell::{Cell, RefCell};
use tmt_cli_style::Role;
use tmt_ops::cron::Schedule;
use tmt_tui::components::{Modal, Placement, ScrollState, strip};

const DAY_MS: i64 = 24 * 60 * 60 * 1000;
/// Bounds production next_after calls on the worker; dense schedules say capped.
const INVOCATION_PREVIEW_LIMIT: usize = 4096;
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Task {
    pub generation: u64,
    pub request: CronRequest,
    pub previous: Option<Schedule>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Preview {
    pub count: usize,
    pub capped: bool,
    pub next: Vec<i64>,
    pub now: i64,
}

fn project(schedule: &Schedule, now: i64) -> Result<Preview, String> {
    let end = now
        .checked_add(DAY_MS)
        .ok_or("Preview time is outside the supported range.")?;
    let mut result = Preview {
        count: 0,
        capped: false,
        next: Vec::new(),
        now,
    };
    let mut cursor = now;
    while let Some(next) = schedule.next_after(cursor).map_err(|e| e.message)? {
        if next <= cursor {
            return Err("Schedule did not advance; preview unavailable.".into());
        }
        if next > end {
            break;
        }
        if result.count == INVOCATION_PREVIEW_LIMIT {
            result.capped = true;
            break;
        }
        result.count += 1;
        if result.next.len() < 3 {
            result.next.push(next);
        }
        cursor = next;
    }
    Ok(result)
}

pub(in crate::board) fn run(task: Task, now: i64) -> (Task, Result<Preview, String>) {
    let schedule = match &task.request {
        CronRequest::Add { schedule, zone, .. } => {
            crate::cron_service::parse_schedule_text(schedule, zone, now).map_err(|e| e.message)
        }
        CronRequest::Existing {
            op:
                super::Op::Edit {
                    schedule: Some(schedule),
                    zone,
                    ..
                },
            ..
        } => crate::cron_service::parse_schedule_text(schedule, zone, now).map_err(|e| e.message),
        _ => task
            .previous
            .clone()
            .ok_or_else(|| "The stored schedule is unavailable; nothing written.".into()),
    };
    let result = schedule.and_then(|schedule| project(&schedule, now));
    (task, result)
}

pub(in crate::board) struct Review {
    pub task: Task,
    result: Option<Result<Preview, String>>,
    clock: String,
    scroll: RefCell<ScrollState>,
    controls: Cell<[Option<Rect>; 2]>,
}
impl Review {
    fn lines(&self) -> Vec<String> {
        let mut lines = match &self.task.request {
            CronRequest::Add {
                squad,
                room_id,
                owner,
                message,
                schedule,
                zone,
                ..
            } => vec![
                format!("Create timer in {squad} · room {room_id}"),
                format!("Owner: {owner}"),
                format!("Message: {message}"),
                format!("Schedule: {schedule} · timezone {zone}"),
            ],
            CronRequest::Existing {
                key, revision, op, ..
            } => {
                let mut lines = vec![
                    format!("Timer {} in {} · room {}", key.id, key.squad, key.room_id),
                    format!("Viewed revision: {revision}; a changed timer will be refused."),
                ];
                match op {
                    super::Op::Edit {
                        message,
                        schedule,
                        zone,
                    } => {
                        lines.push(message.as_ref().map_or_else(
                            || "Message: unchanged".into(),
                            |text| format!("Message: {text}"),
                        ));
                        lines.push(schedule.as_ref().map_or_else(
                            || "Schedule: unchanged".into(),
                            |text| format!("Schedule: {text}"),
                        ));
                        lines.push(format!("Timezone: {zone}"));
                    }
                    super::Op::Reassign { owner } => {
                        lines.push(format!("Reassign owner: {owner}; pause state unchanged."))
                    }
                    super::Op::Pause => lines.push("Pause timer".into()),
                    super::Op::Resume => lines.push("Resume timer".into()),
                    super::Op::Remove => lines.push("Delete timer".into()),
                    super::Op::Send => lines.push("Send once".into()),
                }
                lines
            }
        };
        lines.push(self.clock.clone());
        let zone = match &self.task.request {
            CronRequest::Add { zone, .. }
            | CronRequest::Existing {
                op: super::Op::Edit { zone, .. },
                ..
            } => zone.clone(),
            _ => self
                .task
                .previous
                .as_ref()
                .and_then(|schedule| schedule.document()["zone"].as_str().map(str::to_owned))
                .unwrap_or_else(|| "UTC".into()),
        };
        match &self.result {
            None => lines.push("Reading scheduled invocations…".into()),
            Some(Err(error)) => {
                lines.push(format!("Preview unavailable: {error}. Nothing written."))
            }
            Some(Ok(preview)) => {
                lines.push(format!(
                    "Next 24h: {}{} scheduled invocations{}",
                    if preview.capped { "at least " } else { "" },
                    preview.count,
                    if preview.capped {
                        " (preview capped)"
                    } else {
                        ""
                    }
                ));
                lines.push("Subject to owner and clock state; not tokens or dollars.".into());
                for time in &preview.next {
                    lines.push(format!(
                        "Next: {}",
                        super::line::time(*time, preview.now, &zone)
                            .map(|time| format!("{time} · {zone}"))
                            .unwrap_or_else(|| "time unavailable".into())
                    ));
                }
                if preview.next.is_empty() {
                    lines.push("No scheduled invocation in the next 24h.".into());
                }
            }
        }
        lines
    }
    pub fn invalidate(&self) {
        self.controls.set([None, None]);
        let n = self.scroll.borrow().content();
        self.scroll.borrow_mut().update(Rect::default(), n, None);
    }
    pub fn render(&self, frame: &mut Frame, app: &App, body: Rect) {
        self.invalidate();
        let look = app.look();
        let modal = Modal {
            title: "Review timer · nothing written yet".into(),
            placement: if body.height < 8 {
                Placement::Body
            } else {
                Placement::Center
            },
        };
        let areas = modal.areas(body, [80, 22], true, true);
        modal.paint(areas, frame.buffer_mut(), &look.theme, look.depth);
        let lines: Vec<_> = self
            .lines()
            .into_iter()
            .flat_map(|line| {
                crate::board::notes::wrap(
                    &crate::board::notes::sanitize(&line),
                    usize::from(areas.content.width),
                )
            })
            .collect();
        let mut scroll = self.scroll.borrow_mut();
        scroll.update(areas.content, lines.len(), None);
        for (i, line) in lines
            .iter()
            .skip(scroll.offset())
            .take(usize::from(areas.content.height))
            .enumerate()
        {
            strip::paint_left(
                frame.buffer_mut(),
                Rect::new(
                    areas.content.x,
                    areas.content.y + i as u16,
                    areas.content.width,
                    1,
                ),
                Line::styled(line.clone(), look.role(Role::Text)),
            );
        }
        let mut controls = [None, None];
        let focused = app
            .overlay_focus
            .focused()
            .and_then(|field| field.last())
            .map(String::as_str)
            .unwrap_or("cancel");
        for (i, (name, label, x)) in [
            ("cancel", " Cancel ", 0),
            ("apply", " Apply this exact operation ", 9),
        ]
        .into_iter()
        .enumerate()
        {
            if i == 1 && !matches!(self.result, Some(Ok(_))) {
                continue;
            }
            let width = label.len() as u16;
            if areas.footer.height > 0
                && areas.footer.width >= x + width
                && areas.content.height > 0
            {
                let rect = Rect::new(areas.footer.x + x, areas.footer.y, width, 1);
                strip::paint_left(
                    frame.buffer_mut(),
                    rect,
                    Line::styled(
                        label,
                        if focused == name {
                            look.selection()
                        } else {
                            look.role(Role::Text)
                        },
                    ),
                );
                controls[i] = Some(rect);
            }
        }
        self.controls.set(controls);
    }
}
impl App {
    pub(in crate::board) fn cron_review_request(&mut self, request: CronRequest) -> Effect {
        self.cron_review_generation = self.cron_review_generation.wrapping_add(1);
        let previous = match &request {
            CronRequest::Existing { key, revision, .. } => self
                .cron
                .cron
                .as_ref()
                .and_then(|cron| {
                    cron.jobs.iter().find(|view| {
                        super::rows::key_of(view) == *key && view.job.revision == *revision
                    })
                })
                .map(|view| view.job.schedule.clone()),
            _ => None,
        };
        let task = Task {
            generation: self.cron_review_generation,
            request,
            previous,
        };
        let clock = if let Some(error) = &self.cron.failure {
            format!("clock: unknown · latest read failed: {error}")
        } else {
            self.cron
                .cron
                .as_ref()
                .map(|cron| {
                    super::line::clock(
                        &cron.clock,
                        cron.read_ms,
                        false,
                        self.cron.clock_note(self.clock_place()),
                    )
                    .0
                })
                .unwrap_or_else(|| "clock: unknown".into())
        };
        self.cron_review = Some(Review {
            task: task.clone(),
            result: None,
            clock,
            scroll: Default::default(),
            controls: Cell::new([None, None]),
        });
        Effect::Act(Request::CronPreview(Box::new(task)))
    }
    pub(in crate::board) fn cron_review_ready(
        &mut self,
        task: Task,
        result: Result<Preview, String>,
    ) {
        if let Some(review) = &mut self.cron_review
            && review.task == task
        {
            review.result = Some(result);
        }
    }
    pub(in crate::board) fn cron_review_event(
        &mut self,
        field: &tmt_tui::app::ComponentId,
        event: &Event,
    ) -> Option<Effect> {
        let review = self.cron_review.as_ref()?;
        let action = match event {
            Event::Key(key)
                if key.kind != KeyEventKind::Release
                    && !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                match key.code {
                    KeyCode::Esc => Some(0),
                    KeyCode::Enter => match field.last().map(String::as_str) {
                        Some("cancel") => Some(0),
                        Some("apply") => Some(1),
                        _ => None,
                    },
                    _ => None,
                }
            }
            Event::Mouse(mouse) if mouse.kind == MouseEventKind::Down(MouseButton::Left) => {
                review.controls.get().iter().position(|area| {
                    area.is_some_and(|area| area.contains((mouse.column, mouse.row).into()))
                })
            }
            _ => None,
        };
        if action == Some(0) {
            self.cron_review = None;
            return Some(Effect::None);
        }
        if action == Some(1) {
            if matches!(review.result, Some(Ok(_))) {
                if review.controls.get()[1].is_none() {
                    self.notice = Some("Enlarge the terminal and repaint the review before applying; nothing written.".into());
                    return Some(Effect::None);
                }
                let request = review.task.request.clone();
                self.cron_review = None;
                return Some(Effect::Act(Request::Cron(request)));
            }
            return Some(Effect::None);
        }
        if field.last().is_some_and(|s| s == "read") && review.scroll.borrow_mut().input(event) {
            return Some(Effect::None);
        }
        // Unhandled Tab belongs to the shared focus router, not this controller.
        None
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use tmt_ops::cron::ScheduleInput;
    #[test]
    fn invocation_counts_use_production_schedule_and_cap_dense_intervals() {
        for (duration, count, capped) in [
            ("30m", 48, false),
            ("5m", 288, false),
            ("1s", INVOCATION_PREVIEW_LIMIT, true),
        ] {
            let schedule = Schedule::parse(
                ScheduleInput::Every {
                    duration,
                    from: None,
                },
                "UTC",
                0,
            )
            .unwrap();
            let p = project(&schedule, 0).unwrap();
            assert_eq!((p.count, p.capped), (count, capped));
            assert_eq!(p.next.len(), 3);
            assert!(p.next.windows(2).all(|w| w[0] < w[1]));
        }
    }
    #[test]
    fn calendar_and_unknown_time_are_not_invented() {
        let now = jiff::Timestamp::from_millisecond(0)
            .unwrap()
            .as_millisecond();
        let schedule = Schedule::parse(
            ScheduleInput::At {
                time: "09:00",
                on: None,
            },
            "UTC",
            now,
        )
        .unwrap();
        assert_eq!(project(&schedule, now).unwrap().count, 1);
        assert!(project(&schedule, i64::MAX).is_err());
    }
    fn request() -> CronRequest {
        CronRequest::Add {
            actor: super::super::test_cron(vec![], tmt_ops::cron::ClockStatus::NoClock)
                .actor
                .unwrap(),
            squad: "product".into(),
            room_id: "room".into(),
            owner: "owner".into(),
            message: "do this".into(),
            schedule: "every 30m".into(),
            zone: "UTC".into(),
        }
    }
    #[test]
    fn an_unpainted_or_clipped_apply_control_cannot_submit() {
        let mut app = crate::board::app::workspace::tests::board();
        let Effect::Act(Request::CronPreview(task)) = app.cron_review_request(request()) else {
            panic!("preview")
        };
        let (task, result) = run(*task, 0);
        app.cron_review_ready(task, result);
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(20, 4)).unwrap();
        terminal
            .draw(|frame| crate::board::view::render(frame, &app))
            .unwrap();
        let enter = Event::Key(ratatui::crossterm::event::KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        ));
        let field = vec!["cron-review".into(), "apply".into()];
        assert_eq!(app.cron_review_event(&field, &enter), Some(Effect::None));
        assert!(app.cron_review.is_some());
        assert!(app.notice.as_deref().unwrap().contains("nothing written"));
    }
    #[test]
    fn failed_acquisition_does_not_present_the_retained_clock_as_current() {
        let mut app = crate::board::app::workspace::tests::board();
        app.cron.replace(Ok(super::super::test_cron(
            vec![],
            tmt_ops::cron::ClockStatus::NoClock,
        )));
        app.cron.replace(Err("clock read refused".into()));
        assert!(matches!(
            app.cron_review_request(request()),
            Effect::Act(Request::CronPreview(_))
        ));
        let review = app.cron_review.as_ref().unwrap();
        assert!(review.clock.contains("clock: unknown"));
        assert!(review.clock.contains("clock read refused"));
        assert!(!review.clock.contains("no clock"));
    }
    #[test]
    fn pending_cancellation_and_replacement_do_not_apply_or_reopen() {
        let mut app = crate::board::app::workspace::tests::board();
        let effect = app.cron_review_request(request());
        let Effect::Act(Request::CronPreview(task)) = effect else {
            panic!("preview is worker only");
        };
        let field = vec!["cron-review".into(), "apply".into()];
        let enter = Event::Key(ratatui::crossterm::event::KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        ));
        assert_eq!(app.cron_review_event(&field, &enter), Some(Effect::None));
        let esc = Event::Key(ratatui::crossterm::event::KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        ));
        assert_eq!(app.cron_review_event(&field, &esc), Some(Effect::None));
        let (old, result) = run(*task, 0);
        app.cron_review_ready(old.clone(), result.clone());
        assert!(app.cron_review.is_none());
        app.cron_review_request(request());
        app.cron_review_ready(old, result);
        assert!(
            app.cron_review.as_ref().unwrap().result.is_none(),
            "cancelled prior generation cannot fill a replacement"
        );
    }
    #[test]
    fn apply_uses_the_frozen_request_after_preview_and_failed_preview_cannot_apply() {
        let mut app = crate::board::app::workspace::tests::board();
        let frozen = request();
        let Effect::Act(Request::CronPreview(task)) = app.cron_review_request(frozen.clone())
        else {
            panic!("preview");
        };
        let (task, result) = run(*task, 0);
        app.cron_review_ready(task, result);
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 24)).unwrap();
        terminal
            .draw(|frame| crate::board::view::render(frame, &app))
            .unwrap();
        let enter = Event::Key(ratatui::crossterm::event::KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        ));
        let field = vec!["cron-review".into(), "apply".into()];
        assert_eq!(
            app.cron_review_event(&field, &enter),
            Some(Effect::Act(Request::Cron(frozen.clone())))
        );
        assert!(app.cron_review.is_none());
        let Effect::Act(Request::CronPreview(task)) = app.cron_review_request(frozen) else {
            panic!("preview");
        };
        app.cron_review_ready(*task, Err("schedule unavailable".into()));
        assert_eq!(app.cron_review_event(&field, &enter), Some(Effect::None));
    }
}

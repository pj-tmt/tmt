//! Bounded scheduled-invocation preview on the existing action lane, never paint.
use super::CronRequest;
use crate::board::app::{App, Effect, Request};
use ratatui::{Frame, crossterm::event::Event, layout::Rect, text::Line};
use std::cell::{Cell, RefCell};
use tmt_cli_style::Role;
use tmt_ops::cron::Schedule;
use tmt_tui::components::{
    Button, Dialog, DialogHits, DialogInput, DialogKind, DialogState, Placement, ScrollState, strip,
};

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
    /// The Cancel / Apply cursor; the text scrolls with ↑↓ and the page keys.
    state: DialogState,
    /// What the last paint showed; Apply is never chosen from a stale or clipped frame.
    hits: RefCell<DialogHits>,
    readable: Cell<bool>,
}
impl Review {
    fn buttons(&self) -> [Button<'static>; 2] {
        let readable = self.readable.get();
        [
            Button::secondary("Cancel").enabled(readable),
            Button::primary("Apply this exact operation")
                .enabled(readable && matches!(self.result, Some(Ok(_)))),
        ]
    }
    fn lines(&self) -> Vec<String> {
        let mut lines = match &self.task.request {
            CronRequest::Add {
                squad,
                owner,
                message,
                schedule,
                zone,
                ..
            } => vec![
                format!("Create timer in {squad}"),
                format!("Owner: {owner}"),
                format!("Message: {message}"),
                format!("Schedule: {schedule} · timezone {zone}"),
            ],
            CronRequest::Existing {
                key,
                squad,
                revision,
                op,
                ..
            } => {
                let mut lines = vec![
                    format!("Timer {} in {}", key.id, squad),
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
        *self.hits.borrow_mut() = DialogHits::default();
        self.readable.set(false);
        let n = self.scroll.borrow().content();
        self.scroll.borrow_mut().update(Rect::default(), n, None);
    }
    pub fn render(&self, frame: &mut Frame, app: &App, body: Rect) {
        self.invalidate();
        let look = app.look();
        let mut dialog = Dialog {
            title: "Review timer · nothing written yet".into(),
            kind: DialogKind::Confirm,
            placement: if body.height < 8 {
                Placement::Body
            } else {
                Placement::Center
            },
            // The labels reserve the button line; whether they are enabled follows the areas.
            buttons: self.buttons().into(),
        };
        let areas = dialog.areas(body, [80, 22], true);
        if areas.content.height == 0 {
            dialog.title = "Enlarge terminal · Esc closes timer review".into();
        }
        self.readable.set(areas.content.height > 0);
        dialog.buttons = self.buttons().into();
        let hits = crate::board::buttons::paint_dialog(
            app,
            frame.buffer_mut(),
            &dialog,
            areas,
            &self.state,
            crate::board::app::Overlay::CronReview,
        );
        app.peek_areas
            .set(crate::board::app::Overlay::CronReview, Some(areas.outer));
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
        *self.hits.borrow_mut() = hits;
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
            state: DialogState::new(&[], &[]),
            hits: Default::default(),
            readable: Cell::new(false),
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
    pub(in crate::board) fn cron_review_event(&mut self, event: &Event) -> Option<Effect> {
        let review = self.cron_review.as_mut()?;
        let buttons = review.buttons();
        let input = match event {
            Event::Key(key) => review.state.key(DialogKind::Confirm, key, &[], &buttons),
            Event::Mouse(mouse) => {
                let hits = review.hits.borrow().clone();
                review.state.mouse(mouse, &hits, &buttons)
            }
            _ => DialogInput::Ignored,
        };
        match input {
            DialogInput::Cancel | DialogInput::Choose(0) => {
                Some(self.close_overlay(crate::board::app::Overlay::CronReview))
            }
            DialogInput::Choose(_) => {
                if review
                    .hits
                    .borrow()
                    .buttons
                    .get(1)
                    .copied()
                    .flatten()
                    .is_none()
                {
                    self.set_notice(Some("Enlarge the terminal and repaint the review before applying; nothing written.".into()), false);
                    return Some(Effect::None);
                }
                let request = review.task.request.clone();
                self.cron_review = None;
                Some(Effect::Act(Request::Cron(request)))
            }
            DialogInput::Moved => Some(Effect::None),
            _ => {
                review.scroll.borrow_mut().input(event);
                Some(Effect::None)
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};
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
    fn the_review_names_the_squad_and_never_the_room_id() {
        const ROOM: &str = "6f1d2c3a-0b4e-4c55-9a7d-123456789abc";
        let actor = super::super::test_cron(vec![], tmt_ops::cron::ClockStatus::NoClock)
            .actor
            .unwrap();
        let requests = [
            CronRequest::Add {
                actor: actor.clone(),
                squad: "product".into(),
                room_id: ROOM.into(),
                owner: "owner".into(),
                message: "do this".into(),
                schedule: "every 30m".into(),
                zone: "UTC".into(),
            },
            CronRequest::Existing {
                actor,
                squad: "product".into(),
                key: crate::cron_service::JobKey {
                    room_id: ROOM.into(),
                    id: "job-1".into(),
                },
                revision: 3,
                op: super::super::Op::Pause,
            },
        ];
        for request in requests {
            let mut app = crate::board::app::workspace::tests::board();
            let Effect::Act(Request::CronPreview(task)) = app.cron_review_request(request) else {
                panic!("preview")
            };
            let (task, result) = run(*task, 0);
            app.cron_review_ready(task, result);
            let lines = app.cron_review.as_ref().unwrap().lines();
            let text = lines.join("\n");
            assert!(text.contains("in product"), "{text}");
            assert!(!text.contains(ROOM) && !text.contains("room"), "{text}");
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
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("Enlarge terminal"));
        let enter = Event::Key(ratatui::crossterm::event::KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        ));
        let right = Event::Key(ratatui::crossterm::event::KeyEvent::new(
            KeyCode::Right,
            KeyModifiers::NONE,
        ));
        // Both buttons are disabled while the review is clipped, so neither key applies.
        assert_eq!(app.cron_review_event(&right), Some(Effect::None));
        assert_eq!(app.cron_review_event(&enter), Some(Effect::None));
        assert!(app.cron_review.is_some());
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
        let enter = Event::Key(ratatui::crossterm::event::KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        ));
        assert_eq!(app.cron_review_event(&enter), Some(Effect::None));
        let esc = Event::Key(ratatui::crossterm::event::KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        ));
        assert_eq!(app.cron_review_event(&esc), Some(Effect::None));
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
    fn the_cron_review_follows_the_close_rule() {
        let open = || {
            let mut app = crate::board::app::workspace::tests::board();
            let Effect::Act(Request::CronPreview(task)) = app.cron_review_request(request()) else {
                panic!("preview");
            };
            let (task, result) = run(*task, 0);
            app.cron_review_ready(task, result);
            app
        };
        crate::board::home::tests::dialog_dismiss::assert_close_rule(
            &mut open(),
            &mut open(),
            crate::board::app::Overlay::CronReview,
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
        let right = Event::Key(ratatui::crossterm::event::KeyEvent::new(
            KeyCode::Right,
            KeyModifiers::NONE,
        ));
        assert_eq!(
            app.cron_review_event(&right),
            Some(Effect::None),
            "→ reaches Apply"
        );
        assert_eq!(
            app.cron_review_event(&enter),
            Some(Effect::Act(Request::Cron(frozen.clone())))
        );
        assert!(app.cron_review.is_none());
        let Effect::Act(Request::CronPreview(task)) = app.cron_review_request(frozen) else {
            panic!("preview");
        };
        app.cron_review_ready(*task, Err("schedule unavailable".into()));
        assert_eq!(app.cron_review_event(&enter), Some(Effect::None));
    }
}

//! Presentation state, with injected monotonic time and no terminal I/O.
use super::rate::{Input, Rate, Reading};
use crate::config::{TokenRate, TokenWindow};
use std::time::{Duration, Instant};

const FRAME: Duration = Duration::from_millis(250);
const DURATION: Duration = Duration::from_millis(600);
pub const NUMBER_WIDTH: usize = 10;

#[derive(Debug)]
struct Animation {
    from: f64,
    to: f64,
    start: Instant,
    next: Instant,
}

impl Animation {
    fn value(&self, now: Instant) -> f64 {
        let t = now.saturating_duration_since(self.start).as_secs_f64() / DURATION.as_secs_f64();
        self.from + (self.to - self.from) * (1.0 - (1.0 - t.min(1.0)).powi(3))
    }
}

#[derive(Debug)]
pub struct Meter {
    pub settings: TokenRate,
    pub room: String,
    origin: Instant,
    sampled: Instant,
    rate: Rate,
    reading: Option<Reading>,
    displayed: f64,
    animation: Option<Animation>,
    window: TokenWindow,
    trend: [Option<f64>; 8],
}

impl Meter {
    pub fn new(settings: TokenRate, input: &Input, now: Instant) -> Self {
        let mut meter = Self {
            settings,
            room: input.room.clone(),
            origin: now,
            sampled: now,
            rate: Rate::default(),
            reading: None,
            displayed: 0.0,
            animation: None,
            window: settings.window,
            trend: [None; 8],
        };
        meter.sample(Ok(input), now);
        meter
    }

    /// Leaving a tab closes continuity; retained buckets remain evidence.
    pub fn suspend(&mut self, now: Instant) {
        self.rate.failed(
            self.milliseconds(now),
            self.settings.every.as_millis() as u64,
        );
        self.animation = None;
    }

    /// Re-entering a cached tab refreshes time/coverage without a fake receipt.
    pub fn resume(&mut self, window: TokenWindow, now: Instant) {
        let ms = self.milliseconds(now);
        self.rate.failed(ms, self.settings.every.as_millis() as u64);
        self.window = window.available(self.settings.every);
        self.reading = self.rate.reading(ms, self.window);
        self.displayed = self.reading.map(|reading| reading.rate).unwrap_or(0.0);
        self.animation = None;
        self.trend = self.rate.trend(ms, self.window);
    }

    pub fn due(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.sampled) >= self.settings.every
    }

    fn milliseconds(&self, now: Instant) -> u64 {
        now.saturating_duration_since(self.origin)
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX)
    }

    pub fn sample(&mut self, input: Result<&Input, ()>, now: Instant) {
        let ms = self.milliseconds(now);
        match input {
            Ok(input) => self.rate.sample(input, ms),
            Err(()) => self.rate.failed(ms, self.settings.every.as_millis() as u64),
        }
        self.sampled = now;
        let reading = self.rate.reading(ms, self.window);
        if let Some(rate) = reading.map(|reading| reading.rate) {
            if self.reading.map(|reading| reading.rate) != Some(rate) {
                if self.settings.reduced_motion || self.displayed == rate {
                    self.displayed = rate;
                    self.animation = None;
                } else {
                    // Retarget from the value actually displayed, without a jump.
                    self.animation = Some(Animation {
                        from: self.displayed,
                        to: rate,
                        start: now,
                        next: now + FRAME,
                    });
                }
            }
        } else {
            self.animation = None;
        }
        self.reading = reading;
        self.trend = self.rate.trend(ms, self.window);
    }

    /// Only changed visible digits wake the normal renderer. No idle motion.
    pub fn tick(&mut self, now: Instant) -> bool {
        let before = self.digits();
        if let Some(animation) = &mut self.animation
            && now >= animation.next
        {
            self.displayed = animation.value(now);
            let end = animation.start + DURATION;
            if now >= end {
                self.displayed = animation.to;
                self.animation = None;
            } else {
                animation.next = (now + FRAME).min(end);
            }
        }
        before != self.digits()
    }

    pub fn wait(&self, now: Instant) -> Option<Duration> {
        self.animation
            .as_ref()
            .map(|animation| animation.next.saturating_duration_since(now))
    }

    pub fn digits(&self) -> Option<String> {
        let reading = self.reading?;
        let number = crate::source::render_value(
            &serde_json::json!(self.displayed),
            crate::source::Format::Tokens,
            0,
        )?;
        Some(format!(
            "{}{number}",
            if reading.partial { "≥" } else { "" }
        ))
    }

    /// Window selection belongs to App; this cache settles immediately on switches.
    pub fn select(&mut self, window: TokenWindow, now: Instant) {
        let window = window.available(self.settings.every);
        if self.window == window {
            return;
        }
        self.window = window;
        let ms = self.milliseconds(now);
        self.reading = self.rate.reading(ms, window);
        self.displayed = self.reading.map(|reading| reading.rate).unwrap_or(0.0);
        self.animation = None;
        self.trend = self.rate.trend(ms, window);
    }

    pub fn label(&self) -> Option<String> {
        let seconds = self.reading?.span / 1000;
        Some(if self.reading?.span == self.window.milliseconds() {
            self.window.label().into()
        } else if seconds >= 60 {
            format!("{}m", seconds / 60)
        } else {
            format!("{seconds}s")
        })
    }
    pub fn full_default(&self) -> bool {
        self.window == TokenWindow::Minute
            && self.reading.is_some_and(|reading| reading.span == 60_000)
    }

    pub fn excluded<'a>(&self, input: &'a Input) -> Vec<&'a str> {
        input
            .resumes
            .iter()
            .filter(|(id, _)| !self.rate.reporter(id))
            .map(|(id, _)| id.as_str())
            .collect()
    }

    pub fn sparkline(&self) -> String {
        const MARKS: [char; 7] = ['▂', '▃', '▄', '▅', '▆', '▇', '█'];
        let high = self.trend.iter().flatten().copied().fold(0.0_f64, f64::max);
        self.trend
            .iter()
            .map(|value| match value {
                None => ' ',
                Some(value) if *value == 0.0 => '▁',
                Some(value) => MARKS[((value / high * 6.0).round() as usize).min(6)],
            })
            .collect()
    }

    /// Step aside without dropping covered-span/non-default qualifications.
    pub fn layout(&self, available: usize) -> Option<Layout> {
        if !self.settings.enabled || self.digits()?.chars().count() > NUMBER_WIDTH {
            return None;
        }
        let label = self.label()?;
        let make = |spark: bool, show_label: bool, short: bool| Layout {
            label: show_label.then(|| label.clone()),
            spark,
            unit: if short { "/s" } else { " tok/s" },
            width: NUMBER_WIDTH
                + if show_label { 4 } else { 0 }
                + if short { 2 } else { 6 }
                + if spark { 9 } else { 0 },
        };
        [
            make(true, true, false),
            make(false, true, false),
            make(false, !self.full_default(), false),
            make(false, !self.full_default(), true),
        ]
        .into_iter()
        .find(|layout| layout.width <= available)
    }
}

#[derive(Debug)]
pub struct Layout {
    pub label: Option<String>,
    pub spark: bool,
    pub unit: &'static str,
    pub width: usize,
}

#[cfg(test)]
mod tests;

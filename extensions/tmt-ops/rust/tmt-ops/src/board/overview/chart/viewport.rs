//! Absolute bucket windows; scrolling never acquires or rebuckets evidence.
use std::ops::Range;

#[derive(Default)]
pub(super) struct Viewport {
    start: Option<usize>, // None follows the newest buckets.
    pub width: usize,
}
impl Viewport {
    pub fn reset(&mut self) {
        self.start = None;
    }
    pub fn window(&self, total: usize) -> Range<usize> {
        let count = self.width.max(1).min(total);
        let start = self
            .start
            .unwrap_or(total.saturating_sub(count))
            .min(total.saturating_sub(count));
        start..start + count
    }
    pub fn scroll(&mut self, total: usize, delta: isize) {
        let current = self.window(total);
        let newest = total.saturating_sub(current.len());
        let start = current.start.saturating_add_signed(delta).min(newest);
        self.start = (start < newest).then_some(start);
    }
    pub fn oldest(&mut self) {
        self.start = Some(0);
    }
    pub fn newest(&mut self) {
        self.start = None;
    }
    pub fn reveal(&mut self, total: usize, index: usize) {
        let window = self.window(total);
        if index >= total.saturating_sub(1) {
            self.newest();
        } else if index < window.start {
            self.start = Some(index);
        } else if index >= window.end {
            self.start = Some(index + 1 - window.len());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windows_follow_newest_resize_and_bound_absolute_navigation() {
        let mut view = Viewport {
            width: 30,
            ..Default::default()
        };
        assert_eq!(view.window(168), 138..168);
        view.scroll(168, -40);
        assert_eq!(view.window(168), 98..128);
        view.reveal(168, 20);
        assert_eq!(view.window(168), 20..50);
        view.reveal(168, 167);
        assert_eq!(view.window(168), 138..168);
        view.oldest();
        view.scroll(168, -1_000);
        assert_eq!(view.window(168), 0..30);
        view.newest();
        view.width = 200;
        assert_eq!(view.window(168), 0..168);
        view.width = 50;
        assert_eq!(view.window(168), 118..168);
        view.scroll(168, 1_000);
        assert_eq!(view.window(168), 118..168);
        view.width = 20;
        assert_eq!(
            view.window(168),
            148..168,
            "scrolling to now follows on shrink"
        );
        view.oldest();
        view.reveal(168, 167);
        view.width = 10;
        assert_eq!(
            view.window(168),
            158..168,
            "revealing now follows on shrink"
        );
    }
}

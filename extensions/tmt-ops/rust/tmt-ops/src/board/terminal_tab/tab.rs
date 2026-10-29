//! The Terminal tab's state: where each built-in-host member's stream was found, whether the tab
//! is shown, and the one open stream with the screen it last drew. Nothing here paints or reads a
//! socket; the board loop calls [`Tab::sync`] each pass, and the events the stream thread sends
//! come back through [`Tab::apply`].
use super::{
    discovery::{Address, Located},
    stream::{Stream, StreamEvent},
};
use ratatui::layout::Rect;
use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};
use tmt_pty_protocol::{Eviction, Exit, Refusal, Size};
use tmt_tui::components::terminal::{Frame as Screenful, Modes};

/// How long a member with no stream is believed before core is asked again.
const NO_STREAM_FOR_MS: u64 = 60_000;
/// How long a failed answer waits before core is asked again.
const FAILED_FOR_MS: u64 = 10_000;

/// Opens the stream for one wanted terminal; the epoch tags every event it sends back.
pub(in crate::board) type Connect = Box<dyn Fn(u64, Address) -> Stream>;

/// What the open stream is doing, which decides the line under or instead of the screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::board) enum Condition {
    /// The first attempt is `1`; a higher one is a retry after the stream was lost.
    Connecting {
        attempt: u32,
    },
    Live,
    Exited(Exit),
    Evicted(Eviction),
    Refused(Refusal),
    Lost(String),
}

impl Condition {
    /// The stream is over for now: only leaving the tab and coming back tries again.
    pub(in crate::board) fn ended(&self) -> bool {
        !matches!(self, Self::Connecting { .. } | Self::Live)
    }
}

/// What the tab shows of one terminal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::board) struct View {
    pub condition: Condition,
    /// The last screen the program drew, kept while the stream reconnects or after it ended.
    pub screen: Option<Arc<Screenful>>,
}

struct Seen {
    located: Located,
    at_ms: u64,
    /// The address is still used, but core is asked again: the stream it names may be gone.
    stale: bool,
}

struct Open {
    identity: String,
    address: Address,
    epoch: u64,
    view: View,
    /// The window this viewer last asked the terminal for; none until it types.
    window: Option<Size>,
    /// Dropping it detaches from the supervisor.
    stream: Stream,
}

#[derive(Default)]
pub(in crate::board) struct Tab {
    seen: HashMap<String, Seen>,
    shown: bool,
    connect: Option<Connect>,
    open: Option<Open>,
    epoch: u64,
    /// Members whose stream ended while shown: the next time the tab opens on one, core is asked.
    ended: HashSet<String>,
    /// Keys go to the program, byte for byte.
    typing: bool,
    /// The tab fills the board.
    zoomed: bool,
    /// Where the last frame drew the screen, which is the size the program is given.
    area: Cell<Option<Rect>>,
}

impl Tab {
    pub(in crate::board) fn set_connect(&mut self, connect: Connect) {
        self.connect = Some(connect);
    }

    /// The stream core named for `identity`, when it named one.
    pub(in crate::board) fn address(&self, identity: &str) -> Option<&Address> {
        match &self.seen.get(identity)?.located {
            Located::Stream(address) => Some(address),
            _ => None,
        }
    }

    /// Core has not answered for `identity`, or its answer is old enough to ask again.
    pub(in crate::board) fn needs_locating(&self, identity: &str, now_ms: u64) -> bool {
        self.seen.get(identity).is_none_or(|seen| {
            seen.stale || Self::recheck_in(seen, now_ms).is_some_and(|wait| wait.is_zero())
        })
    }

    /// How long until `identity`'s answer is old enough to ask again; none when it stays.
    pub(in crate::board) fn locating_wait(&self, identity: &str, now_ms: u64) -> Option<Duration> {
        self.seen
            .get(identity)
            .and_then(|seen| Self::recheck_in(seen, now_ms))
    }

    fn recheck_in(seen: &Seen, now_ms: u64) -> Option<Duration> {
        let after = match seen.located {
            Located::Stream(_) => return None,
            Located::NoStream => NO_STREAM_FOR_MS,
            Located::Failed(_) => FAILED_FOR_MS,
        };
        Some(Duration::from_millis(
            (seen.at_ms + after).saturating_sub(now_ms),
        ))
    }

    pub(in crate::board) fn located(&mut self, identity: &str, located: Located, now_ms: u64) {
        self.seen.insert(
            identity.to_owned(),
            Seen {
                located,
                at_ms: now_ms,
                stale: false,
            },
        );
    }

    pub(in crate::board) fn shown(&self) -> bool {
        self.shown
    }

    /// Asks for the terminal rather than the chat; it shows once its member has a stream.
    pub(in crate::board) fn show(&mut self) {
        self.shown = true;
    }

    /// The tab is on for `identity`: asked for, and a stream exists to show.
    pub(in crate::board) fn showing(&self, identity: &str) -> bool {
        self.shown && self.address(identity).is_some()
    }

    /// Switches between the chat and the terminal. Coming back to a terminal whose stream ended
    /// asks core again, since the program may have been started anew.
    pub(in crate::board) fn toggle(&mut self, identity: &str) {
        self.shown = !self.shown;
        if self.shown
            && self.ended.remove(identity)
            && let Some(seen) = self.seen.get_mut(identity)
        {
            seen.stale = true;
        }
    }

    /// What the loop should keep open: the terminal of `wanted`, or nothing.
    pub(in crate::board) fn sync(&mut self, wanted: Option<(&str, &Address)>) -> bool {
        let current = self
            .open
            .as_ref()
            .map(|open| (open.identity.as_str(), &open.address));
        if current == wanted {
            return false;
        }
        self.open = None;
        if let (Some((identity, address)), Some(connect)) = (wanted, &self.connect) {
            self.epoch += 1;
            self.open = Some(Open {
                identity: identity.to_owned(),
                address: address.clone(),
                epoch: self.epoch,
                view: View {
                    condition: Condition::Connecting { attempt: 1 },
                    screen: None,
                },
                window: None,
                stream: connect(self.epoch, address.clone()),
            });
        }
        // Typing and zoom belong to one stream: they end with it.
        self.typing = false;
        self.zoomed &= self.open.is_some();
        true
    }

    /// Drops the open stream so the next [`Tab::sync`] opens a fresh one: `⏎` after the stream
    /// ended. Core is asked again, since the program may have been started anew.
    pub(in crate::board) fn reattach(&mut self) {
        if let Some(open) = self.open.take() {
            self.ended.remove(&open.identity);
            if let Some(seen) = self.seen.get_mut(&open.identity) {
                seen.stale = true;
            }
        }
        self.typing = false;
    }

    pub(in crate::board) fn typing(&self) -> bool {
        self.typing
    }

    /// Starts typing when a live stream is open. True when it did.
    pub(in crate::board) fn start_typing(&mut self) -> bool {
        let live = self
            .open
            .as_ref()
            .is_some_and(|open| !open.view.condition.ended());
        self.typing = live;
        if live && let Some(open) = self.open.as_mut() {
            // The window is asked for on the next pass, once the area it has is known.
            open.window = None;
        }
        live
    }

    /// Typing ended: the window is no longer this viewer's, and nothing is resized back.
    pub(in crate::board) fn stop_typing(&mut self) {
        if !self.typing {
            return;
        }
        self.typing = false;
        if let Some(open) = self.open.as_mut() {
            open.window = None;
            open.stream.stop_typing();
        }
    }

    /// Bytes for the program; dropped by the stream while it is not attached.
    pub(in crate::board) fn send(&self, bytes: &[u8]) {
        if let Some(open) = &self.open {
            open.stream.input(bytes);
        }
    }

    /// The input modes the program has set on the screen last shown.
    pub(in crate::board) fn modes(&self) -> Modes {
        self.open
            .as_ref()
            .and_then(|open| open.view.screen.as_ref())
            .map(|screen| screen.modes)
            .unwrap_or_default()
    }

    pub(in crate::board) fn zoomed(&self) -> bool {
        self.zoomed
    }

    pub(in crate::board) fn set_zoomed(&mut self, zoomed: bool) {
        self.zoomed = zoomed;
    }

    /// Where the screen was drawn in the last frame.
    pub(in crate::board) fn area(&self) -> Option<Rect> {
        self.area.get()
    }

    pub(in crate::board) fn drew(&self, area: Rect) {
        self.area.set(Some(area));
    }

    /// While typing, the program is given the room the screen has: the size when typing began,
    /// and again whenever the area changes. A viewer that only watches never asks.
    pub(in crate::board) fn fit_window(&mut self) {
        if !self.typing {
            return;
        }
        let (Some(open), Some(area)) = (self.open.as_mut(), self.area.get()) else {
            return;
        };
        let Ok(size) = Size::new(area.width, area.height) else {
            return;
        };
        if open.window != Some(size) {
            open.window = Some(size);
            open.stream.resize(size);
        }
    }

    /// What the stream thread reported; events of a stream that was since replaced are dropped.
    pub(in crate::board) fn apply(&mut self, epoch: u64, event: StreamEvent) -> bool {
        let Some(open) = self.open.as_mut().filter(|open| open.epoch == epoch) else {
            return false;
        };
        match event {
            StreamEvent::Connecting { attempt } => {
                open.view.condition = Condition::Connecting { attempt };
            }
            StreamEvent::Screen(screen) => {
                open.view.screen = Some(screen);
                open.view.condition = Condition::Live;
            }
            StreamEvent::Exited(exit) => open.view.condition = Condition::Exited(exit),
            StreamEvent::Evicted(eviction) => open.view.condition = Condition::Evicted(eviction),
            StreamEvent::Refused(refusal) => open.view.condition = Condition::Refused(refusal),
            StreamEvent::Lost(reason) => open.view.condition = Condition::Lost(reason),
        }
        if open.view.condition.ended() {
            self.ended.insert(open.identity.clone());
            // The program is gone or the stream is over: keys are the board's again.
            self.typing = false;
        }
        true
    }

    /// The terminal of `identity` as the tab shows it, when its stream is the open one.
    pub(in crate::board) fn view(&self, identity: &str) -> Option<&View> {
        self.open
            .as_ref()
            .filter(|open| open.identity == identity)
            .map(|open| &open.view)
    }
}

#[cfg(test)]
pub(in crate::board) mod tests {
    use super::*;
    use crate::board::terminal_tab::stream::Timing;
    use std::{
        path::PathBuf,
        sync::{Arc, Mutex},
    };

    const ID: &str = "22222222-2222-4222-8222-222222222222";
    const OTHER: &str = "33333333-3333-4333-8333-333333333333";

    pub(in crate::board) fn address(name: &str) -> Address {
        Address {
            socket: PathBuf::from(format!("/nonexistent/tmt-terminal/{name}.sock")),
            pane: name.to_owned(),
        }
    }

    /// A tab whose streams attach to nothing; `opened` lists the epochs it was asked for.
    pub(in crate::board) type Opened = Arc<Mutex<Vec<(u64, Address)>>>;

    pub(in crate::board) fn tab() -> (Tab, Opened) {
        let opened = Arc::new(Mutex::new(Vec::new()));
        let record = Arc::clone(&opened);
        let mut tab = Tab::default();
        tab.set_connect(Box::new(move |epoch, address| {
            record.lock().unwrap().push((epoch, address.clone()));
            Stream::open_with(
                address,
                Timing {
                    attempts: 1,
                    ..Timing::default()
                },
                |_| {},
            )
        }));
        (tab, opened)
    }

    fn screen(text: &str) -> Arc<Screenful> {
        let mut screen = tmt_tui::components::terminal::Screen::new(10, 2);
        screen.feed(text.as_bytes());
        Arc::new(screen.frame())
    }

    #[test]
    fn a_member_is_asked_for_until_core_answers_and_again_after_the_wait() {
        let mut tab = Tab::default();
        assert!(tab.needs_locating(ID, 0));
        tab.located(ID, Located::NoStream, 1_000);
        assert!(!tab.needs_locating(ID, 1_000 + NO_STREAM_FOR_MS - 1));
        assert!(tab.needs_locating(ID, 1_000 + NO_STREAM_FOR_MS));
        assert_eq!(
            tab.locating_wait(ID, 1_000),
            Some(Duration::from_millis(NO_STREAM_FOR_MS))
        );
        tab.located(ID, Located::Failed("boom".into()), 5_000);
        assert!(!tab.needs_locating(ID, 5_000 + FAILED_FOR_MS - 1));
        assert!(tab.needs_locating(ID, 5_000 + FAILED_FOR_MS));
        assert!(tab.address(ID).is_none(), "a failure shows no tab");
        tab.located(ID, Located::Stream(address("a")), 9_000);
        assert!(!tab.needs_locating(ID, u64::MAX / 2), "a stream stays");
        assert_eq!(tab.locating_wait(ID, 9_000), None);
        assert_eq!(tab.address(ID), Some(&address("a")));
    }

    #[test]
    fn the_tab_shows_only_where_a_stream_exists() {
        let mut tab = Tab::default();
        tab.toggle(ID);
        assert!(tab.shown());
        assert!(!tab.showing(ID), "no stream known yet");
        tab.located(ID, Located::Stream(address("a")), 0);
        tab.located(OTHER, Located::NoStream, 0);
        assert!(tab.showing(ID));
        assert!(
            !tab.showing(OTHER),
            "a member without a stream shows its chat"
        );
        tab.toggle(ID);
        assert!(!tab.showing(ID));
    }

    #[test]
    fn sync_opens_replaces_and_drops_the_one_stream() {
        let (mut tab, opened) = tab();
        let (a, b) = (address("a"), address("b"));
        assert!(!tab.sync(None), "nothing wanted, nothing open");
        assert!(tab.sync(Some((ID, &a))));
        assert!(!tab.sync(Some((ID, &a))), "the same wish changes nothing");
        assert!(tab.sync(Some((OTHER, &b))));
        assert!(tab.sync(None));
        assert!(tab.view(OTHER).is_none());
        assert_eq!(
            *opened.lock().unwrap(),
            vec![(1, a), (2, b)],
            "each open gets a new epoch"
        );
    }

    #[test]
    fn events_of_a_replaced_stream_are_dropped() {
        let (mut tab, _) = tab();
        let (a, b) = (address("a"), address("b"));
        tab.sync(Some((ID, &a)));
        tab.sync(Some((OTHER, &b)));
        assert!(!tab.apply(1, StreamEvent::Screen(screen("old"))));
        assert!(tab.view(ID).is_none());
        assert_eq!(
            tab.view(OTHER).unwrap().condition,
            Condition::Connecting { attempt: 1 }
        );
        assert!(tab.apply(2, StreamEvent::Screen(screen("new"))));
        assert_eq!(tab.view(OTHER).unwrap().condition, Condition::Live);
        assert!(
            !tab.apply(9, StreamEvent::Lost("late".into())),
            "unknown epoch"
        );
    }

    #[test]
    fn the_last_screen_is_kept_while_the_stream_reconnects_and_after_it_ends() {
        let (mut tab, _) = tab();
        let a = address("a");
        tab.sync(Some((ID, &a)));
        let drawn = screen("$ ls");
        tab.apply(1, StreamEvent::Screen(drawn.clone()));
        tab.apply(1, StreamEvent::Connecting { attempt: 2 });
        let view = tab.view(ID).unwrap();
        assert_eq!(view.condition, Condition::Connecting { attempt: 2 });
        assert_eq!(view.screen.as_ref(), Some(&drawn));
        tab.apply(1, StreamEvent::Exited(Exit::Code(0)));
        let view = tab.view(ID).unwrap();
        assert_eq!(view.condition, Condition::Exited(Exit::Code(0)));
        assert_eq!(view.screen.as_ref(), Some(&drawn));
        assert!(view.condition.ended());
    }

    #[test]
    fn coming_back_to_an_ended_terminal_asks_core_again_but_keeps_the_address() {
        let (mut tab, _) = tab();
        let a = address("a");
        tab.located(ID, Located::Stream(a.clone()), 0);
        tab.toggle(ID);
        tab.sync(Some((ID, &a)));
        assert!(!tab.needs_locating(ID, 1), "a live view is not asked again");
        tab.apply(1, StreamEvent::Refused(Refusal::Full));
        assert!(!tab.needs_locating(ID, 1), "nor does the end ask by itself");
        tab.toggle(ID);
        tab.sync(None);
        tab.toggle(ID);
        assert!(tab.needs_locating(ID, 1));
        assert_eq!(tab.address(ID), Some(&a), "the chip does not flicker");
        tab.located(ID, Located::Stream(address("b")), 2);
        assert!(!tab.needs_locating(ID, 3));
        assert_eq!(tab.address(ID), Some(&address("b")));
    }
}

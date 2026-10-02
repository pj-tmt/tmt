use super::*;
use crate::theme::{Base, Depth, Paint, Role, Theme};
use std::{cell::Cell, collections::VecDeque};

#[test]
fn colorfgbg_uses_the_last_field_and_rejects_unusable_indices() {
    for index in 0..16 {
        let expected = if index <= 6 || index == 8 {
            Background::Dark
        } else {
            Background::Light
        };
        assert_eq!(colorfgbg(&format!("15;{index}")), Some(expected));
        assert_eq!(colorfgbg(&format!("0;15;{index}")), Some(expected));
    }
    assert_eq!(colorfgbg("default;default;7"), Some(Background::Light));
    assert_eq!(colorfgbg("15;default;0"), Some(Background::Dark));
    for bad in [
        "",
        "15",
        "15;",
        "15;16",
        "15;default",
        "15;default;default",
        "15;-1",
        "15;+7",
        "15; 7",
        "15;7\n",
        "15;256",
        "15;garbage",
        "a;7",
        "0;1;2;7",
    ] {
        assert_eq!(colorfgbg(bad), None, "{bad:?}");
    }
}

#[test]
fn osc11_requires_a_complete_rgb_frame_and_normalizes_channel_precision() {
    for white in [
        "f/f/f",
        "ff/ff/ff",
        "fff/fff/fff",
        "ffff/ffff/ffff",
        "F/FF/FFFF",
    ] {
        for end in ["\x07", "\x1b\\"] {
            assert_eq!(
                osc11(format!("\x1b]11;rgb:{white}{end}").as_bytes()),
                Some(Background::Light)
            );
        }
    }
    assert_eq!(osc11(b"\x1b]11;rgb:0/00/0000\x07"), Some(Background::Dark));
    assert_eq!(
        osc11(b"\x1b]11;rgb:2a2a/2c2c/3838\x1b\\"),
        Some(Background::Dark)
    );
    for bad in [
        b"garbage".as_slice(),
        b"\x1b]10;rgb:f/f/f\x07",
        b"\x1b]11;rgb:f/f/f",
        b"\x1b]11;rgb:fffff/f/f\x07",
        b"\x1b]11;rgb:/f/f\x07",
        b"\x1b]11;rgb:g/f/f\x07",
        b"\x1b]11;rgb:f/f/f/f\x07",
        b"\x1b]11;rgb:f/f/f\x07q",
        b"\x1b]11;rgb:\xff/f/f\x07",
    ] {
        assert_eq!(osc11(bad), None, "{bad:?}");
    }
}

#[test]
fn luminance_is_linear_and_weights_green_more_than_red_or_blue() {
    assert_eq!(classify(0, 0, 0), Background::Dark);
    assert_eq!(classify(65535, 65535, 65535), Background::Light);
    assert_eq!(classify(65535, 0, 0), Background::Light);
    assert_eq!(classify(0, 65535, 0), Background::Light);
    assert_eq!(classify(0, 0, 65535), Background::Dark);
    assert_eq!(classify(30000, 30000, 30000), Background::Dark);
    assert_eq!(classify(30200, 30200, 30200), Background::Light);
}

#[test]
fn only_auto_resolves_and_overrides_survive_with_the_default_unchanged() {
    assert_eq!(Theme::default().base, Base::Tmt);
    for base in Base::ALL {
        for signal in [None, Some(Background::Dark), Some(Background::Light)] {
            let requested = Theme::new(base).with(Role::Waiting, Paint::Bold);
            let resolved = requested.resolve(signal);
            let expected = if base == Base::Auto {
                if signal == Some(Background::Light) {
                    Base::TmtLight
                } else {
                    Base::Tmt
                }
            } else {
                base
            };
            assert_eq!(resolved.base, expected);
            assert_eq!(
                resolved.style(Role::Waiting, Depth::TrueColor),
                anstyle::Style::new().bold()
            );
        }
    }
    assert_eq!(
        Theme::parse("board.theme", [("base", "auto")])
            .unwrap()
            .base,
        Base::Auto
    );
    for role in Role::ALL {
        for depth in [Depth::None, Depth::Ansi16, Depth::TrueColor] {
            assert_eq!(
                Theme::new(Base::Auto).style(role, depth),
                Theme::new(Base::Tmt).style(role, depth)
            );
        }
    }
}

fn scripted(chunks: Vec<(u64, Vec<u8>)>) -> (QueryReply, Vec<Duration>) {
    let clock = Cell::new(Duration::ZERO);
    let mut chunks: VecDeque<_> = chunks.into();
    let mut waits = Vec::new();
    let reply = read_osc11(
        Duration::ZERO,
        || clock.get(),
        |wait, buffer| {
            waits.push(wait);
            let Some((millis, chunk)) = chunks.pop_front() else {
                return Ok::<_, ()>(0);
            };
            clock.set(Duration::from_millis(millis));
            assert!(chunk.len() <= buffer.len());
            buffer[..chunk.len()].copy_from_slice(&chunk);
            Ok(chunk.len())
        },
    );
    (reply, waits)
}

#[test]
fn fragmented_replies_preserve_unrelated_input_and_use_one_deadline() {
    let (reply, waits) = scripted(vec![
        (10, b"q\x1b]11;r".to_vec()),
        (40, b"gb:ff/ff/ff\x1b".to_vec()),
        (99, b"\\j".to_vec()),
    ]);
    assert_eq!(reply.background, Some(Background::Light));
    assert_eq!(reply.reply, Some(1..reply.received.len() - 1));
    assert_eq!(reply.received.first(), Some(&b'q'));
    assert_eq!(reply.received.last(), Some(&b'j'));
    assert_eq!(
        waits,
        [
            Duration::from_millis(100),
            Duration::from_millis(90),
            Duration::from_millis(60)
        ]
    );
    let (dark, _) = scripted(vec![(1, b"\x1b]11;rgb:0/0/0\x07".to_vec())]);
    assert_eq!(dark.background, Some(Background::Dark));
}

#[test]
fn exact_and_late_deadlines_never_accept_a_complete_reply() {
    for millis in [100, 101] {
        let (reply, waits) = scripted(vec![(millis, b"\x1b]11;rgb:f/f/f\x07".to_vec())]);
        assert_eq!(reply.background, None);
        assert_eq!(reply.reply, None);
        assert_eq!(waits.len(), 1);
        assert!(
            !reply.received.is_empty(),
            "late bytes remain available to the input owner"
        );
    }
    let result = read_osc11::<()>(
        Duration::ZERO,
        || QUERY_TIMEOUT,
        |_, _| panic!("expired budget must not read"),
    );
    assert!(result.received.is_empty());
}

#[test]
fn timeout_garbage_eof_error_and_byte_limit_are_bounded() {
    for chunks in [
        vec![],
        vec![(1, b"garbage".to_vec())],
        vec![(100, vec![])],
        vec![(1, b"\x1b]11;rgb:ff/ff/ff".to_vec())],
    ] {
        assert_eq!(scripted(chunks).0.background, None);
    }
    let (reply, waits) = scripted(vec![(1, vec![b'x'; REPLY_LIMIT])]);
    assert_eq!(reply.received.len(), REPLY_LIMIT);
    assert_eq!(
        waits.len(),
        1,
        "byte cap stops even when the clock has not advanced"
    );
    let error = read_osc11(
        Duration::ZERO,
        || Duration::ZERO,
        |_, _| Err::<usize, _>("I/O failed"),
    );
    assert_eq!(error.background, None);
    let invalid = read_osc11(
        Duration::ZERO,
        || Duration::ZERO,
        |_, buffer| Ok::<_, ()>(buffer.len() + 1),
    );
    assert_eq!(invalid.background, None);
}

#[test]
fn elapsed_write_time_counts_against_the_read_budget() {
    let mut wait = None;
    let reply = read_osc11(
        Duration::from_millis(500),
        || Duration::from_millis(540),
        |remaining, _| {
            wait = Some(remaining);
            Ok::<_, ()>(0)
        },
    );
    assert_eq!(wait, Some(Duration::from_millis(60)));
    assert_eq!(reply.background, None);
}

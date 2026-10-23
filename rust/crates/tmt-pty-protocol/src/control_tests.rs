use super::*;

fn round_trip<F: Frame + PartialEq + std::fmt::Debug>(value: F) {
    let mut decoder = Decoder::new();
    decoder.push(&value.encode()).unwrap();
    assert_eq!(decoder.frame::<F>().unwrap(), Some(value));
    assert_eq!(decoder.frame::<F>().unwrap(), None);
}

#[test]
fn core_controls_round_trip_separately_from_attach_frames() {
    for request in [
        Control::Inspect,
        Control::AwaitLaunch,
        Control::WaitLauncher,
        Control::LaunchReady { pane_pid: 123 },
        Control::Capture,
        Control::Publish {
            pane_pid: 123,
            key: "binding".into(),
            marker: "marker".into(),
        },
        Control::Clear {
            key: "binding".into(),
        },
        Control::Paste(b"two lines\nkept together".to_vec()),
        Control::Enter,
    ] {
        let frame = ClientMessage::Control(request);
        assert!((0x20..=0x2f).contains(&frame.kind()));
        round_trip(frame);
    }
    for answer in [
        ControlReply::State {
            supervisor_pid: 12,
            pane_pid: 123,
            key_idle_ms: None,
            marker: None,
        },
        ControlReply::State {
            supervisor_pid: 12,
            pane_pid: 123,
            key_idle_ms: Some(0),
            marker: Some("marker".into()),
        },
        ControlReply::Launched("committed-marker".into()),
        ControlReply::Applied(true),
        ControlReply::Applied(false),
        ControlReply::Screen(String::new()),
        ControlReply::Screen("visible".into()),
        ControlReply::Error("screen exceeds capture bound".into()),
    ] {
        round_trip(ServerMessage::Control(answer));
    }
}

#[test]
fn malformed_controls_never_become_input() {
    for (kind, data) in [
        (0x20, vec![1]),
        (0x21, vec![]),
        (0x22, vec![]),
        (0x23, vec![1]),
        (0x24, vec![0; MAX_PAYLOAD + 1]),
        (0x25, vec![1]),
    ] {
        assert!(ClientMessage::read(kind, &data).is_err(), "kind={kind:x}");
    }
    assert!(ServerMessage::read(0xa1, &[2]).is_err());
    assert!(ServerMessage::read(0xa0, &[0; 16]).is_err());
}

#[test]
fn screen_dimensions_are_bounded_before_allocating_a_screen() {
    assert!(Size::new(1000, 1000).is_ok());
    assert!(Size::new(1001, 1).is_err());
    assert!(Size::new(1, 1001).is_err());
    assert!(ClientMessage::read(0x03, &[0x03, 0xe9, 0, 1]).is_err());
    assert!(ServerMessage::read(0x86, &[0, 1, 0x03, 0xe9]).is_err());
}

#[test]
fn a_core_paste_is_one_bounded_frame_even_above_the_keystroke_chunk_size() {
    round_trip(ClientMessage::Control(Control::Paste(vec![
        b'x';
        MAX_PAYLOAD
    ])));
    assert!(Control::read(0x24, &vec![b'x'; MAX_PAYLOAD + 1]).is_err());
}

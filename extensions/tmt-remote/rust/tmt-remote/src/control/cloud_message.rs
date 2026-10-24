//! One finite handoff to the existing serving-owner sender. No retry or state opener.
use super::*;
use crate::cloud_command::{Answer, Request};

pub fn cloud_message(
    directory: &Path,
    raw: &[u8],
    explicit: Option<&str>,
) -> Result<Value, RemoteError> {
    message_until(
        directory,
        raw,
        explicit,
        Instant::now() + crate::limits::CLOUD_MESSAGE_CONTROL_WAIT,
    )
}
fn message_until(
    directory: &Path,
    raw: &[u8],
    explicit: Option<&str>,
    until: Instant,
) -> Result<Value, RemoteError> {
    let request = Request::read(raw)?;
    let selected = request.selected(explicit)?;
    let Some(mut stream) = connect_until(directory, Some(until))? else {
        return Err(not_running());
    };
    let unknown =
        || serde_json::to_value(request.unconfirmed(selected)).expect("fixed message answer");
    let frame = json!({"op":"cloud.message","remote":explicit,"request":canonical::base64url(raw)});
    // Once writing starts, a partial/complete submission cannot be called a proven refusal.
    if write_line_until(
        &mut stream,
        &frame,
        until.min(Instant::now() + REQUEST_WAIT),
    )
    .is_err()
    {
        return Ok(unknown());
    }
    let Some(answer) = read_line(&mut stream, &mut Vec::new(), &AtomicBool::new(false), until)
    else {
        return Ok(unknown());
    };
    if answer.get("error").is_some() {
        let Some(error) = answer["error"].as_object() else {
            return Ok(unknown());
        };
        if answer.as_object().is_none_or(|row| row.len() != 1) || error.len() != 2 {
            return Ok(unknown());
        }
        let (Some(code), Some(message)) = (error["code"].as_str(), error["message"].as_str())
        else {
            return Ok(unknown());
        };
        if code == UNSUPPORTED
            || (code == "REMOTE_INPUT_INVALID" && message == LEGACY_UNSUPPORTED_MESSAGE)
        {
            return Err(outdated_serve());
        }
        if !code.starts_with("REMOTE_") || !crate::cloud_command::fixed_code(code) {
            return Ok(unknown());
        }
        return Err(RemoteError::new(code, message));
    }
    let Ok(typed) = serde_json::from_value::<Answer>(answer.clone()) else {
        return Ok(unknown());
    };
    if typed.version != 1
        || typed.remote != selected
        || typed.target != request.target
        || typed
            .reason
            .as_deref()
            .is_some_and(|reason| !crate::cloud_command::fixed_code(reason))
    {
        return Ok(unknown());
    }
    Ok(answer)
}

pub(super) fn serve(
    frame: &Value,
    cloud: Option<&crate::cloud_pairing::service::Service>,
) -> Value {
    let result = (|| {
        let row = frame.as_object().ok_or_else(crate::operations::invalid)?;
        if row.len() != 3
            || !row
                .keys()
                .all(|key| ["op", "remote", "request"].contains(&key.as_str()))
            || !(frame["remote"].is_null() || frame["remote"].is_string())
        {
            return Err(crate::operations::invalid());
        }
        let encoded = frame["request"]
            .as_str()
            .ok_or_else(crate::operations::invalid)?;
        if encoded.len() > crate::limits::CLOUD_COMMAND_BYTES.div_ceil(3) * 4 {
            return Err(crate::operations::invalid());
        }
        let raw = canonical::base64url_decode(encoded).map_err(|_| crate::operations::invalid())?;
        if canonical::base64url(&raw) != encoded {
            return Err(crate::operations::invalid());
        }
        let request = Request::read(&raw)?;
        request.selected(frame["remote"].as_str())?;
        let cloud = cloud.ok_or_else(crate::cloud_command::upgrade)?;
        cloud.send(&raw, frame["remote"].as_str())
    })();
    result.unwrap_or_else(|error| error.json_value())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prepared_frames_do_not_raise_pairing_or_ordinary_control_limits() {
        for (op, messages, accepted) in [
            ("cloud.message", true, true),
            ("cloud.message", false, false),
            ("cloud.invite", true, false),
        ] {
            let (mut client, _peer) = UnixStream::pair().unwrap();
            let mut bytes = json!({"op":op,"request":"a".repeat(LINE_BYTES),"remote":null})
                .to_string()
                .into_bytes();
            bytes.push(b'\n');
            assert!(bytes.len() > LINE_BYTES);
            assert_eq!(
                poll_line_bounded(&mut client, &mut bytes, messages).is_ok(),
                accepted
            );
        }
    }
    #[test]
    fn maximally_escaped_prepared_message_fits_only_the_message_frame() {
        let raw = json!({"version":1,"remote":"peer","requestId":"44444444-4444-4444-8444-444444444444","createdAtMs":1,"from":{"id":"33333333-3333-4333-8333-333333333333","name":"sender"},"target":{"name":"target"},"message":"\u{0001}".repeat(crate::limits::CLOUD_MESSAGE_BYTES)}).to_string();
        assert!(raw.len() <= crate::limits::CLOUD_COMMAND_BYTES);
        let frame = json!({"op":"cloud.message","remote":null,"request":canonical::base64url(raw.as_bytes())});
        let mut bytes = serde_json::to_vec(&frame).unwrap();
        assert!(bytes.len() > LINE_BYTES);
        assert!(bytes.len() <= crate::limits::CLOUD_MESSAGE_CONTROL_BYTES);
        bytes.push(b'\n');
        let (mut stream, _peer) = UnixStream::pair().unwrap();
        let admitted = poll_line_bounded(&mut stream, &mut bytes, true)
            .unwrap()
            .unwrap();
        assert_eq!(
            serve(&admitted, None)["error"]["code"],
            "REMOTE_CLOUD_UPGRADE_REQUIRED"
        );
    }
    #[test]
    fn silent_owner_deadline_is_unconfirmed_and_closes_the_one_connection() {
        let root = Path::new("/tmp").join(format!("tmt-msg-{}", crate::store::uuid_v4().unwrap()));
        fs::create_dir(&root).unwrap();
        let socket = UnixListener::bind(root.join(SOCKET)).unwrap();
        fs::set_permissions(root.join(SOCKET), fs::Permissions::from_mode(0o600)).unwrap();
        let peer = std::thread::spawn(move || {
            let (mut stream, _) = socket.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = vec![];
            stream.read_to_end(&mut bytes).unwrap();
            assert_eq!(
                serde_json::from_slice::<Value>(&bytes).unwrap()["op"],
                "cloud.message"
            );
            socket.set_nonblocking(true).unwrap();
            assert!(
                matches!(socket.accept(),Err(error) if error.kind()==io::ErrorKind::WouldBlock)
            );
        });
        let raw=json!({"version":1,"remote":"peer","requestId":"44444444-4444-4444-8444-444444444444","createdAtMs":1,"from":{"id":"33333333-3333-4333-8333-333333333333","name":"sender"},"target":{"name":"target"},"message":"hi"}).to_string();
        let answer = message_until(
            &root,
            raw.as_bytes(),
            None,
            Instant::now() + Duration::from_millis(100),
        )
        .unwrap();
        assert_eq!(answer["state"], "unconfirmed");
        peer.join().unwrap();
        fs::remove_dir_all(root).unwrap();
    }
}

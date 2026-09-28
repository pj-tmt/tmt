use super::*;
use crate::local_service::tests::call_api_bytes;

fn capture() -> Request {
    request(
        "POST",
        "lobby/snapshots",
        json!({
            "expectedRevision":1, "operationId":NEXT_OPERATION,
            "selectedElementIds":[], "annotation":"Review the routing diagram."
        }),
    )
}

fn snapshot_request(method: &str, suffix: &str, body: Vec<u8>) -> Request {
    let mut input = request(method, "lobby", Value::Null);
    input.path = format!("/api/v1/local/whiteboard-snapshots/{NEXT_OPERATION}{suffix}");
    input.body = body;
    input
}

fn upload(pixel: [u8; 4]) -> Request {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 1600, 1000);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .add_text_chunk("Comment".into(), "INPUT-METADATA".into())
            .unwrap();
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&pixel.repeat(1600 * 1000)).unwrap();
        writer.finish().unwrap();
    }
    let mut input = snapshot_request("PUT", "/image", bytes);
    input
        .headers
        .iter_mut()
        .find(|(name, _)| name == "Content-Type")
        .unwrap()
        .1 = "image/png".into();
    input
}

fn png_response(wire: &[u8]) -> &[u8] {
    let index = wire
        .windows(4)
        .position(|part| part == b"\r\n\r\n")
        .unwrap();
    let headers = std::str::from_utf8(&wire[..index]).unwrap();
    assert!(headers.starts_with("HTTP/1.1 200"), "{headers}");
    assert!(headers.contains("Content-Type: image/png\r\n"));
    assert!(headers.contains("Cache-Control: no-store\r\n"));
    assert!(headers.contains("X-Content-Type-Options: nosniff\r\n"));
    &wire[index + 4..]
}

#[test]
fn capture_upload_read_and_retry_return_retained_content_without_dispatch() {
    let fixture = Fixture::new();
    assert!(fixture.call(save(0, OPERATION)).starts_with("HTTP/1.1 200"));
    let captured = fixture.call(capture());
    assert!(captured.starts_with("HTTP/1.1 200"), "{captured}");
    let retained = response_value(&captured);
    assert_eq!(retained["id"], NEXT_OPERATION);
    assert_eq!(retained["scene"], scene());
    assert_eq!(retained["annotation"], "Review the routing diagram.");
    let missing = fixture.call(snapshot_request("GET", "/image", vec![]));
    assert!(missing.starts_with("HTTP/1.1 404"));
    assert_eq!(response_value(&missing)["error"], "WHITEBOARD_NOT_FOUND");

    let put = call_api_bytes(upload([24, 98, 81, 255]), &fixture.paths, &test_receipt());
    let png = png_response(&put);
    assert!(!png.windows(14).any(|part| part == b"INPUT-METADATA"));
    let mut decoder = png::Decoder::new(std::io::Cursor::new(png))
        .read_info()
        .unwrap();
    assert_eq!((decoder.info().width, decoder.info().height), (1600, 1000));
    let mut pixels = vec![0; decoder.output_buffer_size().unwrap()];
    decoder.next_frame(&mut pixels).unwrap();
    assert!(
        pixels
            .chunks_exact(4)
            .all(|pixel| pixel == [24, 98, 81, 255])
    );

    let mut cleared = scene();
    cleared["elements"] = json!([]);
    let edit = fixture.call(request("PUT", "lobby", json!({"expectedRevision":1,"operationId":"33333333-3333-4333-8333-333333333333","scene":cleared})));
    assert_eq!(response_value(&edit)["revision"], 2);
    assert_eq!(response_value(&fixture.call(capture())), retained);
    assert_eq!(
        response_value(&fixture.call(snapshot_request("GET", "", vec![]))),
        retained
    );
    for operation in [
        snapshot_request("GET", "/image", vec![]),
        upload([24, 98, 81, 255]),
    ] {
        let wire = call_api_bytes(operation, &fixture.paths, &test_receipt());
        assert_eq!(png_response(&wire), png);
    }
    let conflict = fixture.call(upload([25, 98, 81, 255]));
    assert!(conflict.starts_with("HTTP/1.1 409"));
    assert_eq!(
        response_value(&conflict)["error"],
        "WHITEBOARD_IDEMPOTENCY_CONFLICT"
    );
    let observer = rusqlite::Connection::open(&fixture.paths.database).unwrap();
    let counts: (i64, i64, i64) = observer.query_row("SELECT (SELECT count(*) FROM office_whiteboard_snapshots), (SELECT count(*) FROM office_whiteboard_snapshot_images), (SELECT count(*) FROM request_attempts)", [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).unwrap();
    assert_eq!(counts, (1, 1, 0));
    let bytes: Vec<u8> = observer
        .query_row(
            "SELECT png FROM office_whiteboard_snapshot_images WHERE snapshot_id=?",
            [NEXT_OPERATION],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(bytes, png);
}

#[test]
fn new_resources_reject_unauthorized_or_malformed_writes_before_storage() {
    let fixture = Fixture::new();
    for mut input in [
        capture(),
        upload([24, 98, 81, 255]),
        snapshot_request("GET", "", vec![]),
        snapshot_request("GET", "/image", vec![]),
    ] {
        input.headers.retain(|(name, _)| name != "Authorization");
        assert!(fixture.call(input).starts_with("HTTP/1.1 401"));
    }
    for header in ["Origin", "Content-Type"] {
        for mut input in [capture(), upload([24, 98, 81, 255])] {
            input
                .headers
                .iter_mut()
                .find(|(name, _)| name == header)
                .unwrap()
                .1 = "invalid".into();
            assert!(fixture.call(input).starts_with("HTTP/1.1 403"));
        }
    }
    let mut malformed = capture();
    let mut body: Value = serde_json::from_slice(&malformed.body).unwrap();
    body["scene"] = scene();
    malformed.body = serde_json::to_vec(&body).unwrap();
    assert!(fixture.call(malformed).starts_with("HTTP/1.1 400"));
    let mut malformed = upload([24, 98, 81, 255]);
    malformed.body = b"not a png".to_vec();
    assert!(fixture.call(malformed).starts_with("HTTP/1.1 400"));
    for suffix in ["/image/extra", "/image%2fextra", "/"] {
        assert!(
            fixture
                .call(snapshot_request("PUT", suffix, vec![]))
                .starts_with("HTTP/1.1 404")
        );
    }
    assert!(
        fixture
            .call(snapshot_request("DELETE", "", vec![]))
            .starts_with("HTTP/1.1 405")
    );
    assert!(!fixture.paths.database.exists());
}

#[test]
fn snapshot_budgets_apply_only_to_exact_resource_and_method() {
    let capture = "/api/v1/local/whiteboards/lobby/snapshots";
    let image = format!("/api/v1/local/whiteboard-snapshots/{NEXT_OPERATION}/image");
    for (method, path, limit) in [
        ("POST", capture, CAPTURE_INPUT_LIMIT),
        ("PUT", image.as_str(), SNAPSHOT_PNG_LIMIT),
    ] {
        assert_eq!(input_limit(method, path), Some(limit));
        for length in [limit, limit + 1] {
            let mut bytes = format!(
                "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:1\r\nContent-Length: {length}\r\n\r\n"
            )
            .into_bytes();
            bytes.resize(bytes.len() + length, b' ');
            assert_eq!(parse_wire(&bytes).is_ok(), length == limit);
        }
        assert_eq!(input_limit("GET", path), None);
        assert_eq!(input_limit(method, &format!("{path}/extra")), None);
    }
    assert_eq!(
        input_limit("PUT", "/api/v1/local/whiteboard-snapshots/lobby/image"),
        None
    );
    assert_eq!(input_limit("POST", &image), None);
    assert_eq!(input_limit("PUT", capture), None);
}

//! Plain notice presentation from originator-owned request context. Persisted
//! notice strings and responder-authored final bodies are never parsed here.
use super::{HintKind, OriginatorHint, RequestService, Storage, current};
use crate::request_runtime::wall_time_ms;
use tmt_core::request::notification::batch::Notice;
use unicode_width::UnicodeWidthStr;

struct Fields {
    recipient: String,
    preview: Option<String>,
    result_id: String,
}

fn line(text: &str, limit: usize) -> String {
    let mut chars = text.chars();
    let mut value: String = chars
        .by_ref()
        .take(limit)
        .map(|c| {
            if c.is_control() || matches!(c, '\u{2028}' | '\u{2029}') {
                ' '
            } else {
                c
            }
        })
        .collect();
    if chars.next().is_some() {
        value.push('…');
    }
    value
}

fn fields(storage: &mut Storage, hint: &OriginatorHint, clock: impl Fn() -> u64) -> Fields {
    let context = RequestService::new(&mut *storage, clock)
        .notice_context(&hint.request_id)
        .ok()
        .flatten();
    let recipient_id = context
        .as_ref()
        .and_then(|context| context.recipient_id.as_deref())
        .or(hint.recipient_id.as_deref());
    let recipient = recipient_id
        .and_then(|id| current(storage, id).ok().flatten())
        .map(|entry| entry.identity.name)
        .unwrap_or_else(|| "recipient".into());
    let (preview, result_id) = context
        .map(|context| (context.prompt, context.result_id))
        .unwrap_or_else(|| (None, hint.request_id.clone()));
    // A request can quote its own ID, or an identity can be named after it.
    // Keep that ID solely in the generated command, even inside such previews.
    let display = |text: &str, limit| {
        line(text, limit)
            .replace(&hint.request_id, "…")
            .replace(&result_id, "…")
    };
    Fields {
        recipient: display(&recipient, 64),
        preview: preview.map(|text| display(&text, 48)),
        result_id,
    }
}

fn duration(timeout_ms: u64) -> String {
    if timeout_ms.is_multiple_of(60_000) {
        format!("{}m", timeout_ms / 60_000)
    } else if timeout_ms.is_multiple_of(1000) {
        format!("{}s", timeout_ms / 1000)
    } else {
        format!("{timeout_ms}ms")
    }
}

fn single(fields: &Fields, kind: HintKind, timeout_ms: u64) -> String {
    match (kind, &fields.preview) {
        (HintKind::Reply, Some(preview)) => format!(
            "▚ ✓ {} · {preview} · tmt result {}",
            fields.recipient, fields.result_id
        ),
        (HintKind::Reply, None) => format!(
            "[tmt] reply from {}: tmt result {}",
            fields.recipient, fields.result_id
        ),
        (HintKind::Timeout, Some(preview)) => format!(
            "▚ … {} · {preview} · no reply yet · {}",
            fields.recipient,
            duration(timeout_ms)
        ),
        (HintKind::Timeout, None) => format!(
            "[tmt] no reply yet from {} after {}; still pending",
            fields.recipient,
            duration(timeout_ms)
        ),
    }
}

pub(super) fn hint(storage: &mut Storage, hint: &OriginatorHint) -> String {
    single(
        &fields(storage, hint, wall_time_ms),
        hint.kind,
        hint.timeout_ms,
    )
}

fn block(fields: &[Fields]) -> String {
    if let [one] = fields {
        return single(one, HintKind::Reply, 0);
    }
    let name_width = fields
        .iter()
        .map(|field| field.recipient.width())
        .max()
        .unwrap_or(0);
    let previews: Vec<_> = fields
        .iter()
        .map(|field| field.preview.as_deref().unwrap_or("reply received"))
        .collect();
    let preview_width = previews.iter().map(|text| text.width()).max().unwrap_or(0);
    let mut text = format!("▚ tmt · {} updates", fields.len());
    for (field, preview) in fields.iter().zip(previews) {
        text.push_str(&format!(
            "\n  ✓ {}{}  {preview}{}  tmt result {}",
            field.recipient,
            " ".repeat(name_width - field.recipient.width()),
            " ".repeat(preview_width - preview.width()),
            field.result_id
        ));
    }
    text
}

pub(super) fn reply_batch(storage: &mut Storage, notices: &[Notice]) -> (Vec<Notice>, String) {
    let fields: Vec<_> = notices
        .iter()
        .map(|notice| {
            fields(
                storage,
                &OriginatorHint {
                    request_id: notice.request_id.clone(),
                    originator_id: String::new(),
                    recipient_id: None,
                    kind: HintKind::Reply,
                    timeout_ms: 0,
                },
                wall_time_ms,
            )
        })
        .collect();
    let frames = notices
        .iter()
        .zip(&fields)
        .map(|(notice, fields)| Notice {
            request_id: notice.request_id.clone(),
            text: single(fields, HintKind::Reply, 0),
        })
        .collect();
    (frames, block(&fields))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDirectory;
    use tmt_core::{
        identity::{Lifetime, create_or_resolve},
        request::{
            Originator, PrepareRequest, RequestKind, RequestRoute, ResponseProof, SubmitResponse,
            correlation,
        },
    };

    struct Fixture {
        _directory: TestDirectory,
        storage: Storage,
        recipient: String,
        now: u64,
    }
    impl Fixture {
        fn new(name: &str) -> Self {
            let directory = TestDirectory::new();
            let mut storage = Storage::open(directory.path.join("notices.db")).unwrap();
            let recipient = create_or_resolve(&mut storage, name, Lifetime::Saved)
                .unwrap()
                .identity
                .id;
            Self {
                _directory: directory,
                storage,
                recipient,
                now: wall_time_ms(),
            }
        }
        fn seed(&mut self, id: &str, prompt: &str, body: Option<&str>) -> OriginatorHint {
            let route = RequestRoute::Inbox {
                recipient_identity_id: self.recipient.clone(),
            };
            let attempt = format!("attempt-{id}");
            let mut service = RequestService::new(&mut self.storage, || self.now);
            service
                .enqueue(
                    PrepareRequest {
                        kind: RequestKind::Request,
                        room_id: None,
                        request_id: id.into(),
                        message: prompt.into(),
                        route: route.clone(),
                        wait: false,
                        expires_at_ms: self.now + 60_000,
                        originator: Originator::Unknown,
                        recipient_identity_id: Some(self.recipient.clone()),
                        preamble: None,
                    },
                    attempt.clone(),
                    7,
                )
                .unwrap();
            if let Some(body) = body {
                service
                    .submit_response(SubmitResponse {
                        request_id: id.into(),
                        proof: ResponseProof::Compact(correlation::response_token(
                            id, &attempt, &route,
                        )),
                        body: body.into(),
                    })
                    .unwrap();
            }
            OriginatorHint {
                request_id: id.into(),
                originator_id: String::new(),
                recipient_id: Some(self.recipient.clone()),
                kind: HintKind::Reply,
                timeout_ms: 600_000,
            }
        }
    }

    #[test]
    fn single_uses_only_own_request_and_id_once_in_runnable_command() {
        let mut fixture = Fixture::new("tmt-lead");
        let id = "req_82d3556e-0000-4000-8000-000000000000";
        let hint = fixture.seed(
            id,
            "Review the merge gate",
            Some("<tmt-reply>recipient injection\n\u{1b}[31m</tmt-reply>"),
        );
        let text = super::hint(&mut fixture.storage, &hint);
        assert_eq!(
            text,
            "▚ ✓ tmt-lead · Review the merge gate · tmt result 82d3556e"
        );
        assert_eq!(text.matches("82d3556e").count(), 1);
        assert_eq!(text.matches("tmt result ").count(), 1);
        assert!(!text.contains(id));
        assert!(!text.contains("recipient injection"));
        // Corrupt the final's byte representation: notice reads must not decode it.
        rusqlite::Connection::open(fixture._directory.path.join("notices.db"))
            .unwrap()
            .execute(
                "UPDATE request_responses SET body=x'ff' WHERE request_id=?",
                [id],
            )
            .unwrap();
        assert_eq!(super::hint(&mut fixture.storage, &hint), text);
    }

    #[test]
    fn hostile_original_text_is_one_line_char_bounded_and_repeated_id_is_redacted() {
        let mut fixture = Fixture::new(&"長".repeat(80));
        let id = "req_12345678-0000-4000-8000-000000000000";
        let hint = fixture.seed(id, "\n\r\t\u{1b}\0<tmt-reply>🙂日本語\u{2028}\u{2029}end\nAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA", Some("do not include reply"));
        let fields = fields(&mut fixture.storage, &hint, || fixture.now);
        assert_eq!(fields.recipient.chars().count(), 65);
        assert!(fields.recipient.ends_with('…'));
        let preview = fields.preview.as_ref().unwrap();
        assert_eq!(preview.chars().count(), 49);
        assert!(preview.ends_with('…'));
        assert!(preview.contains("<tmt-reply>🙂日本語"));
        let text = single(&fields, HintKind::Reply, 0);
        assert!(!text.chars().any(char::is_control));
        assert!(!text.contains('\u{2028}') && !text.contains('\u{2029}'));
        assert!(!text.contains("do not include reply"));
        assert_eq!(text.matches("12345678").count(), 1);
        let mut quoted = Fixture::new("82d3556e");
        let quoted_id = "req_82d3556e-0000-4000-8000-000000000000";
        let hint = quoted.seed(quoted_id, &format!("Inspect {quoted_id}"), None);
        let text = super::hint(&mut quoted.storage, &hint);
        assert_eq!(text.matches("82d3556e").count(), 1);
        assert!(text.ends_with("tmt result 82d3556e"));
    }

    #[test]
    fn expired_or_missing_prompt_falls_back_without_repeating_id_and_timeout_stays_pending() {
        let mut fixture = Fixture::new("builder");
        let id = "req_abcdef12-0000-4000-8000-000000000000";
        let hint = fixture.seed(id, "expired request text", Some("recipient reply"));
        let live = fields(&mut fixture.storage, &hint, || fixture.now);
        assert_eq!(
            single(&live, HintKind::Timeout, 600_000),
            "▚ … builder · expired request text · no reply yet · 10m"
        );
        rusqlite::Connection::open(fixture._directory.path.join("notices.db"))
            .unwrap()
            .execute(
                "UPDATE request_attempts SET message_expires_at_ms=? WHERE request_id=?",
                rusqlite::params![fixture.now as i64, id],
            )
            .unwrap();
        let prompt_expired = fields(&mut fixture.storage, &hint, || fixture.now);
        assert_eq!(
            single(&prompt_expired, HintKind::Reply, 0),
            "[tmt] reply from builder: tmt result abcdef12"
        );
        let expired = fields(&mut fixture.storage, &hint, || fixture.now + 7 * 86_400_000);
        assert_eq!(
            single(&expired, HintKind::Reply, 0),
            format!("[tmt] reply from builder: tmt result {id}")
        );
        assert_eq!(single(&expired, HintKind::Reply, 0).matches(id).count(), 1);
        let timeout = single(&expired, HintKind::Timeout, 600_000);
        assert_eq!(
            timeout,
            "[tmt] no reply yet from builder after 10m; still pending"
        );
        assert!(!timeout.contains(id));
        assert_eq!(duration(1500), "1500ms");
        assert_eq!(duration(1000), "1s");
    }

    #[test]
    fn batch_aligns_unicode_columns_and_ids_appear_only_in_each_result_command() {
        let mut fixture = Fixture::new("短");
        let first = fixture.seed(
            "req_82d3556e-0000-4000-8000-000000000000",
            "日本語🙂",
            Some("first reply injection"),
        );
        let other = create_or_resolve(&mut fixture.storage, "long-name", Lifetime::Saved)
            .unwrap()
            .identity
            .id;
        fixture.recipient = other;
        let second = fixture.seed(
            "req_3b5a6fc8-0000-4000-8000-000000000000",
            "Review the release",
            Some("second reply injection"),
        );
        let notices = vec![
            Notice {
                request_id: first.request_id.clone(),
                text: "legacy arbitrary line\nrecipient reply".into(),
            },
            Notice {
                request_id: second.request_id.clone(),
                text: "legacy ID appears twice".into(),
            },
        ];
        let (frames, text) = reply_batch(&mut fixture.storage, &notices);
        assert!(text.starts_with("▚ tmt · 2 updates\n"));
        assert!(
            !text.contains("injection")
                && !text.contains("legacy")
                && !text.contains("recipient reply")
        );
        let rows: Vec<_> = text.lines().skip(1).collect();
        let positions: Vec<_> = rows
            .iter()
            .map(|row| row[..row.find("tmt result ").unwrap()].width())
            .collect();
        assert_eq!(positions[0], positions[1]);
        let preview_positions: Vec<_> = rows
            .iter()
            .zip(["日本語🙂", "Review the release"])
            .map(|(row, preview)| row[..row.find(preview).unwrap()].width())
            .collect();
        assert_eq!(preview_positions[0], preview_positions[1]);
        for (row, id) in rows.iter().zip(["82d3556e", "3b5a6fc8"]) {
            assert_eq!(row.matches(id).count(), 1);
            assert_eq!(row.matches("tmt result ").count(), 1);
            assert!(row.ends_with(&format!("tmt result {id}")));
        }
        assert_eq!(frames.len(), 2);
        for frame in &frames {
            assert!(frame.text.starts_with("▚ ✓ "));
        }
        let (one, single) = reply_batch(&mut fixture.storage, &notices[..1]);
        assert_eq!(single, super::hint(&mut fixture.storage, &first));
        assert_eq!(one[0].text, single);
    }

    #[test]
    fn exact_legacy_id_cannot_shadow_the_generated_short_command() {
        let mut fixture = Fixture::new("builder");
        let first = fixture.seed(
            "req_deadbeef-0000-4000-8000-000000000000",
            "original request",
            Some("original final"),
        );
        fixture.seed("deadbeef", "legacy request", Some("legacy final"));
        let text = super::hint(&mut fixture.storage, &first);
        assert!(text.ends_with(&format!("tmt result {}", first.request_id)));
        assert_eq!(text.matches(&first.request_id).count(), 1);
        let (id, _) = RequestService::new(&mut fixture.storage, || fixture.now)
            .get_response_by_prefix("deadbeef")
            .unwrap();
        assert_eq!(id, "deadbeef", "exact IDs retain their result precedence");
    }

    #[test]
    fn render_time_collision_uses_full_id_and_prior_short_notice_can_become_ambiguous() {
        let mut fixture = Fixture::new("builder");
        let first = fixture.seed(
            "req_deadbeef-0000-4000-8000-000000000000",
            "first request",
            Some("first final"),
        );
        let short = super::hint(&mut fixture.storage, &first);
        assert!(short.ends_with("tmt result deadbeef"));
        fixture.seed(
            "req_deadbeef-0001-4000-8000-000000000000",
            "second request",
            None,
        );
        let full = super::hint(&mut fixture.storage, &first);
        assert!(full.ends_with(&format!("tmt result {}", first.request_id)));
        assert_eq!(full.matches(&first.request_id).count(), 1);
        let stored = [Notice {
            request_id: first.request_id.clone(),
            text: short,
        }];
        let (_, batch) = reply_batch(&mut fixture.storage, &stored);
        assert_eq!(batch, full);
        assert!(matches!(
            RequestService::new(&mut fixture.storage, || fixture.now)
                .get_response_by_prefix("deadbeef"),
            Err(tmt_core::request::RequestError::ResultSelection(
                tmt_core::request::ResultSelectionRejection::Ambiguous(_)
            ))
        ));
    }
}

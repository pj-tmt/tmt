use super::{request_failure, unavailable};
use crate::{
    identity_context,
    invocation::OutputMode,
    output::{Failure, after_cleanup},
};
use serde_json::json;
use std::{
    collections::HashMap,
    io::{self, Write},
    time::{Duration, Instant},
};
use tmt_adapters::{
    config::ConfigPaths, interrupt::Interrupt, request_runtime::wall_time_ms, storage::Storage,
};
use tmt_core::identity::Identity;
use tmt_core::request::{
    RequestService,
    attention::{IncomingItem, IncomingPage},
};

enum Reason {
    Messages,
    Timeout,
}

#[derive(Debug, PartialEq)]
enum Step {
    Wait(Duration),
    Finish,
}

struct Timing {
    timeout: Duration,
    debounce: Duration,
    observed: u64,
    quiet_since: Option<Duration>,
}

impl Timing {
    fn new(timeout: Duration, debounce: Duration) -> Self {
        Self {
            timeout,
            debounce,
            observed: 0,
            quiet_since: None,
        }
    }

    fn step(&mut self, now: Duration, watermark: u64) -> Step {
        if watermark == 0 {
            self.quiet_since = None;
        } else if watermark > self.observed {
            self.observed = watermark;
            self.quiet_since = Some(now);
        }
        if now >= self.timeout
            || self
                .quiet_since
                .is_some_and(|since| now.saturating_sub(since) >= self.debounce)
        {
            return Step::Finish;
        }
        let until_timeout = self.timeout.saturating_sub(now);
        let until_quiet = self
            .quiet_since
            .map(|since| since.saturating_add(self.debounce).saturating_sub(now))
            .unwrap_or(until_timeout);
        Step::Wait(
            until_timeout
                .min(until_quiet)
                .min(Duration::from_millis(250)),
        )
    }
}

fn participant_document(
    identity_id: Option<&str>,
    identities: &HashMap<String, Identity>,
) -> serde_json::Value {
    let Some(identity_id) = identity_id else {
        return serde_json::Value::Null;
    };
    match identities.get(identity_id) {
        Some(identity) => json!({
            "identityId": identity.id,
            "name": identity.name,
            "canonicalName": identity.canonical_name,
            "lifetime": identity.lifetime.as_str(),
        }),
        None => json!({"identityId": identity_id}),
    }
}

fn follow_up_commands(item: &IncomingItem, identity: &Identity) -> (String, String) {
    let request = crate::output::shell_word(&item.exchange.request_id);
    let selector = crate::output::shell_word(&identity.canonical_name);
    let qualifier = match item.kind {
        tmt_core::request::attention::IncomingKind::Request
        | tmt_core::request::attention::IncomingKind::Announcement => " --incoming",
        tmt_core::request::attention::IncomingKind::Response => "",
    };
    (
        format!("tmt x show {request}{qualifier} --identity {selector}"),
        format!(
            "tmt x ack {request}{qualifier} --revision {} --identity {selector}",
            item.exchange.revision
        ),
    )
}

fn item_document(
    item: &IncomingItem,
    identities: &HashMap<String, Identity>,
    selected_identity: &Identity,
) -> serde_json::Value {
    let (inspect_command, ack_command) = follow_up_commands(item, selected_identity);
    let mut document = json!({
        "requestId": item.exchange.request_id,
        "revision": item.exchange.revision,
        "kind": item.kind.as_str(),
        "direction": "incoming",
        "sender": participant_document(item.sender_identity_id.as_deref(), identities),
        "recipient": participant_document(item.recipient_identity_id.as_deref(), identities),
        "preparedAtMs": item.exchange.prepared_at_ms,
        "delivery": item.exchange.delivery.as_str(),
        "finalStatus": item.exchange.final_state.as_str(),
        "acknowledged": item.exchange.acknowledged,
        "settled": item.exchange.settled,
        "retentionExpiresAtMs": item.exchange.retention_expires_at_ms,
        "inspectCommand": inspect_command,
        "ackCommand": ack_command,
    });
    if let Some(room_id) = &item.exchange.room_id {
        document["roomId"] = room_id.clone().into();
    }
    document
}

fn run(
    identity: Option<String>,
    room: Option<String>,
    timeout: Duration,
    debounce: Duration,
    interrupt: &Interrupt,
    started: Instant,
) -> Result<(Identity, Reason, IncomingPage, HashMap<String, Identity>), Failure> {
    let selector = identity_context::required(identity.as_deref())?;
    let paths = ConfigPaths::discover().map_err(unavailable)?;
    let mut storage = Storage::open(&paths.database).map_err(|error| {
        Failure::storage_access(
            error,
            &paths.global_dir,
            "No exchange attention was changed.",
            "X_ERROR",
            "Could not complete the exchange operation.",
        )
    })?;
    let pending = (|| {
        let identity = identity_context::resolve(&mut storage, selector)?;
        // Resolve once; delivered work remains visible after the recipient leaves.
        let room = room
            .map(|selector| crate::room_command::resolve_history(&mut storage, &selector))
            .transpose()?;
        let room_id = room.as_ref().map(|room| room.id.as_str());
        let mut timing = Timing::new(timeout, debounce);
        loop {
            if storage
                .find_active_identity_by_id(&identity.id)
                .map_err(unavailable)?
                .is_none()
            {
                return Err(Failure::new(
                    "IDENTITY_RETIRED",
                    "The selected identity was retired while listening.",
                    3,
                ));
            }
            let watermark = RequestService::new(&mut storage, wall_time_ms)
                .incoming_watermark(&identity.id, room_id)
                .map_err(request_failure)?;
            let Step::Wait(delay) = timing.step(started.elapsed(), watermark) else {
                let page = RequestService::new(&mut storage, wall_time_ms)
                    .list_incoming(&identity.id, room_id, None, None)
                    .map_err(request_failure)?;
                let reason = if page.items.is_empty() {
                    Reason::Timeout
                } else {
                    Reason::Messages
                };
                let mut identities = HashMap::new();
                for identity_id in page.items.iter().flat_map(|item| {
                    [
                        item.sender_identity_id.as_ref(),
                        item.recipient_identity_id.as_ref(),
                    ]
                    .into_iter()
                    .flatten()
                }) {
                    if !identities.contains_key(identity_id)
                        && let Some(participant) = storage
                            .find_identity_by_id(identity_id)
                            .map_err(unavailable)?
                    {
                        identities.insert(identity_id.clone(), participant);
                    }
                }
                return Ok((identity, reason, page, identities));
            };
            interrupt
                .wait_until(Instant::now() + delay)
                .map_err(|error| {
                    Failure::new("X_ERROR", "Could not wait for incoming activity.", 1)
                        .caused_by(error)
                })?;
            if interrupt.is_interrupted() {
                return Err(Failure::new(
                    "INTERRUPTED",
                    "Interrupted while listening for incoming activity.",
                    1,
                ));
            }
        }
    })();
    after_cleanup(pending, || storage.close())
}

pub(super) fn execute(
    identity: Option<String>,
    room: Option<String>,
    timeout_seconds: f64,
    debounce_seconds: f64,
    mode: OutputMode,
) -> io::Result<u8> {
    let started = Instant::now();
    let interrupt = match Interrupt::install() {
        Ok(value) => value,
        Err(error) => {
            return Failure::new(
                "X_ERROR",
                "Could not install listener interruption handling.",
                1,
            )
            .caused_by(error)
            .publish(mode);
        }
    };
    let result = run(
        identity,
        room,
        Duration::from_secs_f64(timeout_seconds),
        Duration::from_secs_f64(debounce_seconds),
        &interrupt,
        started,
    );
    let (identity, reason, page, identities) = match result {
        Ok(value) => value,
        Err(error) => return error.publish(mode),
    };
    let mut stdout = tmt_cli_style::stream::stdout(mode.json);
    if mode.json {
        writeln!(
            stdout,
            "{}",
            json!({
                "reason": match reason { Reason::Messages => "messages", Reason::Timeout => "timeout" },
                "identityId": identity.id,
                "items": page.items.iter().map(|item| item_document(item, &identities, &identity)).collect::<Vec<_>>(),
                "nextAfter": page.next_after,
            })
        )?;
    } else if page.items.is_empty() {
        writeln!(stdout, "No incoming messages before the listener timeout.")?;
    } else {
        let terminal = stdout.terminal();
        write_incoming(&mut stdout, terminal, &identity, &page, &identities)?;
    }
    Ok(0)
}

fn write_incoming(
    output: &mut impl Write,
    terminal: tmt_cli_style::Terminal,
    identity: &Identity,
    page: &IncomingPage,
    identities: &HashMap<String, Identity>,
) -> io::Result<()> {
    use tmt_cli_style::{
        list::{self, Section},
        table::{Cell, Column, Table},
    };
    let name = |id: Option<&String>| {
        id.and_then(|value| identities.get(value))
            .map(|value| value.name.clone())
            .unwrap_or_else(|| "-".into())
    };
    // Request ids stay whole: the follow-up commands take them.
    let mut rows = Table::new(&[
        Column::Fixed,
        Column::Name,
        Column::Detail,
        Column::Fixed,
        Column::Fixed,
    ]);
    for item in &page.items {
        rows.row([
            Cell::from(item.kind.as_str()),
            Cell::from(&item.exchange.request_id),
            Cell::from(format!(
                "{} → {}",
                name(item.sender_identity_id.as_ref()),
                name(item.recipient_identity_id.as_ref())
            )),
            Cell::from(item.exchange.delivery.as_str()),
            Cell::from(item.exchange.final_state.as_str()),
        ]);
    }
    list::write(
        output,
        terminal,
        &[Section {
            title: "incoming",
            count: Some(page.items.len()),
            rows,
            note: page
                .next_after
                .map(|_| "More incoming work remains; acknowledge handled items and listen again."),
            hint: None,
        }],
    )?;
    for item in &page.items {
        let (inspect_command, ack_command) = follow_up_commands(item, identity);
        tmt_cli_style::message::hint(
            output,
            terminal,
            &format!("{inspect_command}, then {ack_command}"),
        )?;
    }
    Ok(())
}

// Source-checked command samples for the printed-command guard.
#[cfg(test)]
pub(crate) const PRINTED_HINTS: &[crate::cli_style_tests::HintSpec] = &[
    crate::cli_style_tests::HintSpec::core(
        "tmt x ack {request}{qualifier} --revision {} --identity {selector}",
        &[""],
        &[("{qualifier}", " --incoming"), ("{}", "1")],
    ),
    crate::cli_style_tests::HintSpec::core(
        "tmt x show {request}{qualifier} --identity {selector}",
        &[""],
        &[("{qualifier}", " --incoming"), ("{}", "1")],
    ),
];

#[cfg(test)]
mod tests {
    use super::{Step, Timing};
    use std::time::Duration;

    fn seconds(value: u64) -> Duration {
        Duration::from_secs(value)
    }

    #[test]
    fn trailing_debounce_resets_only_for_changed_revisions() {
        let mut timing = Timing::new(seconds(60), seconds(10));
        assert_eq!(
            timing.step(seconds(0), 1),
            Step::Wait(Duration::from_millis(250))
        );
        assert_eq!(
            timing.step(seconds(5), 1),
            Step::Wait(Duration::from_millis(250))
        );
        assert_eq!(
            timing.step(seconds(5), 2),
            Step::Wait(Duration::from_millis(250))
        );
        assert_eq!(
            timing.step(seconds(12), 3),
            Step::Wait(Duration::from_millis(250))
        );
        assert_eq!(
            timing.step(seconds(21), 3),
            Step::Wait(Duration::from_millis(250))
        );
        assert_eq!(timing.step(seconds(22), 3), Step::Finish);
    }

    #[test]
    fn hard_deadline_wins_and_empty_attention_does_not_start_debounce() {
        let mut shorter = Timing::new(seconds(5), seconds(10));
        assert!(matches!(shorter.step(seconds(0), 1), Step::Wait(_)));
        assert_eq!(shorter.step(seconds(5), 2), Step::Finish);
        let mut idle = Timing::new(seconds(15), seconds(10));
        assert!(matches!(idle.step(seconds(10), 0), Step::Wait(_)));
        assert_eq!(idle.step(seconds(15), 0), Step::Finish);
    }

    #[test]
    fn acknowledgments_do_not_restart_the_trailing_edge() {
        let mut timing = Timing::new(seconds(60), seconds(10));
        assert!(matches!(timing.step(seconds(0), 3), Step::Wait(_)));
        assert!(matches!(timing.step(seconds(5), 2), Step::Wait(_)));
        assert_eq!(timing.step(seconds(10), 2), Step::Finish);

        let mut cleared = Timing::new(seconds(20), seconds(10));
        assert!(matches!(cleared.step(seconds(0), 3), Step::Wait(_)));
        assert!(matches!(cleared.step(seconds(5), 0), Step::Wait(_)));
        assert!(matches!(cleared.step(seconds(6), 4), Step::Wait(_)));
        assert_eq!(cleared.step(seconds(16), 4), Step::Finish);
    }
    #[test]
    fn human_listener_notes_only_a_page_with_more_incoming_work() {
        use super::{HashMap, Identity, IncomingItem, IncomingPage, write_incoming};
        use tmt_core::{
            identity::Lifetime,
            request::{
                AttemptStatus,
                attention::{Exchange, FinalState, IncomingKind},
            },
        };
        let identity = Identity {
            id: "recipient-id".into(),
            name: "Recipient".into(),
            canonical_name: "recipient".into(),
            lifetime: Lifetime::Saved,
            created_at: String::new(),
            updated_at: String::new(),
        };
        let item = IncomingItem {
            exchange: Exchange {
                request_id: "request-id".into(),
                room_id: None,
                recipient_identity_id: Some(identity.id.clone()),
                prepared_at_ms: 1,
                delivery: AttemptStatus::Queued,
                final_state: FinalState::NotSubmitted,
                revision: 1,
                acknowledged: false,
                settled: false,
                retention_expires_at_ms: 10,
            },
            kind: IncomingKind::Request,
            sender_identity_id: None,
            recipient_identity_id: Some(identity.id.clone()),
        };
        let mut page = IncomingPage {
            items: vec![item],
            next_after: None,
        };
        let identities = HashMap::from([(identity.id.clone(), identity.clone())]);
        let mut complete = Vec::new();
        write_incoming(
            &mut complete,
            tmt_cli_style::Terminal::PLAIN,
            &identity,
            &page,
            &identities,
        )
        .unwrap();
        page.next_after = Some(1);
        let mut continued = Vec::new();
        write_incoming(
            &mut continued,
            tmt_cli_style::Terminal::PLAIN,
            &identity,
            &page,
            &identities,
        )
        .unwrap();
        let complete = String::from_utf8(complete).unwrap();
        let continued = String::from_utf8(continued).unwrap();
        let note = "More incoming work remains; acknowledge handled items and listen again.";
        assert!(!complete.contains(note));
        assert_eq!(continued.matches(note).count(), 1);
        assert_eq!(continued.replace(&format!("    {note}\n"), ""), complete);
        assert!(complete.contains("INCOMING 1"));
        assert!(continued.contains("INCOMING 1"));
        let follow_up = "tmt x show request-id --incoming --identity recipient";
        assert!(complete.contains(follow_up));
        assert!(continued.contains(follow_up));
    }
}

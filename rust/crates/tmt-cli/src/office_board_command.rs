//! Local Office board composition through the verified companion.

use crate::{
    invocation::{
        BoardActorSelection, BoardCategorySelection, ContentInput, OfficeBoardOperation,
        OfficeOperation, OutputMode,
    },
    office_pairing_command::resolve_identity,
    output::{Failure, after_cleanup},
};
use serde_json::{Value, json};
use std::{
    io::{self, Write},
    path::Path,
    time::{Duration, Instant},
};
use tmt_adapters::{
    config::ConfigPaths,
    office_companion::invoke_office_board,
    response_input::{read_file, read_stdin},
    storage::Storage,
};
use tmt_office_model::office_protocol::OfficeInvocation;

pub fn run(executable: &Path, operation: OfficeOperation, mode: OutputMode) -> Result<u8, Failure> {
    let OfficeOperation::Board(operation) = operation else {
        return Err(Failure::new(
            "USAGE_ERROR",
            "Expected an Office board operation.",
            1,
        ));
    };
    let (invocation, input, mutation_id) = match operation {
        OfficeBoardOperation::Post {
            category,
            actor,
            title,
            body,
            operation_id,
        } => {
            let operation_id = operation_id.unwrap_or_else(tmt_core::operation::new_operation_id);
            (
                OfficeInvocation::BoardPost,
                json!({"category":category_value(category)?,"actor":actor_value(actor)?,"title":title,"body":body_value(body)?,"operationId":operation_id.clone()}),
                Some(operation_id),
            )
        }
        OfficeBoardOperation::Reply {
            thread_id,
            actor,
            body,
            operation_id,
        } => {
            let operation_id = operation_id.unwrap_or_else(tmt_core::operation::new_operation_id);
            (
                OfficeInvocation::BoardReply,
                json!({"threadId":thread_id,"actor":actor_value(actor)?,"body":body_value(body)?,"operationId":operation_id.clone()}),
                Some(operation_id),
            )
        }
        OfficeBoardOperation::Edit {
            entry_id,
            actor,
            title,
            body,
            if_revision,
            operation_id,
        } => {
            if title.is_none() && body.is_none() {
                return Err(Failure::new(
                    "BOARD_INVALID",
                    "Edit requires --title, --body, or --file.",
                    1,
                ));
            }
            let operation_id = operation_id.unwrap_or_else(tmt_core::operation::new_operation_id);
            (
                OfficeInvocation::BoardEdit,
                json!({"entryId":entry_id,"actor":actor_value(actor)?,"title":title,"body":body.map(body_value).transpose()?,"ifRevision":if_revision,"operationId":operation_id.clone()}),
                Some(operation_id),
            )
        }
        OfficeBoardOperation::Delete {
            entry_id,
            actor,
            moderate,
            if_revision,
            operation_id,
        } => {
            let operation_id = operation_id.unwrap_or_else(tmt_core::operation::new_operation_id);
            (
                OfficeInvocation::BoardDelete,
                json!({"entryId":entry_id,"actor":actor_value(actor)?,"moderate":moderate,"ifRevision":if_revision,"operationId":operation_id.clone()}),
                Some(operation_id),
            )
        }
        OfficeBoardOperation::List {
            category,
            view,
            author_id,
            owner,
            since,
            limit,
            cursor,
        } => (
            OfficeInvocation::BoardList,
            json!({"category":category_value(category)?,"view":view,"author":if owner{Some(json!({"kind":"owner"}))}else{author_id.map(|id|json!({"kind":"identity","identityId":id}))},"sinceMs":since.map(|v|parse_rfc3339(&v)).transpose()?,"limit":limit,"cursor":cursor}),
            None,
        ),
        OfficeBoardOperation::Show {
            thread_id,
            reply_limit,
            reply_cursor,
        } => (
            OfficeInvocation::BoardShow,
            json!({"threadId":thread_id,"replyLimit":reply_limit,"replyCursor":reply_cursor}),
            None,
        ),
    };
    let bytes = serde_json::to_vec(&input).map_err(io_failure)?;
    let result = invoke_office_board(
        executable,
        invocation,
        &bytes,
        Instant::now() + Duration::from_secs(30),
    )
    .map_err(|error| {
        if error.kind() == io::ErrorKind::Unsupported {
            Failure::new(
                "OFFICE_INCOMPATIBLE",
                "The installed Office companion does not support the local discussion board. Update Office before retrying.",
                1,
            )
            .caused_by(error)
        } else if let Some(operation_id) = mutation_id.as_deref() {
            uncertain_mutation(operation_id).caused_by(error)
        } else {
            io_failure(error)
        }
    })?
    .map_err(|code| {
        if code == tmt_office_model::office_board::BoardErrorCode::Storage
            && let Some(operation_id) = mutation_id.as_deref()
        {
            uncertain_mutation(operation_id)
        } else {
            board_failure(code)
        }
    })?;
    if mode.json {
        writeln!(io::stdout().lock(), "{result}").map_err(io_failure)?;
    } else {
        write_plain(&result)?;
    }
    Ok(0)
}

fn body_value(input: ContentInput) -> Result<String, Failure> {
    match input {
        ContentInput::Inline(value) => Ok(value),
        ContentInput::File(path) => read_file(Path::new(&path)).map_err(input_failure),
        ContentInput::Stdin => read_stdin().map_err(input_failure),
    }
}
fn actor_value(actor: BoardActorSelection) -> Result<Value, Failure> {
    match actor {
        BoardActorSelection::Owner => Ok(json!({"kind":"owner"})),
        BoardActorSelection::Identity(selector) => {
            let identity = resolve_identity(selector.as_deref())?;
            Ok(json!({"kind":"identity","identityId":identity.id,"name":identity.name}))
        }
    }
}
fn category_value(category: BoardCategorySelection) -> Result<Value, Failure> {
    match category {
        BoardCategorySelection::General => Ok(json!({"kind":"general"})),
        BoardCategorySelection::Room(selector) => {
            let paths = ConfigPaths::discover().map_err(|error| {
                Failure::new("CONFIG_ERROR", "Could not resolve configuration paths.", 1)
                    .caused_by(error)
            })?;
            let mut storage = Storage::open(paths.database)
                .map_err(|error| crate::room_command::failure(error.into()))?;
            // Category resolution is classification, not permission to create a
            // new thread. The board transaction fences new posts after replay.
            let room = crate::room_command::resolve_history(&mut storage, &selector);
            after_cleanup(room, || storage.close())
                .map(|room| json!({"kind":"room","roomId":room.id}))
        }
        BoardCategorySelection::Repository(remote) => {
            let cwd = std::env::current_dir().map_err(io_failure)?;
            let id =
                tmt_adapters::repository_remote::resolve_remote(&cwd, &remote).map_err(|e| {
                    Failure::new(
                        "BOARD_INVALID",
                        "The selected Git remote is not a valid repository category.",
                        1,
                    )
                    .caused_by(e)
                })?;
            Ok(json!({"kind":"repository","repositoryId":id}))
        }
    }
}
fn write_plain(value: &Value) -> Result<(), Failure> {
    let mut stdout = io::stdout().lock();
    write_plain_to(&mut stdout, value).map_err(io_failure)
}

fn write_plain_to(output: &mut impl Write, value: &Value) -> io::Result<()> {
    if let Some(id) = value.get("entryId").and_then(Value::as_str) {
        write!(
            output,
            "{id} revision {} operation {}",
            value.get("revision").and_then(Value::as_u64).unwrap_or(0),
            value
                .get("operationId")
                .and_then(Value::as_str)
                .unwrap_or("-")
        )?;
        for key in ["created", "changed", "deleted", "moderated"] {
            if let Some(flag) = value.get(key).and_then(Value::as_bool) {
                write!(output, " {key}={flag}")?;
            }
        }
        return writeln!(output);
    }
    if let Some(threads) = value.get("threads").and_then(Value::as_array) {
        return crate::output::table::write(
            output,
            ["ID", "AUTHOR", "REVISION", "REPLIES", "TITLE"],
            threads.iter().map(|thread| {
                [
                    text_field(thread, "id"),
                    author_label(thread),
                    number_field(thread, "revision"),
                    number_field(thread, "replyCount"),
                    text_field(thread, "title"),
                ]
            }),
        );
    }
    if let Some(thread) = value.get("thread") {
        let replies = value
            .get("replies")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        write_entry(output, "thread", thread)?;
        for reply in &replies {
            writeln!(output)?;
            write_entry(output, "reply", reply)?;
        }
        return Ok(());
    }
    serde_json::to_writer_pretty(&mut *output, value)?;
    writeln!(output)
}

fn write_entry(output: &mut impl Write, kind: &str, entry: &Value) -> io::Result<()> {
    writeln!(
        output,
        "{kind} {} · {} · revision {}",
        text_field(entry, "id"),
        author_label(entry),
        number_field(entry, "revision")
    )?;
    if entry.get("deleted").and_then(Value::as_bool) == Some(true) {
        return writeln!(output, "[deleted]");
    }
    if let Some(title) = entry.get("title").and_then(Value::as_str) {
        writeln!(output, "title: {title}")?;
    }
    if let Some(body) = entry.get("body").and_then(Value::as_str) {
        writeln!(output, "{body}")?;
    }
    Ok(())
}
fn text_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("-")
        .to_owned()
}
fn number_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_u64)
        .map_or_else(|| "-".into(), |v| v.to_string())
}
fn author_label(value: &Value) -> String {
    match value.get("author") {
        Some(author) if author.get("kind").and_then(Value::as_str) == Some("owner") => {
            "owner".into()
        }
        Some(author) => author
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("identity")
            .to_owned(),
        None => "-".into(),
    }
}
fn board_failure(code: tmt_office_model::office_board::BoardErrorCode) -> Failure {
    let exit = if matches!(
        code,
        tmt_office_model::office_board::BoardErrorCode::ThreadNotFound
            | tmt_office_model::office_board::BoardErrorCode::EntryNotFound
    ) {
        3
    } else {
        1
    };
    Failure::new(code.code(), code.code().replace('_', " "), exit)
}
fn uncertain_mutation(operation_id: &str) -> Failure {
    Failure::new(
        "OFFICE_LOCAL_UNCERTAIN",
        format!(
            "Office did not confirm the board mutation. Retry with --operation-id {operation_id}."
        ),
        1,
    )
}
fn input_failure(error: tmt_adapters::response_input::ResponseInputError) -> Failure {
    Failure::new("BOARD_INVALID", "Could not read exact board content.", 1).caused_by(error)
}
fn io_failure(error: impl std::error::Error + 'static) -> Failure {
    Failure::new("OFFICE_IO_ERROR", "Could not access local Office board.", 1).caused_by(error)
}

fn parse_rfc3339(value: &str) -> Result<u64, Failure> {
    let (date, time) = value.split_once('T').ok_or_else(invalid_time)?;
    if date.len() != 10
        || date.as_bytes().get(4) != Some(&b'-')
        || date.as_bytes().get(7) != Some(&b'-')
        || !date
            .bytes()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit())
    {
        return Err(invalid_time());
    }
    let mut d = date.split('-');
    let y: i64 = d
        .next()
        .and_then(|v| v.parse().ok())
        .ok_or_else(invalid_time)?;
    let m: u32 = d
        .next()
        .and_then(|v| v.parse().ok())
        .ok_or_else(invalid_time)?;
    let day: u32 = d
        .next()
        .and_then(|v| v.parse().ok())
        .ok_or_else(invalid_time)?;
    if d.next().is_some() || !(1..=12).contains(&m) || day == 0 || day > days_in_month(y, m) {
        return Err(invalid_time());
    }
    let (clock, offset) = if let Some(v) = time.strip_suffix('Z') {
        (v, 0)
    } else {
        let pos = time
            .rfind(['+', '-'])
            .filter(|p| *p >= 8)
            .ok_or_else(invalid_time)?;
        let sign = if time.as_bytes()[pos] == b'+' { 1 } else { -1 };
        let zone = &time[pos + 1..];
        if zone.len() != 5
            || zone.as_bytes().get(2) != Some(&b':')
            || !zone
                .bytes()
                .enumerate()
                .all(|(index, byte)| index == 2 || byte.is_ascii_digit())
        {
            return Err(invalid_time());
        }
        let (zh, zm) = zone.split_once(':').ok_or_else(invalid_time)?;
        let zh: i64 = zh.parse().map_err(|_| invalid_time())?;
        let zm: i64 = zm.parse().map_err(|_| invalid_time())?;
        if zh > 23 || zm > 59 {
            return Err(invalid_time());
        }
        (&time[..pos], sign * (zh * 60 + zm))
    };
    let (whole, fraction) = match clock.split_once('.') {
        Some((whole, fraction)) if !fraction.is_empty() => (whole, Some(fraction)),
        Some(_) => return Err(invalid_time()),
        None => (clock, None),
    };
    if whole.len() != 8
        || whole.as_bytes().get(2) != Some(&b':')
        || whole.as_bytes().get(5) != Some(&b':')
        || !whole
            .bytes()
            .enumerate()
            .all(|(index, byte)| matches!(index, 2 | 5) || byte.is_ascii_digit())
    {
        return Err(invalid_time());
    }
    let mut t = whole.split(':');
    let h: i64 = t
        .next()
        .and_then(|v| v.parse().ok())
        .ok_or_else(invalid_time)?;
    let min: i64 = t
        .next()
        .and_then(|v| v.parse().ok())
        .ok_or_else(invalid_time)?;
    let sec: i64 = t
        .next()
        .and_then(|v| v.parse().ok())
        .ok_or_else(invalid_time)?;
    if t.next().is_some()
        || h > 23
        || min > 59
        || sec > 59
        || fraction.is_some_and(|value| !value.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(invalid_time());
    }
    let mut millis = 0;
    for (i, b) in fraction.unwrap_or_default().bytes().take(3).enumerate() {
        millis += i64::from(b - b'0') * [100, 10, 1][i];
    }
    let days = days_from_civil(y, m, day);
    let total = (days * 86_400 + h * 3600 + min * 60 + sec - offset * 60)
        .checked_mul(1000)
        .and_then(|v| v.checked_add(millis))
        .filter(|v| *v >= 0 && *v <= tmt_core::limits::MAX_JS_SAFE_INTEGER as i64)
        .ok_or_else(invalid_time)?;
    Ok(total as u64)
}
fn invalid_time() -> Failure {
    Failure::new(
        "BOARD_INVALID",
        "--since must be a valid RFC3339 timestamp.",
        1,
    )
}
fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) => 29,
        2 => 28,
        _ => 0,
    }
}
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = y - i64::from(m <= 2);
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = i64::from(m) + if m > 2 { -3 } else { 9 };
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

#[cfg(test)]
mod tests {
    use super::{parse_rfc3339, write_plain_to};
    use serde_json::json;

    #[test]
    fn rfc3339_requires_canonical_field_widths_and_nonempty_fraction() {
        for value in [
            "1970-01-01T00:00:00Z",
            "1970-01-01T00:00:00.1Z",
            "1970-01-01T01:02:03.123456789+01:00",
        ] {
            assert!(parse_rfc3339(value).is_ok(), "expected acceptance: {value}");
        }
        for value in [
            "1-1-1T1:1:1Z",
            "1970-1-01T00:00:00Z",
            "1970-01-1T00:00:00Z",
            "1970-01-01T0:00:00Z",
            "1970-01-01T00:0:00Z",
            "1970-01-01T00:00:0Z",
            "1970-01-01T00:00:00.Z",
            "1970-01-01T00:00:00+1:00",
            "1970-01-01T00:00:00+01:0",
        ] {
            assert!(parse_rfc3339(value).is_err(), "expected rejection: {value}");
        }
    }

    #[test]
    fn plain_receipts_include_operation_id_and_show_preserves_multiline_content() {
        let mut output = Vec::new();
        write_plain_to(
            &mut output,
            &json!({"entryId":"entry","revision":2,"operationId":"op","changed":true,"deleted":false}),
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "entry revision 2 operation op changed=true deleted=false\n"
        );

        let mut output = Vec::new();
        write_plain_to(
            &mut output,
            &json!({
                "thread":{"id":"thread","author":{"kind":"identity","name":"Alice"},"revision":2,"deleted":false,"title":"Subject","body":"first\nsecond"},
                "replies":[{"id":"reply","author":{"kind":"owner"},"revision":3,"deleted":true}]
            }),
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "thread thread · Alice · revision 2\ntitle: Subject\nfirst\nsecond\n\nreply reply · owner · revision 3\n[deleted]\n"
        );
    }
}

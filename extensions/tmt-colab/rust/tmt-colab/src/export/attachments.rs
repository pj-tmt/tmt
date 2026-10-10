//! The attachments an export lists, each read at export time under current authority. An entry
//! is `included` only with the exact verified bytes; otherwise it says `missing` or
//! `unavailable` and a named reason, and nothing of the object is disclosed. The browser builds
//! the same entries (`typescript/app/src/export.ts`); `vectors/export-v1.json` pins both.
use crate::{Result, keyring::Layout, page::Fault};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use tmt_colab_model::attachment::{AttachmentSelector, Descriptor};

/// All included attachment bytes of one export never exceed this many.
pub const TOTAL_BYTES: usize = 64 * 1024 * 1024;
/// One export spends at most this long reading attachments; later entries are unavailable.
pub const BUDGET: Duration = Duration::from_secs(120);

/// Where an export gets verified attachment bytes. Only the serve holds the object channel, so
/// the CLI reads through it and a stopped serve leaves every entry unavailable.
pub trait Source {
    fn read(&self, page: &str, selector: &AttachmentSelector) -> Result<Vec<u8>>;
}
/// Reads through the owned serve socket of this data root.
pub struct Serve<'a>(pub &'a Layout);
impl Source for Serve<'_> {
    fn read(&self, page: &str, selector: &AttachmentSelector) -> Result<Vec<u8>> {
        Ok(crate::attachments::ipc::read(self.0, page, selector)?.bytes)
    }
}

/// No serve to ask: every attachment is unavailable.
pub struct Unserved;
impl Source for Unserved {
    fn read(&self, _: &str, _: &AttachmentSelector) -> Result<Vec<u8>> {
        Err(Fault::Unavailable.into())
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentRow {
    attachment_id: String,
    source: &'static str,
    reference: AttachmentSelector,
    filename: String,
    media_type: String,
    plaintext_bytes: String,
    state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    file: Option<String>,
}
#[cfg(test)]
impl AttachmentRow {
    /// A row as the shared vector lists it, to pin the serialized bytes.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn fixture(
        attachment_id: &str,
        source: &'static str,
        reference: AttachmentSelector,
        (filename, media_type): (&str, &str),
        plaintext_bytes: &str,
        state: &'static str,
        reason: Option<&'static str>,
        sha256: Option<String>,
        file: Option<String>,
    ) -> Self {
        Self {
            attachment_id: attachment_id.into(),
            source,
            reference,
            filename: filename.into(),
            media_type: media_type.into(),
            plaintext_bytes: plaintext_bytes.into(),
            state,
            reason,
            sha256,
            file,
        }
    }
}
/// What the entries need beyond the view: the page revision a document attachment is fenced by
/// and the epoch whose messages the export covers.
pub(crate) struct GatherScope<'a> {
    pub space: &'a str,
    pub page: &'a str,
    pub epoch: u64,
    pub revision: &'a str,
    /// The most included bytes, and the longest time, one export spends on attachments.
    pub total_bytes: usize,
    pub budget: Duration,
}
/// The decoded page the entries come from: its metadata and each writer's `own` projection.
pub(crate) struct PageParts<'a> {
    pub meta: &'a Value,
    pub own: &'a BTreeMap<String, Value>,
}
struct Candidate {
    source: &'static str,
    reference: AttachmentSelector,
    descriptor: Descriptor,
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
/// Document attachments in list order, then the live messages of the current epoch ordered by
/// writer, message and revision. Superseded and deleted messages list nothing.
fn candidates(page: &PageParts<'_>, scope: &GatherScope<'_>) -> Result<Vec<Candidate>> {
    let mut found = Vec::new();
    let mut add = |source,
                   list: Option<&Value>,
                   make: &dyn Fn(String, String) -> AttachmentSelector|
     -> Result<()> {
        for value in list.and_then(Value::as_array).into_iter().flatten() {
            let descriptor = Descriptor::from_json(&serde_json::to_vec(value)?)?;
            if descriptor.space != scope.space || descriptor.page != scope.page {
                return Err(Fault::Invalid.into());
            }
            found.push(Candidate {
                source,
                reference: make(descriptor.attachment_id.clone(), hex(&descriptor.hash()?)),
                descriptor,
            });
        }
        Ok(())
    };
    add(
        "document",
        page.meta.get("attachments"),
        &|attachment_id, descriptor_hash| AttachmentSelector::DocumentCurrent {
            attachment_id,
            descriptor_hash,
            content_revision: scope.revision.to_owned(),
        },
    )?;
    let epoch = scope.epoch.to_string();
    for (writer, roots) in page.own {
        let Some(messages) = roots["messages"].as_object() else {
            continue;
        };
        let mut live: Vec<(&str, u64, &Value)> = Vec::new();
        for (key, message) in messages {
            let (id, revision) = key.split_once(':').ok_or(Fault::Invalid)?;
            if message["kind"] == "comment"
                && message["deleted"] == false
                && message["senderDevice"] == *writer
                && message["messageId"] == id
                && message["revision"] == revision
                && message["spaceId"] == scope.space
                && message["pageId"] == scope.page
                && message["epoch"].as_str() == Some(epoch.as_str())
            {
                live.push((id, revision.parse().map_err(|_| Fault::Invalid)?, message));
            }
        }
        live.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
        for (id, revision, message) in live {
            add(
                "message",
                message.get("attachments"),
                &|attachment_id, descriptor_hash| AttachmentSelector::Message {
                    writer_id: writer.clone(),
                    message_id: id.to_owned(),
                    message_revision: revision.to_string(),
                    attachment_id,
                    descriptor_hash,
                },
            )?;
        }
    }
    Ok(found)
}
/// Why a read did not return bytes, as the manifest names it.
fn outcome(error: &(dyn std::error::Error + 'static)) -> (&'static str, Option<&'static str>) {
    let code = error
        .downcast_ref::<crate::page::ipc::WriteError>()
        .map(|failure| failure.code())
        .or_else(|| error.downcast_ref::<Fault>().map(Fault::code));
    match code {
        Some("COLAB_STATE_MISSING") => ("missing", None),
        Some("COLAB_DENIED") | Some("COLAB_PAGE_INACTIVE") => ("unavailable", Some("denied")),
        Some("COLAB_STALE_BASE") => ("unavailable", Some("changed")),
        _ => ("unavailable", Some("unavailable")),
    }
}
/// Every listed attachment, and the bytes of the included ones keyed by their file path.
pub(crate) struct Gathered {
    pub entries: Vec<AttachmentRow>,
    pub files: Vec<(String, Vec<u8>)>,
}
pub(crate) fn gather(
    page: &PageParts<'_>,
    scope: &GatherScope<'_>,
    source: &dyn Source,
    started: Instant,
) -> Result<Gathered> {
    let mut entries = Vec::new();
    let mut files = Vec::new();
    let mut total = 0usize;
    for Candidate {
        source: kind,
        reference,
        descriptor,
    } in candidates(page, scope)?
    {
        let mut entry = AttachmentRow {
            attachment_id: descriptor.attachment_id.clone(),
            source: kind,
            reference: reference.clone(),
            filename: descriptor.filename.clone(),
            media_type: descriptor.media_type.clone(),
            plaintext_bytes: descriptor.plaintext_bytes.clone(),
            state: "unavailable",
            reason: Some("unavailable"),
            sha256: None,
            file: None,
        };
        let declared: usize = descriptor
            .plaintext_bytes
            .parse()
            .map_err(|_| Fault::Invalid)?;
        if total.saturating_add(declared) > scope.total_bytes {
            entry.reason = Some("too-large");
        } else if started.elapsed() < scope.budget {
            match source.read(scope.page, &reference) {
                Ok(bytes) if bytes.len() == declared => {
                    total += declared;
                    entry.state = "included";
                    entry.reason = None;
                    entry.sha256 = Some(hex(&tmt_colab_model::crypto::digest(&bytes)));
                    let path = format!("attachments/{}", descriptor.attachment_id);
                    entry.file = Some(path.clone());
                    files.push((path, bytes));
                }
                // A length that disagrees with the descriptor is never disclosed.
                Ok(_) => {}
                Err(error) => (entry.state, entry.reason) = outcome(error.as_ref()),
            }
        }
        entries.push(entry);
    }
    Ok(Gathered { entries, files })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::cell::RefCell;

    const SPACE: &str = "4kph3kmtxo7dinlvoixpw642ozfibd2w";
    const PAGE: &str = "00000000-0000-4000-8000-000000000002";
    const WRITER: &str = "00000000-0000-4000-8000-000000000012";
    const REVISION: &str = "v1:0b49b9b4";
    fn id(n: u32) -> String {
        format!("00000000-0000-4000-8000-{n:012}")
    }
    /// A structurally valid descriptor of `bytes` plaintext bytes, from the shared vector.
    fn descriptor(n: u32, source: Value, namespace: &str, bytes: usize) -> Value {
        let mut d = json!({
            "version": 1, "attachmentId": id(n), "space": SPACE, "page": PAGE, "epoch": "1",
            "namespace": namespace,
            "objectId": "01".repeat(32), "authorDevice": WRITER, "membershipRevision": "2",
            "source": source,
            "envelopeHash": "34877068c80ec8aacf400683743d1a3a95855f1c87897ca5f95828093b3f7ba1",
            "signature": "mTOaOQ2Qzt_WcbjBQQt2iUqkblc1b2FAk206Z0JlpWnOrVJLxRZU46fBAScdhWr9MWLORFA6BOiYXOphXcbyBA",
            "payloadSha256": "30c7ae16aac3cf425b9ebde44e837a42ac05a1c7bcd07e57214ab11cef887d39",
            "payloadBytes": (bytes + 16).to_string(), "plaintextBytes": bytes.to_string(),
            "filename": format!("file-{n}.bin"), "mediaType": "application/octet-stream",
        });
        d["objectId"] = json!(format!("{n:02x}").repeat(32));
        d
    }
    fn document(n: u32, bytes: usize) -> Value {
        descriptor(
            n,
            json!({"kind": "document", "sourceDigest": "21".repeat(32)}),
            "content",
            bytes,
        )
    }
    fn message(n: u32, message: u32, bytes: usize) -> Value {
        descriptor(
            n,
            json!({"kind": "message", "writerId": WRITER, "messageId": id(message), "messageRevision": "1"}),
            "own",
            bytes,
        )
    }
    fn record(message_id: u32, revision: u32, attachments: Vec<Value>) -> Value {
        json!({
            "kind": "comment", "deleted": false, "senderDevice": WRITER,
            "messageId": id(message_id), "revision": revision.to_string(),
            "spaceId": SPACE, "pageId": PAGE, "epoch": "3", "attachments": attachments,
        })
    }
    fn scope(total_bytes: usize, budget: Duration) -> GatherScope<'static> {
        GatherScope {
            space: SPACE,
            page: PAGE,
            epoch: 3,
            revision: REVISION,
            total_bytes,
            budget,
        }
    }
    /// Answers each attachment by its ID and records the references asked, in order.
    struct Scripted {
        asked: RefCell<Vec<AttachmentSelector>>,
        answer: fn(&str) -> Result<Vec<u8>>,
    }
    impl Source for Scripted {
        fn read(&self, page: &str, selector: &AttachmentSelector) -> Result<Vec<u8>> {
            assert_eq!(page, PAGE);
            self.asked.borrow_mut().push(selector.clone());
            let (AttachmentSelector::DocumentCurrent { attachment_id, .. }
            | AttachmentSelector::Message { attachment_id, .. }) = selector;
            (self.answer)(attachment_id)
        }
    }
    fn scripted(answer: fn(&str) -> Result<Vec<u8>>) -> Scripted {
        Scripted {
            asked: RefCell::new(Vec::new()),
            answer,
        }
    }
    fn run(
        meta: Value,
        own: BTreeMap<String, Value>,
        scope: GatherScope<'_>,
        source: &dyn Source,
    ) -> (Vec<Value>, Vec<(String, Vec<u8>)>) {
        let Gathered { entries, files } = gather(
            &PageParts {
                meta: &meta,
                own: &own,
            },
            &scope,
            source,
            Instant::now(),
        )
        .unwrap();
        (
            entries
                .iter()
                .map(|entry| serde_json::to_value(entry).unwrap())
                .collect(),
            files,
        )
    }
    fn state(row: &Value) -> (&str, Option<&str>) {
        (row["state"].as_str().unwrap(), row["reason"].as_str())
    }

    #[test]
    fn entries_follow_document_order_then_live_messages_and_each_is_read_under_its_reference() {
        let meta = json!({"attachments": [document(21, 3), document(20, 3)]});
        let mut own = BTreeMap::new();
        // Listed out of order on purpose: message 31 sorts before 32.
        own.insert(
            WRITER.to_owned(),
            json!({"messages": {
                format!("{}:1", id(32)): record(32, 1, vec![message(23, 32, 3)]),
                format!("{}:1", id(31)): record(31, 1, vec![message(22, 31, 3), message(24, 31, 3)]),
            }}),
        );
        let source = scripted(|_| Ok(vec![7; 3]));
        let (entries, files) = run(meta, own, scope(usize::MAX, BUDGET), &source);
        let ids: Vec<_> = entries
            .iter()
            .map(|row| row["attachmentId"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(ids, [id(21), id(20), id(22), id(24), id(23)]);
        assert_eq!(
            entries
                .iter()
                .map(|row| row["source"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["document", "document", "message", "message", "message"]
        );
        for row in &entries {
            assert_eq!(state(row), ("included", None));
            assert_eq!(
                row["file"],
                format!("attachments/{}", row["attachmentId"].as_str().unwrap())
            );
            assert_eq!(
                row["sha256"],
                hex(&tmt_colab_model::crypto::digest(&[7, 7, 7]))
            );
        }
        assert_eq!(files.len(), 5);
        // The document reference carries the page revision; the message reference its message.
        let asked = source.asked.borrow();
        assert!(matches!(
            &asked[0],
            AttachmentSelector::DocumentCurrent { content_revision, .. } if content_revision == REVISION
        ));
        assert!(matches!(
            &asked[2],
            AttachmentSelector::Message { message_id, message_revision, writer_id, .. }
                if *message_id == id(31) && message_revision == "1" && writer_id == WRITER
        ));
    }

    #[test]
    fn only_live_comments_of_the_exported_epoch_with_matching_provenance_are_listed() {
        let mut own = BTreeMap::new();
        let mut messages = serde_json::Map::new();
        let mut put = |key: u32, edit: fn(&mut Value)| {
            let mut value = record(key, 1, vec![message(key + 100, key, 3)]);
            edit(&mut value);
            messages.insert(format!("{}:1", id(key)), value);
        };
        put(1, |_| {});
        put(2, |m| m["deleted"] = json!(true));
        put(3, |m| m["epoch"] = json!("2"));
        put(4, |m| m["senderDevice"] = json!(id(99)));
        put(5, |m| m["pageId"] = json!(id(98)));
        put(6, |m| m["kind"] = json!("thread"));
        put(7, |m| m["revision"] = json!("2"));
        own.insert(WRITER.to_owned(), json!({ "messages": messages }));
        let source = scripted(|_| Ok(vec![0; 3]));
        let (entries, _) = run(json!({}), own, scope(usize::MAX, BUDGET), &source);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["attachmentId"], id(101));
    }

    #[test]
    fn a_failed_read_says_why_without_disclosing_anything() {
        let meta = json!({"attachments": [document(1, 3), document(2, 3), document(3, 3), document(4, 3)]});
        let source = scripted(|attachment| match attachment {
            a if a == id(1) => {
                Err(crate::page::ipc::WriteError::from_error(&Fault::Missing).into())
            }
            a if a == id(2) => Err(crate::page::ipc::WriteError::from_error(&Fault::Denied).into()),
            a if a == id(3) => Err(Fault::StaleBase.into()),
            _ => Err(Fault::Unavailable.into()),
        });
        let (entries, files) = run(meta, BTreeMap::new(), scope(usize::MAX, BUDGET), &source);
        assert_eq!(
            entries.iter().map(state).collect::<Vec<_>>(),
            [
                ("missing", None),
                ("unavailable", Some("denied")),
                ("unavailable", Some("changed")),
                ("unavailable", Some("unavailable")),
            ]
        );
        assert!(files.is_empty());
        for row in &entries {
            assert!(row.get("sha256").is_none() && row.get("file").is_none());
        }
    }

    #[test]
    fn the_total_cap_and_the_time_budget_leave_later_entries_unread_and_unavailable() {
        let meta = json!({"attachments": [document(1, 4), document(2, 4), document(3, 2)]});
        let source = scripted(|_| Ok(vec![1; 4]));
        // 4 + 4 fills an 8-byte cap; the third would exceed it and is not even requested.
        let (entries, files) = run(meta.clone(), BTreeMap::new(), scope(8, BUDGET), &source);
        assert_eq!(
            entries.iter().map(state).collect::<Vec<_>>(),
            [
                ("included", None),
                ("included", None),
                ("unavailable", Some("too-large"))
            ]
        );
        assert_eq!(files.len(), 2);
        assert_eq!(source.asked.borrow().len(), 2);
        // An export past its budget reads nothing more.
        let idle = scripted(|_| Ok(vec![1; 4]));
        let (entries, files) = run(
            meta,
            BTreeMap::new(),
            scope(usize::MAX, Duration::ZERO),
            &idle,
        );
        assert!(
            entries
                .iter()
                .all(|row| state(row) == ("unavailable", Some("unavailable")))
        );
        assert!(files.is_empty() && idle.asked.borrow().is_empty());
    }

    #[test]
    fn bytes_that_disagree_with_the_descriptor_length_are_never_disclosed() {
        let meta = json!({"attachments": [document(1, 4)]});
        let source = scripted(|_| Ok(vec![1; 5]));
        let (entries, files) = run(meta, BTreeMap::new(), scope(usize::MAX, BUDGET), &source);
        assert_eq!(state(&entries[0]), ("unavailable", Some("unavailable")));
        assert!(files.is_empty());
    }

    #[test]
    fn a_descriptor_of_another_page_or_space_refuses_the_whole_export() {
        for foreign in [
            json!({"page": id(77)}),
            json!({"space": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}),
        ] {
            let mut foreign_descriptor = document(1, 3);
            for (key, value) in foreign.as_object().unwrap() {
                foreign_descriptor[key] = value.clone();
            }
            let source = scripted(|_| Ok(vec![1; 3]));
            let meta = json!({"attachments": [foreign_descriptor]});
            let own = BTreeMap::new();
            let outcome = gather(
                &PageParts {
                    meta: &meta,
                    own: &own,
                },
                &scope(usize::MAX, BUDGET),
                &source,
                Instant::now(),
            );
            assert!(outcome.is_err() && source.asked.borrow().is_empty());
        }
    }
}

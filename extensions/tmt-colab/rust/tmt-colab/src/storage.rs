//! Local storage accounting. Byte values are retained encrypted payloads, not SQLite file size.
use crate::{
    Result,
    decoder::Decoder,
    inspection,
    keyring::{Keyring, Layout},
    page,
    store::Store,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Bytes {
    pub history: u64,
    pub files: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PageStorage {
    pub page_id: String,
    pub title: Option<String>,
    pub content_bytes: u64,
    pub history_bytes: u64,
    pub comments_bytes: u64,
    pub attachments_bytes: Option<u64>,
    pub unused_attachment_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
    pub freeable: Bytes,
    pub last_cleaned_automatically_at_ms: Option<u64>,
    pub skipped: Option<String>,
}

impl PageStorage {
    /// Remote's namespace allocation is a display estimate; unknown never becomes zero.
    pub fn apply_attachment_usage(&mut self, usage: crate::attachments::UsageDetails) {
        let bytes = match usage {
            crate::attachments::UsageDetails::Known { charged_bytes, .. } => Some(charged_bytes),
            crate::attachments::UsageDetails::Unknown
            | crate::attachments::UsageDetails::Unsupported { .. } => None,
        };
        self.apply_attachment_bytes(bytes);
    }
    pub fn apply_attachment_bytes(&mut self, bytes: Option<u64>) {
        self.attachments_bytes = bytes;
        self.total_bytes = bytes.and_then(|attachments| {
            self.content_bytes
                .checked_add(self.history_bytes)?
                .checked_add(self.comments_bytes)?
                .checked_add(attachments)
        });
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Storage {
    pub version: u8,
    pub space: String,
    pub db_bytes: u64,
    pub db_free_bytes: u64,
    pub pages: Vec<PageStorage>,
}

/// Build rows only from the owner's verified catalog. Unknown attachment inventory is null.
pub fn read(
    store: &Store,
    key: &Keyring,
    layout: &Layout,
    decoder: &mut Decoder,
) -> Result<Storage> {
    let catalog = inspection::catalog(store, key)?;
    let mut pages = Vec::new();
    for item in catalog["pages"].as_array().ok_or(page::Fault::Invalid)? {
        pages.push(read_page(store, key, item, decoder)?);
    }
    Ok(Storage {
        version: 1,
        space: key.space_id.clone(),
        db_bytes: std::fs::metadata(layout.directory.join("space.db"))?.len(),
        db_free_bytes: store.free_page_bytes()?,
        pages,
    })
}

pub fn read_page(
    store: &Store,
    key: &Keyring,
    item: &Value,
    decoder: &mut Decoder,
) -> Result<PageStorage> {
    let id = item["pageId"].as_str().ok_or(page::Fault::Invalid)?;
    let bytes = store.page_storage_bytes(id)?;
    let archived = item["archived"] == true;
    let shared = item["history"] != "current";
    let public = item["sharing"] == "public";
    let skipped = if archived {
        Some("archived")
    } else if public {
        Some("public")
    } else if shared {
        Some("shared-history")
    } else if !bytes.has_active_tail {
        Some("no-tail")
    } else {
        None
    };
    let title = if archived {
        None
    } else {
        Some(page::read(store, key, id, decoder)?.title)
    };
    let history = if skipped.is_none() && bytes.eligible_history > 0 {
        let streams = page::owner_fold::eligible_streams(store, key, id)?;
        store
            .eligible_history_for_streams(id, &streams)?
            .min(bytes.history)
    } else {
        0
    };
    Ok(PageStorage {
        page_id: id.into(),
        title,
        content_bytes: bytes.content,
        history_bytes: bytes.history,
        comments_bytes: bytes.comments,
        attachments_bytes: None,
        unused_attachment_bytes: None,
        total_bytes: None,
        freeable: Bytes { history, files: 0 },
        last_cleaned_automatically_at_ms: bytes.last_auto_cleaned_at_ms,
        skipped: skipped.map(str::to_owned),
    })
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MeasuredBytes {
    pub history: Option<u64>,
    pub files: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Reclaim {
    pub version: u8,
    pub page: String,
    pub dry_run: bool,
    pub freeable: Bytes,
    pub freed: MeasuredBytes,
    pub kept: Vec<String>,
    pub skipped: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ReclaimError>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReclaimError {
    pub code: String,
    pub message: String,
}

/// Apply verified compaction, or estimate already-eligible prune candidates.
pub fn reclaim(
    store: &mut Store,
    key: &Keyring,
    page_id: &str,
    decoder: &mut Decoder,
    dry_run: bool,
) -> Result<Reclaim> {
    Ok(reclaim_with_report(store, key, page_id, decoder, dry_run, None)?.0)
}

pub(crate) fn reclaim_with_report(
    store: &mut Store,
    key: &Keyring,
    page_id: &str,
    decoder: &mut Decoder,
    dry_run: bool,
    until: Option<std::time::Instant>,
) -> Result<(Reclaim, Option<page::compact::Report>)> {
    let catalog = inspection::catalog(store, key)?;
    let item = catalog["pages"]
        .as_array()
        .ok_or(page::Fault::Invalid)?
        .iter()
        .find(|item| item["pageId"] == page_id)
        .ok_or(page::Fault::Missing)?;
    let before = read_page(store, key, item, decoder)?;
    let mut skipped = before.skipped.clone();
    let mut freed = Some(0);
    let mut combined = None;
    let mut failure = None;
    if !dry_run && skipped.is_none() {
        let initial = before
            .content_bytes
            .checked_add(before.history_bytes)
            .ok_or(page::Fault::Invalid)?;
        let result = page::compact::request(store, key, page_id, decoder, until);
        match result {
            Ok(result) => {
                if !result.compacted {
                    skipped = Some(
                        match result.skipped {
                            Some(page::compact::Skipped::NothingToCombine) => "no-tail",
                            _ => "not-combined",
                        }
                        .into(),
                    );
                }
                combined = Some(result);
            }
            Err(error) => {
                failure = Some(ReclaimError {
                    code: "COLAB_UNAVAILABLE".into(),
                    message: format!(
                        "Cleanup stopped before all streams could be processed: {error}"
                    ),
                })
            }
        }
        freed = measure_freed(initial, store.page_storage_bytes(page_id), &mut failure);
    }
    if !dry_run {
        store.reclaim_free_pages();
    }
    Ok((
        Reclaim {
            version: 1,
            page: page_id.into(),
            dry_run,
            freeable: before.freeable,
            freed: MeasuredBytes {
                history: freed,
                files: 0,
            },
            kept: ["comments", "threads", "archive"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
            skipped,
            error: failure,
        },
        combined,
    ))
}

// A failed post-commit read cannot prove a zero saving. Preserve the original failure.
fn measure_freed(
    initial: u64,
    after: std::result::Result<crate::store::PageStorageBytes, crate::store::Fault>,
    failure: &mut Option<ReclaimError>,
) -> Option<u64> {
    let final_bytes = after
        .ok()
        .and_then(|bytes| bytes.content.checked_add(bytes.history));
    if final_bytes.is_none() && failure.is_none() {
        *failure = Some(ReclaimError {
            code: "COLAB_UNAVAILABLE".into(),
            message: "Cleanup completed, but its freed bytes could not be measured.".into(),
        });
    }
    final_bytes.map(|final_bytes| initial.saturating_sub(final_bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn attachment_usage_reports_known_charged_bytes_and_unknown_separately() {
        let mut row = PageStorage {
            page_id: "page".into(),
            title: None,
            content_bytes: 10,
            history_bytes: 20,
            comments_bytes: 5,
            attachments_bytes: None,
            unused_attachment_bytes: None,
            total_bytes: None,
            freeable: Bytes::default(),
            last_cleaned_automatically_at_ms: None,
            skipped: None,
        };
        row.apply_attachment_usage(crate::attachments::UsageDetails::Known {
            charged_bytes: 2048,
            entries: 1,
            active_uploads: 0,
            retained_identities: 0,
        });
        assert_eq!(row.attachments_bytes, Some(2048));
        assert_eq!(row.total_bytes, Some(2083));
        assert_eq!(row.unused_attachment_bytes, None);
        row.apply_attachment_usage(crate::attachments::UsageDetails::Unknown);
        assert_eq!(row.attachments_bytes, None);
        assert_eq!(row.total_bytes, None);
        row.apply_attachment_usage(crate::attachments::UsageDetails::Unsupported {
            hint: "unsupported".into(),
        });
        assert_eq!(row.attachments_bytes, None);
        assert_eq!(row.total_bytes, None);
    }

    #[test]
    fn space_reclaim_sums_partial_results_and_propagates_unknown() {
        let mut space = SpaceReclaim::new(false);
        let result = |history, failed: bool| Reclaim {
            version: 1,
            page: "page".into(),
            dry_run: false,
            freeable: Bytes::default(),
            freed: MeasuredBytes { history, files: 0 },
            kept: vec![],
            skipped: None,
            error: failed.then(|| ReclaimError {
                code: "COLAB_UNAVAILABLE".into(),
                message: "failed".into(),
            }),
        };
        space.add(result(Some(40), false)).unwrap();
        space.add(result(Some(2), true)).unwrap();
        assert_eq!(space.freed.history, Some(42));
        assert!(space.pages[1].error.is_some());
        space.add(result(None, true)).unwrap();
        assert_eq!(space.freed.history, None);
    }

    #[test]
    fn failed_measurement_reports_unknown_and_preserves_cleanup_error() {
        let mut error = Some(ReclaimError {
            code: "COLAB_UNAVAILABLE".into(),
            message: "injected stream failure".into(),
        });
        let freed = measure_freed(100, Err(crate::store::Fault::Invalid), &mut error);
        assert_eq!(freed, None);
        let report = Reclaim {
            version: 1,
            page: "page".into(),
            dry_run: false,
            freeable: Bytes::default(),
            freed: MeasuredBytes {
                history: freed,
                files: 0,
            },
            kept: vec![],
            skipped: None,
            error,
        };
        let value = serde_json::to_value(report).unwrap();
        assert!(value["freed"]["history"].is_null());
        assert_eq!(value["error"]["message"], "injected stream failure");
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpaceReclaim {
    pub version: u8,
    pub dry_run: bool,
    pub pages: Vec<Reclaim>,
    pub freeable: Bytes,
    pub freed: MeasuredBytes,
}
impl SpaceReclaim {
    pub fn new(dry_run: bool) -> Self {
        Self {
            version: 1,
            dry_run,
            pages: Vec::new(),
            freeable: Bytes::default(),
            freed: MeasuredBytes {
                history: Some(0),
                files: 0,
            },
        }
    }
    pub fn add(&mut self, page: Reclaim) -> Result<()> {
        self.freeable.history = self
            .freeable
            .history
            .checked_add(page.freeable.history)
            .ok_or(page::Fault::Invalid)?;
        self.freeable.files = self
            .freeable
            .files
            .checked_add(page.freeable.files)
            .ok_or(page::Fault::Invalid)?;
        self.freed.history = match (self.freed.history, page.freed.history) {
            (Some(a), Some(b)) => Some(a.checked_add(b).ok_or(page::Fault::Invalid)?),
            _ => None,
        };
        self.freed.files = self
            .freed
            .files
            .checked_add(page.freed.files)
            .ok_or(page::Fault::Invalid)?;
        self.pages.push(page);
        Ok(())
    }
}

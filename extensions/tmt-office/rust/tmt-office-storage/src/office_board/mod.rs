//! SQLite repository for the installation-owned local Office discussion board.

use rusqlite::{OptionalExtension, Transaction, params};
use sha2::{Digest, Sha256};
use tmt_office_model::office_board::{
    Actor, AuthorFilter, BoardError, BoardErrorCode, Category, CategoryListRequest,
    CategoryListResult, CreateReceipt, Cursor, DeleteReceipt, DeleteRequest, EditReceipt,
    EditRequest, Entry, ListRequest, ListResult, ListView, OfficeBoardRepository, PostRequest,
    REPLY_LIMIT, ReplyRequest, ShowRequest, ShowResult, ThreadSummary,
};

use crate::{
    OfficeStore,
    store::{preflight_complete, with_immediate_transaction, with_immediate_transaction_mapped},
};
use tmt_adapters::storage::{StorageError, StorageErrorCode, classify};

pub fn local_owner_actor(storage: &mut OfficeStore) -> Result<Actor, StorageError> {
    with_immediate_transaction(storage, "local Office owner", |transaction| {
        Ok(Actor::Owner {
            world_id: crate::office_world::ensure_world(transaction, timestamp)?,
        })
    })
}

impl OfficeBoardRepository for OfficeStore {
    type Error = StorageError;

    fn post(&mut self, request: &PostRequest) -> Result<CreateReceipt, BoardError<Self::Error>> {
        tmt_office_model::office_board::validate_post(request)?;
        let actor = preflight_actor(self, &request.actor)?;
        // Classify new threads under a real room; membership is not an ACL.
        // Replays and existing content remain usable after room/area changes.
        let room_active = match &request.category {
            Category::Room(id) => self
                .references()
                .room(id)
                .map_err(BoardError::storage)?
                .is_some_and(|room| !room.retired),
            _ => true,
        };
        preflight_complete();
        with_immediate_transaction_mapped(self, "Office board post", BoardError::storage, |tx| {
            let actor = active_actor(tx, &actor)?;
            let digest = intent(&[
                "post",
                &category_key(&request.category),
                &request.title,
                &request.body,
            ]);
            if let Some(receipt) = replay_create(tx, &actor.key(), &request.operation_id, &digest)?
            {
                return Ok(receipt);
            }
            let room_marked = match &request.category {
                Category::Room(id) => {
                    crate::retirement::is_marked(tx, crate::retirement::Marker::Room, id)
                        .map_err(BoardError::storage)?
                }
                _ => false,
            };
            if !room_active || room_marked {
                return Err(BoardError::policy(BoardErrorCode::Invalid));
            }
            let sequence = next_sequence(tx)?;
            let id = uuid::Uuid::new_v4().to_string();
            let now = timestamp().map_err(BoardError::storage)?;
            let (category_kind, category_id) = category_columns(&request.category);
            let (author_kind, author_id, author_name) = actor_columns(&actor);
            tx.execute("INSERT INTO office_board_entries (id,thread_id,is_root,category_kind,category_id,author_kind,author_id,author_name,revision,deleted,created_sequence,activity_sequence,created_at_ms,updated_at_ms,title,body) VALUES (?,?,1,?,?,?,?,?,1,0,?,?,?,?,?,?)",
                params![id,id,category_kind,category_id,author_kind,author_id,author_name,sequence as i64,sequence as i64,now,now,request.title,request.body])
                .map_err(|error| BoardError::storage(classify(error, "Create Office board thread")))?;
            advance_board(tx)?;
            let receipt = CreateReceipt {
                entry_id: id.clone(),
                thread_id: id.clone(),
                revision: 1,
                created: true,
                operation_id: request.operation_id.clone(),
            };
            store_create(tx, &actor.key(), &digest, &receipt)?;
            Ok(receipt)
        })
    }

    fn reply(&mut self, request: &ReplyRequest) -> Result<CreateReceipt, BoardError<Self::Error>> {
        tmt_office_model::office_board::validate_reply(request)?;
        let actor = preflight_actor(self, &request.actor)?;
        preflight_complete();
        with_immediate_transaction_mapped(self, "Office board reply", BoardError::storage, |tx| {
            let actor = active_actor(tx, &actor)?;
            let digest = intent(&["reply", &request.thread_id, &request.body]);
            if let Some(receipt) = replay_create(tx, &actor.key(), &request.operation_id, &digest)?
            {
                return Ok(receipt);
            }
            let root = tx.query_row("SELECT category_kind,category_id,deleted FROM office_board_entries WHERE id=? AND is_root=1", [&request.thread_id], |row| Ok((row.get::<_,String>(0)?,row.get::<_,Option<String>>(1)?,row.get::<_,bool>(2)?))).optional().map_err(|e| BoardError::storage(classify(e,"Read Office board thread")))?.ok_or_else(|| BoardError::policy(BoardErrorCode::ThreadNotFound))?;
            if root.2 {
                return Err(BoardError::policy(BoardErrorCode::ThreadNotFound));
            }
            let count: i64 = tx
                .query_row(
                    "SELECT count(*) FROM office_board_entries WHERE thread_id=? AND is_root=0",
                    [&request.thread_id],
                    |row| row.get(0),
                )
                .map_err(|e| BoardError::storage(classify(e, "Count Office board replies")))?;
            if count >= REPLY_LIMIT as i64 {
                return Err(BoardError::policy(BoardErrorCode::Invalid));
            }
            let sequence = next_sequence(tx)?;
            let id = uuid::Uuid::new_v4().to_string();
            let now = timestamp().map_err(BoardError::storage)?;
            let (author_kind, author_id, author_name) = actor_columns(&actor);
            tx.execute("INSERT INTO office_board_entries (id,thread_id,is_root,category_kind,category_id,author_kind,author_id,author_name,revision,deleted,created_sequence,activity_sequence,created_at_ms,updated_at_ms,title,body) VALUES (?,?,0,?,?,?,?,?,1,0,?,?,?,?,NULL,?)", params![id,request.thread_id,root.0,root.1,author_kind,author_id,author_name,sequence as i64,sequence as i64,now,now,request.body]).map_err(|e| BoardError::storage(classify(e,"Create Office board reply")))?;
            tx.execute(
                "UPDATE office_board_entries SET activity_sequence=?,updated_at_ms=? WHERE id=?",
                params![sequence as i64, now, request.thread_id],
            )
            .map_err(|e| BoardError::storage(classify(e, "Update Office board activity")))?;
            advance_board(tx)?;
            let receipt = CreateReceipt {
                entry_id: id,
                thread_id: request.thread_id.clone(),
                revision: 1,
                created: true,
                operation_id: request.operation_id.clone(),
            };
            store_create(tx, &actor.key(), &digest, &receipt)?;
            Ok(receipt)
        })
    }

    fn edit(&mut self, request: &EditRequest) -> Result<EditReceipt, BoardError<Self::Error>> {
        tmt_office_model::office_board::validate_edit(request)?;
        let actor = preflight_actor(self, &request.actor)?;
        preflight_complete();
        with_immediate_transaction_mapped(self, "Office board edit", BoardError::storage, |tx| {
            let actor = active_actor(tx, &actor)?;
            let digest = edit_intent(request);
            if let Some(value) = replay_edit(tx, &actor.key(), &request.operation_id, &digest)? {
                return Ok(value);
            }
            let row = entry_edit_row(tx, &request.entry_id)?
                .ok_or_else(|| BoardError::policy(BoardErrorCode::EntryNotFound))?;
            if row.revision != request.expected_revision {
                return Err(BoardError::policy(BoardErrorCode::RevisionConflict));
            }
            if row.author_key != actor.key() {
                return Err(BoardError::policy(BoardErrorCode::Forbidden));
            }
            if request.title.is_some() && !row.is_root {
                return Err(BoardError::policy(BoardErrorCode::Invalid));
            }
            if !row.is_root
                && request.body.as_ref().is_some_and(|body| {
                    body.len() > tmt_office_model::office_board::REPLY_BODY_MAX_BYTES
                })
            {
                return Err(BoardError::policy(BoardErrorCode::Invalid));
            }
            if row.deleted {
                return Err(BoardError::policy(BoardErrorCode::Forbidden));
            }
            let body = request.body.as_deref().unwrap_or(&row.body);
            let title = request.title.as_deref().or(row.title.as_deref());
            let changed = body != row.body || title != row.title.as_deref();
            let revision = if changed {
                let sequence = next_sequence(tx)?;
                let revision = row.revision + 1;
                let now = timestamp().map_err(BoardError::storage)?;
                tx.execute("UPDATE office_board_entries SET title=?,body=?,revision=?,updated_at_ms=? WHERE id=?",params![title,body,revision as i64,now,request.entry_id]).map_err(|e|BoardError::storage(classify(e,"Edit Office board entry")))?;
                tx.execute("UPDATE office_board_entries SET activity_sequence=?,updated_at_ms=? WHERE id=?",params![sequence as i64,now,row.thread_id]).map_err(|e|BoardError::storage(classify(e,"Update Office board activity")))?;
                advance_board(tx)?;
                revision
            } else {
                row.revision
            };
            let receipt = EditReceipt {
                entry_id: request.entry_id.clone(),
                revision,
                changed,
                operation_id: request.operation_id.clone(),
            };
            store_edit(tx, &actor.key(), &digest, &receipt)?;
            Ok(receipt)
        })
    }

    fn delete(
        &mut self,
        request: &DeleteRequest,
    ) -> Result<DeleteReceipt, BoardError<Self::Error>> {
        tmt_office_model::office_board::validate_delete(request)?;
        let actor = preflight_actor(self, &request.actor)?;
        preflight_complete();
        with_immediate_transaction_mapped(self, "Office board delete", BoardError::storage, |tx| {
            let actor = active_actor(tx, &actor)?;
            let digest = intent(&[
                "delete",
                &request.entry_id,
                &request.expected_revision.to_string(),
                if request.moderate {
                    "moderate"
                } else {
                    "author"
                },
            ]);
            if let Some(value) = replay_delete(tx, &actor.key(), &request.operation_id, &digest)? {
                return Ok(value);
            }
            let row = entry_edit_row(tx, &request.entry_id)?
                .ok_or_else(|| BoardError::policy(BoardErrorCode::EntryNotFound))?;
            if row.revision != request.expected_revision {
                return Err(BoardError::policy(BoardErrorCode::RevisionConflict));
            }
            let own = row.author_key == actor.key();
            if !own && !(request.moderate && matches!(actor, Actor::Owner { .. })) {
                return Err(BoardError::policy(BoardErrorCode::Forbidden));
            }
            let changed = !row.deleted;
            let revision = if changed {
                let sequence = next_sequence(tx)?;
                let revision = row.revision + 1;
                let now = timestamp().map_err(BoardError::storage)?;
                tx.execute("UPDATE office_board_entries SET deleted=1,title=NULL,body=NULL,revision=?,updated_at_ms=? WHERE id=?",params![revision as i64,now,request.entry_id]).map_err(|e|BoardError::storage(classify(e,"Delete Office board entry")))?;
                tx.execute("UPDATE office_board_entries SET activity_sequence=?,updated_at_ms=? WHERE id=?",params![sequence as i64,now,row.thread_id]).map_err(|e|BoardError::storage(classify(e,"Update Office board activity")))?;
                advance_board(tx)?;
                revision
            } else {
                row.revision
            };
            let receipt = DeleteReceipt {
                entry_id: request.entry_id.clone(),
                revision,
                deleted: true,
                changed,
                moderated: !own,
                operation_id: request.operation_id.clone(),
            };
            store_delete(tx, &actor.key(), &digest, &receipt)?;
            Ok(receipt)
        })
    }

    fn list(&self, request: &ListRequest) -> Result<ListResult, BoardError<Self::Error>> {
        tmt_office_model::office_board::validate_list(request)?;
        let tx = self
            .connection()
            .map_err(BoardError::storage)?
            .unchecked_transaction()
            .map_err(|e| BoardError::storage(classify(e, "Open Office board read snapshot")))?;
        let board_revision = board_revision(&tx)?;
        let binding = list_binding(request);
        let cursor = parse_cursor(
            request.cursor.as_deref(),
            "list",
            request.limit,
            &binding,
            board_revision,
        )?;
        let last_seq = cursor
            .as_ref()
            .map_or(i64::MAX, |c| i64::try_from(c.sequence).unwrap_or(i64::MAX));
        let last_id = cursor.as_ref().map_or("~", |c| c.key.as_str());
        let (kind, repo) = category_columns(&request.category);
        let author_kind = request.author.as_ref().map(|a| {
            if matches!(a, AuthorFilter::Owner) {
                "owner"
            } else {
                "identity"
            }
        });
        let author_id = request.author.as_ref().and_then(|a| match a {
            AuthorFilter::Identity(id) => Some(id.as_str()),
            AuthorFilter::Owner => None,
        });
        let since = i64::try_from(request.since_ms.unwrap_or(0)).unwrap_or(i64::MAX);
        let take = i64::from(request.limit) + 1;
        let sql = list_sql(request.view);
        let mut statement = tx
            .prepare(&sql)
            .map_err(|e| BoardError::storage(classify(e, "Prepare Office board list")))?;
        let mapped = statement
            .query_map(
                params![
                    kind,
                    repo,
                    author_kind,
                    author_kind,
                    author_id,
                    author_id,
                    since,
                    since,
                    last_seq,
                    last_seq,
                    last_id,
                    take
                ],
                decode_summary,
            )
            .map_err(|e| BoardError::storage(classify(e, "List Office board threads")))?;
        let mut threads = mapped
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| BoardError::storage(classify(e, "Read Office board threads")))?;
        let next = if threads.len() > request.limit as usize {
            threads.pop();
            let last = threads.last().expect("lookahead has prior item");
            let sequence = if matches!(request.view, ListView::Recent) {
                entry_sequence(&tx, &last.entry.id)?
            } else {
                last.activity_sequence
            };
            Some(
                Cursor {
                    operation: "list".into(),
                    board_revision,
                    page_size: request.limit,
                    binding,
                    sequence,
                    key: last.entry.id.clone(),
                }
                .encode(),
            )
        } else {
            None
        };
        drop(statement);
        tx.commit()
            .map_err(|e| BoardError::storage(classify(e, "Close Office board read snapshot")))?;
        Ok(ListResult {
            threads,
            next_cursor: next,
            board_revision,
        })
    }

    fn show(&self, request: &ShowRequest) -> Result<ShowResult, BoardError<Self::Error>> {
        tmt_office_model::office_board::validate_show(request)?;
        let tx = self
            .connection()
            .map_err(BoardError::storage)?
            .unchecked_transaction()
            .map_err(|e| BoardError::storage(classify(e, "Open Office board show snapshot")))?;
        let rev = board_revision(&tx)?;
        let binding = request.thread_id.clone();
        let cursor = parse_cursor(
            request.reply_cursor.as_deref(),
            "replies",
            request.reply_limit,
            &binding,
            rev,
        )?;
        let after = cursor.as_ref().map_or(0, |c| c.sequence);
        let thread = query_entry(&tx, &request.thread_id)?
            .filter(|e| e.id == e.thread_id)
            .ok_or_else(|| BoardError::policy(BoardErrorCode::ThreadNotFound))?;
        let mut statement = tx
            .prepare(REPLIES_SQL)
            .map_err(|e| BoardError::storage(classify(e, "Prepare Office board replies")))?;
        let rows = statement
            .query_map(
                params![
                    request.thread_id,
                    after as i64,
                    i64::from(request.reply_limit) + 1
                ],
                |row| Ok((decode_entry(row)?, row.get::<_, i64>(13)? as u64)),
            )
            .map_err(|e| BoardError::storage(classify(e, "List Office board replies")))?;
        let mut values = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| BoardError::storage(classify(e, "Read Office board replies")))?;
        let next = if values.len() > request.reply_limit as usize {
            values.pop();
            let last = values.last().unwrap();
            Some(
                Cursor {
                    operation: "replies".into(),
                    board_revision: rev,
                    page_size: request.reply_limit,
                    binding,
                    sequence: last.1,
                    key: last.0.id.clone(),
                }
                .encode(),
            )
        } else {
            None
        };
        let replies = values.into_iter().map(|v| v.0).collect();
        drop(statement);
        tx.commit()
            .map_err(|e| BoardError::storage(classify(e, "Close Office board show snapshot")))?;
        Ok(ShowResult {
            thread,
            replies,
            next_cursor: next,
            board_revision: rev,
        })
    }

    fn categories(
        &self,
        request: &CategoryListRequest,
    ) -> Result<CategoryListResult, BoardError<Self::Error>> {
        tmt_office_model::office_board::validate_category_list(request)?;
        let tx = self
            .connection()
            .map_err(BoardError::storage)?
            .unchecked_transaction()
            .map_err(|e| BoardError::storage(classify(e, "Open Office board category snapshot")))?;
        let rev = board_revision(&tx)?;
        let cursor = parse_cursor(
            request.cursor.as_deref(),
            "categories",
            request.limit,
            "scoped-v1",
            rev,
        )?;
        let after = cursor.as_ref().map(|c| c.key.as_str());
        let include_general = after.is_none();
        let (after_kind, after_id) = match after {
            None | Some("") => ("", ""),
            Some(key) => key
                .split_once(':')
                .filter(|(kind, _)| matches!(*kind, "repository" | "room"))
                .ok_or_else(|| BoardError::policy(BoardErrorCode::CursorInvalid))?,
        };
        let scoped_take = request.limit as usize - usize::from(include_general);
        let mut statement = tx
            .prepare(CATEGORIES_SQL)
            .map_err(|e| BoardError::storage(classify(e, "Prepare Office board categories")))?;
        let scopes = statement
            .query_map(
                params![
                    after_kind,
                    after_id,
                    i64::try_from(scoped_take + 1).unwrap()
                ],
                |row| decode_category(row.get(0)?, row.get(1)?),
            )
            .map_err(|e| BoardError::storage(classify(e, "List Office board categories")))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| BoardError::storage(classify(e, "Read Office board categories")))?;
        let mut categories = Vec::new();
        if include_general {
            categories.push(Category::General);
        }
        categories.extend(scopes.iter().take(scoped_take).cloned());
        let next = if scopes.len() > scoped_take {
            let key = if scoped_take == 0 {
                String::new()
            } else {
                category_key(&scopes[scoped_take - 1])
            };
            Some(
                Cursor {
                    operation: "categories".into(),
                    board_revision: rev,
                    page_size: request.limit,
                    binding: "scoped-v1".into(),
                    sequence: 0,
                    key,
                }
                .encode(),
            )
        } else {
            None
        };
        drop(statement);
        tx.commit().map_err(|e| {
            BoardError::storage(classify(e, "Close Office board category snapshot"))
        })?;
        Ok(CategoryListResult {
            categories,
            next_cursor: next,
            board_revision: rev,
        })
    }
}

const CATEGORIES_SQL: &str = concat!(
    "SELECT DISTINCT category_kind,category_id FROM office_board_entries ",
    "WHERE is_root=1 AND category_kind!='general' ",
    "AND (category_kind,category_id)>(?,?) ",
    "ORDER BY category_kind COLLATE BINARY,category_id COLLATE BINARY LIMIT ?",
);
const REPLIES_SQL: &str = concat!(
    "SELECT id,thread_id,category_kind,category_id,author_kind,author_id,",
    "author_name,revision,deleted,created_at_ms,updated_at_ms,title,body,created_sequence ",
    "FROM office_board_entries WHERE thread_id=? AND is_root=0 AND created_sequence>? ",
    "ORDER BY created_sequence,id LIMIT ?",
);
fn list_sql(view: ListView) -> String {
    let (sequence, time) = if matches!(view, ListView::Recent) {
        ("created_sequence", "created_at_ms")
    } else {
        ("activity_sequence", "updated_at_ms")
    };
    format!(
        "SELECT id,thread_id,category_kind,category_id,author_kind,author_id,author_name,revision,deleted,created_at_ms,updated_at_ms,title,body,activity_sequence,(SELECT count(*) FROM office_board_entries r WHERE r.thread_id=e.id AND r.is_root=0 AND r.deleted=0) FROM office_board_entries e WHERE is_root=1 AND category_kind=? AND category_id IS ? AND (? IS NULL OR author_kind=?) AND (? IS NULL OR author_id=?) AND (?=0 OR {time}>=?) AND ({sequence}<? OR ({sequence}=? AND id<?)) ORDER BY {sequence} DESC,id DESC LIMIT ?"
    )
}

struct EditRow {
    thread_id: String,
    is_root: bool,
    author_key: String,
    revision: u64,
    deleted: bool,
    title: Option<String>,
    body: String,
}
fn entry_edit_row(
    tx: &Transaction<'_>,
    id: &str,
) -> Result<Option<EditRow>, BoardError<StorageError>> {
    tx.query_row(
        concat!(
            "SELECT thread_id,is_root,author_kind,author_id,revision,deleted,title,",
            "COALESCE(body,'') FROM office_board_entries WHERE id=?",
        ),
        [id],
        |row| {
            Ok(EditRow {
                thread_id: row.get(0)?,
                is_root: row.get(1)?,
                author_key: format!("{}:{}", row.get::<_, String>(2)?, row.get::<_, String>(3)?),
                revision: row.get::<_, i64>(4)? as u64,
                deleted: row.get(5)?,
                title: row.get(6)?,
                body: row.get(7)?,
            })
        },
    )
    .optional()
    .map_err(|e| BoardError::storage(classify(e, "Read Office board entry")))
}
/// Preflight: identity actors must be active; the recorded name comes from core.
fn preflight_actor(store: &OfficeStore, actor: &Actor) -> Result<Actor, BoardError<StorageError>> {
    match actor {
        Actor::Identity { identity_id, .. } => match store
            .references()
            .identity(identity_id)
            .map_err(BoardError::storage)?
        {
            Some(identity) if !identity.retired => Ok(Actor::Identity {
                identity_id: identity_id.clone(),
                name: identity.name,
            }),
            _ => Err(BoardError::policy(BoardErrorCode::Forbidden)),
        },
        Actor::Owner { .. } => Ok(actor.clone()),
    }
}

/// Office-owned actor checks inside the transaction; identities were preflighted.
fn active_actor(tx: &Transaction<'_>, actor: &Actor) -> Result<Actor, BoardError<StorageError>> {
    match actor {
        // A retirement Office recorded after preflight blocks the write.
        Actor::Identity { identity_id, .. } => {
            if crate::retirement::is_marked(tx, crate::retirement::Marker::Identity, identity_id)
                .map_err(BoardError::storage)?
            {
                return Err(BoardError::policy(BoardErrorCode::Forbidden));
            }
            Ok(actor.clone())
        }
        Actor::Owner { world_id } => {
            let matches: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM office_local_worlds WHERE singleton=1 AND id=?)",
                    [world_id],
                    |r| r.get(0),
                )
                .map_err(|e| BoardError::storage(classify(e, "Revalidate Office board owner")))?;
            if matches {
                Ok(actor.clone())
            } else {
                Err(BoardError::policy(BoardErrorCode::Forbidden))
            }
        }
    }
}
fn next_sequence(tx: &Transaction<'_>) -> Result<u64, BoardError<StorageError>> {
    let value: i64 = tx
        .query_row(
            concat!(
                "UPDATE office_board_state SET next_sequence=next_sequence+1 ",
                "WHERE singleton=1 RETURNING next_sequence-1",
            ),
            [],
            |row| row.get(0),
        )
        .map_err(|e| BoardError::storage(classify(e, "Allocate Office board sequence")))?;
    Ok(value as u64)
}
fn advance_board(tx: &Transaction<'_>) -> Result<(), BoardError<StorageError>> {
    tx.execute(
        "UPDATE office_board_state SET revision=revision+1 WHERE singleton=1",
        [],
    )
    .map_err(|e| BoardError::storage(classify(e, "Advance Office board revision")))?;
    Ok(())
}
fn board_revision(tx: &Transaction<'_>) -> Result<u64, BoardError<StorageError>> {
    tx.query_row(
        "SELECT revision FROM office_board_state WHERE singleton=1",
        [],
        |r| r.get::<_, i64>(0).map(|v| v as u64),
    )
    .map_err(|e| BoardError::storage(classify(e, "Read Office board revision")))
}
fn timestamp() -> Result<i64, StorageError> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| {
            StorageError::new(
                tmt_adapters::storage::StorageErrorCode::Unknown,
                "Read system clock failed",
            )
            .caused_by(e)
        })?
        .as_millis()
        .try_into()
        .map_err(|e| {
            StorageError::new(
                tmt_adapters::storage::StorageErrorCode::Unknown,
                "System timestamp is too large",
            )
            .caused_by(e)
        })
}
fn category_columns(c: &Category) -> (&'static str, Option<&str>) {
    match c {
        Category::General => ("general", None),
        Category::Repository(id) => ("repository", Some(id)),
        Category::Room(id) => ("room", Some(id)),
    }
}
fn category_key(c: &Category) -> String {
    match c {
        Category::General => "general".into(),
        Category::Repository(id) => format!("repository:{id}"),
        Category::Room(id) => format!("room:{id}"),
    }
}
fn actor_columns(a: &Actor) -> (&'static str, &str, Option<&str>) {
    match a {
        Actor::Owner { world_id } => ("owner", world_id, None),
        Actor::Identity { identity_id, name } => ("identity", identity_id, Some(name)),
    }
}
fn intent(parts: &[&str]) -> String {
    let mut hash = Sha256::new();
    for part in parts {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part.as_bytes());
    }
    hash.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
fn edit_intent(request: &EditRequest) -> String {
    let revision = request.expected_revision.to_string();
    let mut parts = vec!["edit", request.entry_id.as_str(), revision.as_str()];
    match &request.title {
        Some(value) => parts.extend(["title:some", value.as_str()]),
        None => parts.push("title:none"),
    }
    match &request.body {
        Some(value) => parts.extend(["body:some", value.as_str()]),
        None => parts.push("body:none"),
    }
    intent(&parts)
}
fn list_binding(r: &ListRequest) -> String {
    format!(
        "{}|{:?}|{:?}|{:?}",
        category_key(&r.category),
        r.view,
        r.author,
        r.since_ms
    )
}
fn parse_cursor(
    value: Option<&str>,
    op: &str,
    limit: u32,
    binding: &str,
    revision: u64,
) -> Result<Option<Cursor>, BoardError<StorageError>> {
    let Some(value) = value else { return Ok(None) };
    let c = Cursor::parse(value).map_err(|e| BoardError::policy(e.code))?;
    if c.operation != op
        || c.page_size != limit
        || c.binding != tmt_office_model::office_board::binding_fingerprint(binding)
    {
        return Err(BoardError::policy(BoardErrorCode::CursorInvalid));
    }
    if c.board_revision != revision {
        return Err(BoardError::policy(BoardErrorCode::CursorStale));
    }
    Ok(Some(c))
}
fn decode_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<Entry> {
    let kind: String = row.get(2)?;
    let repo: Option<String> = row.get(3)?;
    let author_kind: String = row.get(4)?;
    let author_id: String = row.get(5)?;
    let name: Option<String> = row.get(6)?;
    Ok(Entry {
        id: row.get(0)?,
        thread_id: row.get(1)?,
        category: decode_category(kind, repo)?,
        author: if author_kind == "owner" {
            Actor::Owner {
                world_id: author_id,
            }
        } else {
            Actor::Identity {
                identity_id: author_id,
                name: name.unwrap_or_default(),
            }
        },
        revision: row.get::<_, i64>(7)? as u64,
        deleted: row.get(8)?,
        created_at_ms: row.get::<_, i64>(9)? as u64,
        updated_at_ms: row.get::<_, i64>(10)? as u64,
        title: row.get(11)?,
        body: row.get(12)?,
    })
}

fn decode_category(kind: String, id: Option<String>) -> rusqlite::Result<Category> {
    match (kind.as_str(), id) {
        ("general", None) => Ok(Category::General),
        ("repository", Some(id)) => Ok(Category::Repository(id)),
        ("room", Some(id)) => Ok(Category::Room(id)),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}
fn decode_summary(row: &rusqlite::Row<'_>) -> rusqlite::Result<ThreadSummary> {
    Ok(ThreadSummary {
        entry: decode_entry(row)?,
        activity_sequence: row.get::<_, i64>(13)? as u64,
        reply_count: row.get::<_, i64>(14)? as u64,
    })
}
fn query_entry(tx: &Transaction<'_>, id: &str) -> Result<Option<Entry>, BoardError<StorageError>> {
    tx.query_row("SELECT id,thread_id,category_kind,category_id,author_kind,author_id,author_name,revision,deleted,created_at_ms,updated_at_ms,title,body FROM office_board_entries WHERE id=?",[id],decode_entry).optional().map_err(|e|BoardError::storage(classify(e,"Read Office board entry")))
}
fn entry_sequence(tx: &Transaction<'_>, id: &str) -> Result<u64, BoardError<StorageError>> {
    tx.query_row(
        "SELECT created_sequence FROM office_board_entries WHERE id=?",
        [id],
        |r| r.get::<_, i64>(0).map(|v| v as u64),
    )
    .map_err(|e| BoardError::storage(classify(e, "Read Office board sequence")))
}

type StoredReceipt = (
    String,
    Option<String>,
    u64,
    Option<bool>,
    Option<bool>,
    Option<bool>,
    Option<bool>,
);

fn replay_row(
    tx: &Transaction<'_>,
    actor: &str,
    op: &str,
    digest: &str,
    kind: &str,
) -> Result<Option<StoredReceipt>, BoardError<StorageError>> {
    let row = tx
        .query_row(
            concat!(
                "SELECT intent_digest,result_kind,entry_id,thread_id,revision,",
                "created,changed,deleted,moderated FROM office_board_operations ",
                "WHERE actor_key=? AND operation_id=?",
            ),
            params![actor, op],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, i64>(4)? as u64,
                    row.get::<_, Option<bool>>(5)?,
                    row.get::<_, Option<bool>>(6)?,
                    row.get::<_, Option<bool>>(7)?,
                    row.get::<_, Option<bool>>(8)?,
                ))
            },
        )
        .optional()
        .map_err(|e| BoardError::storage(classify(e, "Read Office board receipt")))?;
    match row {
        None => Ok(None),
        Some((d, k, id, thread, rev, created, changed, deleted, moderated))
            if d == digest && k == kind =>
        {
            Ok(Some((
                id, thread, rev, created, changed, deleted, moderated,
            )))
        }
        Some(_) => Err(BoardError::policy(BoardErrorCode::IdempotencyConflict)),
    }
}
fn replay_create(
    tx: &Transaction<'_>,
    actor: &str,
    op: &str,
    digest: &str,
) -> Result<Option<CreateReceipt>, BoardError<StorageError>> {
    replay_row(tx, actor, op, digest, "create")?
        .map(|r| {
            valid_receipt_base(&r.0, r.2)?;
            let thread_id =
                r.1.filter(|id| canonical_uuid(id))
                    .ok_or_else(corrupt_receipt)?;
            if r.3 != Some(true) || r.4.is_some() || r.5.is_some() || r.6.is_some() {
                return Err(corrupt_receipt());
            }
            Ok(CreateReceipt {
                entry_id: r.0,
                thread_id,
                revision: r.2,
                created: true,
                operation_id: op.into(),
            })
        })
        .transpose()
}
fn replay_edit(
    tx: &Transaction<'_>,
    actor: &str,
    op: &str,
    digest: &str,
) -> Result<Option<EditReceipt>, BoardError<StorageError>> {
    replay_row(tx, actor, op, digest, "edit")?
        .map(|r| {
            valid_receipt_base(&r.0, r.2)?;
            if r.1.is_some() || r.3.is_some() || r.4.is_none() || r.5.is_some() || r.6.is_some() {
                return Err(corrupt_receipt());
            }
            Ok(EditReceipt {
                entry_id: r.0,
                revision: r.2,
                changed: r.4.unwrap(),
                operation_id: op.into(),
            })
        })
        .transpose()
}
fn replay_delete(
    tx: &Transaction<'_>,
    actor: &str,
    op: &str,
    digest: &str,
) -> Result<Option<DeleteReceipt>, BoardError<StorageError>> {
    replay_row(tx, actor, op, digest, "delete")?
        .map(|r| {
            valid_receipt_base(&r.0, r.2)?;
            if r.1.is_some() || r.3.is_some() || r.4.is_none() || r.5 != Some(true) || r.6.is_none()
            {
                return Err(corrupt_receipt());
            }
            Ok(DeleteReceipt {
                entry_id: r.0,
                revision: r.2,
                deleted: true,
                changed: r.4.unwrap(),
                moderated: r.6.unwrap(),
                operation_id: op.into(),
            })
        })
        .transpose()
}
fn canonical_uuid(value: &str) -> bool {
    uuid::Uuid::parse_str(value)
        .ok()
        .is_some_and(|id| id.to_string() == value)
}
fn corrupt_receipt() -> BoardError<StorageError> {
    BoardError::storage(StorageError::new(
        StorageErrorCode::Corrupt,
        "Read Office board receipt failed",
    ))
}
fn valid_receipt_base(id: &str, revision: u64) -> Result<(), BoardError<StorageError>> {
    if canonical_uuid(id) && (1..=tmt_core::limits::MAX_JS_SAFE_INTEGER).contains(&revision) {
        Ok(())
    } else {
        Err(corrupt_receipt())
    }
}
fn store_create(
    tx: &Transaction<'_>,
    actor: &str,
    digest: &str,
    r: &CreateReceipt,
) -> Result<(), BoardError<StorageError>> {
    tx.execute(
        concat!(
            "INSERT INTO office_board_operations ",
            "(actor_key,operation_id,intent_digest,result_kind,entry_id,thread_id,revision,created) ",
            "VALUES (?,?,?,'create',?,?,?,?)",
        ),
        params![
            actor,
            r.operation_id,
            digest,
            r.entry_id,
            r.thread_id,
            r.revision as i64,
            r.created
        ],
    )
    .map_err(|e| BoardError::storage(classify(e, "Store Office board receipt")))?;
    Ok(())
}
fn store_edit(
    tx: &Transaction<'_>,
    actor: &str,
    digest: &str,
    r: &EditReceipt,
) -> Result<(), BoardError<StorageError>> {
    tx.execute(
        concat!(
            "INSERT INTO office_board_operations ",
            "(actor_key,operation_id,intent_digest,result_kind,entry_id,revision,changed) ",
            "VALUES (?,?,?,'edit',?,?,?)",
        ),
        params![
            actor,
            r.operation_id,
            digest,
            r.entry_id,
            r.revision as i64,
            r.changed
        ],
    )
    .map_err(|e| BoardError::storage(classify(e, "Store Office board receipt")))?;
    Ok(())
}
fn store_delete(
    tx: &Transaction<'_>,
    actor: &str,
    digest: &str,
    r: &DeleteReceipt,
) -> Result<(), BoardError<StorageError>> {
    tx.execute(
        concat!(
            "INSERT INTO office_board_operations ",
            "(actor_key,operation_id,intent_digest,result_kind,entry_id,revision,",
            "changed,deleted,moderated) VALUES (?,?,?,'delete',?,?,?,1,?)",
        ),
        params![
            actor,
            r.operation_id,
            digest,
            r.entry_id,
            r.revision as i64,
            r.changed,
            r.moderated
        ],
    )
    .map_err(|e| BoardError::storage(classify(e, "Store Office board receipt")))?;
    Ok(())
}

#[cfg(test)]
mod tests;

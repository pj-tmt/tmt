//! JSON boundary shared by one-shot companion calls and the local HTTP service.

use serde::Deserialize;
use serde_json::{Value, json};
use tmt_office_model::office_board;
use tmt_office_model::office_board::Actor;
use tmt_office_model::office_board::AuthorFilter;
use tmt_office_model::office_board::BoardErrorCode;
use tmt_office_model::office_board::Category;
use tmt_office_model::office_board::CategoryListRequest;
use tmt_office_model::office_board::DeleteRequest;
use tmt_office_model::office_board::EditRequest;
use tmt_office_model::office_board::ListRequest;
use tmt_office_model::office_board::ListView;
use tmt_office_model::office_board::PostRequest;
use tmt_office_model::office_board::ReplyRequest;
use tmt_office_model::office_board::ShowRequest;
use tmt_office_model::office_protocol::OfficeInvocation;

use crate::{OfficeStore, local_owner_actor};
use tmt_adapters::config::ConfigPaths;
use tmt_office_model::office_board::BOARD_WIRE_LIMIT;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CategoryInput {
    kind: String,
    #[serde(default)]
    repository_id: Option<String>,
    #[serde(default)]
    room_id: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ActorInput {
    kind: String,
    #[serde(default)]
    identity_id: Option<String>,
    #[serde(default)]
    name: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PostInput {
    category: CategoryInput,
    actor: ActorInput,
    title: String,
    body: String,
    operation_id: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReplyInput {
    thread_id: String,
    actor: ActorInput,
    body: String,
    operation_id: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EditInput {
    entry_id: String,
    actor: ActorInput,
    title: Option<String>,
    body: Option<String>,
    if_revision: u64,
    operation_id: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DeleteInput {
    entry_id: String,
    actor: ActorInput,
    moderate: bool,
    if_revision: u64,
    operation_id: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ListInput {
    category: CategoryInput,
    view: Option<String>,
    author: Option<ActorInput>,
    since_ms: Option<u64>,
    limit: Option<u32>,
    cursor: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ShowInput {
    thread_id: String,
    reply_limit: Option<u32>,
    reply_cursor: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CategoriesInput {
    limit: Option<u32>,
    cursor: Option<String>,
}

pub fn execute(operation: OfficeInvocation, input: &[u8]) -> Vec<u8> {
    let prepared = match prepare(operation, input) {
        Ok(prepared) => prepared,
        Err(code) => return serde_json::to_vec(&json!({"error":code.code()})).unwrap(),
    };
    let paths = match ConfigPaths::discover() {
        Ok(paths) => paths,
        Err(_) => return br#"{"error":"STORAGE_ERROR"}"#.to_vec(),
    };
    if !matches!(
        prepared,
        DecodedBoardCall::List { .. }
            | DecodedBoardCall::Show { .. }
            | DecodedBoardCall::Categories { .. }
    ) {
        super::reconcile_before_write(&crate::StorageLayout::new(&paths));
    }
    encode_result(execute_prepared(
        prepared,
        &crate::StorageLayout::new(&paths),
    ))
}

pub fn execute_at(
    operation: OfficeInvocation,
    input: &[u8],
    layout: &crate::StorageLayout,
) -> Vec<u8> {
    let result = prepare(operation, input).and_then(|prepared| execute_prepared(prepared, layout));
    encode_result(result)
}

fn encode_result(result: Result<Value, BoardErrorCode>) -> Vec<u8> {
    let value = result.unwrap_or_else(|code| json!({"error":code.code()}));
    serde_json::to_vec(&value).unwrap_or_else(|_| br#"{"error":"STORAGE_ERROR"}"#.to_vec())
}

enum DecodedBoardCall {
    Post {
        category: Category,
        actor: DecodedBoardActor,
        title: String,
        body: String,
        operation_id: String,
    },
    Reply {
        thread_id: String,
        actor: DecodedBoardActor,
        body: String,
        operation_id: String,
    },
    Edit {
        entry_id: String,
        actor: DecodedBoardActor,
        title: Option<String>,
        body: Option<String>,
        expected_revision: u64,
        operation_id: String,
    },
    Delete {
        entry_id: String,
        actor: DecodedBoardActor,
        moderate: bool,
        expected_revision: u64,
        operation_id: String,
    },
    List(ListRequest),
    Show(ShowRequest),
    Categories(CategoryListRequest),
}

enum DecodedBoardActor {
    Owner,
    Identity { identity_id: String, name: String },
}

fn prepare(operation: OfficeInvocation, input: &[u8]) -> Result<DecodedBoardCall, BoardErrorCode> {
    if input.len() > BOARD_WIRE_LIMIT {
        return Err(BoardErrorCode::Invalid);
    }
    match operation {
        OfficeInvocation::BoardPost => {
            let v: PostInput = decode(input)?;
            let category = category(v.category)?;
            let actor = prepared_actor(v.actor)?;
            let operation_id = operation_id(v.operation_id)?;
            office_board::validate_post_fields::<std::convert::Infallible>(
                &category,
                &v.title,
                &v.body,
                &operation_id,
            )
            .map_err(|e| e.code)?;
            Ok(DecodedBoardCall::Post {
                category,
                actor,
                title: v.title,
                body: v.body,
                operation_id,
            })
        }
        OfficeInvocation::BoardReply => {
            let v: ReplyInput = decode(input)?;
            let actor = prepared_actor(v.actor)?;
            let operation_id = operation_id(v.operation_id)?;
            office_board::validate_reply_fields::<std::convert::Infallible>(
                &v.thread_id,
                &v.body,
                &operation_id,
            )
            .map_err(|e| e.code)?;
            Ok(DecodedBoardCall::Reply {
                thread_id: v.thread_id,
                actor,
                body: v.body,
                operation_id,
            })
        }
        OfficeInvocation::BoardEdit => {
            let v: EditInput = decode(input)?;
            let actor = prepared_actor(v.actor)?;
            let operation_id = operation_id(v.operation_id)?;
            office_board::validate_edit_fields::<std::convert::Infallible>(
                &v.entry_id,
                v.title.as_deref(),
                v.body.as_deref(),
                v.if_revision,
                &operation_id,
            )
            .map_err(|e| e.code)?;
            Ok(DecodedBoardCall::Edit {
                entry_id: v.entry_id,
                actor,
                title: v.title,
                body: v.body,
                expected_revision: v.if_revision,
                operation_id,
            })
        }
        OfficeInvocation::BoardDelete => {
            let v: DeleteInput = decode(input)?;
            let actor = prepared_actor(v.actor)?;
            let operation_id = operation_id(v.operation_id)?;
            office_board::validate_delete_fields::<std::convert::Infallible>(
                &v.entry_id,
                v.moderate,
                matches!(&actor, DecodedBoardActor::Owner),
                v.if_revision,
                &operation_id,
            )
            .map_err(|e| e.code)?;
            Ok(DecodedBoardCall::Delete {
                entry_id: v.entry_id,
                actor,
                moderate: v.moderate,
                expected_revision: v.if_revision,
                operation_id,
            })
        }
        OfficeInvocation::BoardList => {
            let v: ListInput = decode(input)?;
            let request = ListRequest {
                category: category(v.category)?,
                view: match v.view.as_deref().unwrap_or("recent") {
                    "recent" => ListView::Recent,
                    "updated" => ListView::Updated,
                    _ => return Err(BoardErrorCode::Invalid),
                },
                author: author_filter(v.author)?,
                since_ms: v.since_ms,
                limit: v.limit.unwrap_or(office_board::DEFAULT_PAGE_SIZE),
                cursor: v.cursor,
            };
            office_board::validate_list::<std::convert::Infallible>(&request)
                .map_err(|e| e.code)?;
            Ok(DecodedBoardCall::List(request))
        }
        OfficeInvocation::BoardShow => {
            let v: ShowInput = decode(input)?;
            let request = ShowRequest {
                thread_id: v.thread_id,
                reply_limit: v.reply_limit.unwrap_or(office_board::DEFAULT_PAGE_SIZE),
                reply_cursor: v.reply_cursor,
            };
            office_board::validate_show::<std::convert::Infallible>(&request)
                .map_err(|e| e.code)?;
            Ok(DecodedBoardCall::Show(request))
        }
        OfficeInvocation::BoardCategories => {
            let v: CategoriesInput = decode(input)?;
            let request = CategoryListRequest {
                limit: v.limit.unwrap_or(office_board::DEFAULT_PAGE_SIZE),
                cursor: v.cursor,
            };
            office_board::validate_category_list::<std::convert::Infallible>(&request)
                .map_err(|e| e.code)?;
            Ok(DecodedBoardCall::Categories(request))
        }
        _ => Err(BoardErrorCode::Invalid),
    }
}
fn prepared_actor(value: ActorInput) -> Result<DecodedBoardActor, BoardErrorCode> {
    match value {
        ActorInput {
            kind,
            identity_id: None,
            name: None,
        } if kind == "owner" => Ok(DecodedBoardActor::Owner),
        ActorInput {
            kind,
            identity_id: Some(identity_id),
            name: Some(name),
        } if kind == "identity" => {
            office_board::validate_identity_actor::<std::convert::Infallible>(&identity_id, &name)
                .map_err(|e| e.code)?;
            Ok(DecodedBoardActor::Identity { identity_id, name })
        }
        _ => Err(BoardErrorCode::Invalid),
    }
}

fn execute_prepared(
    prepared: DecodedBoardCall,
    layout: &crate::StorageLayout,
) -> Result<Value, BoardErrorCode> {
    let mut storage = OfficeStore::open_configured(layout).map_err(|_| BoardErrorCode::Storage)?;
    let result = (|| match prepared {
        DecodedBoardCall::Post {
            category,
            actor,
            title,
            body,
            operation_id,
        } => {
            let request = PostRequest {
                category,
                actor: resolve_actor(&mut storage, actor)?,
                title,
                body,
                operation_id,
            };
            office_board::create_thread(&mut storage, &request)
                .map(create_value)
                .map_err(|e| e.code)
        }
        DecodedBoardCall::Reply {
            thread_id,
            actor,
            body,
            operation_id,
        } => {
            let request = ReplyRequest {
                thread_id,
                actor: resolve_actor(&mut storage, actor)?,
                body,
                operation_id,
            };
            office_board::reply(&mut storage, &request)
                .map(create_value)
                .map_err(|e| e.code)
        }
        DecodedBoardCall::Edit {
            entry_id,
            actor,
            title,
            body,
            expected_revision,
            operation_id,
        } => {
            let request = EditRequest {
                entry_id,
                actor: resolve_actor(&mut storage, actor)?,
                title,
                body,
                expected_revision,
                operation_id,
            };
            office_board::edit(&mut storage, &request)
                .map(edit_value)
                .map_err(|e| e.code)
        }
        DecodedBoardCall::Delete {
            entry_id,
            actor,
            moderate,
            expected_revision,
            operation_id,
        } => {
            let request = DeleteRequest {
                entry_id,
                actor: resolve_actor(&mut storage, actor)?,
                moderate,
                expected_revision,
                operation_id,
            };
            office_board::delete(&mut storage, &request)
                .map(delete_value)
                .map_err(|e| e.code)
        }
        DecodedBoardCall::List(request) => office_board::list(&storage, &request)
            .map(list_value)
            .map_err(|e| e.code),
        DecodedBoardCall::Show(request) => office_board::show(&storage, &request)
            .map(show_value)
            .map_err(|e| e.code),
        DecodedBoardCall::Categories(request) => office_board::categories(&storage, &request)
            .map(categories_value)
            .map_err(|e| e.code),
    })();
    let close = storage.close().map_err(|_| BoardErrorCode::Storage);
    match (result, close) {
        (Err(e), _) => Err(e),
        (Ok(v), Ok(())) => Ok(v),
        (Ok(_), Err(e)) => Err(e),
    }
}

fn decode<'a, T: Deserialize<'a>>(input: &'a [u8]) -> Result<T, BoardErrorCode> {
    serde_json::from_slice(input).map_err(|_| BoardErrorCode::Invalid)
}
fn operation_id(value: Option<String>) -> Result<String, BoardErrorCode> {
    let value = value.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    if uuid::Uuid::parse_str(&value)
        .ok()
        .is_some_and(|id| id.to_string() == value)
    {
        Ok(value)
    } else {
        Err(BoardErrorCode::Invalid)
    }
}
fn category(value: CategoryInput) -> Result<Category, BoardErrorCode> {
    match value {
        CategoryInput {
            kind,
            repository_id: None,
            room_id: None,
        } if kind == "general" => Ok(Category::General),
        CategoryInput {
            kind,
            repository_id: Some(id),
            room_id: None,
        } if kind == "repository" => Ok(Category::Repository(id)),
        CategoryInput {
            kind,
            repository_id: None,
            room_id: Some(id),
        } if kind == "room" => Ok(Category::Room(id)),
        _ => Err(BoardErrorCode::Invalid),
    }
}
fn resolve_actor(
    storage: &mut OfficeStore,
    actor: DecodedBoardActor,
) -> Result<Actor, BoardErrorCode> {
    match actor {
        DecodedBoardActor::Identity { identity_id, name } => {
            Ok(Actor::Identity { identity_id, name })
        }
        DecodedBoardActor::Owner => local_owner_actor(storage).map_err(|_| BoardErrorCode::Storage),
    }
}
fn author_filter(value: Option<ActorInput>) -> Result<Option<AuthorFilter>, BoardErrorCode> {
    match value {
        None => Ok(None),
        Some(ActorInput {
            kind,
            identity_id: None,
            name: None,
        }) if kind == "owner" => Ok(Some(AuthorFilter::Owner)),
        Some(ActorInput {
            kind,
            identity_id: Some(id),
            name: None,
        }) if kind == "identity" => Ok(Some(AuthorFilter::Identity(id))),
        _ => Err(BoardErrorCode::Invalid),
    }
}
fn category_value(v: &Category) -> Value {
    match v {
        Category::General => json!({"kind":"general"}),
        Category::Repository(repository_id) => {
            json!({"kind":"repository","repositoryId":repository_id})
        }
        Category::Room(room_id) => json!({"kind":"room","roomId":room_id}),
    }
}
fn actor_value(v: &Actor) -> Value {
    match v {
        Actor::Owner { .. } => json!({"kind":"owner"}),
        Actor::Identity { identity_id, name } => {
            json!({"kind":"identity","identityId":identity_id,"name":name})
        }
    }
}
fn entry_value(v: &office_board::Entry) -> Value {
    let mut o = serde_json::Map::from_iter([
        ("id".into(), json!(v.id)),
        ("threadId".into(), json!(v.thread_id)),
        ("category".into(), category_value(&v.category)),
        ("author".into(), actor_value(&v.author)),
        ("revision".into(), json!(v.revision)),
        ("deleted".into(), json!(v.deleted)),
        ("createdAtMs".into(), json!(v.created_at_ms)),
        ("updatedAtMs".into(), json!(v.updated_at_ms)),
    ]);
    if let Some(title) = &v.title {
        o.insert("title".into(), json!(title));
    }
    if let Some(body) = &v.body {
        o.insert("body".into(), json!(body));
    }
    Value::Object(o)
}
fn create_value(v: office_board::CreateReceipt) -> Value {
    json!({"entryId":v.entry_id,"threadId":v.thread_id,"revision":v.revision,"created":v.created,"operationId":v.operation_id})
}
fn edit_value(v: office_board::EditReceipt) -> Value {
    json!({"entryId":v.entry_id,"revision":v.revision,"changed":v.changed,"operationId":v.operation_id})
}
fn delete_value(v: office_board::DeleteReceipt) -> Value {
    json!({"entryId":v.entry_id,"revision":v.revision,"deleted":v.deleted,"changed":v.changed,"moderated":v.moderated,"operationId":v.operation_id})
}
fn list_value(v: office_board::ListResult) -> Value {
    let threads = v
        .threads
        .iter()
        .map(|thread| {
            let mut entry = entry_value(&thread.entry);
            if let Value::Object(ref mut object) = entry {
                object.remove("body");
                object.insert("replyCount".into(), json!(thread.reply_count));
                object.insert("activitySequence".into(), json!(thread.activity_sequence));
            }
            entry
        })
        .collect::<Vec<_>>();
    json!({"threads":threads,"nextCursor":v.next_cursor,"boardRevision":v.board_revision})
}
fn show_value(v: office_board::ShowResult) -> Value {
    let replies = v.replies.iter().map(entry_value).collect::<Vec<_>>();
    json!({"thread":entry_value(&v.thread),"replies":replies,"nextCursor":v.next_cursor,"boardRevision":v.board_revision})
}
fn categories_value(v: office_board::CategoryListResult) -> Value {
    let categories = v.categories.iter().map(category_value).collect::<Vec<_>>();
    json!({"categories":categories,"nextCursor":v.next_cursor,"boardRevision":v.board_revision})
}

#[cfg(test)]
mod contract_tests {
    use super::*;

    #[test]
    fn room_category_round_trips_without_accepting_cross_scope_fields() {
        let room_id = "11111111-1111-4111-8111-111111111111";
        let value = json!({"kind":"room","roomId":room_id});
        let decoded = category(serde_json::from_value(value.clone()).unwrap()).unwrap();
        assert_eq!(decoded, Category::Room(room_id.into()));
        assert_eq!(category_value(&decoded), value);
        for invalid in [
            json!({"kind":"room"}),
            json!({"kind":"general","roomId":room_id}),
            json!({"kind":"room","roomId":room_id,"repositoryId":"example.com/repo"}),
        ] {
            assert_eq!(
                category(serde_json::from_value(invalid).unwrap()),
                Err(BoardErrorCode::Invalid)
            );
        }
        for invalid_id in [
            "",
            "Review",
            "00000000-0000-0000-0000-000000000000",
            "11111111111141118111111111111111",
        ] {
            let input = serde_json::to_vec(&json!({
                "category":{"kind":"room","roomId":invalid_id},
                "actor":{"kind":"owner"},"title":"Review","body":"Body"
            }))
            .unwrap();
            assert!(matches!(
                prepare(OfficeInvocation::BoardPost, &input),
                Err(BoardErrorCode::Invalid)
            ));
        }
    }

    #[test]
    fn operation_inputs_reject_cross_operation_fields() {
        assert!(decode::<CategoriesInput>(br#"{"limit":20,"title":"x"}"#).is_err());
        assert!(decode::<PostInput>(br#"{"category":{"kind":"general"},"actor":{"kind":"owner"},"title":"t","body":"b","cursor":"x"}"#).is_err());
        assert!(decode::<ReplyInput>(br#"{"threadId":"11111111-1111-4111-8111-111111111111","actor":{"kind":"owner"},"body":"b","title":"x"}"#).is_err());
        assert!(decode::<DeleteInput>(br#"{"entryId":"11111111-1111-4111-8111-111111111111","actor":{"kind":"owner"},"ifRevision":1,"operationId":null}"#).is_err());
    }

    #[test]
    fn optional_nulls_are_accepted_but_required_fields_are_not() {
        assert!(decode::<CategoriesInput>(br#"{"limit":null,"cursor":null}"#).is_ok());
        assert!(decode::<PostInput>(br#"{"category":{"kind":"general"},"actor":{"kind":"owner"},"title":"t","body":"b","operationId":null}"#).is_ok());
        assert!(decode::<DeleteInput>(br#"{"entryId":"11111111-1111-4111-8111-111111111111","actor":{"kind":"owner"},"moderate":false,"ifRevision":1,"operationId":null}"#).is_ok());
    }

    #[test]
    fn invalid_wire_input_has_no_database_or_owner_side_effect() {
        let directory = crate::test_support::TestDirectory::new();
        let global = directory.path.join("missing");
        let database = global.join("state.db");
        let output = execute_at(
            OfficeInvocation::BoardPost,
            br#"{"category":{"kind":"general"},"actor":{"kind":"owner"},"title":"","body":"b"}"#,
            &crate::StorageLayout::within(&database, &global, &global.join("config.json")),
        );
        assert_eq!(output, br#"{"error":"BOARD_INVALID"}"#);
        assert!(!database.exists());
    }
}

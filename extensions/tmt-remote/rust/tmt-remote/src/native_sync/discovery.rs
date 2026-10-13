//! Stateless, generation-bound listing positions. No logical stream cursor or
//! retained run ledger; later runs recover arrivals behind an earlier position.
use super::client::Consumer;
use crate::{
    canonical, crypto,
    deploy_plan::sha256_hex,
    firebase_auth::Operation,
    firestore_budget::READ_LOOKUPS,
    limits,
    objects::IoBudget,
    sealed_sync::{
        DiscoveryContinuation, DiscoveryPage, DiscoveryQuery, SyncError, Target,
        validate_discovery, validate_fields,
    },
    wire,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Position {
    version: u8,
    binding: String,
    deadline: u64,
    after: String,
    pages: u16,
}
fn message(payload: &[u8]) -> Result<Vec<u8>, SyncError> {
    canonical::framed(&[b"tmt-native-discovery-v1", payload]).map_err(|_| SyncError::Invalid)
}
fn key(consumer: &Consumer) -> Result<&[u8; 32], SyncError> {
    consumer
        .discovery_key
        .get_or_init(|| {
            let mut key = [0; 32];
            getrandom::fill(&mut key).map_err(|_| SyncError::Unavailable)?;
            Ok(key)
        })
        .as_ref()
        .map_err(|e| *e)
}
fn seal(consumer: &Consumer, position: Position) -> Result<DiscoveryContinuation, SyncError> {
    let payload = serde_json::to_vec(&position).map_err(|_| SyncError::Unavailable)?;
    let mac = crypto::mac(key(consumer)?, &message(&payload)?);
    let token = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(payload),
        URL_SAFE_NO_PAD.encode(mac)
    );
    if token.len() > 1024 {
        return Err(SyncError::CatchUpRequired);
    }
    Ok(DiscoveryContinuation(token))
}
fn open(consumer: &Consumer, token: &DiscoveryContinuation) -> Result<Position, SyncError> {
    let (payload, mac) = token.0.split_once('.').ok_or(SyncError::Invalid)?;
    let payload = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| SyncError::Invalid)?;
    let mac = URL_SAFE_NO_PAD
        .decode(mac)
        .map_err(|_| SyncError::Invalid)?;
    crypto::verify_mac(key(consumer)?, &message(&payload)?, &mac)
        .map_err(|_| SyncError::Invalid)?;
    serde_json::from_slice(&payload).map_err(|_| SyncError::Invalid)
}
pub(super) async fn query(
    consumer: &Consumer,
    query: &DiscoveryQuery,
    io: &IoBudget<'_>,
) -> Result<DiscoveryPage, SyncError> {
    consumer.check(io)?;
    validate_discovery(&consumer.context, query)?;
    let resource = consumer.resource(&query.resource)?;
    let spec = resource
        .sync
        .as_ref()
        .and_then(|s| s.query.as_ref())
        .ok_or(SyncError::Unsupported)?;
    let discovery = spec.discovery.as_ref().ok_or(SyncError::Unsupported)?;
    let scope = serde_json::to_vec(&query.scope).map_err(|_| SyncError::Invalid)?;
    if scope.len() > 4096 {
        return Err(SyncError::Invalid);
    }
    let binding = sha256_hex(
        &json!({"project":consumer.context.identity().project,
        "uid":consumer.context.identity().uid,"extension":consumer.context.extension(),
        "declaration":consumer.context.declaration_digest(),"space":consumer.context.space(),
        "resource":query.resource,"shape":query.shape,"scope":query.scope})
        .to_string()
        .into_bytes(),
    );
    let now = consumer.runtime.clock.now().monotonic_ms;
    let position = match &query.continuation {
        Some(token) => {
            let position = open(consumer, token)?;
            if position.version != 1 || position.binding != binding {
                return Err(SyncError::Invalid);
            }
            if position.deadline <= now {
                return Err(SyncError::Deadline);
            }
            if position.pages >= limits::NATIVE_DISCOVERY_RUN_PAGES {
                return Err(SyncError::CatchUpRequired);
            }
            consumer.context.document_path(&Target {
                resource: query.resource.clone(),
                id: position.after.clone(),
            })?;
            position
        }
        None => Position {
            version: 1,
            binding,
            deadline: now.saturating_add(
                u64::try_from(
                    io.deadline
                        .saturating_duration_since(Instant::now())
                        .as_millis(),
                )
                .unwrap_or(u64::MAX),
            ),
            after: String::new(),
            pages: 0,
        },
    };
    let limit = consumer.limit(resource)?;
    let mut filters: Vec<Value> = query
        .scope
        .iter()
        .map(|(key, value)| {
            json!({"fieldFilter":{
        "field":{"fieldPath":key},"op":"EQUAL","value":value}})
        })
        .collect();
    if let Some(seq) = &discovery.seq_equals {
        filters.push(json!({"fieldFilter":{"field":{"fieldPath":spec.seq_field},"op":"EQUAL","value":{"integerValue":seq}}}));
    }
    let (parent, collection) = resource
        .path
        .rsplit_once('/')
        .unwrap_or(("", &resource.path));
    let path = format!(
        "{}/x/{}{}",
        consumer.root(),
        consumer.context.extension(),
        if parent.is_empty() {
            String::new()
        } else {
            format!("/{parent}")
        }
    );
    let mut structured = json!({"from":[{"collectionId":collection}],
        "where":{"compositeFilter":{"op":"AND","filters":filters}},
        "orderBy":[{"field":{"fieldPath":"__name__"},"direction":"ASCENDING"}],"limit":limit});
    if !position.after.is_empty() {
        let name = format!(
            "{}/{}",
            consumer.root(),
            consumer.context.document_path(&Target {
                resource: query.resource.clone(),
                id: position.after.clone()
            })?
        );
        structured["startAt"] = json!({"values":[{"referenceValue":name}],"before":false});
    }
    let remaining = position
        .deadline
        .saturating_sub(consumer.runtime.clock.now().monotonic_ms);
    if remaining == 0 {
        return Err(SyncError::Deadline);
    }
    let run_io = IoBudget {
        deadline: io
            .deadline
            .min(Instant::now() + Duration::from_millis(remaining)),
        cancelled: io.cancelled,
    };
    let response = consumer
        .send(
            Operation::Query,
            format!("https://firestore.googleapis.com/v1/{path}:runQuery"),
            json!({"structuredQuery":structured})
                .to_string()
                .into_bytes(),
            limit as u64 + READ_LOOKUPS,
            &run_io,
        )
        .await?
        .ok_or(SyncError::Unavailable)?;
    super::client::status(response.status)?;
    if response.body.len() > limits::NATIVE_SYNC_PAGE_BYTES {
        return Err(SyncError::CatchUpRequired);
    }
    let reply = wire::strict_json(&response.body).ok_or(SyncError::Unavailable)?;
    let mut documents = Vec::new();
    let mut after = position.after.clone();
    for row in reply.as_array().ok_or(SyncError::Unavailable)? {
        let Some(value) = row.get("document") else {
            if row.get("readTime").is_none() {
                return Err(SyncError::Unavailable);
            }
            continue;
        };
        if documents.len() >= limit {
            return Err(SyncError::CatchUpRequired);
        }
        let document = consumer.parse_document(value, &query.resource)?;
        let fields = validate_fields(&document.bytes)?;
        if query
            .scope
            .iter()
            .any(|(key, value)| fields.get(key) != Some(value))
        {
            return Err(SyncError::Denied);
        }
        if discovery
            .seq_equals
            .as_ref()
            .is_some_and(|seq| fields[&spec.seq_field]["integerValue"].as_str() != Some(seq))
        {
            return Err(SyncError::Denied);
        }
        if document.target.id <= after {
            return Err(SyncError::CatchUpRequired);
        }
        after = document.target.id.clone();
        documents.push(document);
    }
    consumer.check(io)?;
    if consumer.runtime.clock.now().monotonic_ms >= position.deadline {
        return Err(SyncError::Deadline);
    }
    if !documents.is_empty() && position.pages + 1 >= limits::NATIVE_DISCOVERY_RUN_PAGES {
        return Err(SyncError::CatchUpRequired);
    }
    let continuation = if documents.len() < limit {
        None
    } else {
        Some(seal(
            consumer,
            Position {
                after,
                pages: position.pages + 1,
                ..position
            },
        )?)
    };
    consumer.check(io)?;
    Ok(DiscoveryPage {
        documents,
        continuation,
    })
}

//! Strict local management inputs. Admission precedes the owner engine; DTOs never
//! carry machine-computed cuts, baselines, wraps or published epoch keys.
use serde::Deserialize;
use tmt_colab_model::{auth, bounded::List, crypto, framing, payload, values};

pub use crate::transitions::Code;
pub const PATH: &str = "/api/management";
pub const LOCAL_PATH: &str = "/.tmt/colab/management";
const PAYLOAD_BYTES: usize = 16 * 1024;
type Result<T> = std::result::Result<T, Code>;
type Syntax<T> = tmt_colab_model::Result<T>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    request: String,
    payload: String,
    signature: String,
}
/// Root-authorized IPC input. Only the private socket admits this route; Remote
/// refuses its reserved subtree. No fabricated browser device or trusted header.
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Local {
    space: String,
    page: String,
    expected_revision: String,
    operation_id: String,
    operation: String,
    payload: String,
}

/// A fully parsed selection. Only the service's live authorization plus the
/// engine's atomic revision/receipt fence may turn this into owner statements.
struct Command {
    page: String,
    operation_id: String,
    expected_revision: u64,
    transport_digest: [u8; 32],
    action: Action,
}
enum Action {
    Advance,
    Share(payload::ShareMode),
    History(payload::HistoryMode),
    Retention(payload::Days),
    Archive,
    Delete,
    MemberAdd(payload::MemberAdd),
    MemberRemove(MemberScope),
    MemberRole(RoleScope),
    LinkAdd(Link),
    LinkRemove(Reset),
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct MemberScope {
    member_id: String,
    pages: List<String, 256>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RoleScope {
    member_id: String,
    role: payload::Role,
    pages: List<String, 256>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Link {
    link_id: String,
    role: payload::Role,
    pages: List<String, 256>,
    seed: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Reset {
    link_id: String,
    pages: List<String, 256>,
    // Required even for removal: no omitted/null ambiguity in frozen requests.
    #[serde(deserialize_with = "required_option")]
    replacement: Option<Link>,
}
fn required_option<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<Link>, D::Error> {
    Option::deserialize(d)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Share {
    page_id: String,
    mode: payload::ShareMode,
}
fn typed<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Syntax<T> {
    serde_json::from_slice(bytes).map_err(|_| tmt_colab_model::Invalid)
}
fn pages(pages: &[String], initiating: Option<&str>) -> Syntax<()> {
    framing::id_list(&pages.iter().map(String::as_str).collect::<Vec<_>>(), false)?;
    for page in pages {
        values::generated_id(page)?;
    }
    if pages.is_empty() || initiating.is_some_and(|id| !pages.iter().any(|p| p == id)) {
        return Err(tmt_colab_model::Invalid);
    }
    Ok(())
}
fn link(value: &Link, initiating: Option<&str>) -> Syntax<()> {
    values::generated_id(&value.link_id)?;
    pages(value.pages.as_slice(), initiating)?;
    let mut seed = values::binary(&value.seed, 32)?;
    let valid = seed.len() == 32;
    seed.fill(0);
    if !valid {
        return Err(tmt_colab_model::Invalid);
    }
    Ok(())
}
fn action(operation: &str, bytes: &[u8], page: &str) -> Syntax<Action> {
    if bytes.len() > PAYLOAD_BYTES {
        return Err(tmt_colab_model::Invalid);
    }
    Ok(match operation {
        "epoch.advance" | "page.archive" | "page.delete" => {
            let value: payload::Page = typed(bytes)?;
            if value.page_id != page {
                return Err(tmt_colab_model::Invalid);
            }
            match operation {
                "epoch.advance" => Action::Advance,
                "page.archive" => Action::Archive,
                _ => Action::Delete,
            }
        }
        "page.share" => {
            let value: Share = typed(bytes)?;
            if value.page_id != page {
                return Err(tmt_colab_model::Invalid);
            }
            Action::Share(value.mode)
        }
        "page.history" => {
            let value: payload::PageHistory = typed(bytes)?;
            if value.page_id != page {
                return Err(tmt_colab_model::Invalid);
            }
            Action::History(value.mode)
        }
        "retention.set" => {
            let value: payload::RetentionSet = typed(bytes)?;
            if value.page_id != page {
                return Err(tmt_colab_model::Invalid);
            }
            if let payload::Days::Count(n) = value.days {
                values::time(n)?;
                if n == 0 {
                    return Err(tmt_colab_model::Invalid);
                }
            }
            Action::Retention(value.days)
        }
        "member.add" => {
            let value: payload::MemberAdd = typed(bytes)?;
            values::generated_id(&value.member_id)?;
            pages(value.pages.as_slice(), Some(page))?;
            let sign: [u8; 32] = values::binary(&value.sign_key, 32)?
                .try_into()
                .map_err(|_| tmt_colab_model::Invalid)?;
            crypto::public_key(&sign)?;
            let enc = values::binary(&value.enc_key, 32)?;
            if enc.len() != 32 || enc.iter().all(|b| *b == 0) {
                return Err(tmt_colab_model::Invalid);
            }
            Action::MemberAdd(value)
        }
        "member.remove" => {
            let value: MemberScope = typed(bytes)?;
            values::generated_id(&value.member_id)?;
            pages(value.pages.as_slice(), Some(page))?;
            Action::MemberRemove(value)
        }
        "member.role" => {
            let value: RoleScope = typed(bytes)?;
            values::generated_id(&value.member_id)?;
            pages(value.pages.as_slice(), Some(page))?;
            Action::MemberRole(value)
        }
        "link.add" => {
            let value: Link = typed(bytes)?;
            link(&value, Some(page))?;
            Action::LinkAdd(value)
        }
        "link.remove" => {
            let value: Reset = typed(bytes)?;
            values::generated_id(&value.link_id)?;
            pages(value.pages.as_slice(), Some(page))?;
            if let Some(replacement) = &value.replacement {
                link(replacement, None)?;
            }
            Action::LinkRemove(value)
        }
        _ => return Err(tmt_colab_model::Invalid),
    })
}
impl Command {
    /// The caller first obtains this key and sender from current Registration
    /// admission, never from the envelope or a browser-selected public key.
    fn browser(
        body: &[u8],
        space: &str,
        sender: &str,
        signer: &[u8; 32],
        now: u64,
    ) -> Result<Self> {
        let (request, payload, signature) = (|| -> Syntax<_> {
            if body.len() > crate::limits::HTTP_BODY_BYTES {
                return Err(tmt_colab_model::Invalid);
            }
            let value: Envelope = typed(body)?;
            let request = values::binary(&value.request, 1024)?;
            let payload = values::binary(&value.payload, PAYLOAD_BYTES)?;
            let signature: [u8; 64] = values::binary(&value.signature, 64)?
                .try_into()
                .map_err(|_| tmt_colab_model::Invalid)?;
            values::time(now)?;
            Ok((request, payload, signature))
        })()
        .map_err(|_| Code::Invalid)?;
        let decoded = auth::decode_management(&request, &payload).map_err(|_| Code::Invalid)?;
        if decoded.space != space || decoded.sender_device != sender {
            return Err(Code::Denied);
        }
        crypto::verify_signature(signer, &request, &signature).map_err(|_| Code::Denied)?;
        if now < decoded.issued_at || now >= decoded.expires_at {
            return Err(Code::Expired);
        }
        let selected =
            action(decoded.operation, &payload, decoded.page).map_err(|_| Code::Invalid)?;
        let digest = crypto::digest(
            &framing::frame(&[
                b"tmt-colab-management-transport-v1",
                &request,
                &payload,
                &signature,
            ])
            .map_err(|_| Code::Invalid)?,
        );
        Ok(Self {
            page: decoded.page.into(),
            operation_id: decoded.operation_id.into(),
            expected_revision: values::decimal(decoded.expected_revision, false)
                .map_err(|_| Code::Invalid)?,
            transport_digest: digest,
            action: selected,
        })
    }
    fn local(body: &[u8], space: &str) -> Result<Self> {
        let (selected_space, value) = (|| -> Syntax<_> {
            if body.len() > crate::limits::HTTP_BODY_BYTES {
                return Err(tmt_colab_model::Invalid);
            }
            let value: Local = typed(body)?;
            values::space_id(&value.space)?;
            values::generated_id(&value.page)?;
            values::generated_id(&value.operation_id)?;
            let revision = values::decimal(&value.expected_revision, false)?;
            let payload = values::binary(&value.payload, PAYLOAD_BYTES)?;
            let selected = action(&value.operation, &payload, &value.page)?;
            let digest = crypto::digest(&framing::frame(&[
                b"tmt-colab-local-management-transport-v1",
                body,
            ])?);
            Ok((
                value.space,
                Self {
                    page: value.page,
                    operation_id: value.operation_id,
                    expected_revision: revision,
                    transport_digest: digest,
                    action: selected,
                },
            ))
        })()
        .map_err(|_| Code::Invalid)?;
        if selected_space != space {
            return Err(Code::Denied);
        }
        Ok(value)
    }
}
pub fn status(code: Code) -> u16 {
    match code {
        Code::Invalid => 400,
        Code::Denied | Code::Expired => 403,
        Code::Conflict | Code::StaleHead => 409,
        Code::Capacity | Code::Unavailable => 503,
    }
}

pub fn browser(
    service: &mut crate::registration::Registration,
    space: &str,
    context: Option<&str>,
    body: &[u8],
    now: u64,
) -> Result<Vec<u8>> {
    let (sender, signer) = service
        .management_device(context, now)
        .map_err(|code| match code {
            crate::registration::Code::Invalid => Code::Invalid,
            crate::registration::Code::Denied => Code::Denied,
            crate::registration::Code::Expired => Code::Expired,
            crate::registration::Code::Conflict => Code::Conflict,
            crate::registration::Code::Unavailable | crate::registration::Code::Capacity => {
                Code::Unavailable
            }
        })?;
    apply(
        service,
        Command::browser(body, space, &sender, &signer, now)?,
        true,
        now,
    )
}
/// Offline callers use this same root-authorized service after taking the owner
/// lifecycle lock. Socket callers use it only on LOCAL_PATH under the sync lock.
pub fn local(
    service: &mut crate::registration::Registration,
    space: &str,
    body: &[u8],
    now: u64,
) -> Result<Vec<u8>> {
    apply(service, Command::local(body, space)?, false, now)
}
fn role(value: &payload::Role) -> &'static str {
    match value {
        payload::Role::Viewer => "viewer",
        payload::Role::Commenter => "commenter",
        payload::Role::Editor => "editor",
    }
}
fn link_spec<'a>(value: &'a Link, seed: &'a [u8; 32]) -> crate::transitions::LinkSpec<'a> {
    crate::transitions::LinkSpec {
        id: &value.link_id,
        seed,
        role: role(&value.role),
        pages: value.pages.as_slice().to_vec(),
    }
}
fn apply(
    service: &mut crate::registration::Registration,
    command: Command,
    scoped: bool,
    now: u64,
) -> Result<Vec<u8>> {
    use crate::transitions::{
        HistoryMode, LinkAction, MemberAction, OwnerAction, OwnerRequest, Publication,
        RequestScope, ShareMode,
    };
    let selected_pages = match &command.action {
        Action::MemberAdd(v) => v.pages.as_slice(),
        Action::MemberRemove(v) => v.pages.as_slice(),
        Action::MemberRole(v) => v.pages.as_slice(),
        Action::LinkAdd(v) => v.pages.as_slice(),
        Action::LinkRemove(v) => v.pages.as_slice(),
        _ => std::slice::from_ref(&command.page),
    };
    let scope = scoped.then(|| RequestScope {
        initiating_page: command.page.clone(),
        affected_pages: selected_pages.to_vec(),
    });
    let seed_text = match &command.action {
        Action::LinkAdd(v) => Some(v.seed.as_str()),
        Action::LinkRemove(v) => v.replacement.as_ref().map(|r| r.seed.as_str()),
        _ => None,
    };
    let mut seed: [u8; 32] = if let Some(value) = seed_text {
        values::binary(value, 32)
            .map_err(|_| Code::Invalid)?
            .try_into()
            .map_err(|_| Code::Invalid)?
    } else {
        [0; 32]
    };

    let action = match &command.action {
        Action::Advance => OwnerAction::EpochAdvance {
            page: &command.page,
        },
        Action::Archive => OwnerAction::Archive {
            page: &command.page,
        },
        Action::Delete => OwnerAction::Delete {
            page: &command.page,
        },
        Action::Share(mode) => OwnerAction::Share {
            page: &command.page,
            mode: match mode {
                payload::ShareMode::Private => ShareMode::Private,
                payload::ShareMode::Link => ShareMode::Link,
                payload::ShareMode::Public => ShareMode::Public,
            },
            publication: Publication::Loopback,
        },
        Action::History(mode) => OwnerAction::History {
            page: &command.page,
            mode: match mode {
                payload::HistoryMode::Shared => HistoryMode::Shared,
                payload::HistoryMode::Current => HistoryMode::Current,
            },
        },
        Action::Retention(days) => OwnerAction::Retention {
            page: &command.page,
            days: match days {
                payload::Days::Forever => None,
                payload::Days::Count(n) => Some(*n),
            },
        },
        Action::MemberAdd(v) => {
            OwnerAction::Member(MemberAction::Add(crate::store::owner::Recipient {
                kind: "member".into(),
                id: v.member_id.clone(),
                role: Some(role(&v.role).into()),
                signing_key: values::binary(&v.sign_key, 32)
                    .map_err(|_| Code::Invalid)?
                    .try_into()
                    .map_err(|_| Code::Invalid)?,
                encryption_key: values::binary(&v.enc_key, 32)
                    .map_err(|_| Code::Invalid)?
                    .try_into()
                    .map_err(|_| Code::Invalid)?,
                pages: v.pages.as_slice().to_vec(),
                revoked: false,
            }))
        }
        Action::MemberRemove(v) => OwnerAction::Member(MemberAction::Remove {
            member_id: v.member_id.clone(),
        }),
        Action::MemberRole(v) => OwnerAction::Member(MemberAction::Role {
            member_id: v.member_id.clone(),
            role: role(&v.role).into(),
        }),
        Action::LinkAdd(v) => OwnerAction::Link(LinkAction::Add(link_spec(v, &seed))),
        Action::LinkRemove(v) => OwnerAction::Link(if v.replacement.is_some() {
            LinkAction::Reset {
                link_id: &v.link_id,
                replacement: v.replacement.as_ref().map(|v| link_spec(v, &seed)),
            }
        } else {
            LinkAction::Remove {
                link_id: &v.link_id,
            }
        }),
    };
    let result = service.apply_owner(
        OwnerRequest {
            operation_id: &command.operation_id,
            expected_revision: command.expected_revision,
            action,
            transport_digest: Some(command.transport_digest),
            scope,
        },
        now,
    );
    seed.fill(0);
    let applied = result.map_err(|error| error.code)?;
    serde_json::to_vec(&serde_json::json!({"operationId":command.operation_id,
        "membershipHead":{"revision":applied.head.revision.to_string(),"statementHash":values::encode_binary(&applied.head.hash)}}))
        .map_err(|_| Code::Unavailable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use serde_json::{Value, json};
    const SPACE: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const PAGE: &str = "10000000-0000-4000-8000-000000000001";
    const DEVICE: &str = "20000000-0000-4000-8000-000000000001";
    const OTHER: &str = "20000000-0000-4000-8000-000000000002";
    const OP: &str = "30000000-0000-4000-8000-000000000001";
    fn signed(operation: &str, payload: &[u8], sender: &str, issued: u64) -> Vec<u8> {
        let input = auth::management_input(&auth::Management {
            space: SPACE,
            page: PAGE,
            operation_id: OP,
            expected_revision: "1",
            operation,
            payload,
            sender_device: sender,
            issued_at: issued,
            expires_at: issued + 100,
        })
        .unwrap();
        serde_json::to_vec(&json!({"request":values::encode_binary(&input),
            "payload":values::encode_binary(payload),
            "signature":values::encode_binary(&SigningKey::from_bytes(&[7;32]).sign(&input).to_bytes())})).unwrap()
    }
    fn browser(body: &[u8], now: u64) -> Result<Command> {
        Command::browser(
            body,
            SPACE,
            DEVICE,
            &SigningKey::from_bytes(&[7; 32]).verifying_key().to_bytes(),
            now,
        )
    }
    fn local(operation: &str, payload: Value) -> Vec<u8> {
        serde_json::to_vec(&json!({"space":SPACE,"page":PAGE,"operationId":OP,"expectedRevision":"1",
            "operation":operation,"payload":values::encode_binary(&serde_json::to_vec(&payload).unwrap())})).unwrap()
    }
    #[test]
    fn exact_signed_bytes_bind_sender_time_and_payload_without_reserialization() {
        let payload = serde_json::to_vec(&json!({"pageId":PAGE,"days":30})).unwrap();
        let body = signed("retention.set", &payload, DEVICE, 100);
        let first = browser(&body, 100).unwrap();
        assert!(matches!(
            first.action,
            Action::Retention(payload::Days::Count(30))
        ));
        assert_eq!(
            first.transport_digest,
            browser(&body, 199).unwrap().transport_digest
        );
        let changed = signed("retention.set", &payload, DEVICE, 101);
        assert_ne!(
            first.transport_digest,
            browser(&changed, 101).unwrap().transport_digest
        );
        let whitespace = [b" ".as_slice(), payload.as_slice()].concat();
        assert_ne!(
            first.transport_digest,
            browser(&signed("retention.set", &whitespace, DEVICE, 100), 100)
                .unwrap()
                .transport_digest
        );
        assert_eq!(
            browser(&signed("retention.set", &payload, OTHER, 100), 100).err(),
            Some(Code::Denied)
        );
        assert_eq!(browser(&body, 99).err(), Some(Code::Expired));
        assert_eq!(browser(&body, 200).err(), Some(Code::Expired));
        let mut value: Value = serde_json::from_slice(&body).unwrap();
        value["signature"] = values::encode_binary(&[0; 64]).into();
        assert_eq!(
            browser(&serde_json::to_vec(&value).unwrap(), 100).err(),
            Some(Code::Denied)
        );
    }
    #[test]
    fn strict_payloads_reject_duplicate_unknown_computed_fields_and_wrong_page() {
        let page = format!(r#"{{"pageId":"{PAGE}"}}"#);
        assert!(browser(&signed("page.archive", page.as_bytes(), DEVICE, 100), 100).is_ok());
        for raw in [
            format!(r#"{{"pageId":"{PAGE}","pageId":"{PAGE}"}}"#),
            format!(r#"{{"pageId":"{PAGE}","cuts":[]}}"#),
            format!(r#"{{"pageId":"{OTHER}"}}"#),
            format!("{page}{{}}"),
            format!("\u{feff}{page}"),
        ] {
            assert_eq!(
                browser(&signed("page.archive", raw.as_bytes(), DEVICE, 100), 100).err(),
                Some(Code::Invalid)
            );
        }
        let body = signed("page.archive", page.as_bytes(), DEVICE, 100);
        let mut value: Value = serde_json::from_slice(&body).unwrap();
        value["extra"] = true.into();
        assert_eq!(
            browser(&serde_json::to_vec(&value).unwrap(), 100).err(),
            Some(Code::Invalid)
        );
        let mut value: Value = serde_json::from_slice(&body).unwrap();
        value["request"] = format!("{}=", value["request"].as_str().unwrap()).into();
        assert_eq!(
            browser(&serde_json::to_vec(&value).unwrap(), 100).err(),
            Some(Code::Invalid)
        );
        let body = r#"{"request":"","request":"","payload":"","signature":""}"#;
        assert_eq!(browser(body.as_bytes(), 100).err(), Some(Code::Invalid));
    }
    #[test]
    fn local_dto_is_purpose_separated_and_canonical_scope_is_bounded() {
        let body = local("retention.set", json!({"pageId":PAGE,"days":null}));
        assert!(matches!(
            Command::local(&body, SPACE).unwrap().action,
            Action::Retention(payload::Days::Forever)
        ));
        let payload = serde_json::to_vec(&json!({"pageId":PAGE,"days":null})).unwrap();
        assert_ne!(
            Command::local(&body, SPACE).unwrap().transport_digest,
            browser(&signed("retention.set", &payload, DEVICE, 100), 100)
                .unwrap()
                .transport_digest
        );
        for days in [json!(0), json!(-1), json!(1.5), json!(9007199254740992u64)] {
            assert_eq!(
                Command::local(
                    &local("retention.set", json!({"pageId":PAGE,"days":days})),
                    SPACE
                )
                .err(),
                Some(Code::Invalid)
            );
        }
        let remove = |pages| local("member.remove", json!({"memberId":OTHER,"pages":pages}));
        assert!(Command::local(&remove(json!([PAGE])), SPACE).is_ok());
        for pages in [
            json!([]),
            json!([PAGE, PAGE]),
            json!([OTHER, PAGE]),
            json!([OTHER]),
            json!(vec![PAGE; 257]),
        ] {
            assert_eq!(
                Command::local(&remove(pages), SPACE).err(),
                Some(Code::Invalid)
            );
        }
        let reset = |replacement| {
            local(
                "link.remove",
                json!({"linkId":OTHER,"pages":[PAGE],"replacement":replacement}),
            )
        };
        assert!(Command::local(&reset(Value::Null), SPACE).is_ok());
        let replacement = json!({"linkId":OP,"role":"viewer","pages":[PAGE],"seed":values::encode_binary(&[8;32])});
        assert!(Command::local(&reset(replacement), SPACE).is_ok());
        assert_eq!(
            Command::local(
                &local("link.remove", json!({"linkId":OTHER,"pages":[PAGE]})),
                SPACE
            )
            .err(),
            Some(Code::Invalid)
        );
        assert_eq!(
            Command::local(&body, "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").err(),
            Some(Code::Denied)
        );
    }
}

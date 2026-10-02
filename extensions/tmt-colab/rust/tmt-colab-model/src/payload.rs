//! Strict operation-specific JSON schemas. Syntax does not execute transitions or widen authority.
use crate::{Invalid, Result, bounded::List, crypto, require, stream_cut, values, wrap};
use serde::Deserialize;
pub const MAX_BYTES: usize = 768 * 1024;
#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Viewer,
    Commenter,
    Editor,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShareMode {
    Private,
    Link,
    Public,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScriptMode {
    Static,
    Interactive,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct MemberAdd {
    pub member_id: String,
    pub role: Role,
    pub sign_key: String,
    pub enc_key: String,
    pub pages: List<String, 256>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct MemberRemove {
    pub member_id: String,
    pub cuts: List<Cut, 512>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct MemberRole {
    pub member_id: String,
    pub role: Role,
    pub cuts: List<Cut, 512>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct LinkAdd {
    pub link_id: String,
    pub role: Role,
    pub link_sign_key: String,
    pub link_enc_key: String,
    pub pages: List<String, 256>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct LinkRemove {
    pub link_id: String,
    pub cuts: List<Cut, 512>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct DeviceRevoke {
    pub device_id: String,
    pub cuts: List<Cut, 512>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BridgeAdd {
    pub machine_id: String,
    pub machine_sign_key: String,
    pub enc_key: String,
    pub pages: List<String, 256>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct EpochAdvance {
    pub page_id: String,
    pub epoch: String,
    pub cuts: List<Cut, 512>,
    pub baseline: Baseline,
    pub wraps: List<wrap::Envelope, 512>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PageShare {
    pub page_id: String,
    pub mode: ShareMode,
    pub epoch: String,
    #[serde(default, deserialize_with = "published_keys")]
    pub published_keys: Option<List<PublishedKey, 64>>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PageScripts {
    pub page_id: String,
    pub mode: ScriptMode,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RetentionSet {
    pub page_id: String,
    pub days: Days,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Page {
    pub page_id: String,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Cut {
    pub page_id: String,
    pub epoch: String,
    pub namespace: String,
    pub cut: String,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Baseline {
    pub page_id: String,
    pub epoch: String,
    pub source_digest: String,
    pub baseline_commitment: String,
    pub title: String,
    pub object_envelope_hash: String,
    pub membership_revision: String,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublishedKey {
    pub epoch: String,
    pub key: String,
}
#[derive(Debug)]
pub enum Payload {
    MemberAdd(MemberAdd),
    MemberRemove(MemberRemove),
    MemberRole(MemberRole),
    LinkAdd(LinkAdd),
    LinkRemove(LinkRemove),
    DeviceRevoke(DeviceRevoke),
    BridgeAdd(BridgeAdd),
    EpochAdvance(EpochAdvance),
    PageShare(PageShare),
    PageScripts(PageScripts),
    RetentionSet(RetentionSet),
    Archive(Page),
    Delete(Page),
}
fn key(value: &str) -> Result<[u8; 32]> {
    values::binary(value, 32)?.try_into().map_err(|_| Invalid)
}
fn signing_key(value: &str) -> Result<()> {
    crypto::public_key(&key(value)?)?;
    Ok(())
}
fn pages(v: &List<String, 256>) -> Result<()> {
    let ids: Vec<&str> = v.as_slice().iter().map(String::as_str).collect();
    crate::framing::id_list(&ids, false)?;
    Ok(())
}
fn cuts(v: &List<Cut, 512>) -> Result<()> {
    let mut previous = None;
    for item in v.as_slice() {
        values::generated_id(&item.page_id)?;
        let epoch = values::decimal(&item.epoch, false)?;
        values::namespace(&item.namespace)?;
        let raw = values::binary(&item.cut, 1024)?;
        let cut = stream_cut::decode(&raw)?;
        require(cut.namespace == item.namespace)?;
        let order = (
            &item.page_id,
            epoch,
            &item.namespace,
            cut.stream_id.to_owned(),
        );
        require(previous.as_ref().is_none_or(|p| p < &order))?;
        previous = Some(order);
    }
    Ok(())
}
fn parse<T: for<'a> Deserialize<'a>>(bytes: &[u8]) -> Result<T> {
    serde_json::from_slice(bytes).map_err(|_| Invalid)
}
pub fn decode(operation: &str, bytes: &[u8]) -> Result<Payload> {
    require(bytes.len() <= MAX_BYTES)?;
    let out = match operation {
        "member.add" => {
            let v: MemberAdd = parse(bytes)?;
            values::generated_id(&v.member_id)?;
            signing_key(&v.sign_key)?;
            key(&v.enc_key)?;
            pages(&v.pages)?;
            Payload::MemberAdd(v)
        }
        "member.remove" => {
            let v: MemberRemove = parse(bytes)?;
            values::generated_id(&v.member_id)?;
            cuts(&v.cuts)?;
            Payload::MemberRemove(v)
        }
        "member.role" => {
            let v: MemberRole = parse(bytes)?;
            values::generated_id(&v.member_id)?;
            cuts(&v.cuts)?;
            Payload::MemberRole(v)
        }
        "link.add" => {
            let v: LinkAdd = parse(bytes)?;
            values::generated_id(&v.link_id)?;
            signing_key(&v.link_sign_key)?;
            key(&v.link_enc_key)?;
            pages(&v.pages)?;
            Payload::LinkAdd(v)
        }
        "link.remove" => {
            let v: LinkRemove = parse(bytes)?;
            values::generated_id(&v.link_id)?;
            cuts(&v.cuts)?;
            Payload::LinkRemove(v)
        }
        "device.revoke" => {
            let v: DeviceRevoke = parse(bytes)?;
            values::generated_id(&v.device_id)?;
            cuts(&v.cuts)?;
            Payload::DeviceRevoke(v)
        }
        "bridge.add" => {
            let v: BridgeAdd = parse(bytes)?;
            values::generated_id(&v.machine_id)?;
            signing_key(&v.machine_sign_key)?;
            key(&v.enc_key)?;
            pages(&v.pages)?;
            Payload::BridgeAdd(v)
        }
        "epoch.advance" => {
            let v: EpochAdvance = parse(bytes)?;
            values::generated_id(&v.page_id)?;
            values::decimal(&v.epoch, false)?;
            cuts(&v.cuts)?;
            require(v.baseline.page_id == v.page_id && v.baseline.epoch == v.epoch)?;
            values::decimal(&v.baseline.membership_revision, false)?;
            for hash in [
                &v.baseline.source_digest,
                &v.baseline.baseline_commitment,
                &v.baseline.object_envelope_hash,
            ] {
                key(hash)?;
            }
            let mut previous = None;
            for w in v.wraps.as_slice() {
                let h = w.header()?;
                require(
                    h.page == v.page_id
                        && h.epoch == v.epoch
                        && h.membership_revision == v.baseline.membership_revision,
                )?;
                let order = (h.recipient_kind, h.recipient_id);
                require(previous.as_ref().is_none_or(|p| p < &order))?;
                previous = Some(order);
            }
            Payload::EpochAdvance(v)
        }
        "page.share" => {
            let v: PageShare = parse(bytes)?;
            values::generated_id(&v.page_id)?;
            let epoch = values::decimal(&v.epoch, false)?;
            let mut prior = 0;
            let mut current = false;
            if let Some(keys) = &v.published_keys {
                for k in keys.as_slice() {
                    let n = values::decimal(&k.epoch, false)?;
                    require(n > prior && n <= epoch)?;
                    key(&k.key)?;
                    prior = n;
                    current = n == epoch;
                }
            }
            require(if matches!(v.mode, ShareMode::Public) {
                current
            } else {
                v.published_keys
                    .as_ref()
                    .is_none_or(|k| k.as_slice().is_empty())
            })?;
            Payload::PageShare(v)
        }
        "page.scripts" => {
            let v: PageScripts = parse(bytes)?;
            values::generated_id(&v.page_id)?;
            Payload::PageScripts(v)
        }
        "retention.set" => {
            let v: RetentionSet = parse(bytes)?;
            values::generated_id(&v.page_id)?;
            if let Days::Count(n) = v.days {
                values::time(n)?;
                require(n > 0)?;
            }
            Payload::RetentionSet(v)
        }
        "page.archive" => {
            let v: Page = parse(bytes)?;
            values::generated_id(&v.page_id)?;
            Payload::Archive(v)
        }
        "page.delete" => {
            let v: Page = parse(bytes)?;
            values::generated_id(&v.page_id)?;
            Payload::Delete(v)
        }
        _ => return Err(Invalid),
    };
    Ok(out)
}

#[derive(Debug)]
pub enum Days {
    Forever,
    Count(u64),
}
impl<'de> Deserialize<'de> for Days {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct V;
        impl serde::de::Visitor<'_> for V {
            type Value = Days;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("null or a positive safe-integer day count")
            }
            fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Days, E> {
                Ok(Days::Forever)
            }
            fn visit_u64<E: serde::de::Error>(self, n: u64) -> std::result::Result<Days, E> {
                Ok(Days::Count(n))
            }
        }
        d.deserialize_any(V)
    }
}
fn published_keys<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<List<PublishedKey, 64>>, D::Error> {
    List::deserialize(d).map(Some)
}

//! Remote-owned LP/list semantics, implemented here without a Remote behavior dependency.
use crate::{Invalid, Result, require, values};

pub fn frame(fields: &[&[u8]]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    for field in fields {
        let n = u32::try_from(field.len()).map_err(|_| Invalid)?;
        out.try_reserve(field.len().checked_add(4).ok_or(Invalid)?)
            .map_err(|_| Invalid)?;
        out.extend_from_slice(&n.to_be_bytes());
        out.extend_from_slice(field);
    }
    Ok(out)
}
/// Bounds bytes/count before allocating; borrows the exact input and rejects trailing fields.
pub fn fields(input: &[u8], count: usize, max: usize) -> Result<Vec<&[u8]>> {
    require(input.len() <= max && count <= 256)?;
    let mut rest = input;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let prefix = rest.get(..4).ok_or(Invalid)?;
        let n = u32::from_be_bytes(prefix.try_into().map_err(|_| Invalid)?) as usize;
        rest = &rest[4..];
        out.push(rest.get(..n).ok_or(Invalid)?);
        rest = &rest[n..];
    }
    require(rest.is_empty())?;
    Ok(out)
}
pub fn id_list(ids: &[&str], core: bool) -> Result<Vec<u8>> {
    require(ids.len() <= 256)?;
    let mut out = (ids.len() as u32).to_be_bytes().to_vec();
    let mut previous = None;
    for id in ids {
        if core {
            values::core_id(id)?;
        } else {
            values::generated_id(id)?;
        }
        require(previous.is_none_or(|p| p < *id))?;
        out.extend_from_slice(&frame(&[id.as_bytes()])?);
        previous = Some(*id);
    }
    require(out.len() <= 10_244)?;
    Ok(out)
}
pub(crate) fn text(bytes: &[u8]) -> Result<&str> {
    std::str::from_utf8(bytes).map_err(|_| Invalid)
}

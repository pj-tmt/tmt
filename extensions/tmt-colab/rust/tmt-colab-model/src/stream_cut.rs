//! Namespace-bound cuts; log/chain resolution is a separate caller responsibility.
use crate::{
    Invalid, Result,
    framing::{fields, frame, text},
    require, values,
};

#[derive(Debug, PartialEq, Eq)]
pub struct StreamCut<'a> {
    pub stream_id: &'a str,
    pub namespace: &'a str,
    pub checkpoint_hash: Option<&'a [u8; 32]>,
    pub checkpoint_seq: &'a str,
    pub tail_head_seq: &'a str,
    pub tail_head_hash: &'a [u8; 32],
}
pub fn input(v: &StreamCut<'_>) -> Result<Vec<u8>> {
    values::generated_id(v.stream_id)?;
    values::namespace(v.namespace)?;
    let cp = values::decimal(v.checkpoint_seq, true)?;
    let tail = values::decimal(v.tail_head_seq, true)?;
    require(
        cp <= tail
            && (cp == 0) == v.checkpoint_hash.is_none()
            && (tail != 0 || *v.tail_head_hash == [0; 32]),
    )?;
    frame(&[
        b"tmt-colab-stream-cut-v1",
        b"1",
        v.stream_id.as_bytes(),
        v.namespace.as_bytes(),
        v.checkpoint_hash.map_or(&[][..], |h| h),
        v.checkpoint_seq.as_bytes(),
        v.tail_head_seq.as_bytes(),
        v.tail_head_hash,
    ])
}
pub fn decode(bytes: &[u8]) -> Result<StreamCut<'_>> {
    let f = fields(bytes, 8, 1024)?;
    require(f[0] == b"tmt-colab-stream-cut-v1" && f[1] == b"1")?;
    let v = StreamCut {
        stream_id: text(f[2])?,
        namespace: text(f[3])?,
        checkpoint_hash: if f[4].is_empty() {
            None
        } else {
            Some(f[4].try_into().map_err(|_| Invalid)?)
        },
        checkpoint_seq: text(f[5])?,
        tail_head_seq: text(f[6])?,
        tail_head_hash: f[7].try_into().map_err(|_| Invalid)?,
    };
    require(input(&v)? == bytes)?;
    Ok(v)
}

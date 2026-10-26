//! MCP envelopes over the shared bounded stdio frame owner.
use super::{INPUT_LIMIT, OUTPUT_LIMIT, Session, ToolCall};
use crate::framed_stdio;
use serde_json::Value;
use std::{
    io::{self, Write},
    os::fd::AsFd,
};

struct Encoded {
    bytes: Vec<u8>,
    limit: usize,
}
impl Write for Encoded {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit - self.bytes.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "MCP output exceeds its bound",
            ));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
pub(super) fn encode(value: &Value, limit: usize) -> io::Result<Vec<u8>> {
    let mut output = Encoded {
        bytes: Vec::new(),
        limit,
    };
    serde_json::to_writer(&mut output, value).map_err(io::Error::other)?;
    Ok(output.bytes)
}

pub(super) fn serve(
    input: &impl AsFd,
    output: &impl AsFd,
    mut call: impl FnMut(ToolCall) -> Result<Value, Value>,
) -> io::Result<()> {
    framed_stdio::require_redirected(input, output)?;
    let mut reader = framed_stdio::Reader::new(input, INPUT_LIMIT);
    let mut session = Session::default();
    while let Some(line) = reader.read_frame()? {
        if let Some(value) = session.receive(&line, &mut call) {
            let bytes = encode(&value, OUTPUT_LIMIT - 1)?;
            framed_stdio::publish(output, &bytes, OUTPUT_LIMIT - 1)?;
        }
    }
    Ok(())
}

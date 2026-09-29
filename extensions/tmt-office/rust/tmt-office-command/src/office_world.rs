//! Office world layout file boundary used by the CLI.
use tmt_office_model::codec::office_world::*;
use tmt_office_model::office_world::WorldLayout;

pub fn read_world_file(path: &std::path::Path) -> Result<WorldLayout, WorldCodecError> {
    let bytes = tmt_adapters::bounded_file::read(path, WORLD_DOCUMENT_LIMIT)
        .map_err(|_| WorldCodecError::InvalidJson)?;
    decode_world(&bytes)
}

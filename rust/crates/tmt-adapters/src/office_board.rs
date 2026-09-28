//! Local discussion-board wire limits shared by the CLI and the companion.

use tmt_office_model::office_board;

pub const BOARD_WIRE_LIMIT: usize = 65_536;
pub const BOARD_OUTPUT_LIMIT: usize = office_board::SERIALIZED_RESPONSE_MAX_BYTES;

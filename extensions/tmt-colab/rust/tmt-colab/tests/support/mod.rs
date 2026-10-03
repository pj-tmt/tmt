//! Invocation headroom for semantic tests on loaded hosts.
use std::{path::PathBuf, time::Duration};
use tmt_colab::decoder::Config;

pub const DECODER_DEADLINE: Duration = Duration::from_secs(60);

pub fn decoder_config(program: PathBuf) -> Config {
    Config {
        program,
        deadline: DECODER_DEADLINE,
    }
}

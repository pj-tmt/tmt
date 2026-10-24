//! Private emulator fixture requests may queue on loaded hosted runners even
//! after readiness. This bound is not a production Auth or transport deadline.
use std::time::Duration;

pub const EMULATOR_HTTP_TIMEOUT: Duration = Duration::from_secs(30);

//! One allowlisted structural preflight through the verified companion.

use super::invoke_bytes_bounded;
use std::{io, path::Path, time::Instant};
use tmt_office_model::codec::office_extension::preflight::PROTOCOL_INPUT_LIMIT;
use tmt_office_model::codec::office_extension::preflight::ValidationInput;
use tmt_office_model::codec::office_extension::preflight::ValidationReport;
use tmt_office_model::office_protocol::{OFFICE_PROTOCOL_OUTPUT_LIMIT, OfficeInvocation};

pub fn validate_office_extension(
    executable: &Path,
    input: &ValidationInput,
    deadline: Instant,
) -> io::Result<ValidationReport> {
    let input = serde_json::to_vec(input)?;
    if input.len() > PROTOCOL_INPUT_LIMIT {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Extension input exceeds its bound.",
        ));
    }
    let bytes = invoke_bytes_bounded(
        executable,
        OfficeInvocation::LocalExtensionValidate,
        &input,
        deadline,
        OFFICE_PROTOCOL_OUTPUT_LIMIT,
    )?;
    decode_report(&bytes)
}

fn decode_report(bytes: &[u8]) -> io::Result<ValidationReport> {
    let report: ValidationReport = serde_json::from_slice(bytes)?;
    if let ValidationReport::Valid {
        definition,
        instance,
        ..
    } = &report
        && (!tmt_office_model::indexed_art::valid_key(definition)
            || !tmt_office_model::indexed_art::valid_key(instance))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid extension validation report.",
        ));
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reports_are_exact_and_cannot_claim_runtime_authority() {
        for value in [
            r#"{"status":"valid","definition":"board","instance":"lobby","scope":"structureOnly"}"#,
            r#"{"status":"invalid","problem":"bindingMismatch"}"#,
        ] {
            assert!(decode_report(value.as_bytes()).is_ok());
        }
        for value in [
            r#"{"status":"valid","definition":"board","instance":"lobby","scope":"runtime"}"#,
            r#"{"status":"valid","definition":"../board","instance":"lobby","scope":"structureOnly"}"#,
            r#"{"status":"valid","definition":"board","instance":"lobby","scope":"structureOnly","authorized":true}"#,
            r#"{"status":"invalid","problem":"unknown"}"#,
            r#"{"status":"invalid","problem":"bindingMismatch","problem":"invalidInput"}"#,
        ] {
            assert!(decode_report(value.as_bytes()).is_err(), "{value}");
        }
    }
}

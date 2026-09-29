//! Internal optional companion entrypoint. No implicit authentication or service.

use std::{
    io::{self, Read, Write},
    process::ExitCode,
};
use tmt_office_model::office_protocol::{
    OfficeInvocation, encode_office_capabilities, encode_office_probe,
};

mod hooks_command;
#[cfg(feature = "local-service")]
mod local_assets;
#[cfg(feature = "local-service")]
mod local_service;
mod retirement_consumer;
mod storage_command;

fn main() -> ExitCode {
    let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    if arguments == ["__tmt-office-service", "1", "serve"] {
        #[cfg(feature = "local-service")]
        return local_service::run();
        #[cfg(not(feature = "local-service"))]
        {
            let _ = writeln!(
                io::stderr().lock(),
                "This Office build does not include the local service."
            );
            return ExitCode::FAILURE;
        }
    }
    if arguments == ["__tmt-office-service", "1", "asset-probe"] {
        #[cfg(feature = "local-service")]
        if local_assets::prove() {
            let _ = writeln!(io::stdout().lock(), "TMT-OFFICE-LOCAL/1");
            return ExitCode::SUCCESS;
        }
        let _ = writeln!(
            io::stderr().lock(),
            "This Office build has no valid embedded local UI."
        );
        return ExitCode::FAILURE;
    }
    if arguments
        .first()
        .is_some_and(|argument| argument == storage_command::PREFIX)
    {
        return storage_command::run(&arguments);
    }
    if arguments
        .first()
        .is_some_and(|argument| argument == hooks_command::PREFIX)
    {
        return hooks_command::run(&arguments);
    }
    if !arguments
        .first()
        .is_some_and(|argument| argument.to_string_lossy().starts_with("__tmt-office"))
    {
        return match tmt_office_command::public::execute(&arguments) {
            Ok(code) => ExitCode::from(code),
            Err(error) => {
                let _ = writeln!(
                    io::stderr().lock(),
                    "Could not write Office output: {error}"
                );
                ExitCode::FAILURE
            }
        };
    }
    let text = arguments
        .iter()
        .map(|arg| arg.to_str())
        .collect::<Option<Vec<_>>>();
    let invocation = text
        .as_deref()
        .ok_or("Office arguments must be UTF-8.")
        .and_then(OfficeInvocation::parse);
    let result = match invocation {
        Ok(OfficeInvocation::Probe) => {
            let version = env!("CARGO_PKG_VERSION")
                .parse()
                .expect("Cargo validates the package version");
            io::stdout()
                .lock()
                .write_all(encode_office_probe(&version).as_bytes())
        }
        Ok(OfficeInvocation::Capabilities) => io::stdout()
            .lock()
            .write_all(encode_office_capabilities().as_bytes()),
        Ok(operation) => {
            let mut input = Vec::new();
            match io::stdin()
                .lock()
                .take(input_sentinel_limit(operation))
                .read_to_end(&mut input)
            {
                Ok(_) => {
                    let output = if matches!(
                        operation,
                        OfficeInvocation::LocalWorldShow | OfficeInvocation::LocalWorldApply
                    ) {
                        tmt_office_storage::access::world::execute(operation, &input)
                    } else if matches!(
                        operation,
                        OfficeInvocation::StoragePlan | OfficeInvocation::StorageMigrate
                    ) {
                        tmt_office_storage::access::storage::execute(operation, &input)
                    } else if operation == OfficeInvocation::LocalExtensionValidate {
                        tmt_office_model::codec::office_extension::preflight::execute(&input)
                    } else if matches!(
                        operation,
                        OfficeInvocation::WhiteboardSnapshotShow
                            | OfficeInvocation::WhiteboardSnapshotImage
                    ) {
                        tmt_office_storage::access::whiteboard::execute(operation, &input)
                    } else if matches!(
                        operation,
                        OfficeInvocation::LocalPropValidate
                            | OfficeInvocation::LocalPropInstall
                            | OfficeInvocation::LocalPropRemove
                            | OfficeInvocation::LocalPropList
                            | OfficeInvocation::LocalPropShow
                    ) {
                        tmt_office_storage::access::prop::execute(operation, &input)
                    } else if matches!(
                        operation,
                        OfficeInvocation::LocalAvatarValidate
                            | OfficeInvocation::LocalAvatarInstall
                            | OfficeInvocation::LocalAvatarRemove
                            | OfficeInvocation::LocalAvatarList
                            | OfficeInvocation::LocalAvatarShow
                    ) {
                        tmt_office_storage::access::avatar::execute(operation, &input)
                    } else if matches!(
                        operation,
                        OfficeInvocation::LocalProfileShow | OfficeInvocation::LocalProfileApply
                    ) {
                        tmt_office_storage::access::profile::execute(operation, &input)
                    } else if matches!(
                        operation,
                        OfficeInvocation::BoardPost
                            | OfficeInvocation::BoardList
                            | OfficeInvocation::BoardShow
                            | OfficeInvocation::BoardReply
                            | OfficeInvocation::BoardEdit
                            | OfficeInvocation::BoardDelete
                            | OfficeInvocation::BoardCategories
                    ) {
                        tmt_office_storage::access::board::execute(operation, &input)
                    } else {
                        pairing(operation, &input)
                    };
                    io::stdout().lock().write_all(&output)
                }
                Err(error) => Err(error),
            }
        }
        Err(message) => {
            let _ = writeln!(io::stderr().lock(), "{message}");
            return ExitCode::FAILURE;
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}

/// Pairing operations reach core through the invoking `tmt`; retirement
/// delivery runs in this companion's consumer.
fn pairing(operation: OfficeInvocation, input: &[u8]) -> Vec<u8> {
    if operation == OfficeInvocation::Sync {
        return retirement_consumer::execute(input);
    }
    match retirement_consumer::CoreApi::discover() {
        Ok(core) => tmt_office_pairing::office_pairing::execute(
            operation,
            input,
            &core,
            &tmt_office_storage::retirement::OfficeRetirementFence::discover(),
        ),
        Err(error) => {
            serde_json::to_vec(&serde_json::json!({"error": error.code()})).expect("static error")
        }
    }
}

fn input_sentinel_limit(operation: OfficeInvocation) -> u64 {
    if matches!(
        operation,
        OfficeInvocation::LocalWorldShow | OfficeInvocation::LocalWorldApply
    ) {
        u64::try_from(tmt_office_model::codec::office_world::WORLD_ENVELOPE_LIMIT)
            .expect("world input bound fits u64")
            + 1
    } else if operation == OfficeInvocation::LocalExtensionValidate {
        u64::try_from(tmt_office_model::codec::office_extension::preflight::PROTOCOL_INPUT_LIMIT)
            .expect("extension input bound fits u64")
            + 1
    } else if matches!(
        operation,
        OfficeInvocation::LocalPropValidate | OfficeInvocation::LocalPropInstall
    ) {
        u64::try_from(tmt_office_model::codec::office_prop::PROTOCOL_INPUT_LIMIT)
            .expect("prop input bound fits u64")
            + 1
    } else if matches!(
        operation,
        OfficeInvocation::LocalAvatarValidate | OfficeInvocation::LocalAvatarInstall
    ) {
        u64::try_from(tmt_office_model::codec::office_avatar::PROTOCOL_INPUT_LIMIT)
            .expect("avatar input bound fits u64")
            + 1
    } else if matches!(
        operation,
        OfficeInvocation::BoardPost
            | OfficeInvocation::BoardList
            | OfficeInvocation::BoardShow
            | OfficeInvocation::BoardReply
            | OfficeInvocation::BoardEdit
            | OfficeInvocation::BoardDelete
            | OfficeInvocation::BoardCategories
    ) {
        65_537
    } else {
        4_097
    }
}

#[cfg(test)]
mod input_limit_tests {
    use super::*;
    #[test]
    fn operation_specific_input_sentinels_preserve_legacy_bounds() {
        assert_eq!(input_sentinel_limit(OfficeInvocation::PairBegin), 4_097);
        for operation in [
            OfficeInvocation::LocalWorldShow,
            OfficeInvocation::LocalWorldApply,
        ] {
            assert_eq!(
                input_sentinel_limit(operation),
                u64::try_from(tmt_office_model::codec::office_world::WORLD_ENVELOPE_LIMIT).unwrap()
                    + 1
            );
        }
        assert_eq!(input_sentinel_limit(OfficeInvocation::BoardPost), 65_537);
        assert_eq!(
            input_sentinel_limit(OfficeInvocation::LocalExtensionValidate),
            u64::try_from(
                tmt_office_model::codec::office_extension::preflight::PROTOCOL_INPUT_LIMIT
            )
            .unwrap()
                + 1
        );
        assert_eq!(
            input_sentinel_limit(OfficeInvocation::LocalPropInstall),
            u64::try_from(tmt_office_model::codec::office_prop::PROTOCOL_INPUT_LIMIT).unwrap() + 1
        );
        assert_eq!(
            input_sentinel_limit(OfficeInvocation::LocalAvatarInstall),
            u64::try_from(tmt_office_model::codec::office_avatar::PROTOCOL_INPUT_LIMIT).unwrap()
                + 1
        );
        assert_eq!(
            input_sentinel_limit(OfficeInvocation::BoardCategories),
            65_537
        );
    }
}

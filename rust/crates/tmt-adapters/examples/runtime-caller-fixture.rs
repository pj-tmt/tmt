//! Deterministic process-shape fixture, not a provider or a delivery adapter.
//! The Docker image copies this executable as `codex` so real ps observations
//! exercise the shared/independent runtime boundary without credentials or AI.
//! Native test support also uses its neutral modes to remove runtime ancestry.

use std::io::{self, Read, Write};
use std::os::unix::{net::UnixStream, process::CommandExt, process::ExitStatusExt};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use nix::unistd::getppid;

fn main() {
    let mut args = std::env::args_os().skip(1);
    let mode = args.next().expect("fixture mode");
    if mode == "neutral-setup" {
        let result = Command::new(std::env::current_exe().expect("fixture executable"))
            .arg("neutral-supervise")
            .arg(std::process::id().to_string())
            .args(args)
            .spawn();
        if let Err(error) = result {
            eprintln!("Start neutral supervisor: {error}");
            std::process::exit(1);
        }
        // The supervisor retains stdout/stderr in this owned launcher group.
        // Exiting removes the runtime ancestor before any selected CLI starts.
        return;
    }
    if mode == "neutral-supervise" {
        let parent = args.next().expect("setup PID");
        let socket = args.next().expect("control socket");
        let mut control = UnixStream::connect(&socket).expect("connect supervisor control");
        let result: io::Result<()> = (|| {
            let parent = parent
                .to_string_lossy()
                .parse::<i32>()
                .map_err(io::Error::other)?;
            let deadline = Instant::now() + Duration::from_secs(1);
            while getppid().as_raw() == parent {
                if Instant::now() >= deadline {
                    return Err(io::Error::other("Neutral parent was not adopted."));
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            if getppid().as_raw() != 1 {
                return Err(io::Error::other(
                    "Neutral parent requires adoption by PID 1.",
                ));
            }
            let status = Command::new(std::env::current_exe()?)
                .arg("neutral-execute")
                .arg(socket)
                .args(args)
                .stdin(Stdio::null())
                .process_group(0)
                .status()?;
            let signal = status
                .signal()
                .map(nix::sys::signal::Signal::try_from)
                .transpose()
                .map_err(io::Error::other)?
                .map(|signal| format!("{signal:?}"));
            report(
                &mut control,
                serde_json::json!({"status": status.code(), "signal": signal}),
            )
        })();
        if let Err(error) = result {
            report_error(&mut control, &error);
        }
        return;
    }
    if mode == "neutral-execute" {
        let socket = args.next().expect("control socket");
        let input = args.next().expect("input mode");
        let executable = args.next().expect("selected executable");
        let mut control = UnixStream::connect(socket).expect("connect bootstrap control");
        let result: io::Result<()> = (|| {
            report(
                &mut control,
                serde_json::json!({"group": std::process::id()}),
            )?;
            acknowledge(&mut control)?;
            let stdin = if input == "input" {
                Stdio::from(std::os::fd::OwnedFd::from(control.try_clone()?))
            } else if input == "ignore" {
                Stdio::null()
            } else {
                return Err(io::Error::other("Invalid neutral-parent input mode."));
            };
            // Rust-owned Unix streams are close-on-exec. Only the input clone
            // mapped to fd 0 survives, after the exact acknowledgement is read.
            Err(Command::new(executable).args(args).stdin(stdin).exec())
        })();
        if let Err(error) = result {
            report_error(&mut control, &error);
        }
        return;
    }
    assert!(mode == "app-server" || mode == "--no-daemon" || mode == "orphan");
    let command = args.next().expect("selected CLI executable");
    let mut child = std::process::Command::new(command);
    child
        .args(args)
        .env("CODEX_THREAD_ID", "11111111-1111-4111-8111-111111111111");
    if mode == "orphan" {
        // Preserve the leaked marker but leave no Codex process ancestor.
        panic!("exec fixture: {}", child.exec());
    }
    let status = child.status().expect("start selected CLI");
    std::process::exit(status.code().unwrap_or(1));
}

fn report(control: &mut UnixStream, value: serde_json::Value) -> io::Result<()> {
    let mut bytes = serde_json::to_vec(&value)?;
    bytes.push(b'\n');
    control.write_all(&bytes)
}

fn report_error(control: &mut UnixStream, error: &io::Error) {
    let code = error
        .raw_os_error()
        .map(nix::errno::Errno::from_raw)
        .map(|errno| format!("{errno:?}"))
        .unwrap_or_else(|| "NEUTRAL_PARENT_FAILED".into());
    let _ = report(
        control,
        serde_json::json!({"error": {"message": error.to_string(), "code": code}}),
    );
    std::process::exit(1);
}

fn acknowledge(control: &mut UnixStream) -> io::Result<()> {
    // The harness execution deadline owns this wait and can stop the whole
    // group. Socket options on a peer-closed macOS Unix stream can fail even
    // when the complete acknowledgement and input are already buffered.
    let mut acknowledgement = [0; 6];
    control.read_exact(&mut acknowledgement)?;
    if &acknowledgement != b"ready\n" {
        return Err(io::Error::other("Invalid CLI group acknowledgement."));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acknowledgement_leaves_the_input_bytes_untouched() {
        let (mut reader, mut writer) = UnixStream::pair().unwrap();
        writer.write_all(b"ready\ninput\0bytes").unwrap();
        drop(writer);
        acknowledge(&mut reader).unwrap();
        let mut input = Vec::new();
        reader.read_to_end(&mut input).unwrap();
        assert_eq!(input, b"input\0bytes");
    }

    #[test]
    fn acknowledgement_refuses_invalid_and_incomplete_protocol() {
        for bytes in [b"wrong\n".as_slice(), b"ready"] {
            let (mut reader, mut writer) = UnixStream::pair().unwrap();
            writer.write_all(bytes).unwrap();
            drop(writer);
            assert!(acknowledge(&mut reader).is_err());
        }
    }
}

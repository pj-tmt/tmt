//! Deterministic runtime-shaped parent for isolated hook lifecycle tests.
//! No provider, credentials, transcript access or model calls are involved.

use serde::Deserialize;
use serde_json::json;
use std::{
    fs,
    io::{self, BufRead, Read, Write},
    process::{Command, Stdio},
};

#[derive(Deserialize)]
struct Step {
    args: Vec<String>,
    input: Option<serde_json::Value>,
    #[serde(default)]
    measure: bool,
    checkpoint: Option<std::path::PathBuf>,
}

fn main() {
    let mut args: Vec<_> = std::env::args_os().skip(1).collect();
    if let Some(mock) = std::env::var_os("TMT_TEST_CLAUDE_MOCK") {
        if std::env::var_os("TMT_TEST_CLAUDE_NATIVE_CHANNEL").is_some()
            && let Some(index) = args.iter().position(|arg| arg == "--mcp-config")
        {
            std::process::exit(channel_mock(&args, index, &mock));
        }
        let status = Command::new(std::env::var_os("TMT_TEST_CLAUDE_NODE").expect("fixture node"))
            .arg(mock)
            .args(&args)
            .status()
            .expect("run model-free MCP peer");
        std::process::exit(status.code().unwrap_or(1));
    }

    if args.first().is_some_and(|arg| arg == "app-server") {
        args.remove(0);
    }
    scripted_launch_options(&mut args);
    let listen = args.last().is_some_and(|value| value == "--listen");
    if listen {
        args.pop();
    }
    assert_eq!(
        args.len(),
        3,
        "CLI, scenario and report paths, optional --listen"
    );
    let steps: Vec<Step> = serde_json::from_slice(&fs::read(&args[1]).unwrap()).unwrap();
    let mut results = Vec::new();
    for step in steps {
        if let Some(checkpoint) = step.checkpoint {
            fs::write(&checkpoint, "ready").unwrap();
            let limit = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while fs::read_to_string(&checkpoint).unwrap() != "continue" {
                assert!(
                    std::time::Instant::now() < limit,
                    "checkpoint was not released"
                );
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
        let started = step.measure.then(std::time::Instant::now);
        let mut child = Command::new(&args[0])
            .args(step.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn fixture-owned CLI");
        if let Some(input) = step.input {
            child
                .stdin
                .take()
                .unwrap()
                .write_all(input.to_string().as_bytes())
                .unwrap();
        }
        drop(child.stdin.take());
        let output = child.wait_with_output().expect("reap fixture-owned CLI");
        let elapsed_ms = started.map(|start| start.elapsed().as_secs_f64() * 1000.0);
        let badge = Command::new("tmux")
            .args(["-u", "show-options", "-p", "-qv", "-t"])
            .arg(std::env::var("TMUX_PANE").unwrap())
            .arg("@tmt.badge")
            .output()
            .expect("read fixture pane badge");
        let mut result = json!({"code":output.status.code(),
            "badge": String::from_utf8(badge.stdout).unwrap().trim(),
            "stdout": String::from_utf8(output.stdout).unwrap(),
            "stderr": String::from_utf8(output.stderr).unwrap()});
        if let Some(elapsed_ms) = elapsed_ms {
            result["elapsedMs"] = json!(elapsed_ms);
        }
        results.push(result);
    }
    let stage = std::path::PathBuf::from(&args[2]).with_extension("pending");
    fs::write(&stage, serde_json::to_vec(&results).unwrap()).unwrap();
    fs::rename(stage, &args[2]).unwrap();
    if listen {
        let mut received = Vec::new();
        for line in std::io::stdin().lock().lines() {
            let line = line.unwrap();
            let done = line.starts_with("Submit your response with the command above.");
            received.push(line);
            if done {
                break;
            }
        }
        fs::write(
            std::path::PathBuf::from(&args[2]).with_extension("input.json"),
            serde_json::to_vec(&received).unwrap(),
        )
        .unwrap();
    }
}

// Legacy scripted scenarios retain their three positional paths. Both provider
// launch-hook options are settings, not an extra scenario/report path. The
// model-free channel peers execute the generated hooks instead.
fn scripted_launch_options(args: &mut Vec<std::ffi::OsString>) {
    if args.len() < 2 {
        return;
    }
    let option = &args[args.len() - 2];
    let value = args.last().unwrap().to_str().unwrap();
    if option == "--settings" {
        let settings: serde_json::Value = serde_json::from_str(value).unwrap();
        assert!(settings.is_object(), "session settings must be an object");
    } else if option == "-c" {
        let settings: toml_edit::DocumentMut = value.parse().unwrap();
        assert!(
            settings.get("hooks").is_some_and(|v| v.is_table_like()),
            "session override must contain hooks"
        );
    } else {
        return;
    }
    args.truncate(args.len() - 2);
}

/// Keep the MCP server a direct child of the admitted native provider. The
/// delegated JavaScript peer otherwise becomes the channel's recorded owner,
/// unlike the native process its real lifecycle hooks correctly report.
fn channel_mock(args: &[std::ffi::OsString], index: usize, mock: &std::ffi::OsStr) -> i32 {
    use std::{
        net::Shutdown,
        os::unix::net::UnixListener,
        time::{Duration, Instant},
    };

    let config: serde_json::Value =
        serde_json::from_str(args[index + 1].to_str().unwrap()).unwrap();
    let server = &config["mcpServers"]["tmt"];
    let mut channel = Command::new(server["command"].as_str().unwrap())
        .args(
            server["args"]
                .as_array()
                .unwrap()
                .iter()
                .map(|arg| arg.as_str().unwrap()),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn native provider's MCP child");
    let socket = std::path::PathBuf::from(std::env::var_os("MOCK_CHANNEL_LOG").unwrap())
        .with_extension("mcp.sock");
    let listener = UnixListener::bind(&socket).expect("bind fixture-owned MCP bridge");
    listener.set_nonblocking(true).unwrap();
    let mut peer = Command::new(std::env::var_os("TMT_TEST_CLAUDE_NODE").unwrap())
        .arg(mock)
        .args(args)
        .env("TMT_TEST_CLAUDE_MCP_SOCKET", &socket)
        .spawn()
        .expect("spawn model-free MCP client");
    let deadline = Instant::now() + Duration::from_secs(5);
    let connection = loop {
        match listener.accept() {
            Ok((connection, _)) => break connection,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline || peer.try_wait().unwrap().is_some() {
                    let _ = peer.kill();
                    let _ = peer.wait();
                    let _ = channel.kill();
                    let _ = channel.wait();
                    fs::remove_file(&socket).unwrap();
                    panic!("MCP peer did not connect to its native parent");
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("accept fixture MCP peer: {error}"),
        }
    };
    drop(listener);
    fs::remove_file(socket).unwrap();
    // Streaming copies must wait for the next frame, including on hosts whose
    // accepted socket inherits the listener's nonblocking mode.
    connection.set_nonblocking(false).unwrap();
    let mut input = connection.try_clone().unwrap();
    let mut output = connection.try_clone().unwrap();
    let mut stdin = channel.stdin.take().unwrap();
    let mut stdout = channel.stdout.take().unwrap();
    let status = std::thread::scope(|scope| {
        let client_to_server = scope.spawn(move || {
            // EOF closes the real MCP child's stdin before the peer's close receipt.
            forward_mcp(&mut input, &mut stdin)
        });
        let server_to_client = scope.spawn(move || {
            let result = forward_mcp(&mut stdout, &mut output);
            let _ = output.shutdown(Shutdown::Write);
            result
        });
        let status = peer.wait().expect("reap model-free MCP client");
        let _ = connection.shutdown(Shutdown::Both);
        if !status.success() {
            let _ = channel.kill();
        }
        let channel_status = channel.wait().expect("reap native provider's MCP child");
        assert!(channel_status.success() || !status.success());
        if status.success() {
            client_to_server
                .join()
                .unwrap()
                .expect("forward MCP client to native child");
            server_to_client
                .join()
                .unwrap()
                .expect("forward native MCP child to client");
        }
        status
    });
    status.code().unwrap_or(1)
}

/// Forward each available chunk immediately. The peer must receive short MCP
/// frames while both streams remain open, rather than waiting for copy EOF.
fn forward_mcp(input: &mut impl Read, output: &mut impl Write) -> io::Result<u64> {
    let mut buffer = [0; 8192];
    let mut copied = 0;
    loop {
        let count = match input.read(&mut buffer) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            result => result?,
        };
        if count == 0 {
            return Ok(copied);
        }
        output.write_all(&buffer[..count])?;
        output.flush()?;
        copied += count as u64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{net::Shutdown, os::unix::net::UnixStream, time::Duration};

    #[test]
    fn scripted_paths_and_listen_survive_both_provider_launch_hook_options() {
        let original: Vec<std::ffi::OsString> = ["/cli", "/scenario", "/report", "--listen"]
            .map(Into::into)
            .to_vec();
        for (option, value) in [
            ("--settings", r#"{"hooks":{"Stop":[]}}"#),
            ("-c", "hooks={Stop=[]}"),
        ] {
            let mut args = original.clone();
            args.extend([option.into(), value.into()]);
            scripted_launch_options(&mut args);
            assert_eq!(args, original);
        }
    }

    #[test]
    fn forwards_two_short_frames_before_stream_eof_and_reaps_the_peer() {
        let (connection, mut client) = UnixStream::pair().unwrap();
        let mut input = connection.try_clone().unwrap();
        let mut output = connection.try_clone().unwrap();
        let mut peer = Command::new("cat")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut stdin = peer.stdin.take().unwrap();
        let mut stdout = peer.stdout.take().unwrap();
        std::thread::scope(|scope| {
            let inbound = scope.spawn(move || forward_mcp(&mut input, &mut stdin));
            let outbound = scope.spawn(move || {
                let result = forward_mcp(&mut stdout, &mut output);
                let _ = output.shutdown(Shutdown::Write);
                result
            });
            client
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let receipt = (|| -> io::Result<()> {
                for frame in [b"initialize\n".as_slice(), b"initialized\n"] {
                    client.write_all(frame)?;
                    let mut echoed = vec![0; frame.len()];
                    client.read_exact(&mut echoed)?;
                    if echoed != frame {
                        return Err(io::Error::other("MCP bridge changed the frame"));
                    }
                }
                Ok(())
            })();
            // Close and reap even when a receipt fails; a red test must not
            // leave a blocked forwarding thread or its process behind.
            let _ = client.shutdown(Shutdown::Both);
            let _ = connection.shutdown(Shutdown::Both);
            let status = peer.wait().unwrap();
            let inbound = inbound.join().unwrap();
            let outbound = outbound.join().unwrap();
            assert!(status.success());
            receipt.expect("receive both short frames before closing the stream");
            assert_eq!(inbound.unwrap(), 23);
            assert_eq!(outbound.unwrap(), 23);
        });
    }
}

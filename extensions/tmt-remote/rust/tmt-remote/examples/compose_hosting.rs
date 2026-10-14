//! Developer-only offline Hosting composition; never registered in the shipped CLI.
#[path = "../tests/support/hosting_vector.rs"]
mod hosting_vector;
use hosting_vector::Refusal;
use std::{ffi::OsString, io::Write, path::Path};
use tmt_remote::{deploy_discovery::DiscoveryRefusal, hosting::HostingRefusal, limits};

fn run(args: &[OsString]) -> Result<Vec<u8>, Refusal> {
    if args.len() != 2 {
        return Err(Refusal::Usage);
    }
    let declaration = hosting_vector::read_reply(
        Path::new(&args[0]),
        limits::DEPLOY_DECLARATION_REPLY_BYTES,
        DiscoveryRefusal::Envelope,
    )?;
    let bundle = hosting_vector::read_reply(
        Path::new(&args[1]),
        limits::HOSTING_REPLY_BYTES,
        DiscoveryRefusal::Hosting(HostingRefusal::Bounds),
    )?;
    let view = hosting_vector::compose(&declaration, &bundle)?;
    let mut output = serde_json::to_vec(&view).map_err(|_| Refusal::OutputUnavailable)?;
    output.push(b'\n');
    Ok(output)
}

pub(crate) fn execute(args: &[OsString], stdout: &mut impl Write, stderr: &mut impl Write) -> i32 {
    // Validation and complete serialization precede the only stdout write.
    let result = run(args).and_then(|output| {
        stdout
            .write_all(&output)
            .map_err(|_| Refusal::OutputUnavailable)
    });
    match result {
        Ok(()) => 0,
        Err(refusal) => {
            let _ = writeln!(stderr, "{}", refusal.code());
            1
        }
    }
}

fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    std::process::exit(execute(
        &args,
        &mut std::io::stdout(),
        &mut std::io::stderr(),
    ));
}

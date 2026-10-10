//! Developer-only offline regeneration; never registered in the shipped CLI.
#[path = "../tests/support/deploy_vector.rs"]
mod deploy_vector;
use std::{
    env, fs,
    io::{self, Write},
    path::PathBuf,
};

fn run() -> Result<(), &'static str> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err("usage: compose_firestore <envelope.json> <staging-directory>");
    }
    let input = PathBuf::from(&args[0]);
    let output = PathBuf::from(&args[1]);
    let composed = deploy_vector::compose(&deploy_vector::read_envelope(&input)?)?;
    // All validation precedes file creation. An I/O failure reports failure, never a
    // successful regeneration; run into staging and review before copying goldens.
    fs::create_dir_all(&output).map_err(|_| "output-unavailable")?;
    for (name, bytes) in deploy_vector::OUTPUTS.iter().zip(&composed.files) {
        fs::write(output.join(name), bytes).map_err(|_| "output-unavailable")?;
    }
    io::stdout()
        .write_all(composed.summary.as_bytes())
        .map_err(|_| "output-unavailable")
}
fn main() {
    if let Err(code) = run() {
        eprintln!("{code}");
        std::process::exit(1);
    }
}

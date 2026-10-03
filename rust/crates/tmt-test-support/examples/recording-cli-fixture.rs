//! Native recording delegate for synthetic extension-upgrade driver archives.
//! The archive's synthetic installation note supplies absolute fixture paths,
//! so the verifier can keep its empty PATH and cleared environment.

use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::{ffi::OsStrExt, process::CommandExt};
use std::path::PathBuf;
use std::process::Command;

fn main() -> io::Result<()> {
    let note = env::current_exe()?
        .parent()
        .ok_or_else(|| io::Error::other("Missing fixture archive directory"))?
        .join("NATIVE-INSTALL.md");
    let config: serde_json::Value = serde_json::from_slice(&fs::read(note)?)?;
    let path = |key: &str| -> io::Result<PathBuf> {
        let value = config[key]
            .as_str()
            .ok_or_else(|| io::Error::other(format!("Missing recording fixture {key}")))?;
        let path = PathBuf::from(value);
        if !path.is_absolute() {
            return Err(io::Error::other(format!("Fixture {key} must be absolute")));
        }
        Ok(path)
    };
    let executable = path("executable")?;
    let log = path("log")?;
    let args = env::args_os().skip(1).collect::<Vec<_>>();
    {
        let mut output = OpenOptions::new().create(true).append(true).open(log)?;
        for index in 0..2 {
            if index > 0 {
                output.write_all(b" ")?;
            }
            if let Some(arg) = args.get(index) {
                output.write_all(arg.as_bytes())?;
            }
        }
        output.write_all(b"\n")?;
    }
    // Replace this process so exit status, signals, stdio, cwd and environment
    // remain the selected CLI's, without a shell or a second lifetime owner.
    Err(Command::new(executable).args(args).exec())
}

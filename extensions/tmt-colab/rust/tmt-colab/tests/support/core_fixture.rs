//! Immutable Core and decoder fixture programs; every side effect remains fixture-local.
use sha2::{Digest, Sha256};
use std::{
    fs, io,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
};

#[derive(Clone, Copy)]
pub enum Program {
    Core,
    DataRoot,
    Door,
    Remote,
    Decoder,
}
impl Program {
    fn index(self) -> usize {
        match self {
            Self::Core => 0,
            Self::DataRoot => 1,
            Self::Door => 2,
            Self::Remote => 3,
            Self::Decoder => 4,
        }
    }
}
const PROGRAMS: [(&str, &str); 5] = [
    (
        "core",
        r#"#!/bin/sh
if [ "$1" = __fixture_ready ]; then exit 0; fi
cd "$TMT_COLAB_TEST_ROOT" || exit 9
if [ "$1 $2 $3" = 'identity show --json' ]; then
printf '%s\n' "$*" >> publisher-calls
[ -f publisher ] || exit 9
cat publisher
exit 0
fi
[ "$#" = 1 ] && [ "$1" = api ] || exit 9
printf '%s\n' "$*" >> calls
cat > input
printf '%s\n' "$TMT_COLAB_TEST_REPLY"
"#,
    ),
    (
        "data-root",
        r#"#!/bin/sh
if [ "$1" = __fixture_ready ]; then exit 0; fi
cat >/dev/null
printf '%s\n' "$TMT_COLAB_TEST_REPLY"
"#,
    ),
    (
        "door",
        r#"#!/bin/sh
if [ "$1" = __fixture_ready ]; then exit 0; fi
if [ "$1 $2 $3" = 'remote status --json' ]; then
. "$TMT_COLAB_TEST_ROOT/door-status"
fi
exec "$TMT_COLAB_TEST_ROOT/core" "$@"
"#,
    ),
    (
        "remote",
        r#"#!/bin/sh
if [ "$1" = __fixture_ready ]; then exit 0; fi
cd "$TMT_COLAB_TEST_ROOT" || exit 9
case "$1 $2 $3" in
'remote status --json')
. ./remote-status
;;
'remote devices --json')
[ -f devices.json ] && cat devices.json && exit 0
exit 1
;;
'remote serve --json')
printf '%s\n' "$*" >> serve.calls
echo $$ > serve.pid
. ./remote-serve
;;
esac
exec "$TMT_COLAB_TEST_ROOT/core" "$@"
"#,
    ),
    (
        "decoder",
        r#"#!/bin/sh
if [ "$1" = __fixture_ready ]; then exit 0; fi
. "${0%/*}/behavior"
"#,
    ),
];
static DIRECTORY: OnceLock<PathBuf> = OnceLock::new();

fn verify(directory: &Path) -> io::Result<()> {
    if !fs::symlink_metadata(directory)?.file_type().is_dir()
        || fs::read_dir(directory)?.count() != PROGRAMS.len()
    {
        return Err(io::Error::other("unexpected fake Core fixture directory"));
    }
    for (name, source) in PROGRAMS {
        let path = directory.join(name);
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.file_type().is_file()
            || metadata.permissions().mode() & 0o777 != 0o700
            || fs::read(&path)? != source.as_bytes()
        {
            return Err(io::Error::other("fake Core fixture bytes or mode differ"));
        }
    }
    Ok(())
}
struct Publication(PathBuf);
impl Drop for Publication {
    fn drop(&mut self) {
        // The path is an exclusively created sibling, never the immutable destination.
        if let Err(error) = fs::remove_dir_all(&self.0)
            && error.kind() != io::ErrorKind::NotFound
        {
            if std::thread::panicking() {
                eprintln!("fake Core partial publication cleanup failed: {error}");
            } else {
                panic!("fake Core partial publication cleanup failed: {error}");
            }
        }
    }
}
fn publish() -> io::Result<PathBuf> {
    let base = Path::new(env!("CARGO_TARGET_TMPDIR"));
    fs::create_dir_all(base)?;
    let mut digest = Sha256::new();
    for (name, source) in PROGRAMS {
        digest.update((name.len() as u64).to_be_bytes());
        digest.update(name.as_bytes());
        digest.update((source.len() as u64).to_be_bytes());
        digest.update(source.as_bytes());
    }
    let hash = digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let directory = base.join(format!("colab-core-fixture-{hash}"));
    match fs::symlink_metadata(&directory) {
        Ok(_) => verify(&directory)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let mut random = [0; 16];
            getrandom::fill(&mut random).map_err(io::Error::other)?;
            let suffix = random
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            let staging = base.join(format!(
                "colab-core-publication-{}-{suffix}",
                std::process::id()
            ));
            fs::create_dir(&staging)?;
            let publication = Publication(staging);
            for (name, source) in PROGRAMS {
                tmt_test_support::write_executable(
                    &publication.0.join(name),
                    source.as_bytes(),
                    0o700,
                )?;
            }
            verify(&publication.0)?;
            if let Err(error) = fs::rename(&publication.0, &directory) {
                // A concurrent publisher may have won; never overwrite or repair its files.
                if fs::symlink_metadata(&directory).is_err() {
                    return Err(error);
                }
                verify(&directory)?;
            }
        }
        Err(error) => return Err(error),
    }
    verify(&directory)?;
    for program in [
        Program::Core,
        Program::DataRoot,
        Program::Door,
        Program::Remote,
        Program::Decoder,
    ] {
        let output = Command::new(directory.join(PROGRAMS[program.index()].0))
            .arg("__fixture_ready")
            .output()?;
        if !output.status.success() || !output.stdout.is_empty() || !output.stderr.is_empty() {
            return Err(io::Error::other(
                "fake Core no-effect publication probe failed",
            ));
        }
    }
    Ok(directory)
}

pub fn directory() -> &'static Path {
    DIRECTORY.get_or_init(|| publish().expect("publish immutable fake Core scripts"))
}
pub fn link(root: &Path, name: &str, program: Program) -> PathBuf {
    let path = root.join(name);
    let target = directory().join(PROGRAMS[program.index()].0);
    match fs::symlink_metadata(&path) {
        Ok(_) => assert_eq!(fs::read_link(&path).unwrap(), target),
        Err(error) if error.kind() == io::ErrorKind::NotFound => symlink(&target, &path).unwrap(),
        Err(error) => panic!("fake Core link inspection failed: {error}"),
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{os::unix::fs::MetadataExt, process::Stdio, thread};

    fn root() -> Publication {
        let mut random = [0; 16];
        getrandom::fill(&mut random).unwrap();
        let suffix = random
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let path =
            Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("colab-fixture-control-{suffix}"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::create_dir(&path).unwrap();
        Publication(path)
    }
    #[test]
    fn concurrent_wrappers_keep_replies_and_calls_root_local() {
        let workers = (0..4)
            .map(|i| {
                thread::spawn(move || {
                    let root = root();
                    link(&root.0, "core", Program::Core);
                    let wrapper = link(&root.0, "core-remote", Program::Remote);
                    fs::write(root.0.join("remote-status"), "exit 1\n").unwrap();
                    fs::write(root.0.join("remote-serve"), "exit 2\n").unwrap();
                    let reply = format!("{{\"fixture\":{i}}}");
                    fs::write(root.0.join("request"), b"{\"operation\":\"storage.root\"}").unwrap();
                    let output = Command::new(wrapper)
                        .env_clear()
                        .env("PATH", "/usr/bin:/bin")
                        .env("TMT_COLAB_TEST_ROOT", &root.0)
                        .env("TMT_COLAB_TEST_REPLY", &reply)
                        .arg("api")
                        .stdin(Stdio::from(fs::File::open(root.0.join("request")).unwrap()))
                        .output()
                        .unwrap();
                    assert!(output.status.success(), "{output:?}");
                    assert!(output.stderr.is_empty());
                    assert_eq!(output.stdout, format!("{reply}\n").as_bytes());
                    assert_eq!(
                        fs::read(root.0.join("input")).unwrap(),
                        b"{\"operation\":\"storage.root\"}"
                    );
                    assert_eq!(fs::read_to_string(root.0.join("calls")).unwrap(), "api\n");
                    assert!(!root.0.join("publisher-calls").exists());
                    assert!(!root.0.join("serve.calls").exists());
                })
            })
            .collect::<Vec<_>>();
        for worker in workers {
            worker.join().unwrap();
        }
    }
    #[test]
    fn publication_reuses_exact_files_without_rewriting_or_effects() {
        let directory = directory();
        let before = PROGRAMS.map(|(name, _)| {
            let path = directory.join(name);
            (fs::metadata(&path).unwrap().ino(), fs::read(path).unwrap())
        });
        assert_eq!(publish().unwrap(), directory);
        verify(directory).unwrap();
        for ((name, source), (inode, bytes)) in PROGRAMS.into_iter().zip(before) {
            let path = directory.join(name);
            assert_eq!(fs::metadata(&path).unwrap().ino(), inode);
            assert_eq!(fs::read(path).unwrap(), bytes);
            assert_eq!(bytes, source.as_bytes());
        }
        assert_eq!(fs::read_dir(directory).unwrap().count(), PROGRAMS.len());
    }
    #[test]
    fn publication_refuses_changed_bytes_modes_and_unexpected_entries() {
        let root = root();
        for (name, source) in PROGRAMS {
            tmt_test_support::write_executable(&root.0.join(name), source.as_bytes(), 0o700)
                .unwrap();
        }
        verify(&root.0).unwrap();
        let core = root.0.join("core");
        fs::write(&core, b"different bytes").unwrap();
        assert!(verify(&root.0).is_err());
        assert_eq!(fs::read(&core).unwrap(), b"different bytes");
        fs::write(&core, PROGRAMS[0].1).unwrap();
        fs::set_permissions(&core, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(verify(&root.0).is_err());
        fs::set_permissions(&core, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(root.0.join("unknown"), b"retained").unwrap();
        assert!(verify(&root.0).is_err());
        assert_eq!(fs::read(root.0.join("unknown")).unwrap(), b"retained");
    }
}

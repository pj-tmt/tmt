use super::*;
use std::{
    fs,
    os::unix::fs::symlink,
    sync::atomic::{AtomicUsize, Ordering},
};
const ID: &str = "10000000-0000-4000-8000-000000000001";
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "tmt-export-files-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(fs::canonicalize(path).unwrap())
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn bundle() -> Bundle {
    let html = b"<p>exact\r\n\0</p>".to_vec();
    let manifest = b"{\"plaintext\":true}".to_vec();
    let files = vec![
        FileInfo::new("page.html", &html),
        FileInfo::new("manifest.json", &manifest),
    ];
    Bundle {
        html,
        manifest,
        files,
    }
}
#[test]
fn publication_is_create_only_private_and_cleans_only_its_staging() {
    let dir = Directory::new();
    let foreign = dir.0.join(".tmt-colab-export-foreign");
    fs::create_dir(&foreign).unwrap();
    fs::write(foreign.join("keep"), b"foreign").unwrap();
    let bundle = bundle();
    let published = bundle.publish_as(&dir.0, ID, |_, _| Ok(())).unwrap();
    assert_eq!(
        fs::metadata(&published.directory).unwrap().mode() & 0o777,
        0o700
    );
    for info in &published.files {
        let path = published.directory.join(info.name);
        let m = fs::metadata(&path).unwrap();
        assert_eq!(m.mode() & 0o777, 0o600);
        assert_eq!(m.nlink(), 1, "staging link leaked");
        assert_eq!(FileInfo::new(info.name, &fs::read(path).unwrap()), *info);
    }
    assert!(!dir.0.join(format!(".tmt-colab-export-{ID}")).exists());
    assert!(bundle.publish_as(&dir.0, ID, |_, _| Ok(())).is_err());
    assert_eq!(
        fs::read(published.directory.join("page.html")).unwrap(),
        bundle.html
    );
    assert_eq!(fs::read(foreign.join("keep")).unwrap(), b"foreign");
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 2);
}
#[test]
fn symlink_destinations_and_output_collisions_never_follow_or_replace() {
    let dir = Directory::new();
    let alias = dir.0.join("alias");
    symlink(&dir.0, &alias).unwrap();
    assert!(bundle().publish(&alias).is_err());
    assert!(bundle().publish(&alias.join("subdirectory")).is_err());
    assert!(bundle().publish(&dir.0.join("missing")).is_err());
    assert!(bundle().publish(&dir.0.join("..")).is_err());
    symlink(&dir.0, dir.0.join(ID)).unwrap();
    assert!(bundle().publish_as(&dir.0, ID, |_, _| Ok(())).is_err());
    assert!(
        fs::symlink_metadata(dir.0.join(ID))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(!dir.0.join("page.html").exists());
    assert!(!dir.0.join(format!(".tmt-colab-export-{ID}")).exists());
}
#[test]
fn manifest_collision_reports_partial_output_and_preserves_foreign_bytes() {
    let dir = Directory::new();
    let error = bundle()
        .publish_as(&dir.0, ID, |_, phase| {
            if phase == 1 {
                assert!(dir.0.join(ID).join("page.html").exists());
                assert!(!dir.0.join(ID).join("manifest.json").exists());
                fs::write(dir.0.join(ID).join("manifest.json"), b"foreign")?;
            }
            Ok(())
        })
        .err()
        .unwrap();
    let fault = error.downcast_ref::<Fault>().unwrap();
    assert_eq!(fault.partial_directory(), Some(dir.0.join(ID).as_path()));
    assert!(error.to_string().contains("partial output remains"));
    assert_eq!(
        fs::read(dir.0.join(ID).join("manifest.json")).unwrap(),
        b"foreign"
    );
    assert_eq!(
        fs::read(dir.0.join(ID).join("page.html")).unwrap(),
        bundle().html
    );
    assert!(!dir.0.join(format!(".tmt-colab-export-{ID}")).exists());
}
#[test]
fn changed_staging_identity_is_never_published_or_recursively_cleaned() {
    let dir = Directory::new();
    let stage = dir.0.join(format!(".tmt-colab-export-{ID}"));
    let displaced = dir.0.join("displaced");
    let error = bundle()
        .publish_as(&dir.0, ID, |_, _| {
            fs::rename(&stage, &displaced)?;
            fs::create_dir(&stage)?;
            fs::write(stage.join("keep"), b"foreign")?;
            Ok(())
        })
        .err()
        .unwrap();
    assert!(error.to_string().contains("entry changed"));
    assert_eq!(fs::read(stage.join("keep")).unwrap(), b"foreign");
    assert_eq!(
        fs::read(displaced.join("page.html")).unwrap(),
        bundle().html
    );
    assert!(!dir.0.join(ID).exists());
}
#[test]
fn replaced_staged_file_fails_before_output_and_is_preserved() {
    let dir = Directory::new();
    let stage = dir.0.join(format!(".tmt-colab-export-{ID}"));
    let error = bundle()
        .publish_as(&dir.0, ID, |_, _| {
            fs::remove_file(stage.join("page.html"))?;
            symlink(dir.0.join("foreign"), stage.join("page.html"))?;
            fs::write(dir.0.join("foreign"), b"keep")?;
            Ok(())
        })
        .err()
        .unwrap();
    assert!(error.to_string().contains("staging cleanup"));
    assert_eq!(fs::read(dir.0.join("foreign")).unwrap(), b"keep");
    assert!(
        fs::symlink_metadata(stage.join("page.html"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

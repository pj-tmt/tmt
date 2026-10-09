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
    let contents = vec![
        b"<p>exact\r\n\0</p>".to_vec(),
        b"{\"threads\":[]}".to_vec(),
        b"# Conversations\n".to_vec(),
        b"{\"plaintext\":true}".to_vec(),
    ];
    let names = [
        "page.html",
        "conversations.json",
        "conversations.md",
        "manifest.json",
    ];
    let files = names
        .iter()
        .zip(&contents)
        .map(|(name, bytes)| FileInfo::new(name, bytes))
        .collect();
    Bundle { contents, files }
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
        bundle.contents[0]
    );
    assert_eq!(fs::read(foreign.join("keep")).unwrap(), b"foreign");
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 2);
}
#[test]
fn parent_aliases_resolve_once_but_generated_entries_never_follow_or_replace() {
    let dir = Directory::new();
    let alias = dir.0.join("alias");
    symlink(&dir.0, &alias).unwrap();
    fs::create_dir(dir.0.join("subdirectory")).unwrap();
    let aliased = bundle().publish(&alias.join("subdirectory")).unwrap();
    assert_eq!(
        aliased.directory.parent(),
        Some(dir.0.join("subdirectory").as_path())
    );
    assert_eq!(
        fs::read(aliased.directory.join("page.html")).unwrap(),
        bundle().contents[0]
    );
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
            // The manifest is linked last, after every other file.
            if phase == 3 {
                for name in ["page.html", "conversations.json", "conversations.md"] {
                    assert!(dir.0.join(ID).join(name).exists(), "{name}");
                }
                assert!(!dir.0.join(ID).join("manifest.json").exists());
                fs::write(dir.0.join(ID).join("manifest.json"), b"foreign")?;
            }
            Ok(())
        })
        .err()
        .unwrap();
    let fault = error.downcast_ref::<Fault>().unwrap();
    assert_eq!(fault.partial_directory(), Some(dir.0.join(ID).as_path()));
    assert!(error.to_string().contains("partial output was created"));
    assert_eq!(
        fs::read(dir.0.join(ID).join("manifest.json")).unwrap(),
        b"foreign"
    );
    assert_eq!(
        fs::read(dir.0.join(ID).join("page.html")).unwrap(),
        bundle().contents[0]
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
        bundle().contents[0]
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

#[test]
fn renamed_destination_reports_failure_without_publishing_into_its_replacement() {
    let dir = Directory::new();
    let destination = dir.0.join("destination");
    let moved = dir.0.join("moved");
    fs::create_dir(&destination).unwrap();
    let error = bundle()
        .publish_as(&destination, ID, |_, phase| {
            if phase == 1 {
                fs::rename(&destination, &moved)?;
                fs::create_dir(&destination)?;
                fs::write(destination.join("foreign"), b"keep")?;
            }
            Ok(())
        })
        .err()
        .unwrap();
    assert!(error.to_string().contains("destination changed"));
    assert_eq!(
        error.downcast_ref::<Fault>().unwrap().partial_directory(),
        Some(destination.join(ID).as_path())
    );
    assert_eq!(fs::read(destination.join("foreign")).unwrap(), b"keep");
    assert!(!destination.join(ID).exists());
    assert_eq!(
        fs::read(moved.join(ID).join("page.html")).unwrap(),
        bundle().contents[0]
    );
    assert!(!moved.join(ID).join("manifest.json").exists());
    assert!(!moved.join(format!(".tmt-colab-export-{ID}")).exists());
}

#[test]
fn shared_fixture_matches_the_native_bundle_bytes_and_field_order() {
    use std::collections::BTreeMap;
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../../../contracts/vectors/export-v1.json")).unwrap();
    let input = &fixture["input"];
    let own: BTreeMap<String, serde_json::Value> = input["own"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(writer, roots)| (writer.clone(), roots.clone()))
        .collect();
    let keys: BTreeMap<String, [u8; 32]> = input["signingKeys"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(writer, key)| {
            let hex = key.as_str().unwrap();
            let mut bytes = [0u8; 32];
            for (i, byte) in bytes.iter_mut().enumerate() {
                *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap();
            }
            (writer.clone(), bytes)
        })
        .collect();
    let head = |input: &serde_json::Value| conversations::Head {
        revision: input["membershipHead"]["revision"].as_str().unwrap().into(),
        statement_hash: input["membershipHead"]["statementHash"]
            .as_str()
            .unwrap()
            .into(),
    };
    let conversations = conversations::Conversations::project(
        conversations::Capture {
            space_id: input["spaceId"].as_str().unwrap(),
            page_id: input["pageId"].as_str().unwrap(),
            title: input["title"].as_str().unwrap(),
            epoch: input["epoch"].as_str().unwrap(),
            head: head(input),
        },
        &own,
        &keys,
        &keys.keys().cloned().collect(),
    );
    let json = conversations.json();
    let markdown = conversations.markdown().into_bytes();
    assert_eq!(
        json,
        fixture["conversationsJson"].as_str().unwrap().as_bytes()
    );
    assert_eq!(
        markdown,
        fixture["conversationsMarkdown"]
            .as_str()
            .unwrap()
            .as_bytes()
    );
    let files = [
        FileInfo::new("page.html", input["source"].as_str().unwrap().as_bytes()),
        FileInfo::new("conversations.json", &json),
        FileInfo::new("conversations.md", &markdown),
    ];
    // Exercise the production native serializer, not a Value's map ordering.
    let recipient = input.get("creationRecipient").map(|value| {
        serde_json::from_value::<crate::decoder::CreationRecipient>(value.clone()).unwrap()
    });
    let manifest = serde_json::to_vec(&Manifest {
        format: "tmt-colab-page-export",
        version: 1,
        space_id: input["spaceId"].as_str().unwrap(),
        page_id: input["pageId"].as_str().unwrap(),
        title: input["title"].as_str().unwrap(),
        original_author: input["originalAuthor"].as_str(),
        publisher_agent: input["publisherAgent"].as_str(),
        creation_recipient: recipient.as_ref(),
        exported_at_ms: input["exportedAtMs"].as_u64().unwrap(),
        membership_head: MembershipHead {
            revision: input["membershipHead"]["revision"].as_str().unwrap().into(),
            statement_hash: input["membershipHead"]["statementHash"]
                .as_str()
                .unwrap()
                .into(),
        },
        epoch: input["epoch"].as_str().unwrap().into(),
        plaintext: true,
        discussions: Discussions {
            included: true,
            scope: "current-epoch",
            format: conversations::FORMAT,
            version: 1,
        },
        files: &files,
    })
    .unwrap();
    assert_eq!(
        manifest,
        fixture["manifestUtf8"].as_str().unwrap().as_bytes()
    );
    assert_eq!(hex(&crypto::digest(&manifest)), fixture["manifestSha256"]);
}

#[test]
fn status_projection_matches_browser_literal_bytes_and_historical_scope_binding() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../contracts/vectors/discussion-v1.json"
    ))
    .unwrap();
    for row in fixture["statusCases"].as_array().unwrap() {
        let mut own = std::collections::BTreeMap::new();
        let mut keys = std::collections::BTreeMap::new();
        for record in row["threadHistory"]
            .as_array()
            .into_iter()
            .flatten()
            .chain(std::iter::once(&row["thread"]))
            .chain(row["actions"].as_array().unwrap())
            .chain(row["notifications"].as_array().into_iter().flatten())
        {
            let writer = record["senderDevice"].as_str().unwrap();
            keys.insert(writer.into(), [0; 32]);
            let roots = own.entry(writer.into()).or_insert_with(
                || serde_json::json!({"threads":{},"messages":{},"intents":{},"replies":{}}),
            );
            let (root, key) = if record["kind"] == "thread" {
                (
                    "threads",
                    format!(
                        "{}:{}",
                        record["threadId"].as_str().unwrap(),
                        record["revision"].as_str().unwrap()
                    ),
                )
            } else {
                (
                    "messages",
                    format!(
                        "{}:{}",
                        record[if record["kind"] == "thread-status" {
                            "actionId"
                        } else {
                            "operationId"
                        }]
                        .as_str()
                        .unwrap(),
                        record["kind"].as_str().unwrap()
                    ),
                )
            };
            roots[root][key] = record.clone();
        }
        let scope = conversations::Capture {
            space_id: fixture["scope"]["spaceId"].as_str().unwrap(),
            page_id: fixture["scope"]["pageId"].as_str().unwrap(),
            title: "Status vectors",
            epoch: fixture["scope"]["epoch"].as_str().unwrap(),
            head: conversations::Head {
                revision: "1".into(),
                statement_hash: "00".repeat(32),
            },
        };
        let result = conversations::threads(&scope, &own, &keys, &keys.keys().cloned().collect());
        assert_eq!(
            serde_json::to_string(&result).unwrap(),
            row["expected"]["threadsJson"].as_str().unwrap(),
            "{}",
            row["name"]
        );
        let conversations = conversations::Conversations::project(
            scope.clone(),
            &own,
            &keys,
            &keys.keys().cloned().collect(),
        );
        assert_eq!(
            conversations.markdown(),
            row["expected"]["markdown"].as_str().unwrap(),
            "{}",
            row["name"]
        );
        // A historical key proves authorship, not status authority. Removing only
        // owner provenance leaves the existing thread visible at its legacy state.
        let unauthorized = conversations::threads(&scope, &own, &keys, &Default::default());
        assert_eq!(unauthorized.len(), result.len());
        if let Some(thread) = unauthorized.first() {
            assert_eq!(
                thread.resolved,
                row["thread"]["resolved"].as_bool().unwrap()
            );
            assert!(thread.status.is_none());
            assert!(thread.notifications.is_empty());
        }
        // Envelope keys still gate foreign status and thread visibility.
        keys.clear();
        assert!(
            conversations::threads(&scope, &own, &keys, &keys.keys().cloned().collect()).is_empty()
        );
    }
}

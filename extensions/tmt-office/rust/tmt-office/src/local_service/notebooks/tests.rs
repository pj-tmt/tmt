use super::super::{test_fixture::HttpFixture, tests::response_value};
use super::*;
use tmt_adapters::storage::Storage;
use tmt_core::identity::{Lifetime, NotesIdentityId, create_or_resolve};

fn request(method: &str, path: &str) -> Request {
    Request {
        method: method.into(),
        path: path.into(),
        headers: vec![("Authorization".into(), "Bearer browser".into())],
        body: vec![],
    }
}

#[test]
fn notebook_route_authentication_path_and_method_reject_before_storage() {
    let fixture = HttpFixture::new();
    let target = format!("{PREFIX}11111111-1111-4111-8111-111111111111");
    let mut unauthenticated = request("GET", &target);
    unauthenticated.headers.clear();
    assert!(fixture.call(unauthenticated).starts_with("HTTP/1.1 401"));
    for method in ["POST", "PUT", "DELETE"] {
        assert!(
            fixture
                .call(request(method, &target))
                .starts_with("HTTP/1.1 405")
        );
    }
    for suffix in [
        "../notes.md",
        "%2e%2e",
        "alice",
        "00000000-0000-0000-0000-000000000000",
        "11111111-1111-4111-8111-111111111111/extra",
    ] {
        assert!(
            fixture
                .call(request("GET", &format!("{PREFIX}{suffix}")))
                .starts_with("HTTP/1.1 404")
        );
    }
    assert!(!fixture.paths.database.exists());
    assert!(!fixture.paths.global_dir.join("notes").exists());
}

#[test]
fn notebook_route_reads_existing_text_without_rendering_or_creating_content() {
    let fixture = HttpFixture::new();
    let mut storage = Storage::open(&fixture.paths.database).unwrap();
    let saved = create_or_resolve(&mut storage, "Alice", Lifetime::Saved)
        .unwrap()
        .identity;
    let temporary = create_or_resolve(&mut storage, "Contractor", Lifetime::Temporary)
        .unwrap()
        .identity;
    storage.close().unwrap();
    let target = format!("{PREFIX}{}", saved.id);
    let missing = fixture.call(request("GET", &target));
    assert!(missing.starts_with("HTTP/1.1 404"));
    assert_eq!(
        response_value(&missing),
        json!({ "error": "NOTEBOOK_NOT_FOUND" })
    );
    assert!(!fixture.paths.global_dir.join("notes").exists());
    let forbidden = fixture.call(request("GET", &format!("{PREFIX}{}", temporary.id)));
    assert!(forbidden.starts_with("HTTP/1.1 403"));
    assert_eq!(
        response_value(&forbidden),
        json!({ "error": "NOTEBOOK_SAVED_IDENTITY_REQUIRED" })
    );
    let file = notes::initialize(&fixture.paths, &NotesIdentityId::try_from(&saved).unwrap())
        .unwrap()
        .path;
    let content = "# exact\r\n<script>untrusted</script>\0\u{feff}";
    std::fs::write(&file, content).unwrap();
    let result = fixture.call(request("GET", &target));
    assert!(result.starts_with("HTTP/1.1 200"));
    assert_eq!(
        response_value(&result),
        json!({ "identityId": saved.id, "name": "Alice", "content": content })
    );
    assert_eq!(std::fs::read(file).unwrap(), content.as_bytes());
}

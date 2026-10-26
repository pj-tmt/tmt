//! Read-only publication eligibility controls, using the actual composer output.
use super::*;
use crate::{
    deploy_run::RunState,
    hosting::{HostingBundle, HostingInput, HostingManifest},
};

fn record() -> (DeployRecord, Value) {
    let manifest = HostingManifest::parse(&json!({"version":1,"files":[{
        "path":"/index.html","sha256":sha256_hex(b"shell"),"length":5,"contentType":"text/html"
    }]}))
    .unwrap();
    let bundle = HostingBundle::parse(&manifest, &serde_json::to_vec(&json!({"version":1,"manifestDigest":manifest.digest(),"files":[{"path":"/index.html","bytesBase64":STANDARD.encode(b"shell")}]})).unwrap()).unwrap();
    let content = crate::hosting::compose(&[HostingInput {
        extension: "colab",
        declaration: b"{}",
        bundle: &bundle,
    }])
    .unwrap()
    .unwrap()
    .view();
    let config = json!({"projectId":"demo-remote-1","appId":"1:123:web:test","apiKey":"public-test","authDomain":"demo-remote-1.firebaseapp.com"});
    let mut envelope: Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/deploy_run/view-fresh.json"
    ))
    .unwrap();
    envelope["hosting"] = json!({"site":"dedicated-site","publicUrl":"https://dedicated-site.web.app","createSite":false,"configureSite":false,"siteAppId":"1:123:web:test","webApp":"1:123:web:test","createWebApp":false,"content":content,"replaces":"none","replacedFingerprint":null});
    let record = serde_json::from_value(json!({
        "deploymentId":envelope["deploymentId"],
        "binding":{"planDigest":"test-digest","project":"demo-remote-1","completedAtMs":1000},
        "run":{"planDigest":"test-digest","account":"owner@example.test","authorizedAtMs":1,"state":"complete","rulesAttempted":true,"steps":[],"hosting":{
            "envelope":envelope,"appId":"1:123:web:test","version":"sites/dedicated-site/versions/test","publication":{
                "project":"demo-remote-1","region":"asia-east1","deploymentId":envelope["deploymentId"],"planDigest":"test-digest","appId":"1:123:web:test","siteAppId":"1:123:web:test","publicConfig":config,"entryUrl":"https://dedicated-site.web.app","version":"sites/dedicated-site/versions/test","release":"sites/dedicated-site/releases/test","rulesDigest":envelope["rules"]["digest"],"contentDigest":content["digest"]
            }
        }}
    })).unwrap();
    (record, config)
}

#[test]
fn link_requires_the_verified_selected_site_and_exact_instruction_capability() {
    let (record, config) = record();
    assert_eq!(
        link(&record, &config, "private-code").unwrap(),
        "https://dedicated-site.web.app/join#private-code"
    );
    let mut wrong = config.clone();
    wrong["projectId"] = json!("other-project");
    assert!(link(&record, &wrong, "private-code").is_none());
    let mut changed = record.clone();
    changed.run.as_mut().unwrap().state = RunState::Partial;
    assert!(link(&changed, &config, "private-code").is_none());
    let mut changed = record.clone();
    changed.binding.as_mut().unwrap().plan_digest = "stale".into();
    assert!(link(&changed, &config, "private-code").is_none());
    for field in ["rawDigest", "rawLength", "contentType"] {
        let mut changed = record.clone();
        let files = changed
            .run
            .as_mut()
            .unwrap()
            .hosting
            .as_mut()
            .unwrap()
            .envelope["hosting"]["content"]["files"]
            .as_array_mut()
            .unwrap();
        let page = files
            .iter_mut()
            .find(|file| file["path"] == "/join.html")
            .unwrap();
        page[field] = Value::Null;
        assert!(link(&changed, &config, "private-code").is_none(), "{field}");
    }
    for field in ["files", "config"] {
        let mut changed = record.clone();
        changed
            .run
            .as_mut()
            .unwrap()
            .hosting
            .as_mut()
            .unwrap()
            .envelope["hosting"]["content"][field] = Value::Null;
        assert!(link(&changed, &config, "private-code").is_none(), "{field}");
    }
    for field in ["headers", "rewrites"] {
        let mut changed = record.clone();
        changed
            .run
            .as_mut()
            .unwrap()
            .hosting
            .as_mut()
            .unwrap()
            .envelope["hosting"]["content"]["config"][field] = json!([]);
        assert!(link(&changed, &config, "private-code").is_none(), "{field}");
    }
}

#[test]
fn standalone_document_erases_before_resources_and_uses_its_own_route_policy() {
    let document = document();
    assert!(document.find("history.replaceState").unwrap() < document.find("import(").unwrap());
    assert!(document.find("history.replaceState").unwrap() < document.find("style.href").unwrap());
    assert!(!document.contains("__TMT_JOIN_BOOTSTRAP__"));
    assert!(!document.contains("/sdk/shell.js"));
    assert!(policy().contains("connect-src 'none'"));
    assert!(crate::hosting::response_headers(&assets()[0].0)["Cache-Control"] == "no-store");
}

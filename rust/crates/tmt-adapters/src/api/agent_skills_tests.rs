use super::*;
use crate::runtime::skills::{InvocationSyntax, SkillRoot, SkillSource};
use crate::test_support::TestDirectory;
use serde_json::{Value, json};

const ID: &str = "0199a213-81c0-7800-8aa1-bbab2a035a53";

fn value(bytes: Vec<u8>) -> Value {
    serde_json::from_slice(&bytes).unwrap()
}

#[test]
fn input_is_one_identity_and_an_optional_stamp() {
    let ok = |input: Value| decode(input.to_string().as_bytes());
    assert!(matches!(
        ok(json!({"identityId": ID})),
        Ok(Request::AgentSkills { if_stamp: None, .. })
    ));
    assert!(matches!(
        ok(json!({"identityId": ID, "ifStamp": "a".repeat(64)})),
        Ok(Request::AgentSkills {
            if_stamp: Some(_),
            ..
        })
    ));
    for input in [
        json!({}),
        json!({"identityId": "Ada"}),
        json!({"identityId": ID, "ifStamp": "short"}),
        json!({"identityId": ID, "ifStamp": "g".repeat(64)}),
        json!({"identityId": ID, "extra": true}),
    ] {
        assert!(ok(input.clone()).is_err(), "{input}");
    }
}

#[test]
fn a_matching_stamp_answers_unchanged_and_a_stale_one_lists_again() {
    let root = TestDirectory::new();
    assert!(root.path.starts_with(std::env::temp_dir()));
    let user = root.path.join("skills");
    std::fs::create_dir_all(user.join("review")).unwrap();
    std::fs::write(
        user.join("review/SKILL.md"),
        "---\nname: review\ndescription: Review a diff\n---\n",
    )
    .unwrap();
    let roots = SkillRoots {
        syntax: InvocationSyntax::Prefix { sigil: '$' },
        roots: vec![SkillRoot {
            path: user.clone(),
            source: SkillSource::User,
        }],
    };
    let assets = root.path.join("skill-assets");
    let full = value(respond(&roots, &assets, None, 7).unwrap());
    let stamp = full["rootsStamp"].as_str().unwrap().to_owned();
    assert_eq!(
        full,
        json!({
            "syntax": {"kind": "prefix", "sigil": "$"},
            "skills": [{"name": "review", "description": "Review a diff",
                "source": {"group": "user"}}],
            "truncated": false,
            "rootsStamp": stamp,
            "observedAtMs": 7,
        })
    );
    assert_eq!(
        value(respond(&roots, &assets, Some(&stamp), 8).unwrap()),
        json!({"unchanged": true, "rootsStamp": stamp, "observedAtMs": 8})
    );
    std::fs::remove_dir_all(user.join("review")).unwrap();
    let fresh = value(respond(&roots, &assets, Some(&stamp), 9).unwrap());
    assert_eq!(fresh["skills"], json!([]));
    assert_ne!(fresh["rootsStamp"], json!(stamp));
}

#[test]
fn an_unsupported_driver_reports_unsupported_with_no_skills() {
    let root = TestDirectory::new();
    let report = value(respond(&SkillRoots::unsupported(), &root.path, None, 1).unwrap());
    assert_eq!(report["syntax"], json!({"kind": "unsupported"}));
    assert_eq!(report["skills"], json!([]));
    assert_eq!(report["truncated"], false);
}

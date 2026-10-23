//! Causal fake-host delivery over public Core JSON, with no host server or store.
use super::*;
use crate::core::Core;
use std::path::PathBuf;
struct Fixture {
    root: PathBuf,
    core: Core,
}
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("ops-new-member-{}", crate::id::new_v4().unwrap()));
        std::fs::create_dir(&root).unwrap();
        let driver = root.join("driver.py");
        std::fs::write(&driver,r#"import json, pathlib, sys
r=pathlib.Path(__file__).parent
a=sys.argv[1:]
with (r/'calls').open('a') as f:f.write(json.dumps(a)+'\n')
host=str(r/'host')
if a[:2]==['host','templates']:
 print(json.dumps({'version':1,'hosts':[{'host':'tmux','state':'builtin','launch':{'argv':[host,'{session}','{command}'],'placeholders':['session','command']}}]}))
elif a[:2]==['config','show']:print(json.dumps({'paths':{'global':str(r/'config.json')},'resolved':{}}))
elif a[:1]==['api']:json.load(sys.stdin);print(json.dumps({'dataRoot':str(r/'data')}))
elif a[:1]==['ls']:
 rows=[]
 if (r/'existing').exists():rows.append({'id':'old','name':'new-worker','location':{'host':'tmux'}})
 if (r/'launched').exists():rows.append({'id':'33333333-3333-4333-8333-333333333333','name':'new-worker','location':{'host':'tmux','session':'a space'}})
 print(json.dumps({'identities':rows}))
elif a[:2]==['identity','show']:
 if (r/'existing').exists() or (r/'existing-alias').exists():print(json.dumps({'identity':{'id':'old','name':'new-worker'}}))
 else:print(json.dumps({'error':{'code':'NAME_NOT_FOUND','message':'not found'}}));sys.exit(3)
elif a[:2]==['room','show']:print(json.dumps({'room':{'id':'changed'}}))
else:print(json.dumps({'error':{'code':'UNEXPECTED','message':str(a)}}));sys.exit(1)
"#).unwrap();
        let core = root.join("core");
        crate::test_support::write_ready_executable(
            &core,
            &format!(
                "#!/bin/sh\nexec /usr/bin/python3 {} \"$@\"\n",
                crate::launch::shell_word(driver.to_str().unwrap())
            ),
        );
        let host_driver = root.join("host.py");
        std::fs::write(
            &host_driver,
            r#"import json, pathlib, sys
r=pathlib.Path(__file__).parent
with (r/'launches').open('a') as f:f.write(json.dumps(sys.argv[1:])+'\n')
(r/'launched').write_text('new bound identity')
"#,
        )
        .unwrap();
        crate::test_support::write_ready_executable(
            &root.join("host"),
            &format!(
                "#!/bin/sh\nexec /usr/bin/python3 {} \"$@\"\n",
                crate::launch::shell_word(host_driver.to_str().unwrap())
            ),
        );
        Self {
            root,
            core: Core::at(core),
        }
    }
    fn review(&self) -> model::Review {
        let loaded = match complete(
            &self.core,
            Task {
                owner: 1,
                work: Work::Load,
                config: None,
            },
        )
        .result
        .unwrap()
        {
            Outcome::Loaded(l) => l,
            _ => panic!(),
        };
        let mut draft = model::Draft::from_app(&board());
        draft.fields =
            ["new-worker", "codex", "a'b $c; d", "", "tmux", "a space"].map(Editor::from);
        draft.review(&loaded).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}
#[test]
fn reviewed_one_word_launch_observes_new_uuid_and_never_reuses_an_existing_name() {
    let f = Fixture::new();
    let review = f.review();
    let result = complete(
        &f.core,
        Task {
            owner: 1,
            work: Work::Launch(review.clone()),
            config: None,
        },
    )
    .result
    .unwrap();
    assert!(
        matches!(result,Outcome::Created{ref id,room:None,..}if id=="33333333-3333-4333-8333-333333333333")
    );
    let launches = std::fs::read_to_string(f.root.join("launches")).unwrap();
    assert_eq!(launches.lines().count(), 1);
    let args: serde_json::Value = serde_json::from_str(launches.trim()).unwrap();
    assert_eq!(
        args,
        json!(["a space", crate::launch::line(&review.command)])
    );
    std::fs::write(f.root.join("existing"), "present").unwrap();
    assert!(
        matches!(complete(&f.core,Task{owner:1,work:Work::Launch(review),config:None}).result,Err(error) if error.message.contains("already exists"))
    );
    assert_eq!(
        std::fs::read_to_string(f.root.join("launches")).unwrap(),
        launches
    );
}
#[test]
fn failed_join_still_reports_the_created_uuid_in_pool_with_repair() {
    let f = Fixture::new();
    let mut review = f.review();
    review.room = Some(crate::membership::workspace::RoomTarget {
        id: "11111111-1111-4111-8111-111111111111".into(),
        name: "platform".into(),
        revision: 7,
    });
    let result = complete(
        &f.core,
        Task {
            owner: 1,
            work: Work::Launch(review),
            config: None,
        },
    )
    .result
    .unwrap();
    match result {
        Outcome::Created { id, room, message } => {
            assert!(!id.is_empty());
            assert!(room.is_none());
            assert!(message.contains("Created new-worker in Common pool"));
            assert!(message.contains("ops sq add --squad platform -- 33333333"));
        }
        _ => panic!(),
    }
}

#[test]
fn core_lookup_refuses_a_canonical_alias_missing_from_the_display_name_match() {
    let f = Fixture::new();
    let mut review = f.review();
    review.name = "NEW-WORKER".into();
    std::fs::write(f.root.join("existing-alias"), "present").unwrap();
    assert!(
        matches!(complete(&f.core,Task{owner:1,work:Work::Launch(review),config:None}).result,Err(error)if error.message.contains("already exists"))
    );
    assert!(!f.root.join("launches").exists());
}

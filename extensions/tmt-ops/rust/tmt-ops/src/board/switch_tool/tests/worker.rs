//! Public-wire fixture executes a literal host argv; no tmux or user state.
use super::*;
use crate::core::Core;
use std::{path::PathBuf, time::Duration};
struct Fixture {
    root: PathBuf,
    core: Core,
}
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("ops-switch-{}", crate::id::new_v4().unwrap()));
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("row.json"), serde_json::to_vec(&row()).unwrap()).unwrap();
        let script = root.join("core.py");
        std::fs::write(&script,r#"import json,pathlib,sys
r=pathlib.Path(__file__).parent
a=sys.argv[1:]
with (r/'calls').open('a') as f:f.write(json.dumps(a)+'\n')
if a[:2]==['host','templates']:
 host=str(r/'host') if not (r/'changed-template').exists() else 'different-host'
 print(json.dumps({'version':1,'hosts':[{'host':'tmux','state':'builtin','launch':{'argv':[host,'{session}','{command}'],'placeholders':['session','command']}}]}))
elif a[:2]==['config','show']:print(json.dumps({'paths':{'global':str(r/'config.json')},'resolved':{}}))
elif a[:1]==['api']:json.load(sys.stdin);print(json.dumps({'dataRoot':str(r/'data')}))
elif a[:1]==['ls']:print(json.dumps({'identities':[json.loads((r/'row.json').read_text())]}))
else:raise Exception(a)
"#).unwrap();
        let core = root.join("core");
        crate::test_support::write_ready_executable(
            &core,
            &format!(
                "#!/bin/sh\nexec /usr/bin/python3 {} \"$@\"\n",
                crate::launch::shell_word(script.to_str().unwrap())
            ),
        );
        let host = root.join("host.py");
        std::fs::write(
            &host,
            r#"import json,pathlib,shlex,sys
r=pathlib.Path(__file__).parent
with (r/'launches').open('a') as f:f.write(json.dumps(sys.argv[1:])+'\n')
if (r/'fail-host').exists():print(json.dumps({'error':{'code':'REFUSED','message':'Exact Core refusal text.'}}));sys.exit(1)
a=shlex.split(sys.argv[2]); assert a[-2]=='--'
row=json.loads((r/'row.json').read_text())
row['runningDriver']=next(v.split('=',1)[1] for v in a if v.startswith('--driver='))
if (r/'other-id').exists():row['id']='33333333-3333-4333-8333-333333333333'
if (r/'remembered-only').exists():row['resume']['driver']=row.pop('runningDriver')
row['resume']['observedModel']={'model':'observed-model'}
(r/'row.json').write_text(json.dumps(row))
"#,
        )
        .unwrap();
        crate::test_support::write_ready_executable(
            &root.join("host"),
            &format!(
                "#!/bin/sh\nexec /usr/bin/python3 {} \"$@\"\n",
                crate::launch::shell_word(host.to_str().unwrap())
            ),
        );
        Self {
            root,
            core: Core::at(core),
        }
    }
    fn review(&self) -> Review {
        let loaded = super::super::super::host_launch::read(&self.core).unwrap();
        Review::new(
            &Source::from_app(&app()).unwrap(),
            "claude",
            "a'b $c; d",
            &loaded,
        )
        .unwrap()
    }
    fn launches(&self) -> String {
        std::fs::read_to_string(self.root.join("launches")).unwrap_or_default()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}
#[test]
fn one_literal_launch_observes_same_uuid_and_reports_observed_not_requested_model() {
    let f = Fixture::new();
    let review = f.review();
    let outcome = load::launch(&f.core, &review, Duration::from_secs(2)).unwrap();
    assert!(
        matches!(outcome,Outcome::Switched(ref text) if text.contains("Observed model: observed-model")&&!text.contains("Observed model: a'b"))
    );
    let calls = f.launches();
    assert_eq!(calls.lines().count(), 1);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(calls.trim()).unwrap(),
        json!(["work", crate::launch::line(&review.command)])
    );
}
#[test]
fn stale_target_or_template_refuses_before_any_host_side_effect() {
    for change in ["source", "template"] {
        let f = Fixture::new();
        let review = f.review();
        if change == "template" {
            std::fs::write(f.root.join("changed-template"), "").unwrap();
        } else {
            let mut row = row();
            row["resume"]["session"] = json!("changed");
            std::fs::write(f.root.join("row.json"), serde_json::to_vec(&row).unwrap()).unwrap();
        }
        assert!(load::launch(&f.core, &review, Duration::from_secs(1)).is_err());
        assert!(f.launches().is_empty());
    }
}
#[test]
fn different_uuid_or_remembered_only_never_clears_unknown_and_never_replays() {
    for change in ["other-id", "remembered-only"] {
        let f = Fixture::new();
        let review = f.review();
        std::fs::write(f.root.join(change), "").unwrap();
        assert!(
            matches!(load::launch(&f.core,&review,Duration::from_millis(300)).unwrap(),Outcome::Unknown(ref text) if text.contains("before retrying"))
        );
        assert_eq!(f.launches().lines().count(), 1);
    }
}

#[test]
fn failed_host_reports_uncertainty_with_exact_error_text_and_no_retry() {
    let f = Fixture::new();
    let review = f.review();
    std::fs::write(f.root.join("fail-host"), "").unwrap();
    assert!(
        matches!(load::launch(&f.core,&review,Duration::from_millis(300)).unwrap(),Outcome::Unknown(ref text) if text.contains("Exact Core refusal text."))
    );
    assert_eq!(f.launches().lines().count(), 1);
}

use super::*;
use std::{
    fs,
    sync::atomic::{AtomicUsize, Ordering},
};

const USER: &str = "11111111-1111-4111-8111-111111111111";
const LEAD: &str = "22222222-2222-4222-8222-222222222222";
const WORKER: &str = "33333333-3333-4333-8333-333333333333";
const ROOM: &str = "44444444-4444-4444-8444-444444444444";
struct Fixture {
    directory: PathBuf,
    core: Core,
    config: Config,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let directory = std::env::temp_dir().join(format!(
            "squad-cron-service-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        let model = json!({"room":ROOM,"caller":null,"retired":[],"rosterWithout":[],"hooks":[],"calls":[],"notices":[],"noticeFailure":false,"hookFailure":false,
            "members":[{"id":USER,"name":"Ben","lifetime":"saved","metadata":{}},
            {"id":LEAD,"name":"Sol","lifetime":"saved","metadata":{"squad.product.lead.marker":"true"}},
            {"id":WORKER,"name":"worker","lifetime":"saved","metadata":{}}]});
        fs::write(directory.join("model.json"), model.to_string()).unwrap();
        fs::write(
            directory.join("squad.toml"),
            format!("me='Ben'\nme_id='{USER}'\n"),
        )
        .unwrap();
        fs::write(directory.join("core.py"), r#"
import json,sys,pathlib,fcntl
p=pathlib.Path(__file__).parent; m=json.loads((p/'model.json').read_text()); a=sys.argv[1:]
def fail(code):
 print(json.dumps({'error':{'code':code,'message':code}})); sys.exit(1)
if a[0]=='api':
 q=json.load(sys.stdin); op=q['operation']; i=q['input']; locked=False
 if (p/'squad/cron/jobs.lock').exists():
  with open(p/'squad/cron/jobs.lock','r+') as lock:
   try: fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
   except BlockingIOError: locked=True
 m['calls'].append({'request':q,'locked':locked})
 if op=='storage.root': out={'dataRoot':str(p)}
 elif op=='rooms.roster': out={'members':[x for x in m['members'] if x['id'] not in m['retired'] and x['id'] not in m['rosterWithout']]}
 elif op=='references.resolve':
  out={'identities':[dict(next((x for x in m['members'] if x['id']==id),{'id':id}),found=any(x['id']==id for x in m['members']),retired=id in m['retired']) for id in i.get('identityIds',[])]}
 elif op=='identityHooks.register':
  if m['hookFailure']: fail('HOOK_FAILURE')
  if i not in m['hooks']: m['hooks'].append(i)
  out={'state':'registered'}
 elif op=='identityHooks.pending':
  pending=[h for h in m['hooks'] if h['identityId'] in m['retired']]
  out={'hooks':pending[:i['limit']],'pending':len(pending)}
 elif op=='identityHooks.attempt': out={'recorded':True}
 elif op=='identityHooks.ack':
  m['hooks']=[h for h in m['hooks'] if h!=i]; out={'acknowledged':True}
 elif op=='dispatch.create':
  assert not locked, 'dispatch ran under jobs lock'
  if m['noticeFailure']:
   (p/'model.json').write_text(json.dumps(m)); fail('DISPATCH_FAILURE')
  m['notices'].append(q); out={'items':[{'recipientId':id,'acceptance':'queued'} for id in i['recipientIds']]}
 else: fail('API_INPUT_INVALID')
 (p/'model.json').write_text(json.dumps(m)); print(json.dumps(out))
elif a[0]=='room' and a[1]=='show': print(json.dumps({'room':{'id':m['room']}}))
elif a[0]=='room': print(json.dumps({'rooms':[{'id':m['room'],'name':'squad-product'}]}))
elif a[0]=='identity':
 x=next((x for x in m['members'] if a[2] in [x['id'],x['name']] and x['id'] not in m['retired']),None)
 if x is None: fail('NAME_NOT_FOUND')
 print(json.dumps({'identity':x}))
elif a[0]=='whoami':
 if m['caller']=='ambiguous': fail('CALLER_IDENTITY_AMBIGUOUS')
 print(json.dumps({'bound':False} if m['caller'] is None else dict(next(x for x in m['members'] if x['id']==m['caller']),bound=True)))
else: fail('UNKNOWN_COMMAND')
"#).unwrap();
        let executable = directory.join("tmt");
        crate::test_support::write_ready_executable(
            &executable,
            &format!(
                "#!/bin/sh\nexec /usr/bin/python3 '{}' \"$@\"\n",
                directory.join("core.py").display()
            ),
        );
        let core = Core::at(executable);
        let config = Config::read(directory.join("squad.toml")).unwrap();
        Self {
            directory,
            core,
            config,
        }
    }
    fn model(&self) -> Value {
        serde_json::from_slice(&fs::read(self.directory.join("model.json")).unwrap()).unwrap()
    }
    fn change_model(&self, edit: impl FnOnce(&mut Value)) {
        let mut m = self.model();
        edit(&mut m);
        fs::write(self.directory.join("model.json"), m.to_string()).unwrap();
    }
    fn actor(&self, id: &str) -> CronActor {
        actor(&self.core, &self.config, Some(id)).unwrap()
    }
    fn add(&self, actor: &CronActor, owner: &str) -> Result<Applied, SquadError> {
        apply(
            &self.core,
            &self.config,
            actor,
            Change::Add {
                squad: "product".into(),
                room_id: ROOM.into(),
                owner_id: owner.into(),
                message: "literal {time}\n!\0 message".into(),
                schedule: Schedule::parse(
                    cron::ScheduleInput::Every {
                        duration: "30m",
                        from: None,
                    },
                    "UTC",
                    0,
                )
                .unwrap(),
                paused: false,
            },
            0,
        )
    }
    fn mutate(
        &self,
        actor: &CronActor,
        job: &Job,
        mutation: Mutation,
    ) -> Result<Applied, SquadError> {
        apply(
            &self.core,
            &self.config,
            actor,
            Change::Existing {
                key: JobKey::of(job),
                expected_revision: job.revision,
                mutation,
            },
            100,
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

#[test]
fn reads_are_unrestricted_and_actor_admission_never_falls_back_from_a_member() {
    let f = Fixture::new();
    assert!(
        list_jobs(&f.core, &f.config, None, 0)
            .unwrap()
            .jobs
            .is_empty()
    );
    assert!(!f.directory.join("squad").exists());
    let bytes = fs::read(f.config.path()).unwrap();
    f.change_model(|m| m["caller"] = json!(WORKER));
    let member = actor(&f.core, &f.config, None).unwrap();
    assert_eq!(member.id, WORKER);
    assert_eq!(
        f.add(&member, WORKER).err().unwrap().code,
        "SQUAD_CRON_PERMISSION_DENIED"
    );
    f.change_model(|m| m["caller"] = json!("ambiguous"));
    assert_eq!(
        actor(&f.core, &f.config, None).unwrap_err().code,
        "CALLER_IDENTITY_AMBIGUOUS"
    );
    let job = f.add(&f.actor(LEAD), WORKER).unwrap().job.job;
    assert_eq!(
        show_job(&f.core, &f.config, &JobKey::of(&job), 0)
            .unwrap()
            .next_ms
            .len(),
        3
    );
    assert_eq!(fs::read(f.config.path()).unwrap(), bytes);
}

#[test]
fn locked_revision_room_and_membership_guards_preserve_the_published_job() {
    let f = Fixture::new();
    let user = f.actor(USER);
    let job = f.add(&user, WORKER).unwrap().job.job;
    let paused = f.mutate(&user, &job, Mutation::Pause).unwrap().job.job;
    let before = fs::read(f.directory.join("squad/cron/jobs.json")).unwrap();
    assert_eq!(
        f.mutate(&user, &job, Mutation::Resume).err().unwrap().code,
        "SQUAD_CRON_REVISION_CONFLICT"
    );
    assert_eq!(
        authorize_write(&f.core, &f.config, &JobKey::of(&job), &user, job.revision)
            .unwrap_err()
            .code,
        "SQUAD_CRON_REVISION_CONFLICT"
    );
    assert_eq!(
        admit_scheduled(&f.core, &JobKey::of(&paused), paused.revision)
            .unwrap_err()
            .code,
        "SQUAD_CRON_NOT_ON"
    );
    f.change_model(|m| m["room"] = json!("55555555-5555-4555-8555-555555555555"));
    assert_eq!(
        f.mutate(&user, &paused, Mutation::Resume)
            .err()
            .unwrap()
            .code,
        "SQUAD_CRON_ROOM_CONFLICT"
    );
    assert_eq!(
        f.add(&user, WORKER).err().unwrap().code,
        "SQUAD_CRON_ROOM_CONFLICT"
    );
    assert_eq!(
        fs::read(f.directory.join("squad/cron/jobs.json")).unwrap(),
        before
    );
    let calls = f.model()["calls"].as_array().unwrap().clone();
    assert!(
        calls
            .iter()
            .any(|call| call["locked"] == true && call["request"]["operation"] == "rooms.roster")
    );
}

#[test]
fn notices_target_owners_suppress_actor_and_never_undo_a_committed_change() {
    let f = Fixture::new();
    let user = f.actor(USER);
    let lead = f.actor(LEAD);
    let job = f.add(&user, WORKER).unwrap().job.job;
    let paused = f.mutate(&user, &job, Mutation::Pause).unwrap().job.job;
    let resumed = f.mutate(&user, &paused, Mutation::Resume).unwrap().job.job;
    let reassigned = f
        .mutate(
            &user,
            &resumed,
            Mutation::Reassign {
                owner_id: LEAD.into(),
            },
        )
        .unwrap()
        .job
        .job;
    let notices = f.model()["notices"].as_array().unwrap().clone();
    assert_eq!(notices.len(), 5);
    assert_eq!(notices[3]["input"]["recipientIds"], json!([WORKER]));
    assert_eq!(notices[4]["input"]["recipientIds"], json!([LEAD]));
    assert!(
        notices[4]["input"]["message"]
            .as_str()
            .unwrap()
            .contains("literal {time}\\n!\\u{0} message")
    );
    for notice in &notices {
        assert_eq!(notice["input"]["kind"], "announcement");
        assert!(
            notice["input"]["message"]
                .as_str()
                .unwrap()
                .starts_with("▚ ⏱")
        );
        assert!(!notice["input"]["message"].as_str().unwrap().contains('\n'));
    }
    let paused = f
        .mutate(&lead, &reassigned, Mutation::Pause)
        .unwrap()
        .job
        .job;
    assert_eq!(f.model()["notices"].as_array().unwrap().len(), 5);
    assert!(!f.mutate(&lead, &paused, Mutation::Pause).unwrap().changed);
    f.change_model(|m| m["noticeFailure"] = json!(true));
    let applied = f.mutate(&user, &paused, Mutation::Resume).unwrap();
    assert_eq!(applied.job.job.state(), "on");
    assert_eq!(applied.warnings[0].code, "DISPATCH_FAILURE");
    assert_eq!(
        Store::new(&f.directory).unwrap().read().unwrap().jobs()[0],
        applied.job.job
    );
}

#[test]
fn retirement_is_durable_before_ack_and_obsolete_hooks_cannot_clear_a_new_owner() {
    let f = Fixture::new();
    let user = f.actor(USER);
    let original = f.add(&user, WORKER).unwrap().job.job;
    let new = f
        .mutate(
            &user,
            &original,
            Mutation::Reassign {
                owner_id: LEAD.into(),
            },
        )
        .unwrap()
        .job
        .job;
    f.change_model(|m| m["retired"] = json!([WORKER]));
    assert!(drain_retired(&f.core, &f.config, 200).unwrap().is_empty());
    assert_eq!(
        Store::new(&f.directory).unwrap().read().unwrap().jobs()[0],
        new
    );
    let f = Fixture::new();
    let user = f.actor(USER);
    let owned = f.add(&user, WORKER).unwrap().job.job;
    f.change_model(|m| m["retired"] = json!([WORKER]));
    let view = show_job(&f.core, &f.config, &JobKey::of(&owned), 300).unwrap();
    assert_eq!(view.job.state(), "no owner");
    assert_eq!(view.job.revision, owned.revision + 1);
    assert_eq!(view.job.pause.as_ref().unwrap().at_ms, 300);
    assert!(f.model()["hooks"].as_array().unwrap().is_empty());
    let notices = f.model()["notices"].as_array().unwrap().clone();
    assert_eq!(
        notices.last().unwrap()["input"]["recipientIds"],
        json!([LEAD])
    );
    assert_eq!(notices.last().unwrap()["originator"], "anonymous");
    assert!(drain_retired(&f.core, &f.config, 400).unwrap().is_empty());
    assert_eq!(
        f.model()["notices"].as_array().unwrap().len(),
        notices.len()
    );
    assert_eq!(
        f.mutate(&user, &view.job, Mutation::Resume)
            .err()
            .unwrap()
            .code,
        "SQUAD_CRON_NO_OWNER"
    );
}

#[test]
fn failed_hook_registration_rolls_back_and_non_members_cannot_own_jobs() {
    let f = Fixture::new();
    let user = f.actor(USER);
    f.change_model(|m| m["hookFailure"] = json!(true));
    assert_eq!(f.add(&user, WORKER).err().unwrap().code, "HOOK_FAILURE");
    assert!(
        Store::new(&f.directory)
            .unwrap()
            .read()
            .unwrap()
            .jobs()
            .is_empty()
    );
    f.change_model(|m| m["hookFailure"] = json!(false));
    let job = f.add(&user, WORKER).unwrap().job.job;
    assert_eq!(job.id(), "c1");
    f.change_model(|m| m["rosterWithout"] = json!([WORKER]));
    assert_eq!(
        admit_scheduled(&f.core, &JobKey::of(&job), job.revision)
            .unwrap_err()
            .code,
        "SQUAD_NOT_A_MEMBER"
    );
    let empty = f.mutate(
        &user,
        &job,
        Mutation::Edit {
            message: Some(" \n".into()),
            schedule: None,
        },
    );
    assert_eq!(empty.err().unwrap().code, "SQUAD_CRON_MESSAGE_INVALID");
    assert_eq!(
        Store::new(&f.directory).unwrap().read().unwrap().jobs()[0]
            .owner_id
            .as_deref(),
        Some(WORKER)
    );
}

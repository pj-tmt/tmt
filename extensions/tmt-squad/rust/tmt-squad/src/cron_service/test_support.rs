//! Public-port fixtures shared by management and clock scenario tests.
use super::*;
use std::{
    fs,
    sync::atomic::{AtomicUsize, Ordering},
};

pub(crate) const USER: &str = "11111111-1111-4111-8111-111111111111";
pub(crate) const LEAD: &str = "22222222-2222-4222-8222-222222222222";
pub(crate) const WORKER: &str = "33333333-3333-4333-8333-333333333333";
pub(crate) const ROOM: &str = "44444444-4444-4444-8444-444444444444";
pub(crate) struct Fixture {
    pub(crate) directory: PathBuf,
    pub(crate) core: Core,
    pub(crate) config: Config,
}
impl Fixture {
    pub(crate) fn new() -> Self {
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
import json,sys,pathlib,fcntl,os,signal,tempfile
p=pathlib.Path(__file__).parent
def save(m):
 fd,tmp=tempfile.mkstemp(dir=p)
 with os.fdopen(fd,'w') as f: json.dump(m,f)
 os.replace(tmp,p/'model.json')
guard=open(p/'model.lock','a+'); fcntl.flock(guard,fcntl.LOCK_EX)
m=json.loads((p/'model.json').read_text()); a=sys.argv[1:]
def fail(code):
 print(json.dumps({'error':{'code':code,'message':code}})); sys.exit(1)
if a[0]=='api':
 q=json.load(sys.stdin); op=q['operation']; i=q['input']; locked=False
 if m.get('blockOn')==op:
  fcntl.flock(guard,fcntl.LOCK_UN)
  (p/'blocked.pid').write_text(str(os.getpid()))
  signal.pause()
  fcntl.flock(guard,fcntl.LOCK_EX); m=json.loads((p/'model.json').read_text())
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
  if i['kind']=='announcement' and m['noticeFailure']:
   save(m); fail('DISPATCH_FAILURE')
  if i['kind']=='announcement':
   m['notices'].append(q); out={'items':[{'recipientId':id,'acceptance':'queued'} for id in i['recipientIds']]}
  else:
   intent={'input':i,'identity':q.get('identity'),'originator':q.get('originator')}
   dispatches=m.setdefault('dispatches',{}); previous=dispatches.get(i['operationId'])
   if previous and previous['intent']!=intent: save(m); fail('DISPATCH_OPERATION_CONFLICT')
   if previous: out=previous['receipt']
   else:
    out={'operationId':i['operationId'],'items':[{'recipientId':id,'acceptance':'queued','requestId':'req_'+i['operationId']} for id in i['recipientIds']]}
    dispatches[i['operationId']]={'intent':intent,'receipt':out}; m.setdefault('wakes',[]).append(i['operationId'])
   if m.get('loseResponse'):
    structured=m['loseResponse']=='storage'; m['loseResponse']=False; save(m)
    if structured: fail('STORAGE_UNAVAILABLE')
    print('interrupted'); sys.exit(0)
 elif op=='dispatch.show':
  previous=m.get('dispatches',{}).get(i['operationId'])
  if not previous: save(m); fail('DISPATCH_NOT_FOUND')
  out=previous['receipt']
 else: fail('API_INPUT_INVALID')
 save(m); print(json.dumps(out))
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
    pub(crate) fn model(&self) -> Value {
        serde_json::from_slice(&fs::read(self.directory.join("model.json")).unwrap()).unwrap()
    }
    pub(crate) fn change_model(&self, edit: impl FnOnce(&mut Value)) {
        let lock = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.directory.join("model.lock"))
            .unwrap();
        let _guard = nix::fcntl::Flock::lock(lock, nix::fcntl::FlockArg::LockExclusive).unwrap();
        let mut m = self.model();
        edit(&mut m);
        let temporary = self.directory.join("model.rust.tmp");
        fs::write(&temporary, m.to_string()).unwrap();
        fs::rename(temporary, self.directory.join("model.json")).unwrap();
    }
    pub(crate) fn actor(&self, id: &str) -> CronActor {
        actor(&self.core, &self.config, Some(id)).unwrap()
    }
    pub(crate) fn add(&self, actor: &CronActor, owner: &str) -> Result<Applied, SquadError> {
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
    pub(crate) fn mutate(
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

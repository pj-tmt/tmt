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
        let model = json!({"room":ROOM,"user":USER,"caller":null,"retired":[],"rosterWithout":[],"hooks":[],"calls":[],"notices":[],"noticeFailure":false,"hookFailure":false,
            "members":[{"id":USER,"name":"Ben","lifetime":"saved","metadata":{}},
            {"id":LEAD,"name":"Sol","lifetime":"saved","metadata":{"squad.product.lead.marker":"true"}},
            {"id":WORKER,"name":"worker","lifetime":"saved","metadata":{}}]});
        fs::write(directory.join("model.json"), model.to_string()).unwrap();
        fs::write(directory.join("ops.toml"), "").unwrap();
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
 if (p/'ops/ops.db').exists():
  import sqlite3
  try:
   probe=sqlite3.connect(str(p/'ops/ops.db'),timeout=0,isolation_level=None)
   probe.execute('BEGIN IMMEDIATE'); probe.execute('ROLLBACK'); probe.close()
  except sqlite3.OperationalError: locked=True
 m['calls'].append({'request':q,'locked':locked})
 if m.get('failOperation')==op: save(m); fail('STORAGE_UNAVAILABLE')
 if op=='storage.root': out={'dataRoot':str(p)}
 elif op=='rooms.write':
  assert q.get('identity') or q.get('originator')=='anonymous'
  if m.get('roomWriteFailure'): save(m); fail(m['roomWriteFailure'])
  if i['roomId']!=m['room'] or i['room']['expectedRevision']!=m.get('roomRevision',1): save(m); fail('ROOM_REVISION_CONFLICT')
  m['roomName']=i['room']['name']; m['roomRevision']=m.get('roomRevision',1)+1
  out={'id':m['room'],'name':m['roomName'],'revision':m['roomRevision'],'retired':False,'memberIds':i['room']['memberIds']}
  if m.get('loseRoomWriteResponse'): save(m); print('interrupted'); sys.exit(0)
  if m.get('malformedRoomWriteResponse'): out={}
 elif op=='rooms.roster':
  if i['room']==m['room']:
   ids=[x['id'] for x in m['members'] if x['id'] not in m['rosterWithout']]
   room={'id':m['room'],'name':m.get('roomName','squad-product'),'revision':m.get('roomRevision',1),'retired':False,'memberIds':ids}
  else:
   room=m.get('otherRooms',{}).get(i['room'])
   if room is None: save(m); fail('ROOM_NOT_FOUND')
   ids=room['memberIds']
  out={'room':room,'members':[x for x in m['members'] if x['id'] not in m['retired'] and x['id'] in ids]}
 elif op=='user.identity':
  user=next((x for x in m['members'] if x['id']==m.get('user') and x['id'] not in m['retired']),None)
  if user is None: save(m); fail('USER_NAME_UNAVAILABLE')
  out={'identityId':user['id'],'name':user['name'],'created':False}
 elif op=='identities.status':
  out={'identities':[{'id':id,'found':True,'canPassthrough':m.get('canPassthrough',{})} for id in i['identityIds']]}
 elif op=='references.resolve':
  out={'identities':[dict(next((x for x in m['members'] if x['id']==id),{'id':id}),found=any(x['id']==id for x in m['members']),retired=id in m['retired']) for id in i.get('identityIds',[])]}
  if m.get('malformedReferences'): out={'identities':[]}
 elif op=='identity.meta.apply':
  assert 'identity' not in q and 'originator' not in q
  if m.get('applyFailure'): save(m); fail(m['applyFailure'])
  member=next((x for x in m['members'] if x['id']==i['identityId'] and x['id'] not in m['retired']),None)
  if member is None: save(m); fail('NAME_NOT_FOUND')
  metadata=member['metadata']; current={}
  for change in i['changes']:
   key=change['key']; expect=change['expect']
   assert expect!='any'
   if (expect=='absent' and key in metadata) or (isinstance(expect,dict) and metadata.get(key)!=expect['value']): current[key]=metadata.get(key)
  if current:
   save(m); print(json.dumps({'error':{'code':'METADATA_CONFLICT','message':'conflict','current':current}})); sys.exit(5)
  before=dict(metadata)
  for change in i['changes']:
   if change['then']=='remove': metadata.pop(change['key'],None)
   else: metadata[change['key']]=change['then']['set']
  out={'identityId':i['identityId'],'changed':metadata!=before}
  if m.get('malformedApplyResponse'): out={}
  if m.get('loseApplyResponse'):
   save(m); print('interrupted'); sys.exit(0)
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
  if m.get('dispatchRefusal'): save(m); fail(m['dispatchRefusal'])
  if m.get('rejectOrigin') and 'origin' in i: save(m); fail('API_INPUT_INVALID')
  if i['kind']=='announcement' and m['noticeFailure']:
   save(m); fail('DISPATCH_FAILURE')
  intent={'input':i,'identity':q.get('identity'),'originator':q.get('originator')}
  dispatches=m.setdefault('announcements' if i['kind']=='announcement' else 'dispatches',{}); previous=dispatches.get(i['operationId'])
  if previous and previous['intent']!=intent: save(m); fail('DISPATCH_IDEMPOTENCY_CONFLICT')
  if previous: out=previous['receipt']
  else:
   out={'operationId':i['operationId'],'items':[{'recipientId':id,'acceptance':'recipientUnavailable' if m.get('recipientUnavailable') else 'queued','requestId':'req_'+i['operationId']} for id in i['recipientIds']]}
   dispatches[i['operationId']]={'intent':intent,'receipt':out}
   if i['kind']=='announcement': m['notices'].append(q)
   else: m.setdefault('wakes',[]).append(i['operationId'])
  if m.get('dispatchWake'): out=dict(out,wake={'status':m['dispatchWake']})
  if m.get('dispatchCleanupFailure'):
   journal=p/'board-dispatches'/f"{i['operationId']}.json"; journal.unlink(); journal.mkdir()
  response_key='loseAnnouncementResponse' if i['kind']=='announcement' else 'loseResponse'
  if m.get(response_key):
   structured=m[response_key]=='storage'; m[response_key]=False; save(m)
   if structured: fail('STORAGE_UNAVAILABLE')
   print('interrupted'); sys.exit(0)
 elif op=='requests.show':
  if m.get('requestShowFailure'): save(m); fail('STORAGE_UNAVAILABLE')
  out=m.get('requestShow',{})
 elif op=='dispatch.show':
  if m.get('dispatchShowFailure'): save(m); fail('STORAGE_UNAVAILABLE')
  previous=m.get('dispatches',{}).get(i['operationId']) or m.get('announcements',{}).get(i['operationId'])
  if not previous: save(m); fail('DISPATCH_NOT_FOUND')
  out=previous['receipt']
 else: fail('API_INPUT_INVALID')
 save(m); print(json.dumps(out))
elif a[0]=='config': print(json.dumps({'paths':{'global':str(p/'config.toml')}}))
elif a[0]=='room' and a[1]=='leave':
 m['calls'].append({'command':a})
 import sqlite3
 database=p/'ops/ops.db'
 row=sqlite3.connect(str(database),timeout=5).execute("SELECT body FROM documents WHERE kind='cron' AND key=''").fetchone() if database.exists() else None
 m['jobsAtLeave']=json.loads(row[0]) if row else {'jobs':[]}
 if m.get('leaveFailure'): save(m); fail(m['leaveFailure'])
 id=a[a.index('--identity')+1]
 if id not in m['rosterWithout']: m['rosterWithout'].append(id)
 save(m)
 if m.get('loseLeaveResponse'): print('interrupted'); sys.exit(0)
 print(json.dumps({}))
elif a[0]=='room' and a[1]=='join':
 m['calls'].append({'command':a})
 if m.get('joinFailure'): save(m); fail(m['joinFailure'])
 id=a[a.index('--identity')+1]
 if a[2]==m['room']:
  if id in m['rosterWithout']: m['rosterWithout'].remove(id)
 else:
  room=m.get('otherRooms',{}).get(a[2])
  if room is None: save(m); fail('ROOM_NOT_FOUND')
  if id not in room['memberIds']: room['memberIds'].append(id)
 save(m); print(json.dumps({}))
elif a[0]=='room' and a[1]=='show':
 if a[2] in [m['room'],m.get('roomName','squad-product')]: room={'id':m['room'],'name':m.get('roomName','squad-product'),'revision':m.get('roomRevision',1),'retired':False,'memberIds':[x['id'] for x in m['members'] if x['id'] not in m['rosterWithout']]}
 else:
  room=next((room for room in m.get('otherRooms',{}).values() if a[2] in [room['id'],room['name']]),None)
  if room is None: fail('ROOM_NOT_FOUND')
 print(json.dumps({'room':room}))
elif a[0]=='ls':
 m['calls'].append({'command':a}); save(m); print(json.dumps({'identities':m.get('ls',[])}))
elif a[0]=='room': print(json.dumps({'rooms':[{'id':m['room'],'name':m.get('roomName','squad-product')}]}))
elif a[0]=='identity' and a[1]=='meta':
 id=a[a.index('--identity')+1]
 x=next((x for x in m['members'] if x['id']==id and id not in m['retired']),None)
 if x is None: fail('NAME_NOT_FOUND')
 m['calls'].append({'command':a})
 if a[2] in ['list','ls']: out={'identityId':id,'metadata':x['metadata']}
 elif a[2]=='rm':
  if m.get('metaRemoveFailure'): save(m); fail(m['metaRemoveFailure'])
  x['metadata'].pop(a[3],None); out={}
 elif a[2]=='set': x['metadata'][a[3]]=a[4]; out={}
 else: fail('UNKNOWN_COMMAND')
 save(m); print(json.dumps(out))
elif a[0]=='identity':
 x=next((x for x in m['members'] if a[2] in [x['id'],x['name']] and x['id'] not in m['retired']),None)
 if x is None: fail('NAME_NOT_FOUND')
 print(json.dumps({'identity':x}))
elif a[0]=='send-now':
 m['calls'].append({'command':a})
 failure=m.get('sendNowFailure')
 if failure: save(m); print(json.dumps({'error':failure})); sys.exit(1)
 if 'afterSendNow' in m: m['requestShow']=m['afterSendNow']
 save(m); print(json.dumps({'status':m.get('sendNowStatus','sent'),'requestId':a[1],'state':'delivered'}))
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
        let config = Config::read(directory.join("ops.toml")).unwrap();
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
        actor(&self.core, Some(id)).unwrap()
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

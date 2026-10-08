//! Independent literal public-port fixture; storage assertions read actual files.
use super::*;
use std::{
    fs,
    sync::atomic::{AtomicUsize, Ordering},
};

pub(super) const USER: &str = "11111111-1111-4111-8111-111111111111";
pub(super) const LEAD: &str = "22222222-2222-4222-8222-222222222222";
pub(crate) const WORKER: &str = "33333333-3333-4333-8333-333333333333";
pub(crate) const ROOM: &str = "44444444-4444-4444-8444-444444444444";
pub(crate) const CHECKLIST: &str = "55555555-5555-4555-8555-555555555555";
pub(crate) const ITEM: &str = "66666666-6666-4666-8666-666666666666";
pub(crate) const OTHER: &str = "77777777-7777-4777-8777-777777777777";
pub(crate) fn id(text: &str) -> Id {
    Id::parse(text).unwrap()
}
pub(super) fn create(
    item: &str,
    inventory: model::InventoryExpectation,
    assignee: Option<&str>,
) -> Request {
    Request {
        room_id: id(ROOM),
        checklist_id: id(CHECKLIST),
        action: model::Action::Create {
            item_id: id(item),
            inventory,
            content: model::ItemContent::new(
                "Authored title".into(),
                Some("literal body\n\t$() !".into()),
                Some("https://example.org/ref".into()),
            )
            .unwrap(),
            assignee: assignee.map(id),
        },
    }
}
pub(super) fn mutate(item: &str, revision: u64, mutation: model::Mutation) -> Request {
    Request {
        room_id: id(ROOM),
        checklist_id: id(CHECKLIST),
        action: model::Action::Item {
            item_id: id(item),
            revision,
            mutation,
        },
    }
}

pub(crate) struct Fixture {
    pub root: PathBuf,
    pub core: Core,
    pub config: Config,
}
impl Fixture {
    pub fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "squad-checklist-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let state = json!({"roomId":ROOM,"roomName":"squad-product","roomRetired":false,"caller":WORKER,"retired":[],"absent":[],"removed":[],"lead":LEAD,"calls":[],"lockedReads":0,
            "members":[{"id":USER,"name":"Ben","lifetime":"saved"},{"id":LEAD,"name":"Lead","lifetime":"saved"},{"id":WORKER,"name":"Worker","lifetime":"saved"}],
            "pending":"waiting","agentState":"busy","requests":["unchanged request"]});
        fs::write(root.join("model.json"), state.to_string()).unwrap();
        fs::write(root.join("ops.toml"), format!("me='Ben'\nme_id='{USER}'\n")).unwrap();
        for name in ["pending", "agent-state", "requests", "config.json"] {
            fs::write(root.join(name), format!("untouched {name}\n")).unwrap();
        }
        fs::write(root.join("core.py"), r#"
import json,sys,pathlib,fcntl,os,tempfile
p=pathlib.Path(__file__).parent
guard=open(p/'model.lock','a+');fcntl.flock(guard,fcntl.LOCK_EX)
m=json.loads((p/'model.json').read_text());a=sys.argv[1:]
def save():
 fd,tmp=tempfile.mkstemp(dir=p)
 with os.fdopen(fd,'w') as f:json.dump(m,f)
 os.replace(tmp,p/'model.json')
def fail(code):
 save();print(json.dumps({'error':{'code':code,'message':code}}));sys.exit(1)
def identities(ids):
 return [dict(next((x for x in m['members'] if x['id']==i),{'id':i}),found=any(x['id']==i for x in m['members']) and i not in m['absent'],retired=i in m['retired']) for i in ids]
locked=False
lock=p/'ops/checklist'/m.get('lockRoom',m['roomId'])/'items.lock'
if lock.exists():
 with open(lock,'r') as f:
  try:fcntl.flock(f,fcntl.LOCK_SH|fcntl.LOCK_NB)
  except BlockingIOError:locked=True
if a[0]=='api':
 q=json.load(sys.stdin);op=q['operation'];i=q['input']
 assert op in ['storage.root','references.resolve','rooms.roster'], 'unexpected side effect'
 if locked and op=='references.resolve' and i.get('roomIds'):
  m['lockedReads']+=1
  if m.get('changeAtLockedRead')==m['lockedReads']:
   m.update(m['change']);m['changeAtLockedRead']=None
 m['calls'].append({'operation':op,'input':i,'locked':locked})
 if op=='storage.root':out={'dataRoot':str(p)}
 elif op=='references.resolve':out={'identities':identities(i.get('identityIds',[])),'rooms':[{'id':r,'found':r==m['roomId'],'retired':m['roomRetired']} for r in i.get('roomIds',[])]}
 else:
  if i['room']!=m['roomId'] or m['roomRetired']:fail('ROOM_NOT_FOUND')
  prefix=i['metadataPrefix'];out={'members':[dict(x,metadata={prefix+'lead.marker':'true' if x['id']==m['lead'] else 'false'}) for x in m['members'] if x['id'] not in m['retired']+m['absent']+m['removed']]}
elif a[0]=='config':out={'paths':{'global':str(p/'config.toml')}}
elif a[0]=='whoami':
 if m['caller']=='ambiguous':fail('CALLER_IDENTITY_AMBIGUOUS')
 out={'bound':False} if m['caller'] is None else dict(next(x for x in m['members'] if x['id']==m['caller']),bound=True)
elif a[0]=='identity':
 x=next((x for x in m['members'] if (a[2].lower()==x['id'] or a[2]==x['name']) and x['id'] not in m['retired']+m['absent']),None)
 if x is None:fail('NAME_NOT_FOUND')
 out={'identity':x}
elif a[:2]==['room','show']:
 if a[2]!=m['roomId'] or m['roomRetired']:fail('ROOM_NOT_FOUND')
 out={'room':{'id':m['roomId'],'name':m['roomName'],'retired':False}}
else:fail('UNEXPECTED_COMMAND')
save();print(json.dumps(out))
"#).unwrap();
        let executable = root.join("tmt");
        crate::test_support::write_ready_executable(
            &executable,
            &format!(
                "#!/bin/sh\nexec /usr/bin/python3 '{}' \"$@\"\n",
                root.join("core.py").display()
            ),
        );
        let core = Core::at(executable);
        let config = Config::read(root.join("ops.toml")).unwrap();
        Self { root, core, config }
    }
    pub fn service(&self) -> Service {
        Service::new(&self.core, &self.config, id(ROOM)).unwrap()
    }
    pub fn model(&self) -> serde_json::Value {
        serde_json::from_slice(&fs::read(self.root.join("model.json")).unwrap()).unwrap()
    }
    pub fn change(&self, edit: impl FnOnce(&mut serde_json::Value)) {
        let file = fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(self.root.join("model.lock"))
            .unwrap();
        let _lock = nix::fcntl::Flock::lock(file, nix::fcntl::FlockArg::LockExclusive).unwrap();
        let mut value = self.model();
        edit(&mut value);
        let temp = self.root.join("model.rust.tmp");
        fs::write(&temp, value.to_string()).unwrap();
        fs::rename(temp, self.root.join("model.json")).unwrap();
    }
    pub fn manager(&self) -> Service {
        self.change(|m| m["caller"] = json!(LEAD));
        self.service()
    }
    pub fn directory(&self) -> PathBuf {
        self.root.join("ops/checklist").join(ROOM)
    }
    pub fn bytes(&self) -> Vec<u8> {
        fs::read(self.directory().join("items.json")).unwrap()
    }
    pub fn literal(&self) -> serde_json::Value {
        serde_json::from_slice(&self.bytes()).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

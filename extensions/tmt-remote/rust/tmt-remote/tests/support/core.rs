//! Deterministic public-process core fixture shared by operation and transport tests.
use serde_json::Value;
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
};
use tmt_remote::{core::CoreClient, operations::Operations, store::uuid_v4};
#[path = "executable_fixture.rs"]
pub(crate) mod executable_fixture;
pub struct Core {
    pub root: PathBuf,
}
impl Core {
    pub fn new() -> Self {
        let root = PathBuf::from(format!("/tmp/t1055-c-{}", uuid_v4().unwrap()));
        fs::create_dir(&root).unwrap();
        fs::write(root.join("core.py"),r#"import sys,json,pathlib
root=pathlib.Path(__file__).parent
args=sys.argv[1:]
if args[0] in ('list','check','identity'):
    with (root/'calls').open('a') as f: f.write(json.dumps({'operation':args[0],'argv':args})+'\n')
if args[0]=='list':
    print((root/'agents').read_text());sys.exit(0)
if args[:3]==['identity','show','--json'] and args[3].startswith('--expect-name='):
    assert len(args)==6 and args[4]=='--'
    rows=[r for r in json.loads((root/'identities').read_text())['identities'] if r['id']==args[5]]
    if not rows:
        print(json.dumps({'error':{'code':'NAME_NOT_FOUND','message':'absent'}}));sys.exit(1)
    row=rows[0]
    if row.get('retired',False) or row['name'].lower()!=args[3].split('=',1)[1].lower():
        print(json.dumps({'error':{'code':'IDENTITY_CHANGED','message':'changed'}}));sys.exit(5)
    print(json.dumps({'identity':dict(row,presence='offline')}));sys.exit(0)
if args[:4]==['identity','show','--json','--']:
    rows=[r for r in json.loads((root/'identities').read_text())['identities'] if r['name']==args[4] and not r.get('retired',False)]
    if len(rows)!=1:
        code='NAME_NOT_FOUND' if not rows else 'NAME_AMBIGUOUS'
        print(json.dumps({'error':{'code':code,'message':'name refused'}}));sys.exit(1)
    print(json.dumps({'identity':rows[0]}));sys.exit(0)
if args==['identity','list','--json']:
    print((root/'identities').read_text());sys.exit(0)
if args[0]=='check':
    print(json.dumps({'target':args[1],'pane':'%fixture','lines':int(args[-1]),'output':'bounded capture'}));sys.exit(0)
wire=json.load(sys.stdin)
receipt_file=root/('receipt-'+wire.get('input',{}).get('operationId','') if (root/'multi').exists() else 'receipt')
with (root/'calls').open('a') as f: f.write(json.dumps(wire)+'\n')
if wire['operation']=='storage.root':
    print(json.dumps({'dataRoot':(root/'storage-root').read_text()}));sys.exit(0)
if wire['operation']=='dispatch.show':
    if receipt_file.exists(): print(receipt_file.read_text())
    else:
        print(json.dumps({'error':{'code':'DISPATCH_NOT_FOUND','message':'absent'}}));sys.exit(1)
elif wire['operation']=='dispatch.create':
    if (root/'gate').exists():
        (root/'entered').write_text('blocked')
        with (root/'gate').open() as gate: gate.readline()
    if (root/'fault').exists():
        print('broken');sys.exit(0)
    if receipt_file.exists():
        print(json.dumps({'error':{'code':'DUPLICATE_TEST_SEND','message':'second create'}}));sys.exit(1)
    receipt={'operationId':wire['input']['operationId'],'items':[{'recipientId':wire['input']['recipientIds'][0],'requestId':'req_11111111-1111-4111-8111-111111111111','acceptance':'queued'}]}
    receipt_file.write_text(json.dumps(receipt))
    if (root/'lost').exists(): print('broken')
    else:
        receipt['wake']={'status':'uncertain','paneAttempted':True};print(json.dumps(receipt))
elif wire['operation']=='user.identity':
    if (root/'identity-error').exists():
        code,message=(root/'identity-error').read_text().split('|',1)
        print(json.dumps({'error':{'code':code,'message':message}}));sys.exit(1)
    person={'identityId':'22222222-2222-4222-8222-222222222222','name':'owner','created':False}
    if (root/'identity-renamed').exists(): person.update({'name':'owner-user','renamedFrom':'owner'})
    print(json.dumps(person))
elif wire['operation']=='identities.status':
    print(json.dumps({'identities':[{'id':id,'found':True,'status':{'state':'stale'}} for id in wire['input']['identityIds']]}))
elif wire['operation']=='requests.show':
    detail={'requestId':wire['input']['requestId'],'final':json.loads((root/'final').read_text())}
    if (root/'message-state').exists(): detail['messageState']=json.loads((root/'message-state').read_text())
    print(json.dumps(detail))
elif wire['operation']=='requests.sendNow':
    answer=(root/'send-now').read_text() if (root/'send-now').exists() else 'ok|sent'
    kind,value=answer.split('|',1)
    if kind=='raw':
        print(value);sys.exit(0)
    if kind=='err':
        code,_,text=value.partition('|')
        print(json.dumps({'error':{'code':code,'message':text or 'refused by the fixture'}}));sys.exit(1)
    print(json.dumps({'status':value,'requestId':wire['input']['requestId'],'state':'delivered'}))
else: raise RuntimeError('unexpected operation')
"#).unwrap();
        executable_fixture::write_executable(
            &root.join("tmt"),
            &format!(
                "exec /usr/bin/python3 '{}' \"$@\"",
                root.join("core.py").display()
            ),
        )
        .unwrap();
        Self { root }
    }
    pub fn operations(&self) -> Arc<Operations> {
        Arc::new(Operations::new(
            CoreClient::at(self.root.join("tmt")).unwrap(),
            Arc::new(AtomicBool::new(false)),
            65536,
        ))
    }
    pub fn calls(&self) -> Vec<Value> {
        fs::read_to_string(self.root.join("calls"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
    /// The person the fixture's `user.identity` resolves.
    pub const PERSON: &'static str = "22222222-2222-4222-8222-222222222222";
    /// Make `user.identity` fail the way Core does: a code and its message.
    pub fn identity_fails(&self, code: &str, message: &str) {
        fs::write(
            self.root.join("identity-error"),
            format!("{code}|{message}"),
        )
        .unwrap();
    }
    /// Make `user.identity` answer with the `-user` name an agent's name forced.
    pub fn identity_renamed(&self) {
        fs::write(self.root.join("identity-renamed"), "").unwrap();
    }
    /// What `requests.show` reports as the message state.
    pub fn message_state(&self, state: &Value) {
        fs::write(self.root.join("message-state"), state.to_string()).unwrap();
    }
    /// What `requests.sendNow` answers: `ok|sent`, `ok|already`, `err|<CODE>[|<message>]` or `raw|<unreadable text>`.
    pub fn send_now_answers(&self, answer: &str) {
        fs::write(self.root.join("send-now"), answer).unwrap();
    }
    /// The operations Core was asked for after `since` calls, in order.
    pub fn operations_since(&self, since: usize) -> Vec<String> {
        self.calls()
            .iter()
            .skip(since)
            .map(|call| call["operation"].as_str().unwrap_or_default().to_owned())
            .collect()
    }
    pub fn sends(&self) -> usize {
        self.calls()
            .iter()
            .filter(|call| call["operation"] == "dispatch.create")
            .count()
    }
}
impl Drop for Core {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

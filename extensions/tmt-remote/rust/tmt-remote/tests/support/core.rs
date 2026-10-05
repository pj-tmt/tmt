//! Deterministic public-process core fixture shared by operation and transport tests.
use serde_json::Value;
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
};
use tmt_remote::{core::CoreClient, operations::Operations, store::uuid_v4};
#[path = "executable_fixture.rs"]
mod executable_fixture;
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
if args==['identity','list','--json']:
    print((root/'identities').read_text());sys.exit(0)
if args[0]=='check':
    print(json.dumps({'target':args[1],'pane':'%fixture','lines':int(args[-1]),'output':'bounded capture'}));sys.exit(0)
wire=json.load(sys.stdin)
with (root/'calls').open('a') as f: f.write(json.dumps(wire)+'\n')
if wire['operation']=='storage.root':
    print(json.dumps({'dataRoot':(root/'storage-root').read_text()}));sys.exit(0)
if wire['operation']=='dispatch.show':
    if (root/'receipt').exists(): print((root/'receipt').read_text())
    else:
        print(json.dumps({'error':{'code':'DISPATCH_NOT_FOUND','message':'absent'}}));sys.exit(1)
elif wire['operation']=='dispatch.create':
    if (root/'gate').exists():
        (root/'entered').write_text('blocked')
        with (root/'gate').open() as gate: gate.readline()
    if (root/'fault').exists():
        print('broken');sys.exit(0)
    if (root/'receipt').exists():
        print(json.dumps({'error':{'code':'DUPLICATE_TEST_SEND','message':'second create'}}));sys.exit(1)
    receipt={'operationId':wire['input']['operationId'],'items':[{'recipientId':wire['input']['recipientIds'][0],'requestId':'req_11111111-1111-4111-8111-111111111111','acceptance':'queued'}]}
    (root/'receipt').write_text(json.dumps(receipt))
    if (root/'lost').exists(): print('broken')
    else:
        receipt['wake']={'status':'uncertain','paneAttempted':True};print(json.dumps(receipt))
elif wire['operation']=='identities.status':
    print(json.dumps({'identities':[{'id':id,'found':True,'status':{'state':'stale'}} for id in wire['input']['identityIds']]}))
elif wire['operation']=='requests.show':
    print(json.dumps({'requestId':wire['input']['requestId'],'final':json.loads((root/'final').read_text())}))
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

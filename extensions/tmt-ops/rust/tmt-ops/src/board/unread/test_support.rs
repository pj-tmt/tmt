//! Private Core API fixture for unread acknowledgment and migration tests.
use crate::{config::Config, core::Core};
use std::{fs, path::PathBuf};
pub(super) struct Fixture(pub PathBuf);
impl Fixture {
    pub fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "ops-unread-opening-{}",
            crate::id::new_v4().unwrap()
        ));
        fs::create_dir(&path).unwrap();
        fs::write(path.join("ops.toml"), "# authored preferences\n").unwrap();
        fs::write(path.join(".ops-paths-v1"), "{}").unwrap();
        let payload = r#"import json,sys
from pathlib import Path
root=Path(__file__).parent
args=sys.argv[1:]
if args[0]=='api':
    envelope=json.load(sys.stdin)
    with (root/"calls").open("a") as log: log.write(json.dumps(envelope)+'\n')
    if (root/'fail').exists():
        print(json.dumps({'error':{'code':'API_UNAVAILABLE','message':'ack failed'}}));sys.exit(1)
    stored=root/'acknowledged.json'
    acked=json.loads(stored.read_text()) if stored.exists() else {}
    if envelope['operation']=='requests.list':
        assert set(envelope['input']).issubset({'originatorId','view','limit','before'})
        assert envelope['input']['view']=='answers' and envelope['input']['limit']==50
        pages=json.loads((root/'pages.json').read_text()) if (root/'pages.json').exists() else {}
        cursor=envelope['input'].get('before',{}).get('requestId','')
        page=pages.get(cursor,{'items':[],'nextBefore':None})
        for item in page['items']:
            item['acknowledged']=item.get('acknowledged',False) or acked.get(item['requestId'])==item['revision']
        print(json.dumps(page))
    else:
        assert envelope['operation']=='requests.ack'
        assert 'identity' in envelope
        overrides=json.loads((root/'outcomes.json').read_text()) if (root/'outcomes.json').exists() else {}
        items=[]
        for item in envelope['input']['items']:
            status=overrides.get(item['requestId'],{'status':'already' if acked.get(item['requestId'])==item['revision'] else 'acknowledged'})
            if status['status'] in ('already','acknowledged'): acked[item['requestId']]=item['revision']
            items.append(dict(item,**status))
            stored.write_text(json.dumps(acked))
            if (root/'partial').exists():
                print(json.dumps({'error':{'code':'API_UNAVAILABLE','message':'partial ack failure'}}));sys.exit(1)
        print(json.dumps({'items':items}))
elif args[0]=='config':
    print(json.dumps({'paths':{'global':str(root/'config.json')},'resolved':{}}))
elif args[0]=='focus':
    with (root/"calls").open("a") as log: log.write('focus\n')
    if (root/'fail').exists():
        print(json.dumps({'error':{'code':'STALE','message':'Target changed'}}));sys.exit(5)
    print(json.dumps({'focused':{'pane':'%7'}}))
else: sys.exit(99)
"#;
        fs::write(path.join("mock.py"), payload).unwrap();
        crate::test_support::write_ready_executable(
            &path.join("tmt"),
            r#"#!/bin/sh
exec python3 "$(dirname "$0")/mock.py" "$@"
"#,
        );
        Self(path)
    }
    pub fn core(&self) -> Core {
        Core::at(self.0.join("tmt"))
    }
    pub fn config(&self) -> Config {
        Config::read(self.0.join("ops.toml")).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

"""Independent Python stdlib device-pairing MAC oracle; never imports product code.
Labels from remote-channel-v1 at a835e16b; exact peer enrollment bytes have
independent Python provenance.
"""
import hashlib
import hmac
import json
from pathlib import Path
import struct
import sys

root = Path(__file__).resolve().parents[4]
peer = json.loads((root / 'typescript/remote-client/test/vectors.json').read_text())
enrollment = bytes.fromhex(peer['enrollments'][0]['hex'])
code = bytes(range(16))
# Exact receipt bytes are proven as given; whitespace is deliberately non-canonical.
receipt = ('{"grant": {"clientId":"00000000-0000-4000-8000-000000000004","machineId":'
           '"00000000-0000-4000-8000-000000000002","profile":"local-v1","publicKey":"AAECAw",'
           '"kind":"addon","origin":"chrome-extension://pppppppppppppppppppppppppppppppp",'
           '"name":"Pilot","agents":"all","scopes":["agents.read","check.read","results.read",'
           '"status.read","talk"],"mode":"direct","issuedAtMs":1790770000000,'
           '"expiresAtMs":null,"revision":1,"disabled":false}, '
           '"machinePublicKey":"fixture-only"}\n').encode()
def lp(data):
    return struct.pack('>I', len(data)) + data

def mac(key, data):
    return hmac.digest(key, data, hashlib.sha256)

key = mac(code, lp(b'tmt-device-pair-response-key-v1') + lp(enrollment))
result = dict(provenance='Python 3 stdlib HMAC/SHA256; remote-channel-v1 a835e16b; fixture receipt is not an admitted grant.',
              code=code.hex(), enrollment=enrollment.hex(), receipt=receipt.hex(),
              enrollmentMac=mac(code,enrollment).hex(), responseKey=key.hex(),
              serverProof=mac(key,lp(b'tmt-device-pair-response-v1')+lp(receipt)).hex())
path = Path(__file__).with_name('mac-vectors.json')
if '--check' in sys.argv:
    assert json.loads(path.read_text()) == result, 'MAC fixture mismatch'
    print('Independent MAC fixture verified')
else:
    path.write_text(json.dumps(result,indent=2)+'\n')

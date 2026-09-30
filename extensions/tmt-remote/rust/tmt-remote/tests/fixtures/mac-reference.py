"""Independent Python stdlib local-v1 MAC oracle; never imports product code.
Contract ed7a3436; exact peer enrollment bytes have independent Python provenance.
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
receipt = b'{ "grant": {"mode":"hold"}, "machinePublicKey":"fixture-only" }\n'
def lp(data):
    return struct.pack('>I', len(data)) + data

def mac(key, data):
    return hmac.digest(key, data, hashlib.sha256)

key = mac(code, lp(b'tmt-local-pair-response-key-v1') + lp(enrollment))
result = dict(provenance='Python 3 stdlib HMAC/SHA256; contract ed7a3436; fixture receipt is not an admitted grant.',
              code=code.hex(), enrollment=enrollment.hex(), receipt=receipt.hex(),
              enrollmentMac=mac(code,enrollment).hex(), receiptKey=key.hex(),
              receiptProof=mac(key,lp(b'tmt-local-pair-response-v1')+lp(receipt)).hex())
path = Path(__file__).with_name('mac-vectors.json')
if '--check' in sys.argv:
    assert json.loads(path.read_text()) == result, 'MAC fixture mismatch'
    print('Independent MAC fixture verified')
else:
    path.write_text(json.dumps(result,indent=2)+'\n')

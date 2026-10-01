"""Independent stdlib oracle, never imports/invokes product code.
Field order transcribed from remote-client-v1 at ed7a3436. Unicode inputs establish
exact UTF-8 behavior. Regenerate here; --check compares the committed artifacts.
"""
import hashlib
import json
from pathlib import Path
import struct
import sys


def lp(value):
    raw = value.encode('utf-8', errors='strict') if isinstance(value, str) else value
    return struct.pack('>I', len(raw)) + raw


def framed_list(values):
    return struct.pack('>I', len(values)) + b''.join(lp(value) for value in values)


def uid(number):
    return f'00000000-0000-4000-8000-{number:012d}'


def envelope(value):
    fields = ['tmt-message-v1', str(value['version']), value['profile'], value['kind'],
              value['id'], value['correlationId'] or '', value['machineId'], value['windowId'],
              value['clientId'], value['sessionId'], value['sequence'], str(value['timestampMs']),
              value['origin'], value['operation'], hashlib.sha256(bytes.fromhex(value['payload'])).digest()]
    return b''.join(lp(field) for field in fields)


def enrollment(value):
    fields = ['tmt-local-pair-v1', value['profile'], value['machineId'], value['windowId'],
              value['offerId'], bytes.fromhex(value['serverChallenge']), bytes.fromhex(value['clientNonce']),
              value['kind'], value['origin'], value['name'], bytes.fromhex(value['publicKey'])]
    return (b''.join(lp(field) for field in fields) + framed_list(value['agentIds'])
            + framed_list(value['scopes']) + lp(value['mode']))


def fixture(name, value, raw):
    return dict(name=name, input=value, hex=raw.hex(), sha256=hashlib.sha256(raw).hexdigest())


request = dict(version=1, profile='local-v1', kind='request', id=uid(1), correlationId=None,
               machineId=uid(2), windowId=uid(3), clientId=uid(4), sessionId=uid(5),
               sequence='1', timestampMs=1790770000000, origin='cli', operation='dispatch.create',
               payload=b'{ "text": "hello" }\n'.hex())
response = dict(request, kind='response', id=uid(6), correlationId=uid(1),
                sequence='18446744073709551615', timestampMs=9007199254740991, payload=b'{}'.hex())
control = dict(request, kind='control', sessionId='new', sequence='0', timestampMs=0,
               origin='chrome-extension://' + 'a' * 32, operation='session.open', payload='')
unicode_request = dict(request, operation='probe.🚀.e\u0301', payload='{"x":"🚀e\u0301"}'.encode().hex())
candidate = dict(profile='local-v1', machineId=uid(2), windowId=uid(3), offerId=uid(7),
                 serverChallenge=bytes(range(16)).hex(), clientNonce=bytes(range(16,32)).hex(),
                 kind='addon', origin='chrome-extension://' + 'p' * 32, name='Pilot 🚀 e\u0301',
                 publicKey=bytes(range(32)).hex(), agentIds=[uid(8), uid(9)],
                 scopes=['agents.read','results.own','status.read','talk.hold'], mode='hold')
empty_candidate = dict(candidate, kind='cli', origin='cli', name='CLI', agentIds=[], scopes=[])
mac = bytes(range(32,64))
raw = enrollment(candidate)
result = dict(
    provenance='Independent Python 3 stdlib struct/hashlib oracle, first generated with Python 3.14.7. Contract ed7a3436; example key/MAC bytes prove framing, not cryptographic validity.',
    envelopes=[fixture(name,value,envelope(value)) for name,value in [
        ('request',request),('response-u64-max',response),('session-open',control),('unicode',unicode_request)]],
    enrollments=[fixture(name,value,enrollment(value)) for name,value in [('addon',candidate),('empty-cli',empty_candidate)]],
    possession=fixture('possession',dict(enrollment=raw.hex(),mac=mac.hex()),
                       lp('tmt-local-pair-possession-v1') + lp(raw) + lp(mac)),
)
path = Path(__file__).with_name('vectors.json')
if '--check' in sys.argv:
    if json.loads(path.read_text()) != result:
        raise SystemExit('Independent canonical fixture check failed')
    print('Independent fixture bytes and SHA-256 verified')
else:
    path.write_text(json.dumps(result, ensure_ascii=True, indent=2) + '\n')

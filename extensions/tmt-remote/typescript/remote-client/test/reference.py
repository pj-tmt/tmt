"""Independent stdlib oracle, never imports/invokes product code.
Envelope field order transcribed from remote-client-v1 at ed7a3436; device enrollment,
possession, pairing code and fingerprint rules from remote-channel-v1 (#956, a835e16b);
extension key certificate (tmt-ext-cert-v1) framing from its Extension channel API section.
Unicode inputs establish exact UTF-8 behavior. Regenerate here; --check compares the
committed artifacts.
"""
import base64
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
    fields = ['tmt-device-pair-v1', value['profile'], value['machineId'], value['windowId'],
              value['offerId'], bytes.fromhex(value['serverChallenge']), bytes.fromhex(value['clientNonce']),
              value['kind'], value['origin'], value['name'], bytes.fromhex(value['publicKey'])]
    return b''.join(lp(field) for field in fields)


WORDLIST = Path(__file__).resolve().parents[3] / 'rust/tmt-remote/assets/bip39-english.txt'
WORDLIST_SHA256 = '2f5eed53a4727b4bf8880d8f3f199efc90e58503646d9ff8eff3a2ed3b24dbda'
WORDLIST_SOURCE = ('https://raw.githubusercontent.com/bitcoin/bips/'
                   'ce1862ac6bcffa1dd20aad858380e51e66e949ea/bip-0039/english.txt')


def ext_cert(value):
    fields = ['tmt-ext-cert-v1', value['extension'], value['purpose'],
              bytes.fromhex(value['publicKey']), str(value['issuedAtMs'])]
    return b''.join(lp(field) for field in fields)


def fingerprint(public_key):
    raw = WORDLIST.read_bytes()
    if hashlib.sha256(raw).hexdigest() != WORDLIST_SHA256:
        raise SystemExit('Pinned BIP-39 English list digest mismatch')
    words = raw.decode('ascii').split('\n')[:2048]
    digest = hashlib.sha256(lp('tmt-local-key-fingerprint-v1') + lp(public_key)).digest()
    bits = int.from_bytes(digest[:8], 'big') >> 20
    indexes = [(bits >> shift) & 0x7ff for shift in (33, 22, 11, 0)]
    return dict(publicKey=public_key.hex(), indexes=indexes, words=[words[i] for i in indexes])


def code_text(code):
    symbols = base64.b32encode(code).decode('ascii').rstrip('=')
    return dict(code=code.hex(), text='-'.join(symbols[i:i + 4] for i in range(0, len(symbols), 4)))


def b64url(raw):
    return base64.urlsafe_b64encode(raw).decode('ascii').rstrip('=')


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
                 publicKey=bytes(range(32)).hex())
browser_candidate = dict(candidate, kind='browser', origin='http://127.0.0.1:65535', name='Laptop')
cli_candidate = dict(candidate, kind='cli', origin='cli', name='CLI')
mac = bytes(range(32,64))
raw = enrollment(candidate)
rfc8032_key = bytes.fromhex('d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a')
result = dict(
    provenance='Independent Python 3 stdlib struct/hashlib/base64 oracle. Envelopes first generated with Python 3.14.7 at contract ed7a3436 and unchanged; device enrollment, possession, pairing code and fingerprint vectors follow remote-channel-v1 at a835e16b; extension certificate vectors follow its Extension channel API section. Example key/MAC bytes prove framing, not cryptographic validity.',
    envelopes=[fixture(name,value,envelope(value)) for name,value in [
        ('request',request),('response-u64-max',response),('session-open',control),('unicode',unicode_request)]],
    enrollments=[fixture(name,value,enrollment(value)) for name,value in [
        ('addon',candidate),('browser',browser_candidate),('cli',cli_candidate)]],
    possession=fixture('possession',dict(enrollment=raw.hex(),mac=mac.hex()),
                       lp('tmt-device-pair-possession-v1') + lp(raw) + lp(mac)),
    wordlist=dict(source=WORDLIST_SOURCE, sha256=WORDLIST_SHA256),
    fingerprints=[fingerprint(key) for key in [bytes(range(32)), rfc8032_key, bytes([255]*32)]],
    extCerts=[fixture(name, value, ext_cert(value)) for name, value in [
        ('colab-sign', dict(extension='colab', purpose='sign', publicKey=bytes(range(32)).hex(),
                            issuedAtMs=1790770000000)),
        ('enc-max-time', dict(extension='my-ext2', purpose='enc', publicKey=bytes([255] * 32).hex(),
                              issuedAtMs=9007199254740991)),
        ('zero-time', dict(extension='a', purpose='sign', publicKey=rfc8032_key.hex(), issuedAtMs=0)),
    ]],
    # Each refusal changes one condition of the colab-sign case.
    invalidExtCerts=[
        dict(reason='uppercase extension', extension='Colab', purpose='sign', publicKeyLength=32, issuedAtMs=0),
        dict(reason='empty extension', extension='', purpose='sign', publicKeyLength=32, issuedAtMs=0),
        dict(reason='leading digit', extension='2colab', purpose='sign', publicKeyLength=32, issuedAtMs=0),
        dict(reason='long extension', extension='a' * 33, purpose='sign', publicKeyLength=32, issuedAtMs=0),
        dict(reason='unknown purpose', extension='colab', purpose='verify', publicKeyLength=32, issuedAtMs=0),
        dict(reason='short key', extension='colab', purpose='sign', publicKeyLength=31, issuedAtMs=0),
        dict(reason='time past 2^53-1', extension='colab', purpose='sign', publicKeyLength=32,
             issuedAtMs=9007199254740992),
    ],
    pairingCodes=[code_text(code) for code in [bytes(range(16)), bytes(16), bytes([255]*16)]],
    base64url=dict(
        valid=[dict(hex=raw.hex(), length=len(raw), text=b64url(raw))
               for raw in [b'', bytes([0xfb, 0xff]), bytes(range(32)), bytes([255] * 64)]],
        # Each invalid spelling changes one condition of a valid 2- or 32-byte value.
        invalid=[
            dict(reason='padding', length=2, text=b64url(bytes([0xfb, 0xff])) + '='),
            dict(reason='standard alphabet', length=2, text='-_8'.replace('-', '+').replace('_', '/')),
            dict(reason='nonzero unused bits', length=2, text='-_9'),
            dict(reason='wrong length', length=31, text=b64url(bytes(range(32)))),
            dict(reason='impossible symbol count', length=3, text='AAAAA'),
            dict(reason='whitespace', length=2, text=' -_8'),
        ]),
)
path = Path(__file__).with_name('vectors.json')
if '--check' in sys.argv:
    if json.loads(path.read_text()) != result:
        raise SystemExit('Independent canonical fixture check failed')
    print('Independent fixture bytes and SHA-256 verified')
else:
    path.write_text(json.dumps(result, ensure_ascii=True, indent=2) + '\n')

"""Independent content- and own-job byte oracle using public RFC8032 fixture keys only.

No Rust encoder or runtime is imported. stdlib owns LP/JSON/hash; cryptography
owns AES-GCM and Ed25519. Check frozen bytes by default; --write is deliberate.
"""
import argparse
import base64
import hashlib
import hmac
import json
import struct
from pathlib import Path
from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives.ciphers.aead import AESGCM

DEST = Path(__file__).with_name('publication-content-v1.json')
SEED = bytes.fromhex('9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60')
SPACE = '4kph3kmtxo7dinlvoixpw642ozfibd2w'
PAGE = '00000000-0000-4000-8000-000000000002'
DEVICE = '00000000-0000-4000-8000-000000000012'


def lp(*fields):
    return b''.join(struct.pack('>I', len(field)) + field for field in fields)


def digest(value):
    return hashlib.sha256(value).digest()


def b64(value):
    return base64.urlsafe_b64encode(value).rstrip(b'=').decode('ascii')


def unb64(value):
    return base64.urlsafe_b64decode(value + '=' * (-len(value) % 4))


def compact(value):
    return json.dumps(value, separators=(',', ':'), ensure_ascii=True).encode('ascii')


def envelope(signer, seq, previous, namespace=b'content'):
    header = lp(b'tmt-colab-object-v1', b'1', b'aes256gcm-hkdfsha256-ed25519-v1',
                SPACE.encode(), PAGE.encode(), b'7', b'update', namespace,
                ('%064x' % seq).encode(), DEVICE.encode(), b'3', str(seq).encode(), previous)
    # Existing model-reference HKDF and nonce rules; fixture plaintext is opaque to this codec.
    info = lp(b'tmt-colab-object-key-v1', header)
    prk = hmac.new(bytes(32), bytes(range(32)), hashlib.sha256).digest()
    key = hmac.new(prk, info + b'\x01', hashlib.sha256).digest()
    ciphertext = AESGCM(key).encrypt(bytes(12), b'public opaque update ' + str(seq).encode(), header)
    signature_input = lp(b'tmt-colab-signature-v1', header, bytes(12), digest(ciphertext))
    signature = signer.sign(signature_input)
    wire = {'header': b64(header), 'nonce': b64(bytes(12)), 'ciphertext': b64(ciphertext),
            'signature': b64(signature)}
    # Deliberately vary valid ordering/whitespace: publication binds these original JSON bytes.
    raw = compact(wire) if seq % 2 else json.dumps(dict(reversed(list(wire.items()))), indent=1).encode()
    hash_bytes = digest(lp(b'tmt-colab-envelope-hash-v1', header, bytes(12), ciphertext, signature))
    return {'json': raw.decode(), 'headerHex': header.hex(), 'ciphertextHex': ciphertext.hex(),
            'signatureInputHex': signature_input.hex(), 'signature': b64(signature),
            'envelopeHash': b64(hash_bytes)}, hash_bytes


def signature_input(m):
    entries = struct.pack('>I', len(m['entries'])) + b''.join(
        lp(e['namespace'].encode(), e['seq'].encode(), unb64(e['envelopeHash']),
           str(e['envelopeBytes']).encode()) for e in m['entries'])
    evidence = m.get('nativeEvidence')
    native = b'\0' if evidence is None else b'\1' + lp(
        evidence['sourceSha256'].encode(), evidence['memoryLimit'].encode(),
        unb64(evidence['chainHash']))
    return lp(b'tmt-colab-publication-v1', b'1', m['operationId'].encode(),
              m['spaceId'].encode(), m['pageId'].encode(), m['epoch'].encode(),
              m['streamId'].encode(), m['kind'].encode(), m['membershipHead']['revision'].encode(),
              m['membershipHead']['statementHash'].encode(), m['baseRevision'].encode(),
              entries, str(m['packetBytes']).encode(), unb64(m['packetHash']), native)


def generate():
    signer = Ed25519PrivateKey.from_private_bytes(SEED)
    public = signer.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw)
    fixtures = []
    for index, (form, count, memory) in enumerate([
            ('browser', 1, None), ('browser', 2, None),
            ('native', 1, '512 MiB address-space limit'), ('native', 2, 'memory limit unavailable'),
            ('own', 1, '512 MiB address-space limit'), ('own', 2, 'memory limit unavailable')], 1):
        kind = 'own' if form == 'own' else 'content'
        envelopes, previous = [], bytes(32)
        for seq in range(1, count + 1):
            item, previous = envelope(signer, seq, previous, kind.encode())
            envelopes.append(item)
        packet = b''.join(e['json'].encode() for e in envelopes)
        chain = b'public opaque certified-chain fixture'  # Certificate/issuer admission stays outside.
        manifest = {'version': 1, 'operationId': '00000000-0000-4000-8000-%012d' % (40 + index),
                    'spaceId': SPACE, 'pageId': PAGE, 'epoch': '7', 'streamId': DEVICE,
                    'kind': kind, 'membershipHead': {'revision': '3', 'statementHash': 'aa' * 32},
                    'baseRevision': 'v1:' + 'bb' * 32,
                    'entries': [{'namespace': kind, 'seq': str(i + 1),
                                 'envelopeHash': e['envelopeHash'], 'envelopeBytes': len(e['json'].encode())}
                                for i, e in enumerate(envelopes)],
                    'packetBytes': len(packet), 'packetHash': b64(digest(packet))}
        if memory is not None:
            manifest['nativeEvidence'] = {'sourceSha256': digest(b'public expected source').hex(),
                                         'memoryLimit': memory, 'chainHash': b64(digest(chain))}
        message = signature_input(manifest)
        job_digest = b64(digest(message))
        job = {'manifest': manifest, 'signature': b64(signer.sign(message))}
        key = {'operationId': manifest['operationId'], 'jobDigest': job_digest,
               'spaceId': SPACE, 'pageId': PAGE, 'originalEpoch': '7', 'streamId': DEVICE}
        committed = {'status': 'committed', 'key': key, 'count': count,
                     'finalPosition': {'seq': str(count), 'envelopeHash': envelopes[-1]['envelopeHash']},
                     'committedRevision': 'v1:' + 'cc' * 32}
        if memory is not None:
            committed['nativeEvidence'] = manifest['nativeEvidence']
        fixture = {'name': '%s-%d' % (form, count), 'envelopes': envelopes,
                   'packet': b64(packet), 'packetHash': manifest['packetHash'], 'signedJob': job,
                   'signatureInputHex': message.hex(), 'jobDigest': job_digest, 'key': key,
                   'outcomes': [committed, {'status': 'rejected', 'key': key, 'code': 'COLAB_STALE_BASE'},
                                {'status': 'unknown', 'key': key}],
                   'localStatus': {'version': 2, 'action': 'status', 'key': key}}
        if memory is not None:
            fixture['localWrite'] = {'version': 2, 'action': 'write', 'signedJob': job,
                                     'packet': b64(packet), 'chain': b64(chain)}
        fixtures.append(fixture)
    return {'provenance': 'Independent Python LP/SHA256/AES-GCM/Ed25519; public RFC8032 test-1 seed; no Rust encoder.',
            'seedHex': SEED.hex(), 'publicKey': b64(public), 'fixtures': fixtures}


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--write', action='store_true')
    args = parser.parse_args()
    frozen = json.dumps(generate(), indent=2) + '\n'
    if args.write:
        DEST.write_text(frozen)
    elif DEST.read_text() != frozen:
        raise SystemExit('publication vectors differ; review before --write')
    else:
        print('Independent publication vectors match exactly (6 fixtures).')

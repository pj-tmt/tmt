"""Independent frozen fixture generator: public fixture keys only, never a runtime input."""
import hashlib, hmac, json, struct
from pathlib import Path
from cryptography.hazmat.primitives.ciphers.aead import AESGCM
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives import serialization
ROOT = Path(__file__).resolve().parents[4]
DEST = ROOT / 'extensions/tmt-colab/contracts/vectors/model-v1.json'

def lp(*fields):
    return b''.join(struct.pack('>I', len(f)) + f for f in fields)
def mac(key, message):
    return hmac.new(key, message, hashlib.sha256).digest()
def digest(message):
    return hashlib.sha256(message).digest()
def dump(value):
    return value.hex()

def generate():
    # #829 public RFC8032 fixture seed/master; namespace is the contract's new field.
    seed = bytes.fromhex('9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60')
    signer = Ed25519PrivateKey.from_private_bytes(seed)
    public = signer.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw)
    space = b'4kph3kmtxo7dinlvoixpw642ozfibd2w'
    page = b'00000000-0000-4000-8000-000000000002'
    device = b'00000000-0000-4000-8000-000000000012'
    object_id = b'01' * 32
    master = bytes(range(32))
    header = lp(b'tmt-colab-object-v1', b'1', b'aes256gcm-hkdfsha256-ed25519-v1', space, page, b'1', b'update', b'content', object_id, device, b'1', b'1', bytes(32))
    key = mac(mac(bytes(32), master), lp(b'tmt-colab-object-key-v1', header) + b'\x01')
    plaintext = b'\x01\x00\xffYjs'
    ct = AESGCM(key).encrypt(bytes(12), plaintext, header)
    signature = signer.sign(lp(b'tmt-colab-signature-v1', header, bytes(12), digest(ct)))
    code_id = b'00000000-0000-4000-8000-000000000040'
    signin = lp(b'tmt-colab-signin-v1', b'1', code_id, space, device, public, bytes([7])*32, bytes(range(16)))
    possession = lp(b'tmt-colab-signin-possession-v1', b'1', signin)
    cut = lp(b'tmt-colab-stream-cut-v1', b'1', device, b'content', bytes([8])*32, b'2', b'3', bytes([11])*32)
    payload = json.dumps(dict(pageId=page.decode(),mode="shared"),separators=(",", ":")).encode()
    management = lp(b'tmt-colab-management-v1', b'1', space, page, b'1', code_id, b'page.history', digest(payload), device, b'1790860000000', b'1790860600000')
    return {k:dump(v) for k,v in dict(seed=seed,public=public,master=master,header=header,key=key,plaintext=plaintext,ciphertext=ct,signature=signature,envelopeHash=digest(lp(b'tmt-colab-envelope-hash-v1',header,bytes(12),ct,signature)),code=bytes(range(16)),signin=signin,proof=mac(bytes(range(16)),signin),possession=possession,possessionSignature=signer.sign(possession),cut=cut,payload=payload,management=management).items()}

if __name__ == '__main__':
    import argparse
    parser = argparse.ArgumentParser()
    parser.add_argument('--write', action='store_true')
    args = parser.parse_args()
    frozen = json.dumps(generate(), indent=2) + '\n'
    if args.write:
        DEST.write_text(frozen)
    elif DEST.read_text() != frozen:
        raise SystemExit('colab model vectors differ; review before --write')

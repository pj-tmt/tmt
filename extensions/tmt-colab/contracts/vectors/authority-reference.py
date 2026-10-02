"""Independent RFC9180/LP oracle; all seeds are public test fixtures, never runtime inputs."""
import base64, hashlib, hmac, json, struct
from pathlib import Path
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives.asymmetric.x25519 import X25519PrivateKey
from cryptography.hazmat.primitives.ciphers.aead import AESGCM
from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat
DEST = Path(__file__).with_name("authority-v1.json")
def lp(*v):
    return b"".join(struct.pack(">I", len(x)) + x for x in v)
def mac(k, v):
    return hmac.new(k, v, hashlib.sha256).digest()
def digest(v):
    return hashlib.sha256(v).digest()
def expand(k, info, n):
    return mac(k, info + b"\x01")[:n]
def public(k):
    return k.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw)
def b64(v):
    return base64.urlsafe_b64encode(v).decode().rstrip("=")
def generate():
    seed = bytes.fromhex("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60")
    owner = Ed25519PrivateKey.from_private_bytes(seed)
    pub = public(owner)
    space = base64.b32encode(digest(lp(b"tmt-colab-space-id-v1",pub))[:20]).decode().lower()
    page = "00000000-0000-4000-8000-000000000002"
    device = "00000000-0000-4000-8000-000000000050"
    # RFC9180 AES256GCM recipient/ephemeral keys retained from #829's public fixture.
    sk_r = bytes.fromhex("497b4502664cfea5d5af0b39934dac72242a74f8480451e1aee7d6a53320333d")
    sk_e = bytes.fromhex("179d4b53b6365c45b600c4163b61d95cbc2f4d9e36f1695558dce265ab8bab11")
    r, e = X25519PrivateKey.from_private_bytes(sk_r), X25519PrivateKey.from_private_bytes(sk_e)
    enc, pk_r = public(e), public(r)
    header = lp(b"tmt-colab-wrap-v1", b"1", b"base-x25519-hkdfsha256-aes256gcm", space.encode(), page.encode(), b"1", b"device", device.encode(), pk_r, pub, b"2", b"epoch-key")
    info = lp(b"tmt-colab-hpke-info-v1", header)
    def extract(suite, label, value, salt=b""):
        return mac(salt, b"HPKE-v1" + suite + label + value)
    def labeled_expand(suite, key, label, value, n):
        return expand(key, struct.pack(">H", n) + b"HPKE-v1" + suite + label + value, n)
    kem = b"KEM" + bytes.fromhex("0020")
    shared = labeled_expand(kem, extract(kem, b"eae_prk", e.exchange(r.public_key())), b"shared_secret", enc + pk_r, 32)
    suite = b"HPKE" + bytes.fromhex("002000010002")
    context = b"\x00" + extract(suite, b"psk_id_hash", b"") + extract(suite, b"info_hash", info)
    secret = extract(suite, b"secret", b"", shared)
    key = labeled_expand(suite, secret, b"key", context, 32)
    nonce = labeled_expand(suite, secret, b"base_nonce", context, 12)
    epoch_key = bytes(range(32))
    ct = AESGCM(key).encrypt(nonce, epoch_key, header)
    signature = owner.sign(lp(b"tmt-colab-wrap-signature-v1", b"1", header, enc, digest(ct)))
    wrap = dict(header=b64(header), enc=b64(enc), ciphertext=b64(ct), signature=b64(signature))
    payload = json.dumps(dict(memberId=device,role="editor",signKey=b64(pub),encKey=b64(pk_r),pages=[page]), separators=(",", ":")).encode()
    statement = lp(b"tmt-colab-membership-v1", b"1", space.encode(), b"1", bytes(32), b"member.add", digest(payload))
    sig = owner.sign(statement)
    statement_hash = digest(lp(b"tmt-colab-membership-hash-v1",statement,sig))
    cert = lp(b"tmt-colab-device-cert-v1",b"1",space.encode(),b"member",device.encode(),page.encode(),pub,pk_r,b"1",b"0",b"100")
    cert_sig = owner.sign(cert)
    chain = dict(version=1,issuerStatement=b64(statement_hash),deviceCertificate=b64(cert),issuerSignature=b64(cert_sig))
    link_seed = bytes(range(32))
    link_id = "00000000-0000-4000-8000-000000000060"
    def derive(label):
        return expand(mac(bytes(32),link_seed),lp(label,space.encode(),link_id.encode()),32)
    return dict(space=space,page=page,device=device,seed=seed.hex(),public=pub.hex(),recipientSeed=sk_r.hex(),epochKey=epoch_key.hex(),wrap=wrap,payload=payload.decode(),statement=dict(statement=b64(statement),payload=b64(payload),signature=b64(sig)),statementHash=statement_hash.hex(),chain=chain,chainDigest=digest(lp(b"tmt-colab-chain-v1",b"1",statement_hash,cert,cert_sig)).hex(),linkId=link_id,linkSeed=link_seed.hex(),linkSigningPublic=public(Ed25519PrivateKey.from_private_bytes(derive(b"tmt-colab-link-signing-seed-v1"))).hex(),linkEncryptionPublic=public(X25519PrivateKey.from_private_bytes(derive(b"tmt-colab-link-encryption-seed-v1"))).hex(),joinProof=mac(link_seed,lp(b"tmt-colab-join-v1",space.encode(),link_id.encode())).hex())
def owner_member_vectors(authority):
    owner = Ed25519PrivateKey.from_private_bytes(bytes.fromhex(authority["seed"]))
    cases = []
    for target, member, accepted in [("owner", authority["device"], False), ("peer", "00000000-0000-4000-8000-000000000051", True)]:
        for operation in ["member.remove", "member.role"]:
            value = dict(memberId=member, cuts=[])
            if operation == "member.role":
                value["role"] = "viewer"
            payload = json.dumps(value, separators=(",", ":")).encode()
            statement = lp(b"tmt-colab-membership-v1", b"1", authority["space"].encode(), b"2", bytes.fromhex(authority["statementHash"]), operation.encode(), digest(payload))
            cases.append(dict(name=target + "-" + operation, operation=operation, accepted=accepted, envelope=dict(statement=b64(statement), payload=b64(payload), signature=b64(owner.sign(statement)))))
    return dict(cases=cases)
if __name__ == "__main__":
    import argparse
    parser = argparse.ArgumentParser()
    parser.add_argument("--write",action="store_true")
    args = parser.parse_args()
    authority = generate()
    for destination, value in [(DEST, authority), (DEST.with_name("owner-member-v1.json"), owner_member_vectors(authority))]:
        frozen = json.dumps(value, indent=2) + "\n"
        if args.write:
            destination.write_text(frozen)
        elif destination.read_text() != frozen:
            raise SystemExit(f"{destination.name} differs; review before --write")

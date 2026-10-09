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
def seal_wrap(owner, header, sk_r, sk_e, epoch_key):
    r, e = X25519PrivateKey.from_private_bytes(sk_r), X25519PrivateKey.from_private_bytes(sk_e)
    enc, pk_r = public(e), public(r)
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
    ct = AESGCM(key).encrypt(nonce, epoch_key, header)
    signature = owner.sign(lp(b"tmt-colab-wrap-signature-v1", b"1", header, enc, digest(ct)))
    return dict(header=b64(header), enc=b64(enc), ciphertext=b64(ct), signature=b64(signature))
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
    epoch_key = bytes(range(32))
    wrap = seal_wrap(owner, header, sk_r, sk_e, epoch_key)
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
    # The browser reader's deterministic link device: re-opens reuse one server row.
    link_signer = Ed25519PrivateKey.from_private_bytes(derive(b"tmt-colab-link-signing-seed-v1"))
    link_device_key = Ed25519PrivateKey.from_private_bytes(derive(b"tmt-colab-link-device-seed-v1"))
    link_device_bytes = bytearray(derive(b"tmt-colab-link-device-id-v1")[:16])
    link_device_bytes[6] = (link_device_bytes[6] & 15) | 64
    link_device_bytes[8] = (link_device_bytes[8] & 63) | 128
    h = bytes(link_device_bytes).hex()
    link_device_id = f"{h[:8]}-{h[8:12]}-{h[12:16]}-{h[16:20]}-{h[20:]}"
    link_device_cert = lp(b"tmt-colab-device-cert-v1",b"1",space.encode(),b"link",link_id.encode(),link_device_id.encode(),public(link_device_key),public(X25519PrivateKey.from_private_bytes(derive(b"tmt-colab-link-encryption-seed-v1"))),b"1",b"0",b"9007199254740991")
    link_device_sig = link_signer.sign(link_device_cert)
    link_device = dict(id=link_device_id,signingPublic=public(link_device_key).hex(),chain=dict(version=1,issuerStatement=b64(statement_hash),deviceCertificate=b64(link_device_cert),issuerSignature=b64(link_device_sig)),chainDigest=digest(lp(b"tmt-colab-chain-v1",b"1",statement_hash,link_device_cert,link_device_sig)).hex())
    return dict(space=space,page=page,device=device,seed=seed.hex(),public=pub.hex(),recipientSeed=sk_r.hex(),epochKey=epoch_key.hex(),wrap=wrap,payload=payload.decode(),statement=dict(statement=b64(statement),payload=b64(payload),signature=b64(sig)),statementHash=statement_hash.hex(),chain=chain,chainDigest=digest(lp(b"tmt-colab-chain-v1",b"1",statement_hash,cert,cert_sig)).hex(),linkId=link_id,linkSeed=link_seed.hex(),linkSigningPublic=public(Ed25519PrivateKey.from_private_bytes(derive(b"tmt-colab-link-signing-seed-v1"))).hex(),linkEncryptionPublic=public(X25519PrivateKey.from_private_bytes(derive(b"tmt-colab-link-encryption-seed-v1"))).hex(),joinProof=mac(link_seed,lp(b"tmt-colab-join-v1",space.encode(),link_id.encode())).hex(),linkDevice=link_device)
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
def history_vectors(a):
    owner = Ed25519PrivateKey.from_private_bytes(bytes.fromhex(a["seed"]))
    def signed(operation, raw, signer=owner):
        message = lp(b"tmt-colab-membership-v1", b"1", a["space"].encode(), b"2", bytes.fromhex(a["statementHash"]), operation.encode(), digest(raw))
        return dict(statement=b64(message), payload=b64(raw), signature=b64(signer.sign(message)))
    cases = []
    for name, operation, value, accepted in [
        ("shared", "page.history", dict(pageId=a["page"],mode="shared"), True),
        ("current", "page.history", dict(pageId=a["page"],mode="current"), True),
        ("old-static", "page.scripts", dict(pageId=a["page"],mode="static"), False),
        ("old-interactive", "page.scripts", dict(pageId=a["page"],mode="interactive"), False),
        ("wrong-mode", "page.history", dict(pageId=a["page"],mode="static"), False),
        ("missing-mode", "page.history", dict(pageId=a["page"]), False),
        ("null-mode", "page.history", dict(pageId=a["page"],mode=None), False),
        ("number-mode", "page.history", dict(pageId=a["page"],mode=1), False),
        ("extra-field", "page.history", dict(pageId=a["page"],mode="shared",extra=True), False),
    ]:
        raw = json.dumps(value, separators=(",", ":")).encode()
        cases.append(dict(name=name,operation=operation,payload=raw.decode(),accepted=accepted,envelope=signed(operation,raw)))
    raw = ('{"pageId":"'+a["page"]+'","mode":"shared","mode":"current"}').encode()
    cases.append(dict(name="duplicate-mode",operation="page.history",payload=raw.decode(),accepted=False,envelope=signed("page.history",raw)))
    raw = cases[1]["payload"].encode()
    wrong_owner = signed("page.history",raw,Ed25519PrivateKey.from_private_bytes(bytes([7])*32))
    member = "00000000-0000-4000-8000-000000000051"
    recipient_seed = bytes([7])*32
    recipient = X25519PrivateKey.from_private_bytes(recipient_seed)
    pages = [f"00000000-0000-4000-8000-{n:012x}" for n in range(100,109)]
    pub = bytes.fromhex(a["public"])
    def forward(page, epoch):
        header = lp(b"tmt-colab-wrap-v1",b"1",b"base-x25519-hkdfsha256-aes256gcm",a["space"].encode(),page.encode(),str(epoch).encode(),b"member",member.encode(),public(recipient),pub,b"2",b"epoch-key")
        ephemeral = digest(b"1070-public-fixture-ephemeral"+header)
        return seal_wrap(owner,header,recipient_seed,ephemeral,bytes.fromhex(a["epochKey"]))
    # Current plus63 earlier epochs, nine pages:576 wraps in two atomic caller lists.
    wraps = [json.dumps(forward(page,epoch),separators=(",", ":")) for page in pages for epoch in range(1,65)]
    assert len(wraps)==576 and len(set(wraps))==576
    join_payload = json.dumps(dict(memberId=member,role="viewer",signKey=b64(public(Ed25519PrivateKey.from_private_bytes(bytes([7])*32))),encKey=b64(public(recipient)),pages=pages),separators=(",", ":")).encode()
    return dict(historyCases=cases,historyWrongOwner=wrong_owner,forwardWrap=forward(pages[0],63),historyJoin=dict(currentEpoch="64",membershipRevision="2",recipientSeed=recipient_seed.hex(),memberAdd=signed("member.add",join_payload),wrapLists=[wraps[:512],wraps[512:]]))

def sealed_history_vectors(a):
    # Independent signed owner log and encrypted receipt for the sealed-epoch rule.
    owner = Ed25519PrivateKey.from_private_bytes(bytes.fromhex(a["seed"]))
    secret = bytes.fromhex(a["epochKey"])
    member = management_vectors(a)
    management_seed = expand(mac(b"", bytes.fromhex(a["seed"])), lp(b"tmt-colab-management-signing-seed-v1", a["space"].encode()), 32)
    manager = Ed25519PrivateKey.from_private_bytes(management_seed)
    initial_payload = json.dumps(dict(memberId=member["memberId"],role="editor",signKey=b64(bytes.fromhex(member["signingPublic"])),encKey=b64(bytes.fromhex(member["encryptionPublic"])),pages=[]),separators=(",", ":")).encode()
    initial_input = lp(b"tmt-colab-membership-v1",b"1",a["space"].encode(),b"1",bytes(32),b"member.add",digest(initial_payload))
    initial_signature = owner.sign(initial_input)
    initial = dict(statement=b64(initial_input),payload=b64(initial_payload),signature=b64(initial_signature))
    initial_hash = digest(lp(b"tmt-colab-membership-hash-v1",initial_input,initial_signature))
    cert = lp(b"tmt-colab-device-cert-v1",b"1",a["space"].encode(),b"member",member["memberId"].encode(),a["page"].encode(),bytes.fromhex(a["public"]),bytes.fromhex(member["encryptionPublic"]),b"1",b"0",b"100")
    chain = dict(version=1,issuerStatement=b64(initial_hash),deviceCertificate=b64(cert),issuerSignature=b64(manager.sign(cert)))
    def receipt(seq, kind="update"):
        header = lp(b"tmt-colab-object-v1", b"1", b"aes256gcm-hkdfsha256-ed25519-v1", a["space"].encode(), a["page"].encode(), b"1", kind.encode(), b"own", b"01"*32, a["page"].encode(), b"1", str(seq).encode(), bytes(32))
        key = expand(mac(b"", secret), lp(b"tmt-colab-object-key-v1", header), 32)
        nonce = bytes(12)
        ciphertext = AESGCM(key).encrypt(nonce, b"sealed history", header)
        signature = owner.sign(lp(b"tmt-colab-signature-v1", header, nonce, digest(ciphertext)))
        value = dict(header=b64(header),nonce=b64(nonce),ciphertext=b64(ciphertext),signature=b64(signature))
        return value, digest(lp(b"tmt-colab-envelope-hash-v1",header,nonce,ciphertext,signature))
    cases = []
    for name, sealed, seq, admitted, bad_hash, kind, bad_checkpoint in [
        ("sealed",True,1,True,False,"update",False),
        ("unsealed-uncut",False,1,False,False,"update",False),
        ("beyond-seal",True,2,False,False,"update",False),
        ("wrong-seal-tail-hash",True,1,False,True,"update",False),
        ("sealed-checkpoint",True,1,True,False,"checkpoint",False),
        ("wrong-seal-checkpoint-hash",True,1,False,False,"checkpoint",True),
    ]:
        _, bound_hash = receipt(1, kind)
        # A checkpoint shares the tail sequence, not the tail envelope hash.
        _, tail_hash = receipt(1)
        log = [initial]
        previous = initial_hash
        def append(operation, value):
            nonlocal previous
            raw = json.dumps(value,separators=(",", ":")).encode()
            framed = lp(b"tmt-colab-membership-v1", b"1", a["space"].encode(),str(len(log)+1).encode(),previous,operation.encode(),digest(raw))
            sig = owner.sign(framed)
            log.append(dict(statement=b64(framed),payload=b64(raw),signature=b64(sig)))
            previous = digest(lp(b"tmt-colab-membership-hash-v1",framed,sig))
        if sealed:
            cut = lp(b"tmt-colab-stream-cut-v1",b"1",a["page"].encode(),b"own",(bytes([9])*32 if bad_checkpoint else bound_hash) if kind == "checkpoint" else b"",b"1" if kind == "checkpoint" else b"0",b"1",bytes([9])*32 if bad_hash else tail_hash)
            append("epoch.advance",dict(pageId=a["page"],epoch="2",cuts=[dict(pageId=a["page"],epoch="1",namespace="own",cut=b64(cut))],baseline=dict(pageId=a["page"],epoch="2",sourceDigest=b64(bytes(32)),baselineCommitment=b64(bytes(32)),title="",objectEnvelopeHash=b64(bytes(32)),membershipRevision="2"),wraps=[]))
        append("device.revoke",dict(deviceId=a["page"],cuts=[]))
        value, value_hash = receipt(seq, kind)
        cases.append(dict(name=name,admitted=admitted,log=log,chain=chain,kind=kind,receipt=value,envelopeHash=b64(value_hash),seq=str(seq),plaintext=b64(b"sealed history")))
    return cases

def management_vectors(a):
    import uuid
    seed = bytes.fromhex(a["seed"])
    def derive(label):
        return expand(mac(b"", seed), lp(label.encode(), a["space"].encode()), 32)
    sign = derive("tmt-colab-management-signing-seed-v1")
    enc = derive("tmt-colab-management-encryption-seed-v1")
    identity = bytearray(derive("tmt-colab-management-member-id-v1")[:16])
    identity[6] = (identity[6] & 15) | 64
    identity[8] = (identity[8] & 63) | 128
    device = Ed25519PrivateKey.from_private_bytes(bytes([9])*32)
    key = public(Ed25519PrivateKey.from_private_bytes(bytes([10])*32))
    message = lp(b"tmt-ext-cert-v1", b"colab", b"sign", key, b"1790000000000")
    return dict(ownerSeed=a["seed"],space=a["space"],memberId=str(uuid.UUID(bytes=bytes(identity))),
        signingPublic=public(Ed25519PrivateKey.from_private_bytes(sign)).hex(),
        encryptionPublic=public(X25519PrivateKey.from_private_bytes(enc)).hex(),
        remotePublic=public(device).hex(),extensionPublic=key.hex(),issuedAtMs=1790000000000,
        extCertInput=message.hex(),extCertSignature=device.sign(message).hex())

if __name__ == "__main__":
    import argparse
    parser = argparse.ArgumentParser()
    parser.add_argument("--write",action="store_true")
    args = parser.parse_args()
    authority = generate()
    authority.update(history_vectors(authority))
    authority["sealedHistoryCases"] = sealed_history_vectors(authority)
    for destination, value in [(DEST, authority), (DEST.with_name("owner-member-v1.json"), owner_member_vectors(authority)), (DEST.with_name("management-key-v1.json"), management_vectors(authority))]:
        frozen = json.dumps(value, indent=2) + "\n"
        if args.write:
            destination.write_text(frozen)
        elif destination.read_text() != frozen:
            raise SystemExit(f"{destination.name} differs; review before --write")

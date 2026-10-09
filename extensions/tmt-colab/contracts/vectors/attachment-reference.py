"""Independent attachment known answers; public RFC8032 fixtures, no product imports."""
import argparse
import base64
import copy
import hashlib
import hmac
import json
import struct
from pathlib import Path
from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives.ciphers.aead import AESGCM

DEST = Path(__file__).with_name('attachment-v1.json')

def lp(*fields):
    return b''.join(struct.pack('>I', len(v)) + v for v in fields)
def sha(value):
    return hashlib.sha256(value).digest()
def b64(value):
    return base64.urlsafe_b64encode(value).decode().rstrip('=')
def wire(value):
    return json.dumps(value, ensure_ascii=False, separators=(',', ':'))
def ident(n):
    return f'00000000-0000-4000-8000-{n:012x}'
def descriptor_input(v):
    source = v['source']
    fields = [source['kind'], source['sourceDigest']] if source['kind'] == 'document' else [source['kind'], source['writerId'], source['messageId'], source['messageRevision']]
    values = [b'tmt-colab-attachment-v1', b'1']
    for key, value in list(v.items())[1:]:
        values.append(lp(*(x.encode() for x in fields)) if key == 'source' else value.encode())
    return lp(*values)
def manifest_input(v):
    entries = [descriptor_input(x) for x in v['attachments']]
    return lp(b'tmt-colab-attachment-manifest-v1', b'1', *(v[k].encode() for k in ['space', 'page', 'snapshotId', 'authorDevice', 'membershipRevision', 'sourceDigest']), struct.pack('>I', len(entries)) + lp(*entries))

def generate():
    seed = bytes.fromhex('9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60')
    signer = Ed25519PrivateKey.from_private_bytes(seed)
    public = signer.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw)
    secret = bytes(range(32))
    space, page, author = '4kph3kmtxo7dinlvoixpw642ozfibd2w', ident(2), ident(18)
    plain = b'\x00\xffattachment\n' + 'cat \U0001f408'.encode()
    source_digest = sha(b'<p>original source</p>').hex()
    def asset(namespace, object_id):
        header = lp(b'tmt-colab-object-v1', b'1', b'aes256gcm-hkdfsha256-ed25519-v1', space.encode(), page.encode(), b'1', b'asset', namespace.encode(), object_id.encode(), author.encode(), b'2', b'0', bytes(32))
        prk = hmac.new(bytes(32), secret, hashlib.sha256).digest()
        key = hmac.new(prk, lp(b'tmt-colab-object-key-v1', header) + b'\x01', hashlib.sha256).digest()
        cipher = AESGCM(key).encrypt(bytes(12), plain, header)
        signature = signer.sign(lp(b'tmt-colab-signature-v1', header, bytes(12), sha(cipher)))
        body = dict(header=b64(header), nonce=b64(bytes(12)), ciphertext=b64(cipher), signature=b64(signature))
        payload = wire(body).encode()
        d = dict(version=1, attachmentId=ident(65 if namespace == 'content' else 66), space=space, page=page, epoch='1', namespace=namespace, objectId=object_id, authorDevice=author, membershipRevision='2', source=dict(kind='document', sourceDigest=source_digest) if namespace == 'content' else dict(kind='message', writerId=author, messageId=ident(70), messageRevision='1'), envelopeHash=sha(lp(b'tmt-colab-envelope-hash-v1', header, bytes(12), cipher, signature)).hex(), signature=b64(signature), payloadSha256=sha(payload).hex(), payloadBytes=str(len(payload)), plaintextBytes=str(len(plain)), filename='cat \U0001f408.bin', mediaType='application/octet-stream')
        return d, payload
    d, raw = asset('content', '01' * 32)
    message, own_raw = asset('own', '02' * 32)
    manifest = dict(version=1, space=space, page=page, snapshotId=ident(80), authorDevice=author, membershipRevision='2', sourceDigest=source_digest, attachments=[d, message])
    cases = []
    def case(name, op, value, admit, **extra):
        if op == 'manifest' and admit:
            extra.update(inputBytes=b64(manifest_input(value)), hash=sha(manifest_input(value)).hex())
        cases.append(dict(name=name, operation=op, input=wire(value), admit=admit, **extra))
    for name, value, payload in [('document', d, raw), ('message', message, own_raw)]:
        case(name, 'descriptor', value, True, canonical=wire(value), inputBytes=b64(descriptor_input(value)), hash=sha(descriptor_input(value)).hex())
        case(name+'-asset', 'open', value, True, payload=b64(payload))
    case('snapshot-manifest', 'manifest', manifest, True, inputBytes=b64(manifest_input(manifest)), hash=sha(manifest_input(manifest)).hex())
    changes = dict(version=2, attachmentId=ident(65).upper().replace('4000', '3000'), space='a'*31, page=ident(2).replace('4000','3000'), epoch='01', namespace='other', objectId='AF'*32, authorDevice=ident(18).replace('4000','3000'), membershipRevision='0', source=dict(kind='document', sourceDigest='ab'*31), envelopeHash='ab'*31, signature='AA', payloadSha256='ab'*31, payloadBytes='12582913', plaintextBytes='8388609', filename='\U0001f408'*64, mediaType='image/PNG', unknown='extra')
    for key, replacement in changes.items():
        value = copy.deepcopy(d); value[key] = replacement
        case('invalid-'+key, 'descriptor', value, False)
    for name, replacement in [('unknown-source', dict(kind='document', sourceDigest=source_digest, unknown='x')), ('source-namespace', message['source']), ('message-zero-revision', dict(kind='message', writerId=author, messageId=ident(70), messageRevision='0'))]:
        value = copy.deepcopy(d); value['source'] = replacement
        if name == 'message-zero-revision': value['namespace']='own'
        case(name, 'descriptor', value, False)
    for name, raw_input in [('duplicate-field', wire(d).replace('"version":1','"version":1,"version":1')), ('duplicate-source', wire(d).replace('"kind":"document"','"kind":"document","kind":"document"')), ('float-version', wire(d).replace('"version":1','"version":1.0')), ('oversized-wire', ' '*2048+wire(d))]:
        cases.append(dict(name=name, operation='descriptor', input=raw_input, admit=False))
    formatted=copy.deepcopy(d); formatted['payloadBytes']=str(len(raw)+1)
    case('raw-layout-changed-same-envelope', 'open', formatted, False, payload=b64(b' '+raw))
    for key, replacement in [('payloadSha256', d['envelopeHash']), ('envelopeHash', d['payloadSha256']), ('payloadBytes', str(len(raw)+1)), ('plaintextBytes', str(len(plain)+1))]:
        value=copy.deepcopy(d); value[key]=replacement
        case('open-'+key, 'open', value, False, payload=b64(raw))
    for key, replacement in [('space','a'*32),('page',ident(3)),('epoch','2'),('authorDevice',ident(19)),('membershipRevision','3')]:
        case('admitted-'+key, 'open', d, False, payload=b64(raw), context={key:replacement})
    for name, extra in [('wrong-secret',dict(secret=b64(bytes([9])*32))),('wrong-key',dict(publicKey=b64(bytes([9])*32)))]:
        case(name,'open',d,False,payload=b64(raw),**extra)
    forged=json.loads(raw); forged['signature']=b64(bytes(64)); forged_raw=wire(forged).encode()
    value=copy.deepcopy(d); value['signature']=forged['signature']; value['payloadSha256']=sha(forged_raw).hex(); value['payloadBytes']=str(len(forged_raw)); value['envelopeHash']=sha(lp(b'tmt-colab-envelope-hash-v1',base64.urlsafe_b64decode(forged['header']+'=='),bytes(12),base64.urlsafe_b64decode(forged['ciphertext']+'=='),bytes(64))).hex()
    case('forged-signature-with-consistent-digests','open',value,False,payload=b64(forged_raw))
    for key, replacement in [('page',ident(3)),('sourceDigest','ab'*31),('attachments',[d,d]),('unknown','extra')]:
        value=copy.deepcopy(manifest); value[key]=replacement
        case('manifest-'+key,'manifest',value,False)
    def descriptors(count):
        result=[]
        for i in range(count):
            value=copy.deepcopy(d); value['attachmentId']=ident(1000+i); result.append(value)
        return result
    for count in [128,129]:
        value=copy.deepcopy(manifest); value['attachments']=descriptors(count)
        case('manifest-count-'+str(count),'manifest',value,count==128)
    for name, attachments, admit in [('document', [d], True),('duplicate',[d,d],False),('limit',descriptors(128),True),('overflow',descriptors(129),False),('null',None,False)]:
        case('projection-'+name,'document',dict(html='<p>original source</p>',meta=dict(title='Storage',attachments=attachments)),admit)
    comment=dict(version=1,kind='comment',spaceId=space,pageId=page,epoch='1',senderDevice=author,revision='1',deleted=False,deviceName='Fixture',at='0',messageId=ident(70),thread=dict(writer=author,id=author),body='',attachments=[message])
    for name, attachments, deleted, admit in [('Chat',[message],False,True),('annotation',[d],False,True),('null',None,False,False),('duplicate',[message,message],False,False),('limit',descriptors(16),False,True),('overflow',descriptors(17),False,False),('deleted-attachment',[message],True,False),('deleted-empty',[],True,True)]:
        value=copy.deepcopy(comment); value['attachments']=attachments; value['deleted']=deleted
        if name=='annotation': value['thread']['id']=ident(90)
        case('comment-'+name,'comment',value,admit)
    foreign=copy.deepcopy(comment); foreign['pageId']=ident(3); case('comment-cross-page','comment',foreign,False)
    def publication(value):
        return dict(version=1,kind='attachment-publication',spaceId=value['space'],pageId=value['page'],epoch=value['epoch'],senderDevice=value['authorDevice'],membershipRevision=value['membershipRevision'],attachmentId=value['attachmentId'],descriptorHash=sha(descriptor_input(value)).hex(),source=copy.deepcopy(value['source']),baseRevision='v1:'+sha(b'fixture captured base').hex())
    pub = publication(d)
    for name, value in [('document',d),('message',message)]:
        record=publication(value)
        case('publication-'+name,'publication',record,True,canonical=wire(record))
        case('publication-binding-'+name,'publication-binding',record,True,descriptor=wire(value))
    for key,replacement in [('version',2),('kind','attachment-claim'),('spaceId','a'*31),('pageId',ident(2).replace('4000','3000')),('epoch','01'),('membershipRevision','0'),('descriptorHash','ab'*31),('baseRevision','v1:'+'AF'*32),('unknown','x')]:
        value=copy.deepcopy(pub); value[key]=replacement
        case('invalid-publication-'+key,'publication',value,False)
    for key,replacement in [('senderDevice',ident(19)),('descriptorHash','ab'*32),('epoch','2'),('membershipRevision','3'),('attachmentId',ident(99)),('source',dict(kind='document',sourceDigest='ab'*32))]:
        value=copy.deepcopy(pub); value[key]=replacement
        case('publication-binding-'+key,'publication-binding',value,False,descriptor=wire(d))
    current=dict(kind='document-current',attachmentId=d['attachmentId'],descriptorHash=pub['descriptorHash'],contentRevision=pub['baseRevision'])
    msg=dict(kind='message',writerId=author,messageId=ident(70),messageRevision='1',attachmentId=message['attachmentId'],descriptorHash=publication(message)['descriptorHash'])
    for name,value in [('document',current),('message',msg)]:
        case('selector-'+name,'selector',value,True,canonical=wire(value))
    for key,replacement in [('kind','snapshot'),('attachmentId',ident(65).replace('4000','3000')),('descriptorHash','ab'*31),('contentRevision','v1:'+'ab'*31),('unknown','x')]:
        value=copy.deepcopy(current); value[key]=replacement
        case('invalid-selector-'+key,'selector',value,False)
    value=copy.deepcopy(msg);value['messageRevision']='0';case('invalid-selector-message-revision','selector',value,False)
    for name,operation,value in [('publication','publication',pub),('selector','selector',current)]:
        raw_input=wire(value)
        field='version' if name=='publication' else 'kind'
        encoded=wire(value[field])
        raw_input=raw_input.replace('"'+field+'":'+encoded,'"'+field+'":'+encoded+',"'+field+'":'+encoded)
        cases.append(dict(name=name+'-duplicate',operation=operation,input=raw_input,admit=False))
    # Page-token parity uses fixed independently signed stream entries. It is a
    # byte/position vector, not a membership or source-content authority fixture.
    revisions = []
    for name, specs in [('empty', []), ('own-only', [('update','own','1',bytes(32))]), ('paired-checkpoint-tail', [('checkpoint','content','2',bytes([3])*32),('checkpoint','own','2',bytes([3])*32),('update','own','3',bytes([3])*32)])]:
        entries = []
        positions, checkpoints = {}, {}
        for kind, ns, seq, prev in specs:
            header = lp(b'tmt-colab-object-v1',b'1',b'aes256gcm-hkdfsha256-ed25519-v1',space.encode(),page.encode(),b'1',kind.encode(),ns.encode(),('05'*32).encode(),author.encode(),b'2',seq.encode(),prev)
            key = hmac.new(hmac.new(bytes(32),secret,hashlib.sha256).digest(),lp(b'tmt-colab-object-key-v1',header)+b'\x01',hashlib.sha256).digest()
            cipher = AESGCM(key).encrypt(bytes(12),b'position fixture',header)
            signature = signer.sign(lp(b'tmt-colab-signature-v1',header,bytes(12),sha(cipher)))
            envelope = dict(header=b64(header),nonce=b64(bytes(12)),ciphertext=b64(cipher),signature=b64(signature))
            digest = sha(lp(b'tmt-colab-envelope-hash-v1',header,bytes(12),cipher,signature))
            entries.append(dict(kind=kind,namespace=ns,seq=seq,envelopeHash=b64(digest),envelope=b64(wire(envelope).encode())))
            positions[ns] = (seq,prev if kind=='checkpoint' else digest)
            if kind=='checkpoint': checkpoints[ns] = (seq,digest)
        cuts=[]
        if specs:
            for ns in ['content','own']:
                cp,cp_hash = checkpoints.get(ns,('0',b''))
                tail,tail_hash = positions.get(ns,('0',bytes(32)))
                cut=lp(b'tmt-colab-stream-cut-v1',b'1',author.encode(),ns.encode(),cp_hash,cp.encode(),tail.encode(),tail_hash)
                cuts.append(dict(pageId=page,epoch='1',namespace=ns,cut=b64(cut)))
        head_hash=bytes([7])*32
        token='v1:'+sha(lp(b'tmt-colab-page-revision-v1',space.encode(),page.encode(),b'7',head_hash,b'1',wire(cuts).encode())).hex()
        revisions.append(dict(name=name,space=space,page=page,epoch='1',writer=author,revision='7',headHash=b64(head_hash),entries=entries,cuts=cuts,token=token))
    # Runtime policy answers remain Colab-owned, opaque to Remote/the wire leaf.
    # No labels, creator or full descriptor enter the persisted Remote binding.
    channel_policies=[]
    for name,desc in [('document',d),('message',message)]:
        upload=dict(version=1,space=desc['space'],page=desc['page'],epoch=desc['epoch'],target=desc['source'],attachmentId=desc['attachmentId'],descriptorHash=sha(descriptor_input(desc)).hex(),objectId=desc['objectId'],envelopeHash=desc['envelopeHash'],payloadSha256=desc['payloadSha256'],payloadBytes=desc['payloadBytes'],plaintextBytes=desc['plaintextBytes'])
        reference=dict(kind='document-current',attachmentId=desc['attachmentId'],descriptorHash=upload['descriptorHash'],contentRevision=revisions[0]['token']) if name=='document' else dict(kind='message',writerId=desc['source']['writerId'],messageId=desc['source']['messageId'],messageRevision=desc['source']['messageRevision'],attachmentId=desc['attachmentId'],descriptorHash=upload['descriptorHash'])
        read=dict(version=1,space=desc['space'],page=desc['page'],epoch='2',reference=reference)
        channel_policies.append(dict(name=name,upload=wire(upload),reference=reference,peerEpoch='2',read=wire(read)))
    return dict(version=1, secret=b64(secret),publicKey=b64(public),plaintext=b64(plain),namespace=sha(lp(b'tmt-colab-attachment-namespace-v1',space.encode(),page.encode())).hex(),referenceRevisions=revisions,channelPolicies=channel_policies,cases=cases)

if __name__ == '__main__':
    parser=argparse.ArgumentParser(); parser.add_argument('--write',action='store_true'); args=parser.parse_args()
    frozen=json.dumps(generate(),ensure_ascii=False,indent=2)+'\n'
    if args.write: DEST.write_text(frozen)
    elif DEST.read_text()!=frozen: raise SystemExit('attachment vectors differ; review before --write')

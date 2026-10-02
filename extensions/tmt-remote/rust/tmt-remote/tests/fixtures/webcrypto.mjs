// Local Node22/24 conformance, not the separate real Chrome MV3 security gate.
// Uses RFC8032 TEST1's public fixture seed only, never a product/private user key.
import { readFile, writeFile } from 'node:fs/promises';
import assert from 'node:assert/strict';
import { webcrypto } from 'node:crypto';
const { subtle } = webcrypto;
const read = async (path) => JSON.parse(await readFile(new URL(path, import.meta.url), 'utf8'));
const fromHex = (s) => Buffer.from(s, 'hex');
const hex = (bytes) => Buffer.from(bytes).toString('hex');
const lp = (bytes) => {
  const n = Buffer.alloc(4);
  n.writeUInt32BE(bytes.length);
  return Buffer.concat([n, bytes]);
};
const utf8 = (s) => Buffer.from(s, 'utf8');
const seed = '9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60';
const publicBytes = fromHex('d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a');
const privateKey = await subtle.importKey(
  'pkcs8',
  fromHex('302e020100300506032b657004220420' + seed),
  { name: 'Ed25519' },
  false,
  ['sign']
);
const publicKey = await subtle.importKey('raw', publicBytes, { name: 'Ed25519' }, true, ['verify']);
await assert.rejects(subtle.exportKey('pkcs8', privateKey));
assert.equal(
  hex(await subtle.sign('Ed25519', privateKey, new Uint8Array())),
  'e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b'
);
const peer = await read('../../../../typescript/remote-client/test/vectors.json');
const cases = [];
for (const fixture of [...peer.envelopes, ...peer.enrollments, peer.possession]) {
  const message = fromHex(fixture.hex);
  const signature = await subtle.sign('Ed25519', privateKey, message);
  assert(await subtle.verify('Ed25519', publicKey, signature, message));
  cases.push({ name: fixture.name, message: fixture.hex, signature: hex(signature) });
}
const result = {
  provenance:
    'Node native WebCrypto Ed25519; RFC8032 TEST1 seed; independent Python canonical fixture bytes. Rust verifies and reproduces every deterministic signature.',
  cases,
};
const path = new URL('./webcrypto-vectors.json', import.meta.url);
if (process.argv.includes('--write')) await writeFile(path, JSON.stringify(result, null, 2) + '\n');
else {
  const saved = await read('./webcrypto-vectors.json');
  assert.deepEqual(result, saved);
  // Rust tests independently reproduce these signatures; verify the committed native-matched bytes.
  for (const c of saved.cases)
    assert(await subtle.verify('Ed25519', publicKey, fromHex(c.signature), fromHex(c.message)));
}
const v = await read('./mac-vectors.json');
const mac = async (key, message) => {
  const k = await subtle.importKey('raw', key, { name: 'HMAC', hash: 'SHA-256' }, false, ['sign']);
  return Buffer.from(await subtle.sign('HMAC', k, message));
};
const code = fromHex(v.code),
  enrollment = fromHex(v.enrollment),
  receipt = fromHex(v.receipt);
assert.equal(hex(await mac(code, enrollment)), v.enrollmentMac);
const responseKey = await mac(
  code,
  Buffer.concat([lp(utf8('tmt-device-pair-response-key-v1')), lp(enrollment)])
);
assert.equal(hex(responseKey), v.responseKey);
assert.equal(
  hex(await mac(responseKey, Buffer.concat([lp(utf8('tmt-device-pair-response-v1')), lp(receipt)]))),
  v.serverProof
);
console.log(
  `${process.version}: ${cases.length} Ed25519 signatures and enrollment/serverProof HMACs match; browser security remains a separate gate`
);

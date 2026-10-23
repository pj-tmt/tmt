// Independent application framing/signatures only. HPKE is NOT implemented here:
// RFC 9180 A.3.1 separately tests the Rust suite. Every private key below is public test data.
import assert from 'node:assert/strict';
import { webcrypto } from 'node:crypto';
import { readFile, writeFile } from 'node:fs/promises';
const { subtle } = webcrypto;
const bytes = (s) => Buffer.from(s, 'utf8');
const hex = (s) => Buffer.from(s, 'hex');
const b64 = (s) => Buffer.from(s).toString('base64url');
const lp = (s) => {
  const n = Buffer.alloc(4);
  n.writeUInt32BE(s.length);
  return Buffer.concat([n, s]);
};
const frame = (...parts) => Buffer.concat(parts.map(lp));
const sha = async (s) => Buffer.from(await subtle.digest('SHA-256', s));
const auth = async (n) => {
  const seed = Buffer.alloc(32, n);
  const key = await subtle.importKey(
    'pkcs8',
    Buffer.concat([hex('302e020100300506032b657004220420'), seed]),
    'Ed25519',
    true,
    ['sign']
  );
  const jwk = await subtle.exportKey('jwk', key);
  return { key, publicKey: jwk.x };
};
const owner = await auth(7),
  sender = await auth(8);
const sign = async (key, input) => Buffer.from(await subtle.sign('Ed25519', key, input));
const machineId = '11111111-1111-4111-8111-111111111111',
  keyId = '22222222-2222-4222-8222-222222222222';
const clientId = '33333333-3333-4333-8333-333333333333',
  id = '44444444-4444-4444-8444-444444444444';
const sessionId = '55555555-5555-4555-8555-555555555555',
  windowId = '66666666-6666-4666-8666-666666666666';
const now = 1800000000000;
const point = hex(
  '04fe8c19ce0905191ebc298a9245792531f26f0cece2460639e8bc39cb7f706a826a779b4cf969b8a0e539c7f62fb3d30ad6aa8f80e30f1d128aafd68a2ce72ea0'
);
const descriptor = {
  version: 1,
  profile: 'cloud-hpke-v1',
  machineId,
  keyId,
  revision: 1,
  publicKey: b64(point),
  notBeforeMs: now,
  expiresAtMs: now + 30 * 86400000,
};
const descriptorInput = frame(
  bytes('tmt-cloud-key-v1'),
  bytes('1'),
  bytes('cloud-hpke-v1'),
  bytes(machineId),
  bytes(keyId),
  bytes('1'),
  point,
  bytes(String(descriptor.notBeforeMs)),
  bytes(String(descriptor.expiresAtMs))
);
descriptor.signature = b64(await sign(owner.key, descriptorInput));
const header = (response) => ({
  version: 1,
  profile: 'local-v1',
  kind: response ? 'response' : 'request',
  id: response ? keyId : id,
  correlationId: response ? id : null,
  machineId,
  windowId,
  clientId,
  sessionId,
  sequence: '1',
  timestampMs: now,
  origin: 'cli',
  operation: 'agents.list',
});
const headerInput = (h) =>
  frame(
    ...[
      'tmt-message-v1',
      String(h.version),
      h.profile,
      h.kind,
      h.id,
      h.correlationId ?? '',
      h.machineId,
      h.windowId,
      h.clientId,
      h.sessionId,
      h.sequence,
      String(h.timestampMs),
      h.origin,
      h.operation,
    ].map(bytes)
  );
const payload = bytes('{"message": "exact whitespace"}');
const envelope = async (response) => {
  const h = header(response);
  const input = Buffer.concat([headerInput(h), lp(await sha(payload))]);
  return JSON.stringify({
    ...h,
    payload: b64(payload),
    signature: b64(await sign(response ? owner.key : sender.key, input)),
  });
};
const request = await envelope(false),
  response = await envelope(true);
const binding = (h) =>
  frame(
    bytes('tmt-cloud-carrier-v1'),
    bytes('1'),
    bytes('cloud-hpke-v1'),
    bytes(keyId),
    headerInput(h)
  );
// Public RFC enc/ciphertext stand-ins test app bindings only, not an HPKE round trip.
const enc = hex(
  '04a92719c6195d5085104f469a8b9814d5838ff72b60501e2c4466e5e67b325ac98536d7b61a1af4b78e5b7f951c0900be863c403ce65c9bfcb9382657222d18c4'
);
const ct = hex(
  '5ad590bb8baa577f8619db35a36311226a896e7342a6d836d8b7bcd2f20b6c7f9076ac232e3ab2523f39513434'
);
const requestAad = Buffer.concat([binding(header(false)), lp(enc)]);
const sigInput = async (aad, ciphertext) =>
  Buffer.concat([aad, lp(bytes(String(ciphertext.length))), lp(await sha(ciphertext))]);
const requestSignatureInput = await sigInput(requestAad, ct);
const requestDigest = await sha(requestSignatureInput);
const carrier = {
  version: 1,
  profile: 'cloud-hpke-v1',
  keyId,
  header: header(false),
  enc: b64(enc),
  ciphertext: b64(ct),
  signature: b64(await sign(sender.key, requestSignatureInput)),
};
const responseAad = Buffer.concat([binding(header(true)), lp(enc), lp(requestDigest)]);
const exporterContext = frame(bytes('tmt-cloud-response-v1'), requestDigest);
// Fixed public key/nonce values stand in for the HPKE Export outputs. No HPKE KDF here.
const responseKey = hex('868c066ef58aae6dc589b6cfdd18f97e'),
  responseNonce = hex('4e0bc5018beba4bf004cca59');
const aes = await subtle.importKey('raw', responseKey, 'AES-GCM', false, ['encrypt']);
const responseCiphertext = Buffer.from(
  await subtle.encrypt(
    { name: 'AES-GCM', iv: responseNonce, additionalData: responseAad, tagLength: 128 },
    aes,
    bytes(response)
  )
);
const responseSignatureInput = await sigInput(responseAad, responseCiphertext);
const responseCarrier = {
  version: 1,
  profile: 'cloud-hpke-v1',
  keyId,
  header: header(true),
  enc: b64(enc),
  ciphertext: b64(responseCiphertext),
  signature: b64(await sign(owner.key, responseSignatureInput)),
  requestDigest: b64(requestDigest),
};
const descriptorBytes = JSON.stringify(descriptor);
const result = {
  provenance:
    'Frozen pre-change v1 bytes; public test keys. Node WebCrypto application LP/signatures/AAD/carrier/response framing only, no HPKE implementation or release provenance. P-256 material from RFC 9180 A.3.1.',
  ownerPublic: owner.publicKey,
  senderPublic: sender.publicKey,
  descriptor: descriptorBytes,
  keyring: JSON.stringify({
    version: 1,
    machineId,
    revision: 1,
    keys: [
      {
        descriptor: descriptorBytes,
        ikm: b64(hex('668b37171f1072f3cf12ea8a236a45df23fc13b82af3609ad1e354f6ef817550')),
      },
    ],
  }),
  descriptorInput: descriptorInput.toString('hex'),
  request,
  response,
  headerInput: headerInput(header(false)).toString('hex'),
  binding: binding(header(false)).toString('hex'),
  requestAad: requestAad.toString('hex'),
  requestSignatureInput: requestSignatureInput.toString('hex'),
  carrier: JSON.stringify(carrier),
  responseAad: responseAad.toString('hex'),
  exporterKeyContext: Buffer.concat([exporterContext, lp(bytes('key'))]).toString('hex'),
  exporterNonceContext: Buffer.concat([exporterContext, lp(bytes('nonce'))]).toString('hex'),
  responseKey: responseKey.toString('hex'),
  responseNonce: responseNonce.toString('hex'),
  responseSignatureInput: responseSignatureInput.toString('hex'),
  responseCarrier: JSON.stringify(responseCarrier),
};
const destination = new URL('./cloud-hpke-v1.json', import.meta.url);
if (process.argv.includes('--write'))
  await writeFile(destination, JSON.stringify(result, null, 2) + '\n');
else assert.deepEqual(result, JSON.parse(await readFile(destination, 'utf8')));
console.log(
  'cloud-hpke-v1 application bytes match the independent WebCrypto reference (not HPKE).'
);

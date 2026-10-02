import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { test } from 'vitest';
import { createHmac } from 'node:crypto';
import { readFileSync } from 'node:fs';
import {
  envelopeSigningBytes,
  enrollmentSigningBytes,
  enrollmentPossessionSigningBytes,
  base64url,
  base64urlBytes,
  fingerprintIndexes,
  pairingCode,
  responseKeyInput,
  serverProofInput,
} from '../src/canonical-bytes.js';
import macVectors from '../../../rust/tmt-remote/tests/fixtures/mac-vectors.json' with { type: 'json' };
import type { Envelope, Enrollment } from '../src/canonical-bytes.js';
import vectors from './vectors.json' with { type: 'json' };

const bytes = (hex: string): Uint8Array => Uint8Array.from(Buffer.from(hex, 'hex'));
function request(index = 0): Envelope {
  const input = vectors.envelopes[index]!.input;
  return { ...input, payload: bytes(input.payload) } as Envelope;
}
function candidate(index = 0): Enrollment {
  const input = vectors.enrollments[index]!.input;
  return {
    ...input,
    serverChallenge: bytes(input.serverChallenge),
    clientNonce: bytes(input.clientNonce),
    publicKey: bytes(input.publicKey),
  } as Enrollment;
}
function matches(raw: Uint8Array, expected: { hex: string; sha256: string }): void {
  assert.equal(Buffer.from(raw).toString('hex'), expected.hex);
  assert.equal(createHash('sha256').update(raw).digest('hex'), expected.sha256);
}
for (const [index, vector] of vectors.envelopes.entries()) {
  test(`independent envelope bytes and SHA-256: ${vector.name}`, async () => {
    matches(await envelopeSigningBytes(request(index)), vector);
  });
}
for (const [index, vector] of vectors.enrollments.entries()) {
  test(`independent enrollment bytes and SHA-256: ${vector.name}`, () => {
    matches(enrollmentSigningBytes(candidate(index)), vector);
  });
}
test('independent possession domain, LP order and raw MAC', () => {
  const { input } = vectors.possession;
  matches(
    enrollmentPossessionSigningBytes(bytes(input.enrollment), bytes(input.mac)),
    vectors.possession,
  );
});
test('payload whitespace, supplied bytes, audience and operation stay bound', async () => {
  const baseline = await envelopeSigningBytes(request());
  for (const change of [
    { payload: new TextEncoder().encode('{"text":"hello"}') },
    { payload: bytes('ff0001') },
    { machineId: '00000000-0000-4000-8000-000000000099' },
    { windowId: '00000000-0000-4000-8000-000000000099' },
    { clientId: '00000000-0000-4000-8000-000000000099' },
    { operation: 'dispatch.show' },
    { origin: `chrome-extension://${'b'.repeat(32)}` },
    { sequence: '2' },
  ])
    assert.notDeepEqual(await envelopeSigningBytes({ ...request(), ...change }), baseline);
});
test('snapshots Buffer payload and scalar fields before the asynchronous hash', async () => {
  const value = request();
  value.payload = Buffer.from(value.payload);
  const pending = envelopeSigningBytes(value);
  value.payload.fill(0);
  value.operation = 'changed';
  matches(await pending, vectors.envelopes[0]!);
});
test('UTF-8 byte lengths and normalization remain exact', () => {
  assert.notDeepEqual(
    enrollmentSigningBytes({ ...candidate(), name: 'é' }),
    enrollmentSigningBytes({ ...candidate(), name: 'e\u0301' }),
  );
  assert.ok(enrollmentSigningBytes({ ...candidate(), name: '🚀'.repeat(16) }));
  assert.throws(() => enrollmentSigningBytes({ ...candidate(), name: '🚀'.repeat(17) }), /name/);
});
const badEnvelope: [string, Partial<Envelope>][] = [
  ['decimal trailing newline', { sequence: '1\n' }],
  ['UUID trailing newline', { id: '00000000-0000-4000-8000-000000000001\n' }],
  ['origin trailing newline', { origin: `chrome-extension://${'a'.repeat(32)}\n` }],
  ['high surrogate', { operation: '\ud800' }],
  ['low surrogate', { operation: '\udc00' }],
  ['high followed by ASCII', { operation: '\ud800x' }],
  ['leading zero', { sequence: '01' }],
  ['signed decimal', { sequence: '+1' }],
  ['negative decimal', { sequence: '-1' }],
  ['fractional decimal', { sequence: '1.0' }],
  ['u64 overflow', { sequence: '18446744073709551616' }],
  ['oversized decimal', { sequence: '1'.repeat(21) }],
  ['zero normal sequence', { sequence: '0' }],
  ['negative time', { timestampMs: -1 }],
  ['fractional time', { timestampMs: 0.5 }],
  ['unsafe time', { timestampMs: 9007199254740992 }],
  ['nonfinite time', { timestampMs: NaN }],
  ['request correlation', { correlationId: '00000000-0000-4000-8000-000000000001' }],
  ['noncanonical UUID', { id: '00000000-0000-4000-8000-00000000000A' }],
  ['wrong UUID version', { id: '00000000-0000-5000-8000-000000000001' }],
  ['ordinary web origin', { origin: 'https://example.com' }],
  ['credentialed origin', { origin: `chrome-extension://user@${'a'.repeat(32)}` }],
  ['origin path', { origin: `chrome-extension://${'a'.repeat(32)}/` }],
  ['empty operation', { operation: '' }],
  ['unknown profile', { profile: 'other' as Envelope['profile'] }],
  ['unknown kind', { kind: 'other' as Envelope['kind'] }],
  ['unknown version', { version: 2 as Envelope['version'] }],
];
for (const [name, change] of badEnvelope) {
  test(`reject envelope single condition: ${name}`, async () => {
    await assert.rejects(envelopeSigningBytes({ ...request(), ...change }));
  });
}
for (const [name, change] of [
  ['session.open sequence', { sequence: '1' }],
  ['session.open session', { sessionId: '00000000-0000-4000-8000-000000000005' }],
] as const) {
  test(`reject control single condition: ${name}`, async () => {
    await assert.rejects(envelopeSigningBytes({ ...request(2), ...change }));
  });
}
test('unknown control fails its operation guard with an otherwise valid normal session', async () => {
  const valid = { ...request(), kind: 'control' as const, operation: 'subscribe' };
  assert.ok(await envelopeSigningBytes(valid));
  await assert.rejects(envelopeSigningBytes({ ...valid, operation: 'other' }), /control operation/);
});
test('response requires a canonical correlation UUID', async () => {
  await assert.rejects(envelopeSigningBytes({ ...request(1), correlationId: null }));
});
test('normal controls and integer endpoints encode without numeric rounding', async () => {
  for (const operation of ['subscribe', 'ack'])
    assert.ok(await envelopeSigningBytes({ ...request(), kind: 'control', operation }));
  assert.ok(await envelopeSigningBytes({ ...request(), timestampMs: 0 }));
  assert.ok(
    await envelopeSigningBytes({
      ...request(),
      sequence: '18446744073709551615',
    }),
  );
});
for (const field of ['machineId', 'windowId', 'offerId'] as const) {
  test(`generated enrollment ${field} still requires v4`, () => {
    assert.throws(
      () =>
        enrollmentSigningBytes({
          ...candidate(),
          [field]: '00000000-0000-5000-8000-000000000008',
        }),
      /UUIDv4/,
    );
  });
}
const badEnrollment: [string, Partial<Enrollment>][] = [
  ['surrogate', { name: '\ud800' }],
  ['empty name', { name: '' }],
  ['blank name', { name: '   ' }],
  ['control name', { name: 'Pilot\u0085' }],
  ['challenge length', { serverChallenge: new Uint8Array(15) }],
  ['nonce length', { clientNonce: new Uint8Array(17) }],
  ['public key length', { publicKey: new Uint8Array(31) }],
  ['kind/origin mismatch', { kind: 'cli' }],
  ['addon kind with door origin', { origin: 'http://127.0.0.1:7341' }],
  ['unknown kind', { kind: 'other' as Enrollment['kind'] }],
  ['unknown profile', { profile: 'other' as Enrollment['profile'] }],
];
for (const [name, change] of badEnrollment) {
  test(`reject enrollment single condition: ${name}`, () => {
    assert.throws(() => enrollmentSigningBytes({ ...candidate(), ...change }));
  });
}
test('browser kind accepts exactly the door loopback origin', () => {
  const browser = candidate(1);
  for (const origin of ['http://127.0.0.1:1', 'http://127.0.0.1:7341', 'http://127.0.0.1:65535'])
    assert.ok(enrollmentSigningBytes({ ...browser, origin }));
  for (const origin of [
    'http://127.0.0.1:0',
    'http://127.0.0.1:07341',
    'http://127.0.0.1:65536',
    'http://127.0.0.1',
    'http://127.0.0.1:7341/',
    'http://localhost:7341',
    'https://127.0.0.1:7341',
    'cli',
    `chrome-extension://${'a'.repeat(32)}`,
  ])
    assert.throws(() => enrollmentSigningBytes({ ...browser, origin }), /kind\/origin|origin/);
});
test('door origin is an envelope origin', async () => {
  assert.ok(await envelopeSigningBytes({ ...request(), origin: 'http://127.0.0.1:7341' }));
});
for (const vector of vectors.pairingCodes) {
  test(`independent pairing code text: ${vector.text}`, () => {
    const code = Buffer.from(vector.code, 'hex');
    assert.deepEqual(Buffer.from(pairingCode(vector.text)), code);
    assert.deepEqual(Buffer.from(pairingCode(vector.text.replaceAll('-', ''))), code);
    assert.deepEqual(Buffer.from(pairingCode(vector.text.replaceAll('-', ' '))), code);
  });
}
test('pairing code refuses other characters, lengths and unused bits', () => {
  const compact = vectors.pairingCodes[0]!.text.replaceAll('-', '');
  for (const invalid of [
    compact.toLowerCase(),
    compact.replaceAll('A', '1'),
    compact.replaceAll('A', '='),
    `${compact}A`,
    compact.slice(0, 25),
    `${compact.slice(0, 4)}\t${compact.slice(4)}`,
    `${compact.slice(0, 4)}_${compact.slice(4)}`,
    `${compact.slice(0, 25)}5`,
  ])
    assert.throws(() => pairingCode(invalid), /pairing code/);
});
test('pinned wordlist and independent fingerprint indexes and words', async () => {
  const list = readFileSync(
    new URL('../../../rust/tmt-remote/assets/bip39-english.txt', import.meta.url),
  );
  assert.equal(createHash('sha256').update(list).digest('hex'), vectors.wordlist.sha256);
  const words = list.toString('ascii').split('\n').slice(0, 2048);
  for (const vector of vectors.fingerprints) {
    const indexes = await fingerprintIndexes(bytes(vector.publicKey));
    assert.deepEqual(indexes, vector.indexes);
    assert.deepEqual(
      indexes.map((index) => words[index]),
      vector.words,
    );
  }
  await assert.rejects(fingerprintIndexes(new Uint8Array(31)));
});
test('independent response key and server proof inputs', () => {
  const hmac = (key: Uint8Array, message: Uint8Array): string =>
    createHmac('sha256', key).update(message).digest('hex');
  const code = bytes(macVectors.code);
  const key = hmac(code, responseKeyInput(bytes(macVectors.enrollment)));
  assert.equal(key, macVectors.responseKey);
  assert.equal(
    hmac(bytes(key), serverProofInput(bytes(macVectors.receipt))),
    macVectors.serverProof,
  );
});
for (const length of [0, 31, 33]) {
  test(`possession refuses MAC length ${length}`, () => {
    assert.throws(() =>
      enrollmentPossessionSigningBytes(enrollmentSigningBytes(candidate()), new Uint8Array(length)),
    );
  });
}

test('independent base64url vectors and strict refusals', () => {
  for (const vector of vectors.base64url.valid) {
    assert.equal(base64url(bytes(vector.hex)), vector.text);
    assert.equal(
      Buffer.from(base64urlBytes(vector.text, vector.length)).toString('hex'),
      vector.hex,
    );
  }
  for (const vector of vectors.base64url.invalid)
    assert.throws(() => base64urlBytes(vector.text, vector.length), /base64url/, vector.reason);
});

import assert from 'node:assert/strict';
import { verify } from 'node:crypto';
import { Door, paired, raw, publicKeyObject, ORIGIN, base32 } from './door.js';
import { test } from 'vite-plus/test';
import vectors from './vectors.json' with { type: 'json' };
import signatures from '../../../rust/tmt-remote/tests/fixtures/webcrypto-vectors.json' with { type: 'json' };
import {
  base64url,
  extCertSigningBytes,
  pairingCode,
  type ExtCert,
} from '../src/canonical-bytes.js';
import {
  DeviceKey,
  certify,
  openSession,
  parseLink,
  resolveLink,
  resolveAutomaticLink,
  cancelAutomaticOffer,
  PairingRefusedError,
} from '../src/device.js';
import { RefusalError } from '../src/operations.js';

test('automatic descriptors reuse admitted enrollment bytes but reject redirect targets', async () => {
  const door = new Door();
  const link = door.link().replace('#', '#auto=');
  const urls: string[] = [];
  const offer = { ...door.descriptor, automatic: true, target: '/settings' };
  const value = await resolveAutomaticLink(link, (async (url: string) => {
    urls.push(url);
    return Response.json(offer);
  }) as typeof fetch);
  assert.equal(value.target, '/settings');
  assert.deepEqual(value.descriptor, door.descriptor);
  assert.deepEqual(Buffer.from(value.code), door.code);
  assert.deepEqual(urls, [`${ORIGIN}/sdk/pair-offer`]);
  for (const target of [
    'https://elsewhere.test/',
    '//elsewhere.test/',
    '/\\elsewhere.test/',
    '/settings?x=1',
    '/settings#code',
    '/a/../settings',
  ]) {
    await assert.rejects(
      resolveAutomaticLink(link, (async () => Response.json({ ...offer, target })) as typeof fetch),
    );
  }
  await assert.rejects(
    resolveAutomaticLink(link, (async () => Response.json(door.descriptor)) as typeof fetch),
    PairingRefusedError,
    'a valid manual replacement means the automatic link ended, not a transport/proof failure',
  );
});

test('automatic cancellation has its own possession domain and never sends the raw code', async () => {
  const door = new Door();
  const key = await DeviceKey.generate();
  const {
    enrollmentSigningBytes,
    enrollmentCancellationSigningBytes,
    enrollmentPossessionSigningBytes,
  } = await import('../src/canonical-bytes.js');
  await cancelAutomaticOffer({
    descriptor: door.descriptor,
    code: door.code,
    key,
    kind: 'browser',
    origin: ORIGIN,
    name: 'Browser',
    fetch: (async (url: string, init: RequestInit) => {
      assert.equal(url, `${door.descriptor.address}/pair`);
      const body = JSON.parse(init.body as string) as Record<string, string> & { cancel: boolean };
      assert.equal(body.cancel, true);
      assert.equal(Object.keys(body).length, 13);
      const bytes = enrollmentSigningBytes({
        ...door.descriptor,
        clientNonce: Buffer.from(body.clientNonce!, 'hex'),
        serverChallenge: Buffer.from(body.serverChallenge!, 'hex'),
        kind: 'browser',
        origin: ORIGIN,
        name: 'Browser',
        publicKey: key.publicKey(),
      });
      assert.ok(
        verify(
          null,
          enrollmentCancellationSigningBytes(bytes, raw(body.mac!)),
          publicKeyObject(key.publicKey()),
          raw(body.signature!),
        ),
      );
      assert.equal(
        verify(
          null,
          enrollmentPossessionSigningBytes(bytes, raw(body.mac!)),
          publicKeyObject(key.publicKey()),
          raw(body.signature!),
        ),
        false,
      );
      assert.equal((init.body as string).includes(base32(door.code)), false);
      return new Response(null, { status: 204 });
    }) as typeof fetch,
  });
});

test('pairing retries a pending candidate and accepts only a proven receipt', async () => {
  const door = new Door();
  const { result } = await paired(door);
  assert.equal(door.pending, -1, 'one pending answer, then the receipt');
  assert.equal(result.clientId, door.clientId);
  assert.deepEqual(result.machinePublicKey, new Uint8Array(door.machinePublic));
  assert.equal(result.address, door.descriptor.address);
  for (const tamper of [{ proof: true }, { receiptKey: true }]) {
    const bad = new Door();
    bad.tamper = tamper;
    await assert.rejects(paired(bad), JSON.stringify(tamper));
  }
});

test('session.open is signed by the device and its response by the machine', async () => {
  const door = new Door();
  const { key, result } = await paired(door);
  const session = await openSession(result, key, door.descriptor.windowId, door.fetch);
  assert.equal(session.grantRevision, 1);
  assert.equal(session.expiresAtMs, null);
  for (const tamper of [{ signature: true }, { correlation: true }]) {
    door.tamper = tamper;
    await assert.rejects(
      openSession(result, key, door.descriptor.windowId, door.fetch),
      JSON.stringify(tamper),
    );
  }
});

test('the device key is non-extractable and restores only as itself', async () => {
  const key = await DeviceKey.generate();
  assert.equal(key.handle().extractable, false);
  await assert.rejects(crypto.subtle.exportKey('pkcs8', key.handle()));
  const restored = await DeviceKey.fromHandle(key.handle(), key.publicKey());
  assert.deepEqual(restored.publicKey(), key.publicKey());
  const other = await DeviceKey.generate();
  await assert.rejects(DeviceKey.fromHandle(key.handle(), other.publicKey()));
  const exportable = (await crypto.subtle.generateKey('Ed25519', true, [
    'sign',
    'verify',
  ])) as CryptoKeyPair;
  await assert.rejects(DeviceKey.fromHandle(exportable.privateKey, key.publicKey()));
});

for (const vector of vectors.extCerts) {
  test(`fixed certificate signature binds every field: ${vector.name}`, async () => {
    // Public RFC 8032 TEST 1 seed only, restored as a non-extractable handle.
    const handle = await crypto.subtle.importKey(
      'pkcs8',
      Buffer.from(
        '302e020100300506032b657004220420' +
          '9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60',
        'hex',
      ),
      'Ed25519',
      false,
      ['sign'],
    );
    const devicePublic = Buffer.from(
      'd75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a',
      'hex',
    );
    const key = await DeviceKey.fromHandle(handle, devicePublic);
    const value = {
      ...vector.input,
      publicKey: Buffer.from(vector.input.publicKey, 'hex'),
    } as ExtCert;
    const fixed = signatures.cases.find((v) => v.name === vector.name);
    assert.ok(fixed, 'every Python certificate has a fixed WebCrypto signature');
    const certificate = await certify(key, value, value.issuedAtMs);
    assert.equal(certificate.publicKey, base64url(value.publicKey));
    assert.equal(raw(certificate.signature).toString('hex'), fixed.signature);
    const baseline = extCertSigningBytes(value);
    assert.equal(Buffer.from(baseline).toString('hex'), fixed.message);
    const signature = raw(certificate.signature);
    const verifies = (message: Uint8Array): boolean =>
      verify(null, message, publicKeyObject(devicePublic), signature);
    assert.ok(verifies(baseline));
    for (const change of [
      { extension: 'other' },
      { purpose: value.purpose === 'sign' ? ('enc' as const) : ('sign' as const) },
      { publicKey: Uint8Array.from(value.publicKey, (byte, i) => (i === 0 ? byte ^ 1 : byte)) },
      { issuedAtMs: value.issuedAtMs === 0 ? 1 : value.issuedAtMs - 1 },
    ]) {
      assert.equal(verifies(extCertSigningBytes({ ...value, ...change })), false);
    }
    const domain = baseline.slice();
    domain[4 + 'tmt-ext-cert-v1'.length - 1] = '2'.charCodeAt(0);
    assert.equal(verifies(domain), false);
    const littleEndian = baseline.slice();
    littleEndian.subarray(0, 4).reverse();
    assert.equal(verifies(littleEndian), false);
    const differentSignature = Buffer.from(signature);
    differentSignature[0]! ^= 1;
    assert.equal(verify(null, baseline, publicKeyObject(devicePublic), differentSignature), false);
    const other = await DeviceKey.generate();
    assert.equal(verify(null, baseline, publicKeyObject(other.publicKey()), signature), false);
    await assert.rejects(certify(key, { ...value, extension: 'Colab' }, value.issuedAtMs));
  });
}

test('short pairing links validate the separately obtained descriptor and fragment code', () => {
  const door = new Door();
  const { descriptor, code } = parseLink(door.link(), door.descriptor);
  assert.deepEqual(descriptor, door.descriptor);
  assert.deepEqual(code, pairingCode(base32(door.code)));
  const fragment = `#${base32(door.code)}`;
  for (const link of [
    `${ORIGIN}/pair?x=1${fragment}`,
    `${ORIGIN}/pair`,
    `${ORIGIN}/pair/old${fragment}`,
  ]) {
    assert.throws(() => parseLink(link, door.descriptor));
  }
  for (const value of [
    null,
    [],
    { ...door.descriptor, extra: 1 },
    { ...door.descriptor, address: 'http://127.0.0.1:1/r/x' },
  ]) {
    assert.throws(() => parseLink(door.link(), value));
  }
});

test('offer lookup sends no code and refuses unavailable or invalid offers', async () => {
  const door = new Door();
  const requests: string[] = [];
  const network = (async (url: string, init: RequestInit) => {
    requests.push(url);
    assert.deepEqual(init, { cache: 'no-store' });
    return new Response(JSON.stringify(door.descriptor));
  }) as typeof fetch;
  assert.deepEqual(
    await resolveLink(door.link(), network),
    parseLink(door.link(), door.descriptor),
  );
  assert.deepEqual(requests, [`${ORIGIN}/sdk/pair-offer`]);
  for (const invalid of [
    `${ORIGIN}/pair#BAD`,
    `http://localhost:43210/pair#${base32(door.code)}`,
    `${ORIGIN}/pair/old#${base32(door.code)}`,
  ]) {
    await assert.rejects(resolveLink(invalid, network));
  }
  assert.equal(requests.length, 1, 'invalid links never fetch');
  await assert.rejects(
    resolveLink(door.link(), (async () => new Response('{}', { status: 404 })) as typeof fetch),
  );
  await assert.rejects(
    resolveLink(door.link(), (async () => new Response('{"extra":1}')) as typeof fetch),
  );
});

test('a rejected fresh open verifies its new-session envelope before exposing the signed limit', async () => {
  const door = new Door();
  const { key, result } = await paired(door);
  const error = {
    code: 'REMOTE_SESSION_LIMIT',
    message: 'Limit.',
    limit: 8,
    settingsUrl: '/settings',
  };
  door.mutate = (reply) => {
    reply.sessionId = 'new';
    reply.sequence = '1';
  };
  const refusal: typeof fetch = async (_url, init) =>
    door.response(JSON.parse(init!.body as string), { error });
  await assert.rejects(
    openSession(result, key, door.descriptor.windowId, refusal),
    (caught) =>
      caught instanceof RefusalError &&
      caught.code === 'REMOTE_SESSION_LIMIT' &&
      caught.limit === 8 &&
      caught.settingsUrl === `${ORIGIN}/settings`,
  );
  for (const edit of [
    () => {
      door.tamper.signature = true;
    },
    () => {
      door.mutate = (reply) => {
        reply.sessionId = crypto.randomUUID();
        reply.sequence = '1';
      };
    },
    () => {
      door.mutate = (reply) => {
        reply.sessionId = 'new';
        reply.sequence = '2';
      };
    },
    () => {
      error.settingsUrl = 'https://elsewhere.example/settings';
    },
    () => {
      error.limit = 0;
    },
  ]) {
    door.tamper = {};
    door.mutate = (reply) => {
      reply.sessionId = 'new';
      reply.sequence = '1';
    };
    error.settingsUrl = '/settings';
    error.limit = 8;
    edit();
    await assert.rejects(
      openSession(result, key, door.descriptor.windowId, refusal),
      (caught) => !(caught instanceof RefusalError),
    );
  }
});

for (const code of ['REMOTE_DEVICE_REVOKED', 'REMOTE_GRANT_EXPIRED']) {
  test(`fresh-open ${code} proof is pinned to the exact current request and run`, async () => {
    const door = new Door();
    const { key, result } = await paired(door);
    const failure = { error: { code, message: 'Authority ended.' } };
    door.mutate = (reply) => {
      reply.sessionId = 'new';
      reply.sequence = '1';
    };
    let saved: Response | undefined;
    const refusal: typeof fetch = async (_url, init) => {
      const reply = await door.response(JSON.parse(init!.body as string), failure);
      saved = reply.clone();
      return reply;
    };
    await assert.rejects(
      openSession(result, key, door.descriptor.windowId, refusal),
      (error) => error instanceof RefusalError && error.code === code,
    );
    await assert.rejects(
      openSession(result, key, door.descriptor.windowId, async () => saved!.clone()),
      (error) => !(error instanceof RefusalError),
    );
    for (const field of ['signature', 'window', 'session', 'sequence']) {
      door.tamper = { signature: field === 'signature' };
      door.mutate = (reply) => {
        reply.sessionId = field === 'session' ? crypto.randomUUID() : 'new';
        reply.sequence = field === 'sequence' ? '2' : '1';
        if (field === 'window') reply.windowId = crypto.randomUUID();
      };
      await assert.rejects(
        openSession(result, key, door.descriptor.windowId, refusal),
        (error) => !(error instanceof RefusalError),
      );
    }
  });
}

import { readFileSync, readdirSync } from 'node:fs';
import { describe, expect, it, vi } from 'vite-plus/test';
import * as c from '../src/index.js';
import { mixedMessage, mixedPolicy } from '../src/ed25519-probe.js';
const fixture = JSON.parse(
  readFileSync(new URL('../../../contracts/vectors/model-v1.json', import.meta.url), 'utf8'),
);
const authority = JSON.parse(
  readFileSync(new URL('../../../contracts/vectors/authority-v1.json', import.meta.url), 'utf8'),
);
const hex = (s: string) => Uint8Array.from(s.match(/../g) ?? [], (n) => Number.parseInt(n, 16));
const frozen = () =>
  c.Envelope.fromJson(
    c.text(
      JSON.stringify({
        header: c.encodeBinary(hex(fixture.header)),
        nonce: c.encodeBinary(new Uint8Array(12)),
        ciphertext: c.encodeBinary(hex(fixture.ciphertext)),
        signature: c.encodeBinary(hex(fixture.signature)),
      }),
    ),
  );
async function signer() {
  return crypto.subtle.importKey(
    'pkcs8',
    c.concat(hex('302e020100300506032b657004220420'), hex(fixture.seed)),
    'Ed25519',
    false,
    ['sign'],
  );
}
describe('colab browser values and immutable crypto', () => {
  it('keeps every mixed-order probe answer identical to the frozen native-policy corpus', () => {
    const corpus = readFileSync(
      new URL('../../../contracts/vectors/ed25519-829.jsonl', import.meta.url),
      'utf8',
    )
      .trim()
      .split('\n')
      .map((line) => JSON.parse(line))
      .filter((row) => row.name.startsWith('mixed-'));
    expect(
      mixedPolicy.flatMap((group, a) =>
        group.answers.map(([signature, nativePolicy], r) => ({
          name: `mixed-A${a}-R${r}`,
          public: group.publicKey,
          signature,
          message: mixedMessage,
          nativePolicy,
          verifyStrict: nativePolicy,
        })),
      ),
    ).toEqual(corpus);
  });
  it('passes the real native capability probe including all mixed-order rows', async () => {
    vi.stubGlobal('isSecureContext', true);
    const verifying = vi.spyOn(crypto.subtle, 'verify');
    try {
      await expect(c.probeCapabilities()).resolves.toBeUndefined();
      expect(
        verifying.mock.calls.slice(1).map(([, , signature]) => Array.from(signature as Uint8Array)),
      ).toEqual(
        mixedPolicy.flatMap((group) =>
          group.answers.map(([signature]) => Array.from(hex(signature))),
        ),
      );
    } finally {
      verifying.mockRestore();
      vi.unstubAllGlobals();
    }
  });
  it('fails the capability probe when native verification flips a mixed-order answer', async () => {
    vi.stubGlobal('isSecureContext', true);
    const nativeVerify = crypto.subtle.verify.bind(crypto.subtle);
    try {
      // Cover both an extra acceptance and a refused positive from the Linux divergence.
      for (const [signature, expected] of [mixedPolicy[1].answers[3], mixedPolicy[1].answers[4]]) {
        const verifying = vi
          .spyOn(crypto.subtle, 'verify')
          .mockImplementation(async (...args) =>
            c.equal(args[2] as Uint8Array, hex(signature)) ? !expected : nativeVerify(...args),
          );
        try {
          await expect(c.probeCapabilities()).rejects.toThrow();
          expect(verifying).toHaveBeenCalled();
        } finally {
          verifying.mockRestore();
        }
      }
    } finally {
      vi.unstubAllGlobals();
    }
  });
  it('matches native public_key admission at compressed-point boundaries', () => {
    // Native oracle: tmt-colab-model/tests/conformance.rs browser_public_point_boundaries_match_native_admission.
    // Mixed-order bytes come from the unchanged ed25519-829 mixed-A1-R4 positive.
    for (const [name, encoded, nativeAccepted] of [
      ['negative-zero', '0100000000000000000000000000000000000000000000000000000000000080', false],
      ['y-one', '0100000000000000000000000000000000000000000000000000000000000000', false],
      ['y-p-minus-one', 'ecffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f', false],
      ['non-square', '0200000000000000000000000000000000000000000000000000000000000000', false],
      ['small-order', 'c7176a703d4dd84fba3c0b760d10670f2a2053fa2c39ccc64ec7fd7792ac037a', false],
      ['mixed-order', '9158312a9a8d6e3b34c891d6d61444f8b8211c5117ebad15bdb0bd68b07e0245', true],
      ['normal', 'd75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a', true],
    ] as const)
      expect(c.validEdPoint(hex(encoded)), name).toBe(nativeAccepted);
  });
  it('rejects non-decompressible public keys and signature R before native import', async () => {
    const publicKey = hex(fixture.public),
      signature = hex(fixture.signature),
      input = await c.signatureInput(hex(fixture.header), hex(fixture.ciphertext)),
      off = new Uint8Array(32);
    off[0] = 2;
    const invalidR = new Uint8Array(signature);
    invalidR.set(off);
    const importing = vi.spyOn(crypto.subtle, 'importKey'),
      verifying = vi.spyOn(crypto.subtle, 'verify');
    try {
      expect(await c.strictVerify(off, signature, input)).toBe(false);
      expect(await c.strictVerify(publicKey, invalidR, input)).toBe(false);
      expect(importing).not.toHaveBeenCalled();
      expect(verifying).not.toHaveBeenCalled();
      expect(await c.strictVerify(publicKey, signature, input)).toBe(true);
      expect(importing).toHaveBeenCalledTimes(1);
      expect(verifying).toHaveBeenCalledTimes(1);
    } finally {
      importing.mockRestore();
      verifying.mockRestore();
    }
  });
  it('keeps browser runtime independent of Node, app and network/persistence APIs', () => {
    const src = new URL('../src/', import.meta.url);
    for (const name of readdirSync(src).filter((n) => n.endsWith('.ts'))) {
      const source = readFileSync(new URL(name, src), 'utf8');
      expect(source).not.toMatch(/(?:from|import\s*\()\s*['"](?:node:|[^.])/);
      expect(source).not.toMatch(
        /\b(?:fetch|WebSocket|indexedDB|localStorage)\b|\bdocument\s*(?:\.|\[)/,
      );
    }
  });
  it('rejects duplicate/unknown/type/Unicode/binary/framing mutations', () => {
    for (const value of [
      '{"a":1,"a":1}',
      '{"a":{"b":1,"b":1}}',
      '{"a":"\\ud800"}',
      '[1,]',
      '{} true',
      '\ufeff{}',
    ])
      expect(() => c.strictJson(c.text(value), 1024)).toThrow();
    expect(c.strictJson(c.text('{"a":1,"b":[true,null]}'), 1024)).toEqual({
      a: 1,
      b: [true, null],
    });
    for (const binary of ['Zg=', 'Zh', '+w', '_w==']) expect(() => c.binary(binary, 32)).toThrow();
    expect(() => c.text('\ud800')).toThrow();
    expect(() => c.time(-0)).toThrow();
    expect(() =>
      c.idList(['00000000-0000-4000-8000-000000000002', '00000000-0000-4000-8000-000000000002']),
    ).toThrow();
    expect(() => c.fields(new Uint8Array([0, 0, 0, 2, 1]), 1)).toThrow();
    expect(() => c.decimal('01')).toThrow();
    expect(() => c.decimal('18446744073709551616')).toThrow();
    const w = JSON.parse(c.decodeText(frozen().toJson()));
    const dup = `{"nonce":${JSON.stringify(w.nonce)},${JSON.stringify(w).slice(1)}`;
    expect(() => c.Envelope.fromJson(c.text(dup))).toThrow();
    expect(() => c.Envelope.fromJson(c.text(JSON.stringify({ ...w, extra: true })))).toThrow();
    expect(() =>
      c.Envelope.fromJson(
        c.text(JSON.stringify({ ...w, nonce: c.encodeBinary(new Uint8Array(12).fill(1)) })),
      ),
    ).toThrow();
  });
  it('matches independent ciphertext/hash and snapshots strict verify inputs', async () => {
    expect(await c.deriveSpaceId(hex(authority.public))).toBe(authority.space);
    const e = frozen(),
      h = c.decodeHeader(e.header());
    expect(await e.open(h.context, hex(fixture.master), hex(fixture.public))).toEqual(
      hex(fixture.plaintext),
    );
    expect(await e.hash()).toEqual(hex(fixture.envelopeHash));
    const publicKey = hex(fixture.public),
      sig = e.signature(),
      input = await c.signatureInput(e.header(), e.ciphertext());
    const pending = c.strictVerify(publicKey, sig, input);
    publicKey.fill(0);
    sig.fill(0);
    input.fill(0);
    expect(await pending).toBe(true);
    e.header().fill(9);
    e.ciphertext().fill(9);
    e.signature().fill(9);
    expect(await e.hash()).toEqual(hex(fixture.envelopeHash));
    await expect(
      e.open({ ...h.context, epoch: '2' }, hex(fixture.master), hex(fixture.public)),
    ).rejects.toThrow();
    await expect(e.open(h.context, new Uint8Array(32), hex(fixture.public))).rejects.toThrow();
  });
  it('seals fresh IDs, snapshots before awaits, rejects caller IDs and ceilings', async () => {
    const ctx = c.decodeHeader(hex(fixture.header)).context,
      key = await signer();
    const secret = hex(fixture.master),
      pt = hex(fixture.plaintext);
    const pending = c.Envelope.seal(ctx, secret, key, pt);
    secret.fill(9);
    pt.fill(9);
    ctx.prevHash.fill(9);
    const a = await pending,
      original = c.decodeHeader(hex(fixture.header)).context;
    const b = await c.Envelope.seal(original, hex(fixture.master), key, hex(fixture.plaintext));
    expect(a.header()).not.toEqual(b.header());
    for (const e of [a, b])
      expect(await e.open(original, hex(fixture.master), hex(fixture.public))).toEqual(
        hex(fixture.plaintext),
      );
    await expect(
      c.Envelope.seal(
        Object.assign({}, original, { objectId: '00'.repeat(32) }),
        hex(fixture.master),
        key,
        new Uint8Array(),
      ),
    ).rejects.toThrow();
    await expect(
      c.Envelope.seal(original, hex(fixture.master), key, new Uint8Array(256 * 1024 + 1)),
    ).rejects.toThrow();
    const hiddenId = Object.defineProperty({ ...original }, 'objectId', { value: '00'.repeat(32) });
    await expect(
      c.Envelope.seal(hiddenId, hex(fixture.master), key, new Uint8Array()),
    ).rejects.toThrow();
    const extractable = (await crypto.subtle.generateKey('Ed25519', true, [
      'sign',
      'verify',
    ])) as CryptoKeyPair;
    await expect(
      c.Envelope.seal(original, hex(fixture.master), extractable.privateKey, new Uint8Array()),
    ).rejects.toThrow();
  });
  it('keeps recipient handles non-extractable and validates restored public bindings', async () => {
    const a = await c.RecipientKey.generate(),
      b = await c.RecipientKey.generate();
    await c.RecipientKey.fromHandle(a.handle(), a.publicKey());
    await expect(crypto.subtle.exportKey('pkcs8', a.handle())).rejects.toThrow();
    await expect(c.RecipientKey.fromHandle(a.handle(), b.publicKey())).rejects.toThrow();
    const zero = await crypto.subtle.importKey('raw', new Uint8Array(32), 'X25519', false, []);
    await expect(
      crypto.subtle.deriveBits({ name: 'X25519', public: zero }, a.handle(), 256),
    ).rejects.toThrow();
    const pair = (await crypto.subtle.generateKey('X25519', true, ['deriveBits'])) as CryptoKeyPair;
    await expect(c.RecipientKey.fromHandle(pair.privateKey, new Uint8Array(32))).rejects.toThrow();
  });
});

it('opaque HKDF roots seal/open exactly the frozen raw-root envelope', async () => {
  const root = await crypto.subtle.importKey('raw', hex(fixture.master), 'HKDF', false, [
    'deriveBits',
  ]);
  const { context, objectId } = c.decodeHeader(hex(fixture.header)),
    key = await signer();
  await expect(crypto.subtle.exportKey('raw', root)).rejects.toThrow();
  const entropy = vi.spyOn(crypto, 'getRandomValues');
  // Test-only entropy control reproduces the existing vector. Product seal still owns IDs.
  entropy.mockImplementation(<T extends ArrayBufferView | null>(value: T): T => {
    expect(value).toBeInstanceOf(Uint8Array);
    const bytes = value as Uint8Array;
    expect(bytes.length).toBe(32);
    bytes.set(hex(objectId));
    return value;
  });
  try {
    for (const input of [hex(fixture.master), root]) {
      const sealed = await c.Envelope.seal(context, input, key, hex(fixture.plaintext));
      expect(sealed.toJson()).toEqual(frozen().toJson());
      expect(await sealed.hash()).toEqual(hex(fixture.envelopeHash));
      expect(await sealed.open(context, root, hex(fixture.public))).toEqual(hex(fixture.plaintext));
    }
  } finally {
    entropy.mockRestore();
  }
  const wrongUsage = await crypto.subtle.importKey('raw', hex(fixture.master), 'HKDF', false, [
    'deriveKey',
  ]);
  const wrongAlgorithm = await crypto.subtle.importKey(
    'raw',
    hex(fixture.master),
    'AES-GCM',
    false,
    ['encrypt'],
  );
  for (const invalid of [wrongUsage, wrongAlgorithm]) {
    await expect(c.Envelope.seal(context, invalid, key, hex(fixture.plaintext))).rejects.toThrow();
    await expect(frozen().open(context, invalid, hex(fixture.public))).rejects.toThrow();
  }
});

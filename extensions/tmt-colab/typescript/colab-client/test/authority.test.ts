import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vite-plus/test';
import * as c from '../src/index.js';
const v = JSON.parse(
  readFileSync(new URL('../../../contracts/vectors/authority-v1.json', import.meta.url), 'utf8'),
);
const ownerCases = JSON.parse(
  readFileSync(new URL('../../../contracts/vectors/owner-member-v1.json', import.meta.url), 'utf8'),
).cases;
const hex = (s: string): c.Bytes => Uint8Array.from(s.match(/../g) ?? [], (n) => parseInt(n, 16));
const json = (value: unknown): c.Bytes => c.text(JSON.stringify(value));
const member = JSON.parse(v.payload);
const zero = c.encodeBinary(new Uint8Array(32));
async function signer(): Promise<CryptoKey> {
  return crypto.subtle.importKey(
    'pkcs8',
    c.concat(hex('302e020100300506032b657004220420'), hex(v.seed)),
    'Ed25519',
    false,
    ['sign'],
  );
}
async function signed(
  operation: string,
  value: unknown,
  previous: c.statement.Head | null = null,
): Promise<c.statement.Envelope> {
  const bytes = json(value),
    input = c.statement.input({
      space: v.space,
      revision: String(previous === null ? 1n : previous.revision + 1n),
      previousHash: previous === null ? new Uint8Array(32) : previous.hash,
      operation,
      payloadDigest: await c.digest(bytes),
    });
  return c.statement.Envelope.fromJson(
    json({
      statement: c.encodeBinary(input),
      payload: c.encodeBinary(bytes),
      signature: c.encodeBinary(await c.sign(await signer(), input)),
    }),
  );
}
async function genesis(): Promise<c.statement.Verified> {
  return c.statement.Envelope.fromJson(json(v.statement)).verifyNext(v.space, hex(v.public), null);
}
async function recipient(): Promise<c.RecipientKey> {
  const key = await crypto.subtle.importKey(
    'pkcs8',
    c.concat(hex('302e020100300506032b656e04220420'), hex(v.recipientSeed)),
    'X25519',
    false,
    ['deriveBits'],
  );
  return c.RecipientKey.fromHandle(
    key,
    c.wrap.Envelope.fromJson(json(v.wrap)).header().recipientKey,
  );
}
async function alteredWrap(field: 'enc' | 'ciphertext', raw: Uint8Array): Promise<c.wrap.Envelope> {
  const wire = { ...v.wrap, [field]: c.encodeBinary(raw) };
  wire.signature = c.encodeBinary(
    await c.sign(
      await signer(),
      c.frame(
        c.text('tmt-colab-wrap-signature-v1'),
        c.text('1'),
        c.binary(wire.header, 1024),
        c.binary(wire.enc, 32, 32),
        await c.digest(c.binary(wire.ciphertext, 48, 48)),
      ),
    ),
  );
  return c.wrap.Envelope.fromJson(json(wire));
}
describe('browser authority ports', () => {
  it('pins the URL root and exact independent statement/hash bytes', async () => {
    const envelope = c.statement.Envelope.fromJson(json(v.statement)),
      g = await genesis();
    expect(c.equal(await envelope.hash(), hex(v.statementHash))).toBe(true);
    expect(g.head.revision).toBe(1n);
    expect(g.head.ownerMember.id).toBe(v.device);
    await expect(envelope.verifyNext('a'.repeat(32), hex(v.public), null)).rejects.toThrow();
    const key = (await crypto.subtle.generateKey('Ed25519', false, [
      'sign',
      'verify',
    ])) as CryptoKeyPair;
    await expect(
      envelope.verifyNext(
        v.space,
        new Uint8Array(await crypto.subtle.exportKey('raw', key.publicKey)),
        null,
      ),
    ).rejects.toThrow();
    const wire = { ...v.statement, payload: c.encodeBinary(c.text(v.payload + ' ')) };
    await expect(
      c.statement.Envelope.fromJson(json(wire)).verifyNext(v.space, hex(v.public), null),
    ).rejects.toThrow();
    expect(() =>
      c.statement.Envelope.fromJson(
        c.text(
          JSON.stringify(v.statement).replace('{', '{"payload":"' + v.statement.payload + '",'),
        ),
      ),
    ).toThrow();
    const wrongSig = { ...v.statement, signature: c.encodeBinary(new Uint8Array(64)) };
    await expect(
      c.statement.Envelope.fromJson(json(wrongSig)).verifyNext(v.space, hex(v.public), null),
    ).rejects.toThrow();
  });
  it('fences replay/forks, snapshots retained heads, and protects the pinned owner member', async () => {
    const g = await genesis(),
      e = await signed('page.history', { pageId: v.page, mode: 'shared' }, g.head);
    const prior = {
      ...g.head,
      hash: c.copy(g.head.hash),
      ownerMember: {
        ...g.head.ownerMember,
        signingKey: c.copy(g.head.ownerMember.signingKey),
        encryptionKey: c.copy(g.head.ownerMember.encryptionKey),
      },
    };
    const key = hex(v.public),
      pending = e.verifyNext(v.space, key, prior);
    key.fill(0);
    prior.hash.fill(0);
    prior.ownerMember.signingKey.fill(0);
    prior.ownerMember.id = v.page;
    const next = await pending;
    expect(next.head.revision).toBe(2n);
    expect(next.head.ownerMember.id).toBe(v.device);
    await expect(e.verifyNext(v.space, hex(v.public), next.head)).rejects.toThrow();
    await expect(
      e.verifyNext(v.space, hex(v.public), { ...g.head, hash: new Uint8Array(32) }),
    ).rejects.toThrow();
    for (const test of ownerCases) {
      const wire = test.envelope;
      expect(
        await c.strictVerify(
          hex(v.public),
          c.binary(wire.signature, 64, 64),
          c.binary(wire.statement, 1024),
        ),
      ).toBe(true);
      const pending = c.statement.Envelope.fromJson(json(wire)).verifyNext(
        v.space,
        hex(v.public),
        g.head,
      );
      if (test.accepted) expect((await pending).head.revision).toBe(2n);
      else await expect(pending).rejects.toThrow();
    }
    const peer = (await crypto.subtle.generateKey('Ed25519', false, [
      'sign',
      'verify',
    ])) as CryptoKeyPair;
    const value = {
      ...member,
      memberId: v.page,
      signKey: c.encodeBinary(new Uint8Array(await crypto.subtle.exportKey('raw', peer.publicKey))),
      encKey: zero,
    };
    expect(
      (await (await signed('member.add', value, g.head)).verifyNext(v.space, hex(v.public), g.head))
        .head.revision,
    ).toBe(2n);
    for (const bad of [
      { ...value, memberId: v.device },
      { ...value, signKey: member.signKey },
      { ...value, encKey: member.encKey },
    ])
      await expect(
        (await signed('member.add', bad, g.head)).verifyNext(v.space, hex(v.public), g.head),
      ).rejects.toThrow();
    await expect(
      (await signed('page.archive', { pageId: v.page })).verifyNext(v.space, hex(v.public), null),
    ).rejects.toThrow();
    await expect(
      (await signed('member.add', { ...member, role: 'viewer' })).verifyNext(
        v.space,
        hex(v.public),
        null,
      ),
    ).rejects.toThrow();
  });
  it('validates every payload grammar, bounded lists, numeric ordering and nested epoch scope', async () => {
    const baseline = {
      pageId: v.page,
      epoch: '1',
      sourceDigest: zero,
      baselineCommitment: zero,
      title: 'source',
      objectEnvelopeHash: zero,
      membershipRevision: '2',
    };
    const cut = c.encodeBinary(
      c.streamCut.input({
        streamId: v.device,
        namespace: 'content',
        checkpointHash: null,
        checkpointSeq: '0',
        tailHeadSeq: '0',
        tailHeadHash: new Uint8Array(32),
      }),
    );
    const cuts = [{ pageId: v.page, epoch: '1', namespace: 'content', cut }];
    const samples = {
      'member.add': member,
      'member.remove': { memberId: v.device, cuts: [] },
      'member.role': { memberId: v.device, role: 'editor', cuts: [] },
      'link.add': {
        linkId: v.linkId,
        role: 'viewer',
        linkSignKey: c.encodeBinary(hex(v.linkSigningPublic)),
        linkEncKey: c.encodeBinary(hex(v.linkEncryptionPublic)),
        pages: [v.page],
      },
      'link.remove': { linkId: v.linkId, cuts: [] },
      'device.revoke': { deviceId: v.page, cuts: [] },
      'bridge.add': { machineId: v.page, machineSignKey: member.signKey, encKey: zero, pages: [] },
      'epoch.advance': { pageId: v.page, epoch: '1', cuts: [], baseline, wraps: [v.wrap] },
      'page.share': {
        pageId: v.page,
        mode: 'public',
        epoch: '10',
        publishedKeys: [
          { epoch: '2', key: zero },
          { epoch: '10', key: zero },
        ],
      },
      'page.history': { pageId: v.page, mode: 'shared' },
      'retention.set': { pageId: v.page, days: null },
      'page.archive': { pageId: v.page },
      'page.delete': { pageId: v.page },
    };
    for (const [op, value] of Object.entries(samples)) {
      expect(c.payload.decode(op, json(value)).operation).toBe(op);
      expect(() => c.payload.decode(op, json({ ...value, extra: 1 }))).toThrow();
    }
    for (const days of ['1.0', '1e0', '-0', '9007199254740992'])
      expect(() =>
        c.payload.decode('retention.set', c.text(`{"pageId":"${v.page}","days":${days}}`)),
      ).toThrow();
    const ids = Array.from(
      { length: 257 },
      (_, i) => `00000000-0000-4000-8000-${i.toString(16).padStart(12, '0')}`,
    );
    expect(
      c.payload.decode('member.add', json({ ...member, pages: ids.slice(0, 256) })).operation,
    ).toBe('member.add');
    expect(() => c.payload.decode('member.add', json({ ...member, pages: ids }))).toThrow();
    expect(() =>
      c.payload.decode('member.add', json({ ...member, pages: [v.page, v.page] })),
    ).toThrow();
    expect(() =>
      c.payload.decode('page.share', json({ ...samples['page.share'], publishedKeys: null })),
    ).toThrow();
    expect(() =>
      c.payload.decode(
        'page.share',
        json({
          ...samples['page.share'],
          publishedKeys: samples['page.share'].publishedKeys.slice().reverse(),
        }),
      ),
    ).toThrow();
    expect(() =>
      c.payload.decode('page.share', json({ ...samples['page.share'], mode: 'private' })),
    ).toThrow();
    expect(() =>
      c.payload.decode('member.remove', json({ memberId: v.device, cuts: [...cuts, ...cuts] })),
    ).toThrow();
    const g = await genesis();
    expect(
      (
        await (
          await signed('epoch.advance', samples['epoch.advance'], g.head)
        ).verifyNext(v.space, hex(v.public), g.head)
      ).head.revision,
    ).toBe(2n);
    await expect(
      (await signed('epoch.advance', { ...samples['epoch.advance'], cuts }, g.head)).verifyNext(
        v.space,
        hex(v.public),
        g.head,
      ),
    ).rejects.toThrow();
    await expect(
      (
        await signed(
          'epoch.advance',
          {
            ...samples['epoch.advance'],
            baseline: { ...baseline, membershipRevision: '1' },
            wraps: [],
          },
          g.head,
        )
      ).verifyNext(v.space, hex(v.public), g.head),
    ).rejects.toThrow();
    const badWrap = { ...v.wrap, signature: c.encodeBinary(new Uint8Array(64)) };
    await expect(
      (
        await signed('epoch.advance', { ...samples['epoch.advance'], wraps: [badWrap] }, g.head)
      ).verifyNext(v.space, hex(v.public), g.head),
    ).rejects.toThrow();
    const pageBytes = JSON.stringify({ pageId: v.page });
    const atLimit = c.text(' '.repeat(c.payload.MAX_BYTES - pageBytes.length) + pageBytes);
    expect(c.payload.decode('page.delete', atLimit).operation).toBe('page.delete');
    expect(() => c.payload.decode('page.delete', c.concat(c.text(' '), atLimit))).toThrow();
  });
  it('grants no signature authority to syntactically accepted off-curve subjects', async () => {
    // y=2 has nonsquare x^2=(y^2-1)/(d*y^2+1) over p=2^255-19; public fixture only.
    const off = new Uint8Array(32);
    off[0] = 2;
    expect(c.validEdPoint(off)).toBe(true);
    expect(
      c.payload.decode('member.add', json({ ...member, signKey: c.encodeBinary(off) })).operation,
    ).toBe('member.add');
    const chain = c.certificate.Chain.fromJson(json(v.chain)),
      cert = chain.certificate();
    c.certificate.input({ ...cert, signingKey: off });
    const message = c.certificate.input(cert),
      signature = c.binary(v.chain.issuerSignature, 64, 64);
    expect(await c.strictVerify(hex(v.public), signature, message)).toBe(true);
    expect(await c.strictVerify(off, signature, message)).toBe(false);
    await expect(chain.verify(hex(v.statementHash), cert, off)).rejects.toThrow();
    // Match the off-curve root's space so rejection is by signature, not the URL pin.
    const space = await c.deriveSpaceId(off),
      bytes = c.text(v.payload);
    const input = c.statement.input({
      space,
      revision: '1',
      previousHash: new Uint8Array(32),
      operation: 'member.add',
      payloadDigest: await c.digest(bytes),
    });
    const ownerSignature = await c.sign(await signer(), input);
    expect(await c.strictVerify(hex(v.public), ownerSignature, input)).toBe(true);
    const envelope = c.statement.Envelope.fromJson(
      json({
        statement: c.encodeBinary(input),
        payload: c.encodeBinary(bytes),
        signature: c.encodeBinary(ownerSignature),
      }),
    );
    await expect(envelope.verifyNext(space, off, null)).rejects.toThrow();
  });

  it('binds certificate issuer hash, exact fields, key and validity bytes', async () => {
    const chain = c.certificate.Chain.fromJson(json(v.chain)),
      cert = chain.certificate();
    expect(c.equal(await chain.digest(), hex(v.chainDigest))).toBe(true);
    await chain.verify(hex(v.statementHash), cert, hex(v.public));
    const key = hex(v.public),
      expected = chain.certificate(),
      hash = hex(v.statementHash),
      pending = chain.verify(hash, expected, key);
    key.fill(0);
    hash.fill(0);
    expected.signingKey.fill(0);
    expected.issuerId = v.page;
    await pending;
    await expect(chain.verify(new Uint8Array(32), cert, hex(v.public))).rejects.toThrow();
    for (const field of ['space', 'issuerKind', 'issuerId', 'deviceId', 'membershipRevision'])
      await expect(
        chain.verify(
          hex(v.statementHash),
          {
            ...cert,
            [field]:
              field === 'issuerKind'
                ? 'link'
                : field === 'membershipRevision'
                  ? '2'
                  : field === 'space'
                    ? 'a'.repeat(32)
                    : v.linkId,
          },
          hex(v.public),
        ),
      ).rejects.toThrow();
    await expect(
      chain.verify(hex(v.statementHash), { ...cert, expiresAt: 101 }, hex(v.public)),
    ).rejects.toThrow();
    expect(() =>
      c.certificate.Chain.fromJson(
        c.text(JSON.stringify(v.chain).replace('"version":1', '"version":1.0')),
      ),
    ).toThrow();
    expect(() =>
      c.certificate.Chain.fromJson(c.text(JSON.stringify(v.chain).replace('{', '{"version":1,'))),
    ).toThrow();
    expect(() => c.certificate.input({ ...cert, expiresAt: 0 })).toThrow();
  });
  it('opens the independent wrap using opaque native X25519 and rejects tampering/context/low order', async () => {
    const envelope = c.wrap.Envelope.fromJson(json(v.wrap)),
      key = await recipient(),
      expected = envelope.header();
    expect(c.equal(await envelope.open(expected, key, hex(v.public)), hex(v.epochKey))).toBe(true);
    const owner = hex(v.public),
      h = envelope.header(),
      pending = envelope.open(h, key, owner);
    owner.fill(0);
    h.recipientKey.fill(0);
    h.signerKey.fill(0);
    h.epoch = '2';
    expect(c.equal(await pending, hex(v.epochKey))).toBe(true);
    await expect(envelope.open({ ...expected, epoch: '2' }, key, hex(v.public))).rejects.toThrow();
    await expect(
      envelope.open(expected, await c.RecipientKey.generate(), hex(v.public)),
    ).rejects.toThrow();
    for (const field of ['enc', 'ciphertext', 'signature']) {
      const bytes = c.binary(v.wrap[field], 64);
      bytes[0] ^= 1;
      const e = c.wrap.Envelope.fromJson(json({ ...v.wrap, [field]: c.encodeBinary(bytes) }));
      await expect(e.open(expected, key, hex(v.public))).rejects.toThrow();
    }
    const ct = c.binary(v.wrap.ciphertext, 48);
    ct[0] ^= 1;
    await expect(
      (await alteredWrap('ciphertext', ct)).open(expected, key, hex(v.public)),
    ).rejects.toThrow();
    for (const low of [0, 1]) {
      const enc = new Uint8Array(32);
      enc[0] = low;
      const e = await alteredWrap('enc', enc);
      await e.verifyOwner(hex(v.public));
      await expect(e.open(expected, key, hex(v.public))).rejects.toThrow();
    }
    expect(() =>
      c.wrap.Envelope.fromJson(
        c.text(JSON.stringify(v.wrap).replace('{', '{"enc":"' + v.wrap.enc + '",')),
      ),
    ).toThrow();
  });
});

it('history statements are strict and owner-only against independent vectors', async () => {
  const g = await genesis();
  for (const test of v.historyCases) {
    let parsed = false,
      accepted = false;
    try {
      c.payload.decode(test.operation, c.text(test.payload));
      parsed = true;
    } catch {}
    try {
      await c.statement.Envelope.fromJson(json(test.envelope)).verifyNext(
        v.space,
        hex(v.public),
        g.head,
      );
      accepted = true;
    } catch {}
    expect(parsed, test.name).toBe(test.accepted);
    expect(accepted, test.name).toBe(test.accepted);
  }
  await expect(
    c.statement.Envelope.fromJson(json(v.historyWrongOwner)).verifyNext(
      v.space,
      hex(v.public),
      g.head,
    ),
  ).rejects.toThrow();
});

it('forward wraps and a capped multi-list join keep the existing grammar', async () => {
  const j = v.historyJoin,
    g = await genesis();
  await c.statement.Envelope.fromJson(json(j.memberAdd)).verifyNext(v.space, hex(v.public), g.head);
  const key = await crypto.subtle.importKey(
    'pkcs8',
    c.concat(hex('302e020100300506032b656e04220420'), hex(j.recipientSeed)),
    'X25519',
    false,
    ['deriveBits'],
  );
  const forward = c.wrap.Envelope.fromJson(json(v.forwardWrap)),
    h = forward.header();
  const r = await c.RecipientKey.fromHandle(key, h.recipientKey);
  expect(h.epoch).toBe('63');
  expect(h.membershipRevision).toBe('2');
  expect(c.equal(await forward.open(h, r, hex(v.public)), hex(v.epochKey))).toBe(true);
  expect(j.wrapLists.map((list: string[]) => list.length)).toEqual([512, 64]);
  let previous = '',
    previousEpoch = 0n;
  const pages = new Map<string, number>();
  for (const list of j.wrapLists)
    for (const raw of list) {
      const w = c.wrap.Envelope.fromJson(c.text(raw)),
        h = w.header(),
        epoch = BigInt(h.epoch);
      expect(h.page > previous || (h.page === previous && epoch > previousEpoch)).toBe(true);
      previous = h.page;
      previousEpoch = epoch;
      expect(epoch <= 64n).toBe(true);
      expect(h.membershipRevision).toBe('2');
      expect(h.recipientKind).toBe('member');
      expect(h.recipientId).toBe('00000000-0000-4000-8000-000000000051');
      pages.set(h.page, (pages.get(h.page) ?? 0) + 1);
      expect(c.equal(await w.open(h, r, hex(v.public)), hex(v.epochKey))).toBe(true);
    }
  expect(pages.size).toBe(9);
  expect([...pages.values()].every((n) => n === 64)).toBe(true);
}, 30000);

import { readFileSync } from 'node:fs';
import { expect, test, type BrowserContext, type WebSocketRoute } from '@playwright/test';
import * as c from '@tmt/colab-client';
import * as Y from 'yjs'; // Test-only producer. Foreign update decoding stays in the app Worker.
const v = JSON.parse(
  readFileSync(new URL('../../../contracts/vectors/authority-v1.json', import.meta.url), 'utf8'),
);
const hex = (s: string) => Uint8Array.from(s.match(/../g) ?? [], (n) => parseInt(n, 16));
const json = (value: unknown) => c.text(JSON.stringify(value));
const mount = '/r/abcd/x/colab/';
async function wire(context: BrowserContext) {
  const signer = await crypto.subtle.importKey(
    'pkcs8',
    c.concat(hex('302e020100300506032b657004220420'), hex(v.seed)),
    'Ed25519',
    false,
    ['sign'],
  );
  const owner = hex(v.public),
    g = c.statement.Envelope.fromJson(json(v.statement)),
    genesis = await g.verifyNext(v.space, owner, null);
  const payload = json({ pageId: v.page, mode: 'private', epoch: '1' }),
    input = c.statement.input({
      space: v.space,
      operation: 'page.share',
      revision: '2',
      previousHash: genesis.head.hash,
      payloadDigest: await c.digest(payload),
    });
  const shared = c.statement.Envelope.fromJson(
    json({
      statement: c.encodeBinary(input),
      payload: c.encodeBinary(payload),
      signature: c.encodeBinary(await c.sign(signer, input)),
    }),
  );
  const head = await shared.verifyNext(v.space, owner, genesis.head);
  // Public fixture seeds only: prepopulate opaque handles without replacing them on reload.
  await context.addInitScript(
    ({ seed, recipient, device, pub, enc }) => {
      if (window !== window.top) return;
      const bytes = (s: string) => Uint8Array.from(s.match(/../g) ?? [], (n) => parseInt(n, 16));
      (window as unknown as { fixtureKeys: Promise<void> }).fixtureKeys = (async () => {
        const db = await new Promise<IDBDatabase>((resolve, reject) => {
          const r = indexedDB.open('tmt-colab', 1);
          r.onupgradeneeded = () => r.result.createObjectStore('keys');
          r.onsuccess = () => resolve(r.result);
          r.onerror = () => reject(r.error);
        });
        try {
          const exists = await new Promise((resolve) => {
            const tx = db.transaction('keys'),
              r = tx.objectStore('keys').get(`keys:${device}`);
            tx.oncomplete = () => resolve(r.result);
          });
          if (exists) return;
          const sign = await crypto.subtle.importKey(
              'pkcs8',
              bytes('302e020100300506032b657004220420' + seed),
              'Ed25519',
              false,
              ['sign'],
            ),
            encryption = await crypto.subtle.importKey(
              'pkcs8',
              bytes('302e020100300506032b656e04220420' + recipient),
              'X25519',
              false,
              ['deriveBits'],
            );
          await new Promise<void>((resolve, reject) => {
            const tx = db.transaction('keys', 'readwrite');
            tx.objectStore('keys').put(
              {
                sign,
                signPublic: new Uint8Array(pub),
                enc: encryption,
                encPublic: new Uint8Array(enc),
              },
              `keys:${device}`,
            );
            tx.oncomplete = () => resolve();
            tx.onabort = () => reject(tx.error);
          });
        } finally {
          db.close();
        }
      })();
    },
    {
      seed: v.seed,
      recipient: v.recipientSeed,
      device: v.device,
      pub: [...owner],
      enc: [...c.wrap.Envelope.fromJson(json(v.wrap)).header().recipientKey],
    },
  );
  await context.route('**/sdk/remote-v1.js*', (route) =>
    route.fulfill({
      contentType: 'text/javascript',
      body: `export async function reopenSession(){await window.fixtureKeys;} export async function certifyKey(purpose,bytes){return {publicKey:btoa(String.fromCharCode(...bytes)).replaceAll('+','-').replaceAll('/','_').replace(/=+$/,''),issuedAtMs:Date.now(),signature:'${c.encodeBinary(new Uint8Array(64))}'}}`,
    }),
  );
  await context.route(`**${mount}api/session`, (route) =>
    route.fulfill({
      json: {
        deviceId: v.device,
        publicKey: c.encodeBinary(owner),
        grantRevision: '1',
        name: 'Fixture',
      },
    }),
  );
  await context.route(`**${mount}api/pages`, (route) =>
    route.fulfill({
      json: {
        spaceId: v.space,
        ownerKey: c.encodeBinary(owner),
        revision: '2',
        pages: [
          { pageId: v.page, epoch: '1', sharing: 'private', history: 'current', archived: false },
        ],
      },
    }),
  );
  let chain: unknown;
  await context.route(`**${mount}api/devices/register`, async (route) => {
    const keys = route.request().postDataJSON();
    expect(keys.sign.publicKey).toBe(c.encodeBinary(owner));
    const certificate = c.certificate.input({
      space: v.space,
      issuerKind: 'member',
      issuerId: genesis.head.ownerMember.id,
      deviceId: v.device,
      signingKey: c.binary(keys.sign.publicKey, 32, 32),
      encryptionKey: c.binary(keys.enc.publicKey, 32, 32),
      membershipRevision: '1',
      issuedAt: Date.now() - 1000,
      expiresAt: Date.now() + 86400000,
    });
    chain = {
      version: 1,
      issuerStatement: c.encodeBinary(genesis.head.hash),
      deviceCertificate: c.encodeBinary(certificate),
      issuerSignature: c.encodeBinary(await c.sign(signer, certificate)),
    };
    await route.fulfill({ json: { issuerStatement: JSON.parse(c.decodeText(g.toJson())), chain } });
  });
  const doc = new Y.Doc();
  doc.getText('html').insert(0, '<h1>Live fixture</h1>');
  doc.getMap('meta').set('title', 'Live fixture');
  const initial = await c.Envelope.seal(
    {
      space: v.space,
      page: v.page,
      epoch: '1',
      kind: 'update',
      namespace: 'content',
      authorDevice: v.device,
      membershipRevision: '2',
      streamSeq: '1',
      prevHash: new Uint8Array(32),
    },
    hex(v.epochKey),
    signer,
    new Uint8Array(Y.encodeStateAsUpdate(doc)),
  );
  doc.destroy();
  const entry = async (env: c.Envelope) => ({
    seq: c.decodeHeader(env.header()).context.streamSeq,
    envelopeHash: c.encodeBinary(await env.hash()),
    envelope: c.encodeBinary(env.toJson()),
  });
  const entries = [await entry(initial)],
    peers = new Set<WebSocketRoute>();
  const scope = { version: 1, space: v.space, page: v.page, epoch: '1' };
  let queue = Promise.resolve(),
    drop = false,
    retries = 0,
    chunked = 0,
    hellos = 0;
  const outgoing = new Map<WebSocketRoute, { frames: string[]; waiting: boolean }>();
  const pump = (socket: WebSocketRoute) => {
    const state = outgoing.get(socket);
    if (!state || state.waiting || !state.frames.length) return;
    state.waiting = true;
    socket.send(state.frames.shift()!);
  };
  const send = (socket: WebSocketRoute, type: string, fields: Record<string, unknown>) => {
    const state = outgoing.get(socket)!;
    state.frames.push(JSON.stringify({ ...scope, type, ...fields }));
    pump(socket);
  };
  function deliver(
    socket: WebSocketRoute,
    type: string,
    row: (typeof entries)[number],
    fields: Record<string, unknown>,
  ) {
    const bytes = c.binary(row.envelope, 400 * 1024),
      id = c.decodeHeader(c.Envelope.fromJson(bytes).header()).objectId;
    const envelope = bytes.length > 32768 ? { objectId: id } : row.envelope;
    send(socket, type, { ...fields, ...row, envelope });
    if (typeof envelope !== 'string') {
      const count = Math.ceil(bytes.length / 32768);
      for (let index = 0; index < count; index++)
        send(socket, 'chunk', {
          objectId: id,
          envelopeHash: row.envelopeHash,
          index,
          count,
          bytes: c.encodeBinary(bytes.slice(index * 32768, (index + 1) * 32768)),
        });
    }
  }
  await context.routeWebSocket(`**${mount}sync`, (socket) => {
    let pending: { frame: Record<string, unknown>; parts: Uint8Array[] } | null = null;
    outgoing.set(socket, { frames: [], waiting: false });
    socket.onClose(() => {
      peers.delete(socket);
      outgoing.delete(socket);
    });
    socket.onMessage((message) => {
      const frame = JSON.parse(String(message));
      if (frame.type === 'ack') {
        for (const cursor of frame.cursors)
          expect(entries[Number(cursor.seq) - 1].envelopeHash).toBe(cursor.envelopeHash);
        const state = outgoing.get(socket);
        if (state) {
          state.waiting = false;
          pump(socket);
        }
        return;
      }
      queue = queue.then(async () => {
        expect(frame.space).toBe(v.space);
        expect(frame.page).toBe(v.page);
        expect(frame.epoch).toBe('1');
        if (frame.type === 'hello') {
          hellos++;
          const statements =
            frame.membershipRevision === '0'
              ? [c.encodeBinary(g.toJson()), c.encodeBinary(shared.toJson())]
              : [];
          send(socket, 'catchup', {
            membershipHead: {
              revision: '2',
              statementHash: c.encodeBinary(head.head.hash),
              ownerKey: c.encodeBinary(owner),
              statements,
              more: false,
            },
            baseline: null,
            streams: [],
            more: true,
          });
          send(socket, 'catchup', {
            chains: [{ deviceId: v.device, chain: c.encodeBinary(json(chain)) }],
            wraps: [c.encodeBinary(json(v.wrap))],
            streams: [],
            more: true,
          });
          for (const row of entries) {
            const bytes = c.binary(row.envelope, 400 * 1024),
              id = c.decodeHeader(c.Envelope.fromJson(bytes).header()).objectId;
            send(socket, 'catchup', {
              streams: [
                {
                  streamId: v.device,
                  namespace: 'content',
                  checkpoint: null,
                  tail: [
                    { ...row, envelope: bytes.length > 32768 ? { objectId: id } : row.envelope },
                  ],
                },
              ],
              more: true,
            });
            if (bytes.length > 32768) {
              const count = Math.ceil(bytes.length / 32768);
              for (let index = 0; index < count; index++)
                send(socket, 'chunk', {
                  objectId: id,
                  envelopeHash: row.envelopeHash,
                  index,
                  count,
                  bytes: c.encodeBinary(bytes.slice(index * 32768, (index + 1) * 32768)),
                });
            }
          }
          send(socket, 'catchup', { streams: [], more: false });
          peers.add(socket);
          return;
        }
        if (frame.type === 'append' && typeof frame.envelope !== 'string') {
          pending = { frame, parts: [] };
          chunked++;
          return;
        }
        if (frame.type === 'chunk') {
          expect(pending).not.toBeNull();
          expect(frame.index).toBe(pending!.parts.length);
          pending!.parts.push(c.binary(frame.bytes, 32768));
          if (pending!.parts.length !== frame.count) return;
          pending!.frame.envelope = c.encodeBinary(c.concat(...pending!.parts));
          await append(pending!.frame);
          pending = null;
          return;
        }
        expect(frame.type).toBe('append');
        await append(frame);
      });
      async function append(frame: Record<string, unknown>) {
        const row = {
            seq: frame.seq as string,
            envelopeHash: frame.envelopeHash as string,
            envelope: frame.envelope as string,
          },
          seq = Number(row.seq),
          env = c.Envelope.fromJson(c.binary(row.envelope, 400 * 1024)),
          h = c.decodeHeader(env.header()).context;
        expect(c.encodeBinary(await env.hash())).toBe(row.envelopeHash);
        expect(
          await c.strictVerify(
            owner,
            env.signature(),
            await c.signatureInput(env.header(), env.ciphertext()),
          ),
        ).toBe(true);
        if (seq <= entries.length) {
          expect(row).toEqual(entries[seq - 1]);
          retries++;
        } else {
          expect(seq).toBe(entries.length + 1);
          expect(h.prevHash).toEqual(c.binary(entries.at(-1)!.envelopeHash, 32, 32));
          entries.push(row);
        }
        if (drop) {
          drop = false;
          peers.delete(socket);
          socket.close({ code: 1011 });
        } else
          send(socket, 'receipt', {
            streamId: v.device,
            seq: row.seq,
            envelopeHash: row.envelopeHash,
          });
        if (seq === entries.length && row === entries.at(-1))
          for (const peer of peers) deliver(peer, 'broadcast', row, { streamId: v.device });
      }
    });
  });
  return {
    entries,
    get hellos() {
      return hellos;
    },
    resync() {
      for (const peer of peers) send(peer, 'error', { code: 'RESYNC_REQUIRED' });
    },
    async unsupportedOwn() {
      const env = await c.Envelope.seal(
        {
          space: v.space,
          page: v.page,
          epoch: '1',
          kind: 'update',
          namespace: 'own',
          authorDevice: v.device,
          membershipRevision: '2',
          streamSeq: String(entries.length + 1),
          prevHash: c.binary(entries.at(-1)!.envelopeHash, 32, 32),
        },
        hex(v.epochKey),
        signer,
        new Uint8Array([0]),
      );
      const row = await entry(env);
      for (const peer of peers) deliver(peer, 'broadcast', row, { streamId: v.device });
    },
    get connections() {
      return peers.size;
    },
    get retries() {
      return retries;
    },
    get chunked() {
      return chunked;
    },
    dropNext() {
      drop = true;
    },
    async settled() {
      await queue;
    },
  };
}

test('two same-device tabs edit one durable stream, chunk, reload and retry exact accepted bytes', async ({
  page,
  context,
}) => {
  const f = await wire(context),
    other = await context.newPage();
  for (const tab of [page, other]) {
    await tab.goto(mount);
    await tab.getByRole('link', { name: new RegExp(v.page) }).click();
    await expect(tab.getByRole('heading', { name: 'Live fixture', exact: true })).toBeVisible();
    await tab.getByRole('button', { name: 'Source', exact: true }).click();
  }
  expect(
    await page.evaluate(
      async () =>
        (await navigator.locks.query()).held?.filter((lock) => lock.name?.startsWith('writer:'))
          .length,
    ),
  ).toBe(1);
  await expect(
    page.frameLocator('iframe').getByRole('heading', { name: 'Live fixture' }),
  ).toBeVisible();
  await page.screenshot({ path: '/tmp/1252-live-light.png', fullPage: true });
  await page.getByRole('button', { name: 'Change color theme' }).click();
  await expect(page.locator('iframe')).toHaveCSS('background-color', 'rgb(255, 255, 255)');
  await expect(page.locator('iframe')).toHaveCSS('color-scheme', 'light');
  await page.screenshot({ path: '/tmp/1252-live-dark.png', fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({ path: '/tmp/1252-live-mobile.png', fullPage: true });
  await page.setViewportSize({ width: 1280, height: 720 });
  await page.getByRole('button', { name: 'Change color theme' }).click();
  await page.getByRole('textbox').fill('<h1>First</h1>');
  await page.getByRole('button', { name: 'Save source' }).click();
  await expect(other.getByRole('textbox')).toHaveValue('<h1>First</h1>');
  await expect(other.frameLocator('iframe').getByRole('heading', { name: 'First' })).toBeVisible();
  const large = '<h1>Large</h1>' + 'x'.repeat(50_000);
  await other.getByRole('textbox').fill(large);
  await other.getByRole('button', { name: 'Save source' }).click();
  await expect(page.getByRole('textbox')).toHaveValue(large);
  expect(f.chunked).toBe(1);
  f.dropNext();
  await other.getByRole('textbox').fill('<h1>Recovered</h1>');
  await other.getByRole('button', { name: 'Save source' }).click();
  await expect(other.getByRole('alert')).toContainText('edit was not saved');
  await other.reload();
  await other.getByRole('button', { name: 'Source', exact: true }).click();
  await expect(other.getByRole('textbox')).toHaveValue('<h1>Recovered</h1>');
  await other.getByRole('textbox').fill('<h1>After reload</h1>');
  await other.getByRole('button', { name: 'Save source' }).click();
  await expect(page.getByRole('textbox')).toHaveValue('<h1>After reload</h1>');
  await expect(other.getByRole('button', { name: 'Save source' })).toBeDisabled();
  await f.settled();
  expect(f.retries).toBe(1);
  expect(f.entries.map((row) => row.seq)).toEqual(['1', '2', '3', '4', '5']);
  const before = f.hellos;
  f.resync();
  await expect.poll(() => f.hellos).toBe(before + 2);
  await expect(
    other.frameLocator('iframe').getByRole('heading', { name: 'After reload' }),
  ).toBeVisible();
  await f.unsupportedOwn();
  for (const tab of [page, other]) {
    await expect(tab.getByRole('alert')).toContainText('own-namespace loading is not available');
    await expect(tab.locator('iframe')).toHaveCount(0);
    await expect(tab.getByRole('textbox')).toHaveJSProperty('readOnly', true);
  }
  await expect.poll(() => f.connections).toBe(0);
  await Promise.all(
    [page, other].map((tab) => tab.getByRole('link', { name: 'Space home' }).click()),
  );
  await expect.poll(() => f.connections).toBe(0);
  await expect
    .poll(() =>
      page.evaluate(
        async () =>
          (await navigator.locks.query()).held?.filter((lock) => lock.name?.startsWith('writer:'))
            .length,
      ),
    )
    .toBe(0);
});

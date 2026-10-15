import { pageAction } from '../test/page-actions.js';
import { readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import {
  expect,
  test,
  type BrowserContext,
  type Locator,
  type Page,
  type WebSocketRoute,
} from '@playwright/test';
import { capturePath } from './captures.js';
import * as c from '@tmt/colab-client';
import type { PageView } from '../src/transport.js';
import { text as copy } from '../src/strings.js';
import * as Y from 'yjs'; // Test-only producer. Foreign update decoding stays in the app Worker.
const v = JSON.parse(
  readFileSync(new URL('../../../contracts/vectors/authority-v1.json', import.meta.url), 'utf8'),
);
const ownVector = JSON.parse(
  readFileSync(new URL('../../../contracts/vectors/own-v1.json', import.meta.url), 'utf8'),
);
const hex = (s: string) => Uint8Array.from(s.match(/../g) ?? [], (n) => parseInt(n, 16));
const json = (value: unknown) => c.text(JSON.stringify(value));
const mount = '/r/abcd/x/colab/';
interface Hunk {
  oldStart: number;
  old: string;
  new: string;
}
/** The serve's evaluation in short: each hunk's old text is on the page once, at a line start.
 * The merged page, or the indices of the hunks that do not fit. */
function placeHunks(page: string, hunks: Hunk[]): string | number[] {
  const at = hunks.map((hunk) => {
    const found: number[] = [];
    for (let i = page.indexOf(hunk.old); i >= 0; i = page.indexOf(hunk.old, i + 1))
      if (i === 0 || page[i - 1] === '\n') found.push(i);
    return found.length === 1 ? found[0] : -1;
  });
  const missing = at.flatMap((place, index) => (place < 0 ? [index] : []));
  if (missing.length) return missing;
  let merged = page;
  for (const index of [...hunks.keys()].sort((x, y) => at[y] - at[x]))
    merged =
      merged.slice(0, at[index]) +
      hunks[index].new +
      merged.slice(at[index] + hunks[index].old.length);
  return merged;
}
async function wire(
  context: BrowserContext,
  reset?: { source: string; invalid?: 'commitment' | 'source' | 'descriptor' | 'oldEpoch' },
  compacted?: { invalid?: 'prefix' | 'n' | 'namespace' | 'body' | 'gap' | 'ownBody' | 'ownTail' },
  statementTransfer?: 'valid' | 'hash',
  pageTitle = 'Live fixture',
  largeTail = false,
) {
  const epoch = reset ? '2' : '1';
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
  let head = await shared.verifyNext(v.space, owner, genesis.head);
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
      body: `export async function reopenSession(){sessionStorage.setItem('test:reopens',String(Number(sessionStorage.getItem('test:reopens')??0)+1));await window.fixtureKeys;} export function transportUrl(_session,url){return String(url);} export async function certifyKey(purpose,bytes){return {publicKey:btoa(String.fromCharCode(...bytes)).replaceAll('+','-').replaceAll('/','_').replace(/=+$/,''),issuedAtMs:Date.now(),signature:'${c.encodeBinary(new Uint8Array(64))}'}}`,
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
        revision: String(head.head.revision),
        pageIds: [{ pageId: v.page, deleted: false }],
        pages: [
          {
            pageId: v.page,
            epoch,
            sharing: 'private',
            history: 'current',
            archived: false,
            retentionDays: 30,
            lastUpdateAtMs: null,
            expiresAtMs: null,
            warnings: ['expiry-unavailable'],
            labels: [],
          },
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
  doc.getText('html').insert(0, reset?.source ?? '<h1>Live fixture</h1>');
  doc.getMap('meta').set('title', pageTitle);
  let baseline: Record<string, string> | null = null;
  let baselineEnvelope: c.Envelope | null = null;
  let advance: c.statement.Envelope | null = null;
  let epochWrap = v.wrap;
  if (reset) {
    const update = new Uint8Array(Y.encodeStateAsUpdate(doc));
    baselineEnvelope = await c.Envelope.seal(
      {
        space: v.space,
        page: v.page,
        epoch,
        kind: 'html',
        namespace: 'content',
        authorDevice: genesis.head.ownerMember.id,
        membershipRevision: '3',
        streamSeq: '0',
        prevHash: new Uint8Array(32),
      },
      hex(v.epochKey),
      signer,
      json({
        source: reset.source + (reset.invalid === 'source' ? 'changed' : ''),
        update: c.encodeBinary(update),
      }),
    );
    baseline = {
      pageId: v.page,
      epoch,
      sourceDigest: c.encodeBinary(await c.digest(c.text(reset.source))),
      baselineCommitment: c.encodeBinary(
        reset.invalid === 'commitment'
          ? new Uint8Array(32)
          : await c.digest(
              c.frame(c.text('tmt-colab-baseline-v1'), c.text('1'), c.text(reset.source), update),
            ),
      ),
      title: pageTitle,
      objectEnvelopeHash: c.encodeBinary(await baselineEnvelope.hash()),
      membershipRevision: '3',
    };
    // Generated once by the existing Rust browser_authority example using public
    // authority-v1 seeds, with the wrap header changed to epoch 2 / revision 3.
    epochWrap = JSON.parse(readFileSync(new URL('./baseline-wrap.json', import.meta.url), 'utf8'));
    const value = json({ pageId: v.page, epoch, cuts: [], baseline, wraps: [epochWrap] });
    const input = c.statement.input({
      space: v.space,
      operation: 'epoch.advance',
      revision: '3',
      previousHash: head.head.hash,
      payloadDigest: await c.digest(value),
    });
    advance = c.statement.Envelope.fromJson(
      json({
        statement: c.encodeBinary(input),
        payload: c.encodeBinary(value),
        signature: c.encodeBinary(await c.sign(signer, input)),
      }),
    );
    head = await advance.verifyNext(v.space, owner, head.head);
  }
  let largeStatement: c.statement.Envelope | null = null;
  if (statementTransfer) {
    // Another page's large transition leaves this fixture's requested epoch intact.
    const pageId = '00000000-0000-4000-8000-000000000099',
      revision = String(head.head.revision + 1n),
      value = json({
        pageId,
        epoch: '2',
        cuts: [],
        wraps: [],
        baseline: {
          pageId,
          epoch: '2',
          membershipRevision: revision,
          title: 'x'.repeat(250 * 1024),
          sourceDigest: c.encodeBinary(new Uint8Array(32)),
          baselineCommitment: c.encodeBinary(new Uint8Array(32)),
          objectEnvelopeHash: c.encodeBinary(new Uint8Array(32)),
        },
      }),
      input = c.statement.input({
        space: v.space,
        operation: 'epoch.advance',
        revision,
        previousHash: head.head.hash,
        payloadDigest: await c.digest(value),
      });
    largeStatement = c.statement.Envelope.fromJson(
      json({
        statement: c.encodeBinary(input),
        payload: c.encodeBinary(value),
        signature: c.encodeBinary(await c.sign(signer, input)),
      }),
    );
    head = await largeStatement.verifyNext(v.space, owner, head.head);
    expect(largeStatement.toJson().length).toBeGreaterThan(64 * 1024);
  }
  const initial = await c.Envelope.seal(
    {
      space: v.space,
      page: v.page,
      epoch: reset?.invalid === 'oldEpoch' ? '1' : epoch,
      kind: 'update',
      namespace: 'content',
      authorDevice: v.device,
      membershipRevision: String(head.head.revision),
      streamSeq: '1',
      prevHash: new Uint8Array(32),
    },
    hex(v.epochKey),
    signer,
    reset ? new Uint8Array([0, 0]) : new Uint8Array(Y.encodeStateAsUpdate(doc)),
  );
  // The content every peer holds, so a save replaces exactly what the browser shows.
  const liveContent = new Y.Doc();
  if (!compacted) Y.applyUpdate(liveContent, new Uint8Array(Y.encodeStateAsUpdate(doc)));
  const entry = async (env: c.Envelope) => ({
    seq: c.decodeHeader(env.header()).context.streamSeq,
    envelopeHash: c.encodeBinary(await env.hash()),
    envelope: c.encodeBinary(env.toJson()),
  });
  const entries = [await entry(initial)],
    peers = new Set<WebSocketRoute>();
  let tailBytes = 0;
  if (largeTail) {
    let previous = await initial.hash();
    for (let index = 0; index < 205; index++) {
      const vector = Y.encodeStateVector(doc);
      doc.getText('html').insert(doc.getText('html').length, 'é' + 'x'.repeat(1598));
      const update = new Uint8Array(Y.encodeStateAsUpdate(doc, vector));
      tailBytes += update.length;
      Y.applyUpdate(liveContent, update);
      const env = await c.Envelope.seal(
        {
          space: v.space,
          page: v.page,
          epoch,
          kind: 'update',
          namespace: 'content',
          authorDevice: v.device,
          membershipRevision: String(head.head.revision),
          streamSeq: String(index + 2),
          prevHash: previous,
        },
        hex(v.epochKey),
        signer,
        update,
      );
      entries.push(await entry(env));
      previous = await env.hash();
    }
  }
  doc.destroy();
  const checkpoints: { namespace: string; row: (typeof entries)[number] }[] = [];
  if (compacted) {
    const cp = JSON.parse(
      readFileSync(
        new URL('../../../contracts/vectors/checkpoint-v1.json', import.meta.url),
        'utf8',
      ),
    );
    const writer = new Y.Doc();
    Y.applyUpdate(writer, c.binary(cp.checkpoint, 256 * 1024));
    const before = Y.encodeStateVector(writer);
    writer.getText('html').insert(writer.getText('html').length, 'x'.repeat(180_000));
    const firstPadding = Y.encodeStateAsUpdate(writer, before),
      middle = Y.encodeStateVector(writer);
    writer.getText('html').insert(writer.getText('html').length, 'x'.repeat(120_000));
    const secondPadding = Y.encodeStateAsUpdate(writer, middle),
      padding = Y.mergeUpdates([firstPadding, secondPadding]);
    Y.applyUpdate(liveContent, Y.mergeUpdates([c.binary(cp.checkpoint, 256 * 1024), padding]));
    Y.applyUpdate(liveContent, c.binary(cp.tail, 256 * 1024));
    writer.destroy();
    entries.length = 0;
    let previous = new Uint8Array(32);
    const prefixUpdates = [
      c.binary(cp.contentUpdates[0], 256 * 1024),
      new Uint8Array([0, 0]),
      Y.mergeUpdates([c.binary(cp.contentUpdates[1], 256 * 1024), firstPadding]),
      new Uint8Array([0, 0]),
      secondPadding,
      new Uint8Array([0, 0]),
    ];
    for (let index = 0; index < 6; index++) {
      const env = await c.Envelope.seal(
        {
          space: v.space,
          page: v.page,
          epoch,
          kind: 'update',
          namespace: index % 2 === 0 ? 'content' : 'own',
          authorDevice: v.device,
          membershipRevision: '2',
          streamSeq: String(index + 1),
          prevHash: previous,
        },
        hex(v.epochKey),
        signer,
        new Uint8Array(prefixUpdates[index]),
      );
      entries.push(await entry(env));
      previous = await env.hash();
    }
    for (const namespace of ['content', 'own']) {
      const env = await c.Envelope.seal(
        {
          space: v.space,
          page: v.page,
          epoch,
          kind: 'checkpoint',
          namespace:
            compacted.invalid === 'namespace' && namespace === 'own'
              ? 'content'
              : (namespace as 'content' | 'own'),
          authorDevice: v.device,
          membershipRevision: '2',
          streamSeq: compacted.invalid === 'n' && namespace === 'own' ? '5' : '6',
          prevHash:
            compacted.invalid === 'prefix' && namespace === 'own'
              ? new Uint8Array(32).fill(7)
              : previous,
        },
        hex(v.epochKey),
        signer,
        namespace === 'content'
          ? compacted.invalid === 'body'
            ? new Uint8Array([255])
            : new Uint8Array(Y.mergeUpdates([c.binary(cp.checkpoint, 256 * 1024), padding]))
          : compacted.invalid === 'ownBody'
            ? new Uint8Array([255])
            : c.binary(ownVector.checkpoint, 256 * 1024),
      );
      checkpoints.push({ namespace, row: await entry(env) });
    }
    for (const [namespace, bytes] of [
      ['content', c.binary(cp.tail, 256 * 1024)],
      [
        'own',
        compacted.invalid === 'ownTail'
          ? new Uint8Array([255])
          : c.binary(ownVector.tail, 256 * 1024),
      ],
    ] as const) {
      const env = await c.Envelope.seal(
        {
          space: v.space,
          page: v.page,
          epoch,
          kind: 'update',
          namespace,
          authorDevice: v.device,
          membershipRevision: '2',
          streamSeq: String(entries.length + 1),
          prevHash: previous,
        },
        hex(v.epochKey),
        signer,
        bytes,
      );
      entries.push(await entry(env));
      previous = await env.hash();
    }
  }
  const otherDevice = '00000000-0000-4000-8000-000000000126';
  const otherSigner = await crypto.subtle.generateKey('Ed25519', false, ['sign', 'verify']);
  const otherCertificate = c.certificate.input({
    space: v.space,
    issuerKind: 'member',
    issuerId: genesis.head.ownerMember.id,
    deviceId: otherDevice,
    signingKey: new Uint8Array(await crypto.subtle.exportKey('raw', otherSigner.publicKey)),
    encryptionKey: c.wrap.Envelope.fromJson(json(v.wrap)).header().recipientKey,
    membershipRevision: '1',
    issuedAt: Date.now() - 1000,
    expiresAt: Date.now() + 86400000,
  });
  const otherChain = {
    version: 1,
    issuerStatement: c.encodeBinary(genesis.head.hash),
    deviceCertificate: c.encodeBinary(otherCertificate),
    issuerSignature: c.encodeBinary(await c.sign(signer, otherCertificate)),
  };
  const otherObject = compacted
    ? await entry(
        await c.Envelope.seal(
          {
            space: v.space,
            page: v.page,
            epoch,
            kind: 'update',
            namespace: 'own',
            authorDevice: otherDevice,
            membershipRevision: '2',
            streamSeq: '1',
            prevHash: new Uint8Array(32),
          },
          hex(v.epochKey),
          otherSigner.privateKey,
          c.binary(ownVector.checkpoint, 256 * 1024),
        ),
      )
    : null;
  const scope = { version: 1, space: v.space, page: v.page, epoch };
  let queue = Promise.resolve(),
    drop = false,
    retries = 0,
    chunked = 0,
    hellos = 0,
    statementChunks = 0,
    statusChecks = 0;
  // Saves the fixture committed, by operation ID, so a lost reply can be settled by status.
  const saved = new Map<string, string>();
  // The source of the last save: the next save's base is that, or its hunks are placed on it.
  let lastSource: string | null = null;
  // Every save frame the fixture received, in order.
  const saveFrames: Record<string, unknown>[] = [];
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
  let accepting = true;
  await context.routeWebSocket(`**${mount}sync`, (socket) => {
    if (!accepting) {
      socket.close({ code: 1011 });
      return;
    }
    let pending: { frame: Record<string, unknown>; parts: Uint8Array[] } | null = null;
    let upload: { frame: Record<string, unknown>; parts: Uint8Array[] } | null = null;
    outgoing.set(socket, { frames: [], waiting: false });
    socket.onClose((code, reason) => {
      peers.delete(socket);
      outgoing.delete(socket);
      // Installing onClose disables Playwright's default close forwarding.
      void socket.close({ code, reason });
    });
    socket.onMessage((message) => {
      const frame = JSON.parse(String(message));
      if (frame.type === 'subscribe') {
        expect(frame.cursors).toEqual([]);
        return;
      }
      if (frame.type === 'ack') {
        for (const cursor of frame.cursors) {
          if (cursor.streamId === otherDevice) {
            expect(cursor.namespace).toBe('own');
            expect(cursor.seq).toBe('1');
            expect(cursor.envelopeHash).toBe(otherObject?.envelopeHash);
            continue;
          }
          const checkpoint = checkpoints.find(
            (x) =>
              x.namespace === cursor.namespace &&
              x.row.seq === cursor.seq &&
              x.row.envelopeHash === cursor.envelopeHash,
          );
          expect(checkpoint?.row.envelopeHash ?? entries[Number(cursor.seq) - 1].envelopeHash).toBe(
            cursor.envelopeHash,
          );
        }
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
        expect(frame.epoch).toBe(epoch);
        if (frame.type === 'hello') {
          hellos++;
          const statements =
            frame.membershipRevision === '0'
              ? [g, shared, ...(advance ? [advance] : [])].map((e) => c.encodeBinary(e.toJson()))
              : [];
          send(socket, 'catchup', {
            membershipHead: {
              revision: String(head.head.revision),
              statementHash: c.encodeBinary(head.head.hash),
              ownerKey: c.encodeBinary(owner),
              statements,
              more: statements.length > 0 && largeStatement !== null,
            },
            baseline: baseline
              ? c.encodeBinary(
                  json(
                    reset?.invalid === 'descriptor'
                      ? { ...baseline, title: 'substituted' }
                      : baseline,
                  ),
                )
              : null,
            ...(baselineEnvelope
              ? {
                  baselineObject: {
                    envelopeHash: baseline!.objectEnvelopeHash,
                    envelope:
                      baselineEnvelope.toJson().length > 32768
                        ? { objectId: c.decodeHeader(baselineEnvelope.header()).objectId }
                        : c.encodeBinary(baselineEnvelope.toJson()),
                  },
                }
              : {}),
            streams: [],
            more: true,
          });
          if (baselineEnvelope && baselineEnvelope.toJson().length > 32768) {
            const bytes = baselineEnvelope.toJson(),
              count = Math.ceil(bytes.length / 32768);
            for (let index = 0; index < count; index++)
              send(socket, 'chunk', {
                objectId: c.decodeHeader(baselineEnvelope.header()).objectId,
                envelopeHash: baseline!.objectEnvelopeHash,
                index,
                count,
                bytes: c.encodeBinary(bytes.slice(index * 32768, (index + 1) * 32768)),
              });
          }
          if (statements.length && largeStatement) {
            const bytes = largeStatement.toJson(),
              count = Math.ceil(bytes.length / 32768),
              statementHash =
                statementTransfer === 'hash'
                  ? c.encodeBinary(new Uint8Array(32))
                  : c.encodeBinary(await largeStatement.hash());
            send(socket, 'catchup', {
              membership: { statements: [{ statementHash }], more: false },
              streams: [],
              more: true,
            });
            for (let index = 0; index < count; index++) {
              statementChunks++;
              send(socket, 'chunk', {
                statementHash,
                index,
                count,
                bytes: c.encodeBinary(bytes.slice(index * 32768, (index + 1) * 32768)),
              });
            }
          }
          send(socket, 'catchup', {
            chains: [
              { deviceId: v.device, chain: c.encodeBinary(json(chain)) },
              ...(otherObject
                ? [{ deviceId: otherDevice, chain: c.encodeBinary(json(otherChain)) }]
                : []),
            ],
            wraps: [c.encodeBinary(json(epochWrap))],
            streams: [],
            more: true,
          });
          const objects = [
            ...checkpoints.map((x) => ({
              streamId: v.device,
              row: x.row,
              namespace: x.namespace,
              checkpoint: true,
            })),
            ...entries.slice(compacted ? (compacted.invalid === 'gap' ? 7 : 6) : 0).map((row) => ({
              row,
              streamId: v.device,
              namespace: c.decodeHeader(
                c.Envelope.fromJson(c.binary(row.envelope, 400 * 1024)).header(),
              ).context.namespace,
              checkpoint: false,
            })),
          ];
          if (otherObject)
            objects.push({
              streamId: otherDevice,
              row: otherObject,
              namespace: 'own',
              checkpoint: false,
            });
          for (const { streamId, row, namespace, checkpoint } of objects) {
            const bytes = c.binary(row.envelope, 400 * 1024),
              id = c.decodeHeader(c.Envelope.fromJson(bytes).header()).objectId;
            send(socket, 'catchup', {
              streams: [
                {
                  streamId,
                  namespace,
                  checkpoint: checkpoint
                    ? { ...row, envelope: bytes.length > 32768 ? { objectId: id } : row.envelope }
                    : null,
                  tail: checkpoint
                    ? []
                    : [
                        {
                          ...row,
                          envelope: bytes.length > 32768 ? { objectId: id } : row.envelope,
                        },
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
        if (frame.type === 'save') {
          expect(upload).toBeNull();
          if (typeof frame.source === 'string') {
            await finishSave(socket, frame, c.binary(frame.source, 32768));
            return;
          }
          upload = { frame, parts: [] };
          return;
        }
        if (frame.type === 'savestatus') {
          statusChecks++;
          send(socket, 'saveresult', {
            operationId: frame.operationId,
            ...(saved.has(frame.operationId)
              ? { state: 'committed', revision: saved.get(frame.operationId) }
              : { state: 'absent' }),
          });
          return;
        }
        if (frame.type === 'chunk' && upload) {
          expect(frame.objectId).toBe((upload.frame.source as { objectId: string }).objectId);
          expect(frame.envelopeHash).toBe(upload.frame.sourceSha256);
          expect(frame.index).toBe(upload.parts.length);
          upload.parts.push(c.binary(frame.bytes, 32768));
          if (upload.parts.length !== frame.count) return;
          const done = upload;
          upload = null;
          await finishSave(socket, done.frame, c.concat(...done.parts));
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
        if (frame.type === 'usage') {
          // The page-size surface asks when a panel opens; this serve holds a nearly empty page.
          send(socket, 'usageresult', {
            usage: {
              updates: 1,
              updateLimit: 200,
              tailBytes: 1024,
              tailBytesLimit: 4 * 1024 * 1024,
              checkpointBytes: 0,
              otherDeviceUpdates: 0,
              stateBytes: 1024,
              gzipLimit: 5_000_000,
              percent: 0,
              limiting: 'changes',
            },
          });
          return;
        }
        expect(frame.type).toBe('append');
        await append(frame);
      });
      // A save arrives whole: check its digests, publish it as the one update every peer sees,
      // then answer the originator, or drop the reply once to model a lost one.
      async function finishSave(
        socket: WebSocketRoute,
        frame: Record<string, unknown>,
        bytes: Uint8Array,
      ) {
        saveFrames.push(frame);
        expect(frame.sourceSha256).toBe(c.encodeBinary(await c.digest(bytes)));
        let source = new TextDecoder().decode(bytes);
        const operationId = frame.operationId as string;
        let merged = false;
        if (
          lastSource !== null &&
          frame.baseSha256 !== c.encodeBinary(await c.digest(c.text(lastSource)))
        ) {
          // The page moved since the editor opened it: like the real serve, place its hunks on
          // the page as it is now, or refuse them naming the ones that do not fit.
          const hunks = frame.hunks as Hunk[] | undefined;
          expect(hunks, 'a save from a moved page carries its hunks').toBeDefined();
          const placed = placeHunks(lastSource, hunks!);
          if (Array.isArray(placed)) {
            send(socket, 'saveresult', {
              operationId,
              state: 'rejected',
              code: 'COLAB_PATCH_CONFLICT',
              message: `Hunk ${placed[0] + 1} does not match the page.`,
              conflicts: placed,
            });
            return;
          }
          source = placed;
          merged = true;
        }
        lastSource = source;
        await fixture.contentUpdate(source);
        saved.set(operationId, String(entries.length));
        if (drop) {
          drop = false;
          peers.delete(socket);
          socket.close({ code: 1011 });
          return;
        }
        send(socket, 'saveresult', {
          operationId,
          state: 'committed',
          revision: saved.get(operationId),
          ...(merged
            ? { merged: true, sourceSha256: c.encodeBinary(await c.digest(c.text(source))) }
            : {}),
        });
      }
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
  const fixture = {
    head: head.head,
    entries,
    tailBytes,
    statement: largeStatement ? c.encodeBinary(largeStatement.toJson()) : null,
    get statementChunks() {
      return statementChunks;
    },
    get saveFrames() {
      return saveFrames;
    },
    get hellos() {
      return hellos;
    },
    stopSync() {
      accepting = false;
      for (const peer of peers) peer.close({ code: 1011 });
    },
    resumeSync() {
      accepting = true;
    },
    resync() {
      for (const peer of peers) send(peer, 'error', { code: 'RESYNC_REQUIRED' });
    },
    async contentUpdate(source: string) {
      const vector = Y.encodeStateVector(liveContent);
      const html = liveContent.getText('html');
      // The smallest replaced span, as native preparation publishes it.
      const old = html.toString();
      let start = 0;
      while (start < old.length && start < source.length && old[start] === source[start]) start++;
      let end = 0;
      while (
        end < old.length - start &&
        end < source.length - start &&
        old[old.length - 1 - end] === source[source.length - 1 - end]
      )
        end++;
      liveContent.transact(() => {
        html.delete(start, old.length - start - end);
        html.insert(start, source.slice(start, source.length - end));
      });
      const env = await c.Envelope.seal(
        {
          space: v.space,
          page: v.page,
          epoch,
          kind: 'update',
          namespace: 'content',
          authorDevice: v.device,
          membershipRevision: String(head.head.revision),
          streamSeq: String(entries.length + 1),
          prevHash: c.binary(entries.at(-1)!.envelopeHash, 32, 32),
        },
        hex(v.epochKey),
        signer,
        new Uint8Array(Y.encodeStateAsUpdate(liveContent, vector)),
      );
      const row = await entry(env);
      entries.push(row);
      for (const peer of peers) deliver(peer, 'broadcast', row, { streamId: v.device });
    },
    async ownUpdate() {
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
        c.binary(ownVector.checkpoint, 256 * 1024),
      );
      const row = await entry(env);
      entries.push(row);
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
    get statusChecks() {
      return statusChecks;
    },
  };
  return fixture;
}

async function recoverySdk(context: BrowserContext) {
  await context.route('**/sdk/remote-v1.js*', (route) =>
    route.fulfill({
      contentType: 'text/javascript',
      body: `export async function reopenSession(){
      sessionStorage.setItem('test:reopens',String(Number(sessionStorage.getItem('test:reopens')??0)+1));
      const response=await fetch('/test-recovery-open',{method:'POST'});
      if(!response.ok) throw new Error('No paired key or reopen unavailable');
      await window.fixtureKeys;
    }
    export function transportUrl(_session,url){return String(url);}
    export async function certifyKey(purpose,bytes){return {publicKey:btoa(String.fromCharCode(...bytes)).replaceAll('+','-').replaceAll('/','_').replace(/=+$/,''),issuedAtMs:Date.now(),signature:'${c.encodeBinary(new Uint8Array(64))}'}}`,
    }),
  );
}
const privateGuidance = `<!doctype html><html lang="en"><h1>TMT Colab</h1>
<p id="colab-recovery-status">Opening your paired browser…</p>
<p id="colab-guidance" hidden>This Colab space is private. Open it from a browser paired with tmt remote pair, or use a share link.</p>
<script type="module" src="./assets/recovery.js"></script></html>`;

test('paired guidance reopens once, reloads into the owner app and clears its recovery marker', async ({
  page,
  context,
}) => {
  await wire(context);
  await recoverySdk(context);
  let owner = false,
    opens = 0;
  await context.route('**/test-recovery-open', (route) => {
    opens++;
    owner = true;
    return route.fulfill({ json: {} });
  });
  await context.route(`**${mount}`, (route) =>
    owner ? route.continue() : route.fulfill({ contentType: 'text/html', body: privateGuidance }),
  );
  await page.goto(mount);
  await page.locator(`[data-page-id="${v.page}"] a`).click();
  await expect(
    page.frameLocator('iframe').getByRole('heading', { name: 'Live fixture' }),
  ).toBeVisible();
  expect(opens).toBe(2); // Guidance recovery, then normal authenticated registration.
  expect(
    await page.evaluate((path) => sessionStorage.getItem(`colab-recovery:${path}`), mount),
  ).toBeNull();
});

test('authenticated tail past write limits opens and renders exact source', async ({
  page,
  context,
}) => {
  test.setTimeout(90_000);
  const heading = '<h1>Live fixture</h1>';
  const source = heading + ('é' + 'x'.repeat(1598)).repeat(205);
  const f = await wire(context, undefined, undefined, undefined, undefined, true);
  expect(f.entries).toHaveLength(206);
  expect(f.tailBytes).toBeGreaterThan(256 * 1024);
  await page.goto(mount);
  await page.locator(`[data-page-id="${v.page}"] a`).click();
  await expect(
    page.frameLocator('iframe').getByRole('heading', { name: 'Live fixture' }),
  ).toBeVisible();
  await expect(page.frameLocator('iframe').locator('body')).toHaveText(
    'Live fixture' + source.slice(heading.length),
  );
  await (await pageAction(page, 'Source')).click();
  await expect(page.getByRole('textbox')).toHaveValue(source);
});

for (const mode of ['unpaired', 'failed', 'network', 'cookie-lost'] as const) {
  test(`private guidance ${mode} stays visible without an automatic reopen loop`, async ({
    page,
    context,
  }) => {
    await wire(context);
    await recoverySdk(context);
    let opens = 0;
    await context.route('**/test-recovery-open', (route) => {
      opens++;
      if (mode === 'network') return route.abort('connectionrefused');
      return route.fulfill({ status: mode === 'cookie-lost' ? 200 : 503, json: {} });
    });
    await context.route(`**${mount}`, (route) =>
      route.fulfill({ contentType: 'text/html', body: privateGuidance }),
    );
    await page.goto(mount);
    await expect(page.locator('#colab-guidance')).toBeVisible();
    expect(opens).toBe(1);
    expect(
      await page.evaluate((path) => sessionStorage.getItem(`colab-recovery:${path}`), mount),
    ).toBe('attempted');
    await page.reload();
    await expect(page.locator('#colab-guidance')).toBeVisible();
    expect(opens).toBe(1);
  });
}

test('a disconnected active tab replaces its session on its own, with no click and no reload', async ({
  page,
  context,
}) => {
  const f = await wire(context);
  await recoverySdk(context);
  let opens = 0;
  await context.route('**/test-recovery-open', (route) => {
    opens++;
    f.resumeSync();
    return route.fulfill({ json: {} });
  });
  await page.goto(mount);
  await page.locator(`[data-page-id="${v.page}"] a`).click();
  const heading = page.frameLocator('iframe').getByRole('heading', { name: 'Live fixture' });
  await expect(heading).toBeVisible();
  f.stopSync();
  await page.screenshot({
    path: capturePath('colab-reconnect.png'),
    fullPage: true,
  });
  // The connection owner admits one replacement after its first wait, without a click or a reload.
  await expect.poll(() => opens, { timeout: 15_000 }).toBe(2);
  await expect(heading).toBeVisible();
  await expect(page.getByRole('button', { name: 'Reconnect', exact: true })).toHaveCount(0);
  await expect.poll(() => f.connections).toBe(1);
  await page.getByRole('link', { name: 'Space home' }).click();
  await expect.poll(() => f.connections).toBe(0);
});

test('Ask publishes owner own envelopes through production Connection before Remote dispatch', async ({
  page,
  context,
}, testInfo) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const f = await wire(context),
    agentId = '00000000-0000-4000-8000-000000000006',
    requestId = 'req_00000000-0000-4000-8000-000000000007',
    reply = 'Wire reply <script>inert</script>';
  let sends = 0;
  let operationId: string | null = null;
  let opens = 0;
  let doorDown = false;
  let oldSessionEnded = false;
  let terminalReplacement = false;
  let pendingOpen: Promise<void> | undefined;
  let releaseOpen: (() => void) | undefined;
  // Only Remote and the signed sync server are doubles: registration, controller,
  // Worker preparation, Writer, Connection, receipts and projection are production.
  await context.route('**/sdk/remote-v1.js*', (route) =>
    route.fulfill({
      contentType: 'text/javascript',
      body: `export class RefusalError extends Error {constructor(code){super(code);this.code=code;}}
export async function reopenSession(){const response=await fetch('/test-wire-reopen',{method:'POST'});if(!response.ok)throw new RefusalError('REMOTE_DEVICE_REVOKED');await window.fixtureKeys;return {sessionId:'fixture-session',serverTimeMs:Date.now(),grantRevision:'1',expiresAtMs:null}}
export async function certifyKey(purpose,bytes){return {publicKey:btoa(String.fromCharCode(...bytes)).replaceAll('+','-').replaceAll('/','_').replace(/=+$/,''),issuedAtMs:Date.now(),signature:'${c.encodeBinary(new Uint8Array(64))}'}}
export function transportUrl(_session,url){return String(url);}
export function operations(){return {
listAgents:async()=>{const response=await fetch('/test-wire-directory');if(response.status===410)throw new RefusalError('REMOTE_SESSION_ENDED');return [{id:'${agentId}',name:'Wire agent',presence:'active'}]},
send:async(input)=>(await fetch('/test-wire-send',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(input)})).json(),
operation:async(operationId)=>({state:'accepted',operationId,requestId:'${requestId}'}),
result:async()=>({state:'replied',requestId:'${requestId}',message:${JSON.stringify(reply)}})
}}`,
    }),
  );
  await context.route('**/test-wire-reopen', async (route) => {
    opens++;
    if (pendingOpen) await pendingOpen;
    if (terminalReplacement) return route.fulfill({ status: 403, json: {} });
    if (doorDown) return route.abort('connectionrefused');
    oldSessionEnded = false;
    f.resumeSync();
    return route.fulfill({ json: {} });
  });
  await context.route('**/test-wire-directory', (route) =>
    route.fulfill({ status: oldSessionEnded ? 410 : 200, json: {} }),
  );
  await context.route('**/sdk/mount', (route) =>
    route.fulfill({
      json: {
        machineId: '00000000-0000-4000-8000-000000000005',
        windowId: 'fixture',
        address: 'fixture',
        extension: 'colab',
        mount,
      },
    }),
  );
  await context.route('**/test-wire-send', async (route) => {
    const input = route.request().postDataJSON();
    await f.settled();
    const own = new Y.Doc();
    try {
      for (const row of f.entries.slice(1)) {
        const envelope = c.Envelope.fromJson(c.binary(row.envelope, 400 * 1024)),
          header = c.decodeHeader(envelope.header()).context;
        expect(header.namespace).toBe('own');
        expect(header.authorDevice).toBe(v.device);
        Y.applyUpdate(own, await envelope.open(header, hex(v.epochKey), hex(v.public)));
      }
      const intent = own.getMap('intents').get(input.operationId) as {
        signed: { signature: string; input: string; finalBytes: string };
      };
      expect(intent).toBeDefined();
      expect(
        await c.strictVerify(
          hex(v.public),
          c.binary(intent.signed.signature, 64, 64),
          c.binary(intent.signed.input, 16 * 1024),
        ),
      ).toBe(true);
      expect(c.decodeText(c.binary(intent.signed.finalBytes, 64 * 1024))).toBe(input.message);
      expect([...own.getMap('messages').values()]).toContainEqual(
        expect.objectContaining({ operationId: input.operationId, state: 'dispatching' }),
      );
      expect(input.agentId).toBe(agentId);
      expect(input.message).toContain('Explain this page.');
      operationId = input.operationId;
      sends++;
      await route.fulfill({ json: { state: 'accepted', operationId, requestId } });
    } finally {
      own.destroy();
    }
  });
  await page.goto(mount);
  await page.locator(`[data-page-id="${v.page}"] a`).click();
  const heading = page.frameLocator('iframe').getByRole('heading', { name: 'Live fixture' });
  await expect(heading).toBeVisible();
  const renderId = (await page.locator('iframe').getAttribute('data-render-id'))!;
  await page
    .frames()
    .find((value) => value.url().endsWith('/renderer.html'))!
    .evaluate((renderId) => {
      parent.postMessage(
        {
          type: 'colab.render.selection',
          renderId,
          text: 'Cosmetic claim',
          selector: { exact: 'Cosmetic claim', prefix: '', suffix: '' },
          rect: { x: -1000000, y: -1000000, width: 1, height: 1 },
        },
        '*',
      );
    }, renderId);
  const bubble = page.getByTestId('selection-ask');
  await expect(bubble).toBeVisible();
  const bounds = (await bubble.boundingBox())!,
    frameBounds = (await page.locator('iframe').boundingBox())!;
  expect(bounds.x).toBeGreaterThanOrEqual(frameBounds.x);
  expect(bounds.y).toBeGreaterThanOrEqual(frameBounds.y);
  expect(bounds.x + bounds.width).toBeLessThanOrEqual(frameBounds.x + frameBounds.width);
  expect(bounds.y + bounds.height).toBeLessThanOrEqual(page.viewportSize()!.height);
  await expect(page.getByTestId('annotation-compose')).toHaveCount(0);
  await heading.evaluate((node) => {
    const range = document.createRange();
    range.selectNodeContents(node);
    getSelection()!.removeAllRanges();
    getSelection()!.addRange(range);
  });
  await (await pageAction(page, 'Chat')).click();
  const input = page.getByTestId('chat-panel').getByRole('combobox', { name: 'Message' });
  await input.fill('@');
  await page.getByRole('option').click();
  await input.fill('@Wire agent Explain this page.');
  await expect(page.getByTestId('chat-panel').locator('details')).toHaveCount(0);
  await input.press('Enter');
  await expect(page.getByTestId('ask-reply')).toHaveText(reply);
  expect(sends).toBe(1);
  await expect(page.getByTestId('ask-entry')).toHaveAttribute('data-operation-id', operationId!);
  await expect(page.getByTestId('ask-state')).toHaveCount(0);
  await expect(page.getByTestId('ask-entry')).toHaveAttribute('data-ledger-state', 'accepted');
  await expect(page.getByTestId('ask-reply-attribution')).toContainText('Wire agent');
  await expect(page.getByTestId('ask-panel').locator('script')).toHaveCount(0);
  await expect(heading).toHaveText('Live fixture');
  await f.settled();
  expect(f.entries.map((row) => row.seq)).toEqual(['1', '2', '3', '4', '5', '6']);
  await page.reload();
  await (await pageAction(page, 'Chat')).click();
  await expect(page.getByTestId('ask-reply')).toHaveText(reply);
  await expect(page.getByTestId('ask-entry')).toHaveAttribute('data-operation-id', operationId!);
  await expect(heading).toHaveText('Live fixture');
  await f.settled();
  expect(sends).toBe(1);
  expect(f.entries).toHaveLength(6);
  let draft = 'Keep this composer draft while the door is down.';
  await input.fill(draft);
  const admittedOpens = opens;
  doorDown = true;
  oldSessionEnded = true;
  f.stopSync();
  // The connection owner keeps trying on its own (#2572): the chip says so, and no card or button
  // interrupts. The page, the composer draft and the answered thread stay where they were.
  await expect(page.locator('.status')).toHaveText('Private · Reconnecting…');
  await expect.poll(() => opens).toBeGreaterThanOrEqual(admittedOpens + 1);
  await expect(page.getByRole('heading', { name: 'Connection lost', exact: true })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Reconnect', exact: true })).toHaveCount(0);
  await expect(page.locator('.tmt-ui-notice')).toHaveCount(0);
  await expect(page.getByText('Failed to fetch', { exact: true })).toHaveCount(0);
  await expect(heading).toBeVisible();
  await expect(input).toHaveText(draft);
  await expect(page.getByTestId('ask-entry')).toHaveAttribute('data-operation-id', operationId!);
  await expect(page.getByTestId('ask-entry')).toHaveAttribute('data-ledger-state', 'accepted');
  await expect(page.getByTestId('ask-reply')).toHaveText(reply);
  expect(sends).toBe(1);
  expect(f.entries).toHaveLength(6);
  await expect(input).toBeFocused(); // Recovery must not take focus from a draft.
  await expect(input).toHaveAttribute('contenteditable', 'true');
  draft += ' Still editable.';
  await input.fill(draft);
  await expect(
    page.getByRole('button', { name: /^Send(?: · to .+)?$/, exact: true }),
  ).toBeDisabled();
  await expect(
    page
      .getByTestId('chat-panel')
      .getByRole('status')
      .filter({ hasText: /^Reconnecting…$/ }),
  ).toBeVisible();
  await expect(page.getByTestId('chat-panel')).not.toContainText(copy.sendOnHold);
  await input.press('Enter');
  expect(sends).toBe(1);
  await f.settled();
  expect(f.entries).toHaveLength(6);
  async function capture(state: string) {
    for (const theme of ['light', 'dark']) {
      await page.evaluate((theme) => {
        document.documentElement.dataset.theme = theme;
      }, theme);
      for (const width of [1440, 390]) {
        await page.setViewportSize({ width, height: width === 390 ? 844 : 900 });
        await page.screenshot({
          path: testInfo.outputPath(`${state}-${theme}-${width}.png`),
          fullPage: true,
        });
      }
    }
  }
  await capture('reconnecting');
  await page.setViewportSize({ width: 1440, height: 900 });
  doorDown = false;
  f.resumeSync();
  // The owner's next attempt lands without a click: it admits the replacement.
  await expect(heading).toBeVisible({ timeout: 20_000 });
  await expect(page.locator('.status.live .status-label')).toHaveText('Live', { timeout: 20_000 });
  await expect.poll(() => f.connections).toBe(1);
  await expect(page.getByTestId('chat-panel')).toBeVisible();
  await expect(page.getByTestId('ask-entry')).toHaveAttribute('data-operation-id', operationId!);
  await expect(page.getByTestId('ask-entry')).toHaveAttribute('data-ledger-state', 'accepted');
  await expect(page.getByTestId('ask-reply')).toHaveText(reply);
  // Silent attempts against the closed door are timing-dependent; the single admitted
  // connection above is the exactly-once check. One failed attempt and the admission are two opens.
  expect(opens).toBeGreaterThanOrEqual(admittedOpens + 2);
  await expect(input).toHaveText(draft);
  await input.fill('A new draft after verified recovery.');
  await expect(
    page.getByRole('button', { name: /^Send(?: · to .+)?$/, exact: true }),
  ).toBeEnabled();
  expect(sends).toBe(1);
  await f.settled();
  expect(f.entries).toHaveLength(6);
  // The anchored composer shares draft editing but cannot publish Ask or Post.
  await page.getByRole('button', { name: 'Close Chat', exact: true }).click();
  async function annotate() {
    await heading.evaluate((node) => {
      const range = node.ownerDocument.createRange();
      range.selectNodeContents(node);
      const selection = node.ownerDocument.getSelection()!;
      selection.removeAllRanges();
      selection.addRange(range);
    });
    await page.getByTestId('selection-ask').click();
  }
  await annotate();
  const annotation = page.getByRole('dialog', { name: 'Annotate selection' });
  const annotationInput = annotation.getByRole('combobox', { name: 'Message', exact: true });
  await annotationInput.fill('Keep this anchored draft. @Wire agent');
  const post = annotation.getByRole('button', { name: /^Send(?: · to .+)?$/, exact: true });
  const askAction = annotation.getByRole('button', { name: /^Send(?: · to .+)?$/, exact: true });
  await expect(post).toBeEnabled();
  await expect(askAction).toBeEnabled();
  // Successful mounted-session replacement rebinds this same draft to the new
  // Ask facade. It must not require closing and opening the annotation again.
  await annotationInput.evaluate((node) => {
    Object.assign(window, { recoveryDraftNode: node });
  });
  const beforeReplacement = opens;
  pendingOpen = new Promise<void>((resolve) => {
    releaseOpen = resolve;
  });
  oldSessionEnded = true;
  f.stopSync();
  f.resumeSync();
  await expect.poll(() => opens).toBe(beforeReplacement + 1);
  await expect(askAction).toBeDisabled();
  await expect(annotationInput).toHaveText('Keep this anchored draft. @Wire agent');
  releaseOpen!();
  pendingOpen = undefined;
  await expect.poll(() => f.connections).toBe(1);
  await expect(heading).toBeVisible();
  await expect(post).toBeEnabled();
  await expect(askAction).toBeEnabled();
  await expect(annotationInput).toBeFocused();
  expect(
    await annotationInput.evaluate(
      (node) => node === (window as unknown as { recoveryDraftNode: Element }).recoveryDraftNode,
    ),
  ).toBe(true);
  await expect(annotationInput).toHaveText('Keep this anchored draft. @Wire agent');

  doorDown = true;
  oldSessionEnded = true;
  f.stopSync();
  await expect(page.locator('.status')).toHaveText('Private · Reconnecting…');
  await expect(annotationInput).toBeFocused();
  await expect(annotationInput).toHaveAttribute('contenteditable', 'true');
  await expect(annotation.getByRole('status').filter({ hasText: /^Reconnecting…$/ })).toBeVisible();
  await expect(annotation).not.toContainText(copy.sendOnHold);
  await annotationInput.fill('Still editable anchored draft.');
  await expect(post).toBeDisabled();
  await expect(askAction).toBeDisabled();
  await annotationInput.press('Enter');
  await expect(annotationInput).toHaveText('Still editable anchored draft.');
  expect(sends).toBe(1);
  await f.settled();
  expect(f.entries).toHaveLength(6);
  doorDown = false;
  f.resumeSync();
  await expect(heading).toBeVisible({ timeout: 20_000 });
  await expect.poll(() => f.connections).toBe(1);
  await expect(annotationInput).toHaveText('Still editable anchored draft.');
  await expect(annotationInput).toBeFocused();
  expect(
    await annotationInput.evaluate(
      (node) => node === (window as unknown as { recoveryDraftNode: Element }).recoveryDraftNode,
    ),
  ).toBe(true);
  await expect(post).toBeEnabled();
  await expect(askAction).toBeEnabled();
  await annotationInput.press('Escape');
  // An authority refusal is terminal, with a distinct blocked presentation.
  terminalReplacement = true;
  oldSessionEnded = true;
  f.stopSync();
  await expect(page.getByRole('heading', { name: 'Preview stopped', exact: true })).toBeVisible();
  await expect(page.locator('.tmt-ui-notice')).toHaveAttribute('data-tone', 'blocked');
  // The body is a sentence; the raw token is only the small reference under it.
  await expect(page.locator('.tmt-ui-notice')).toContainText(copy.failureAccessEnded);
  await expect(page.locator('[data-failure-reference]')).toHaveText('REMOTE_DEVICE_REVOKED');
  await expect(page.locator('.failure-reference')).toContainText(copy.failureCodeLabel);
  await expect(page.getByRole('button', { name: 'Reconnect', exact: true })).toHaveCount(0);
  // The one action fits the reason: pairing again, with its command.
  await expect(page.locator('.tmt-ui-notice')).toContainText(copy.pairAgain);
  await expect(page.locator('.tmt-ui-notice')).toContainText(copy.reconnectCommand);
  await capture('terminal-refusal');
  await page.getByRole('link', { name: 'Space home' }).click();
  await expect.poll(() => f.connections).toBe(0);
});

/** The mounted/registration/Connection/Worker/Writer path is production; only
 * the signed server and Remote protocol are fixtures, as in the Ask wire case. */
async function draftRecoveryWire(context: BrowserContext) {
  const f = await wire(context);
  const agent = '00000000-0000-4000-8000-000000000006';
  let down = false,
    ended = false,
    terminal = false,
    capped = false,
    opens = 0,
    sends = 0;
  let pending: Promise<void> | undefined, release: (() => void) | undefined;
  await context.route('**/sdk/remote-v1.js*', (route) =>
    route.fulfill({
      contentType: 'text/javascript',
      body: `export class RefusalError extends Error {constructor(code,retryAfterMs,limit,settingsUrl){super(code);this.name='RefusalError';this.code=code;this.limit=limit;this.settingsUrl=settingsUrl;}}
export class ReopenSessionError extends Error {constructor(reason,detail,cause){super(reason);this.name='ReopenSessionError';this.reason=reason;this.cause=cause;}}
export async function reopenSession(previous,options){const response=await fetch('/test-draft-reopen',{method:'POST'});if(response.status===429){const body=await response.json();const refusal=new RefusalError('REMOTE_SESSION_LIMIT',undefined,body.limit,body.settingsUrl);throw options&&options.retry==='bounded'?new ReopenSessionError('capacity','admission-refused',refusal):refusal}if(!response.ok)throw new RefusalError('REMOTE_DEVICE_REVOKED');await window.fixtureKeys;return {sessionId:'fixture-session',serverTimeMs:Date.now(),grantRevision:'1',expiresAtMs:null}}
export async function certifyKey(purpose,bytes){return {publicKey:btoa(String.fromCharCode(...bytes)).replaceAll('+','-').replaceAll('/','_').replace(/=+$/,''),issuedAtMs:Date.now(),signature:'${c.encodeBinary(new Uint8Array(64))}'}}
export function transportUrl(_session,url){return String(url);}
export function operations(){return {
listAgents:async()=>{const response=await fetch('/test-draft-directory');if(response.status===410)throw new RefusalError('REMOTE_SESSION_ENDED');return [{id:'${agent}',name:'Draft agent',presence:'active'}]},
send:async(input)=>(await fetch('/test-draft-send',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(input)})).json(),
operation:async(operationId)=>({state:'accepted',operationId,requestId:'req_00000000-0000-4000-8000-000000000007'}),
result:async()=>({state:'replied',requestId:'req_00000000-0000-4000-8000-000000000007',message:'Draft recovery reply'})
}}`,
    }),
  );
  await context.route('**/test-draft-reopen', async (route) => {
    opens++;
    if (pending) await pending;
    if (capped)
      return route.fulfill({
        status: 429,
        json: { limit: 2, settingsUrl: 'https://127.0.0.1:9000/settings' },
      });
    if (terminal) return route.fulfill({ status: 403, json: {} });
    if (down) return route.abort('connectionrefused');
    ended = false;
    f.resumeSync();
    return route.fulfill({ json: {} });
  });
  await context.route('**/test-draft-directory', (route) =>
    route.fulfill({ status: ended ? 410 : 200, json: {} }),
  );
  await context.route('**/sdk/mount', (route) =>
    route.fulfill({
      json: {
        machineId: '00000000-0000-4000-8000-000000000005',
        windowId: 'fixture',
        address: 'fixture',
        extension: 'colab',
        mount,
      },
    }),
  );
  await context.route('**/test-draft-send', async (route) => {
    const input = route.request().postDataJSON();
    expect(input.agentId).toBe(agent);
    await f.settled();
    expect(f.entries.length).toBeGreaterThan(1); // Fresh Writer publication preceded dispatch.
    sends++;
    return route.fulfill({
      json: {
        state: 'accepted',
        operationId: input.operationId,
        requestId: 'req_00000000-0000-4000-8000-000000000007',
      },
    });
  });
  return {
    ...f,
    get connections() {
      return f.connections;
    },
    get opens() {
      return opens;
    },
    get sends() {
      return sends;
    },
    end() {
      terminal = true;
      ended = true;
      f.stopSync();
    },
    /** The device is at its Remote session limit: every fresh open is refused until `free()`. */
    cap() {
      capped = true;
      ended = true;
      f.stopSync();
    },
    free() {
      capped = false;
      ended = false;
      f.resumeSync();
    },
    stop() {
      down = true;
      ended = true;
      f.stopSync();
    },
    resume() {
      down = false;
      f.resumeSync();
    },
    hold() {
      pending = new Promise<void>((resolve) => {
        release = resolve;
      });
    },
    release() {
      release!();
      pending = undefined;
    },
  };
}

for (const width of [1440, 390])
  for (const surface of ['Chat', 'annotation'] as const)
    for (const selected of [false, true]) {
      test(`automatic recovery keeps ${surface} ${selected ? 'selection' : 'caret'} and mounted draft at ${width}`, async ({
        page,
        context,
      }) => {
        await page.setViewportSize({ width, height: 900 });
        const f = await draftRecoveryWire(context);
        await page.goto(mount);
        await page.locator(`[data-page-id="${v.page}"] a`).click();
        const heading = page.frameLocator('iframe').getByRole('heading', { name: 'Live fixture' });
        await expect(heading).toBeVisible();
        async function openChat() {
          if (await page.locator('.tmt-ui-drawer[data-panel=chat][open]').isVisible()) return;
          const toggle = await pageAction(page, 'Chat');
          await toggle.click();
        }
        if (surface === 'Chat') await openChat();
        else {
          await heading.evaluate((node) => {
            const range = document.createRange();
            range.selectNodeContents(node);
            const selection = getSelection()!;
            selection.removeAllRanges();
            selection.addRange(range);
          });
          await page.getByTestId('selection-ask').click();
        }
        const container =
          surface === 'Chat'
            ? page.getByTestId('chat-panel')
            : page.getByRole('dialog', { name: 'Annotate selection' });
        const input = container.getByRole('combobox', { name: 'Message', exact: true });
        const action = container.getByRole('button', {
          name: /^Send(?: · to .+)?$/,
          exact: true,
          includeHidden: true,
        });
        await input.pressSequentially('@');
        await page.getByRole('option').filter({ hasText: '@Draft agent ·' }).click();
        const draft = 'Keep this exact recovery draft. @Draft agent';
        await input.fill(draft);
        await input.evaluate((node, selected) => {
          const text = document.createTreeWalker(node, NodeFilter.SHOW_TEXT).nextNode()!;
          const range = document.createRange();
          range.setStart(text, 5);
          range.setEnd(text, selected ? 9 : 5);
          const selection = getSelection()!;
          selection.removeAllRanges();
          selection.addRange(range);
        }, selected);
        await input.evaluate((node) => {
          Object.assign(window, { recoveryDraftNode: node });
        });
        const selection = () =>
          input.evaluate((node) => {
            const selection = getSelection()!;
            return {
              anchor: selection.anchorOffset,
              focus: selection.focusOffset,
              text: selection.toString(),
              inside: node.contains(selection.anchorNode) && node.contains(selection.focusNode),
            };
          });
        const before = await selection();
        expect(before).toEqual({
          anchor: 5,
          focus: selected ? 9 : 5,
          text: selected ? draft.slice(5, 9) : '',
          inside: true,
        });
        f.stop();
        const admittedOpens = f.opens;
        const navigations: string[] = [];
        page.on('framenavigated', (frame) => {
          if (frame === page.mainFrame()) navigations.push(frame.url());
        });
        // The connection owner's first attempt is one fetch; with the door down it fails and the
        // same read-only draft stays.
        await expect.poll(() => f.opens, { timeout: 15_000 }).toBe(admittedOpens + 1);
        await expect(page.locator('.status')).toHaveText('Private · Reconnecting…');
        await expect(input).toHaveText(draft);
        await expect(action).toBeDisabled();
        if (surface === 'annotation')
          await expect(
            container.getByRole('button', { name: /^Send(?: · to .+)?$/, exact: true }),
          ).toBeDisabled();
        // The next automatic attempt is admitted by a fresh replacement, held while we look.
        f.resume();
        f.hold();
        await expect.poll(() => f.opens, { timeout: 15_000 }).toBe(admittedOpens + 2);
        await expect(action).toBeDisabled();
        if (surface === 'Chat') await openChat();
        await input.focus();
        await expect.poll(selection).toEqual(before);
        await expect(input).toHaveText(draft);
        expect(f.sends).toBe(0);
        await f.settled();
        expect(f.entries).toHaveLength(1);
        f.release();
        await expect(page.locator('.status.live .status-label')).toHaveText('Live');
        await expect(heading).toBeVisible();
        if (surface === 'Chat') await openChat();
        await input.focus();
        await expect.poll(selection).toEqual(before);
        expect(
          await input.evaluate(
            (node) =>
              node === (window as unknown as { recoveryDraftNode: Element }).recoveryDraftNode,
          ),
        ).toBe(true);
        await expect(input).toHaveText(draft);
        await expect(action).toBeEnabled();
        expect(navigations).toEqual([]);
        expect(f.sends).toBe(0);
        await f.settled();
        expect(f.entries).toHaveLength(1);
        await input.press('X');
        const edited = draft.slice(0, 5) + 'X' + draft.slice(selected ? 9 : 5);
        await expect(input).toHaveText(edited);
        if (surface === 'annotation') {
          const post = container.getByRole('button', { name: /^Send(?: · to .+)?$/, exact: true });
          await expect(post).toBeEnabled();
          const plain = edited.replace(' @Draft agent', '');
          await input.fill(plain);
          await post.click();
          await expect(input).toHaveText('');
          await expect(container.getByText(plain, { exact: true })).toBeVisible();
          expect(f.sends).toBe(0);
          await input.fill('@Draft agent Explicit Ask after verified recovery.');
        }
        await action.click();
        await expect.poll(() => f.sends).toBe(1);
        await expect(input).toHaveText('');
        await expect(page.getByTestId('ask-reply')).toHaveText('Draft recovery reply');
        if (surface === 'Chat')
          await page.getByRole('button', { name: 'Close Chat', exact: true }).click();
        await page.getByRole('link', { name: 'Space home' }).click();
        await expect.poll(() => f.connections).toBe(0);
      });
    }

/** The control is the topmost element at its own center: nothing sits over it. */
async function expectUncovered(control: Locator) {
  await expect(control).toBeVisible();
  await expect
    .poll(
      () =>
        control.evaluate((node) => {
          const box = node.getBoundingClientRect();
          const top = document.elementFromPoint(box.x + box.width / 2, box.y + box.height / 2);
          return !top || !(node === top || node.contains(top));
        }),
      { timeout: 1000 },
    )
    .toBe(false);
}

for (const surface of ['Chat', 'annotation'] as const) {
  test(`ended connection copy owns the ${surface} composer and keeps its draft`, async ({
    page,
    context,
  }, testInfo) => {
    const f = await draftRecoveryWire(context);
    await page.goto(mount);
    await page.locator(`[data-page-id="${v.page}"] a`).click();
    const heading = page.frameLocator('iframe').getByRole('heading', { name: 'Live fixture' });
    await expect(heading).toBeVisible();
    if (surface === 'Chat') await (await pageAction(page, 'Chat')).click();
    else {
      await heading.evaluate((node) => {
        const range = document.createRange();
        range.selectNodeContents(node);
        const selection = getSelection()!;
        selection.removeAllRanges();
        selection.addRange(range);
      });
      await page.getByTestId('selection-ask').click();
    }
    const container =
      surface === 'Chat'
        ? page.getByTestId('chat-panel')
        : page.getByRole('dialog', { name: 'Annotate selection' });
    const input = container.getByRole('combobox', { name: 'Message', exact: true });
    const draft = 'Keep this draft. @Draft agent';
    await input.fill(draft);
    f.end();
    const status = container.getByTestId('annotation-compose').getByRole('status');
    await expect(status).toContainText(copy.failureAccessEnded);
    await expect(container.getByText(copy.failureAccessEnded, { exact: true })).toHaveCount(1);
    await expect(status).toContainText(copy.pairAgain);
    await expect(status).toContainText(copy.reconnectCommand);
    await expect(status).not.toContainText(copy.messageAgentsUnavailable);
    await expect(status).not.toContainText(copy.sendOnHold);
    await expect(
      container.getByRole('button', { name: /^Send(?: · to .+)?$/, exact: true }),
    ).toBeDisabled();
    await expect(input).toHaveText(draft);
    expect(f.sends).toBe(0);
    await f.settled();
    expect(f.entries).toHaveLength(1);
    for (const width of [1440, 390]) {
      await page.setViewportSize({ width, height: width === 390 ? 844 : 900 });
      const command = status.locator('.tmt-ui-command');
      const scroll = container.locator('.conversation-composer');
      await scroll.evaluate((node) => {
        node.scrollTop = node.scrollHeight;
      });
      const inside = async () => {
        const action = (await command.boundingBox())!,
          area = (await scroll.boundingBox())!;
        return action.y >= area.y && action.y + action.height <= area.y + area.height + 1;
      };
      await expect.poll(inside).toBe(true);
      if (surface === 'annotation') {
        await scroll.evaluate((node) => {
          node.scrollTop = 0;
        });
        await container.getByRole('button', { name: copy.threadCollapse, exact: true }).focus();
        await page.keyboard.press('Tab');
        await expect(status).toBeFocused();
        // Focus alignment varies by browser; keyboard scrolling must expose the action.
        await status.press('End');
        await expect.poll(inside).toBe(true);
      }
      await page.screenshot({
        path: testInfo.outputPath(`ended-${surface}-${width}.png`),
        fullPage: true,
      });
    }
    if (surface === 'Chat') {
      await (await pageAction(page, 'Source')).click();
      const source = page.locator('.tmt-ui-drawer[data-panel=source]');
      const notice = source.locator('.drawer-recovery');
      await expect(notice).toContainText(copy.failureAccessEnded);
      await expect(notice).toContainText(copy.reconnectCommand);
      await expect(source.getByText(copy.failureAccessEnded, { exact: true })).toHaveCount(1);
    }
  });
}

// Remote never evicts an attached session (#2585): at the device cap a fresh open is refused, and
// the page says so, names the one command that raises the limit, and a reload opens it once a
// session frees (#2625).
test('a tab refused at the session limit names the limit and opens on reload once one frees', async ({
  page,
  context,
}) => {
  const f = await draftRecoveryWire(context);
  f.cap();
  await page.goto(mount);
  const card = page.locator('main');
  await expect(card.getByRole('heading', { name: copy.sessionLimitTitle })).toBeVisible();
  await expect(card).toContainText(copy.sessionLimitClose);
  await expect(card).toContainText(copy.sessionLimitReached(2));
  await expect(card).toContainText('tmt remote settings sessions-per-device 3');
  // The cause comes first, then the fixes, so the two remedies sit together.
  const order = (await card.innerText()).replace(/\s+/g, ' ');
  const at = (part: string) => order.indexOf(part);
  expect(at(copy.sessionLimitReached(2))).toBeGreaterThan(-1);
  expect(at(copy.sessionLimitClose)).toBeGreaterThan(at(copy.sessionLimitReached(2)));
  expect(at(copy.sessionLimitCommand)).toBeGreaterThan(at(copy.sessionLimitClose));
  // The settings link looks actionable, like the Reload text action.
  const link = card.getByRole('link', { name: copy.remoteSettings });
  expect(await link.evaluate((node) => getComputedStyle(node).textDecorationLine)).toContain(
    'underline',
  );
  await expect(card.getByRole('link', { name: copy.remoteSettings })).toHaveAttribute(
    'href',
    'https://127.0.0.1:9000/settings',
  );
  await expect(card).not.toContainText(copy.registrationFailed);
  for (const width of [1440, 390])
    for (const theme of ['light', 'dark'] as const) {
      await page.setViewportSize({ width, height: width === 390 ? 844 : 900 });
      await page.emulateMedia({ colorScheme: theme });
      await page.screenshot({ path: await capturePath(`2625-limit-boot-${width}-${theme}.png`) });
    }
  f.free();
  await card.getByRole('button', { name: copy.reload, exact: true }).click();
  await expect(page.locator(`[data-page-id="${v.page}"] a`)).toBeVisible();
});

for (const surface of ['Chat', 'annotation'] as const)
  test(`an open page refused at the session limit ends with the cap card, keeps the ${surface} draft and reloads`, async ({
    page,
    context,
  }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    const f = await draftRecoveryWire(context);
    await page.goto(mount);
    await page.locator(`[data-page-id="${v.page}"] a`).click();
    const heading = page.frameLocator('iframe').getByRole('heading', { name: 'Live fixture' });
    await expect(heading).toBeVisible();
    if (surface === 'Chat') await (await pageAction(page, 'Chat')).click();
    else {
      await heading.evaluate((node) => {
        const range = document.createRange();
        range.selectNodeContents(node);
        const selection = getSelection()!;
        selection.removeAllRanges();
        selection.addRange(range);
      });
      await page.getByTestId('selection-ask').click();
    }
    const container =
      surface === 'Chat'
        ? page.getByTestId('chat-panel')
        : page.getByRole('dialog', { name: 'Annotate selection' });
    const input = container.getByRole('combobox', { name: 'Message', exact: true });
    const draft = 'Keep this draft. @Draft agent';
    await input.fill(draft);
    f.cap();
    const status = container.getByTestId('annotation-compose').getByRole('status');
    await expect(status).toContainText(copy.sessionLimitClose);
    await expect(status).toContainText(copy.sessionLimitReached(2));
    await expect(status).toContainText('tmt remote settings sessions-per-device 3');
    await expect(status.getByRole('link', { name: copy.remoteSettings })).toBeVisible();
    await expect(status).not.toContainText(copy.failureGeneric);
    await expect(
      container.getByRole('button', { name: /^Send(?: · to .+)?$/, exact: true }),
    ).toBeDisabled();
    await expect(input).toHaveText(draft);
    expect(f.sends).toBe(0);
    // The status keeps its whole body and Reload reachable: a keyboard reader scrolls it to its
    // top and to its end. The annotation popover is a few lines tall, so it is captured at both.
    const shot = (name: string) =>
      page.screenshot({ path: capturePath(`2625-limit-${surface}-${name}.png`) });
    const focusStatus = async () => {
      await container.getByRole('button', { name: copy.threadCollapse, exact: true }).focus();
      await page.keyboard.press('Tab');
      await expect(status).toBeFocused({ timeout: 1000 });
    };
    const reach = async (name?: string) => {
      if (surface === 'annotation' && name) {
        await expect(async () => {
          await focusStatus();
          // The status starts below the draft: bring its first sentence into the window.
          await status.evaluate((node) => node.scrollIntoView({ block: 'start' }));
          await expect(status.getByText(copy.sessionLimitReached(2))).toBeInViewport({
            timeout: 1000,
          });
        }).toPass({ timeout: 10_000 });
        await shot(`${name}-start`);
      }
      // The popover re-lays out after a resize and keyboard scrolling settles after the key
      // press, so the whole reach is retried until Reload is on screen.
      await expect(async () => {
        if (surface === 'annotation') {
          await focusStatus();
          await status.press('End');
        }
        await expectUncovered(status.getByRole('button', { name: copy.reload, exact: true }));
      }).toPass({ timeout: 10_000 });
      if (name) await shot(surface === 'annotation' ? `${name}-end` : name);
    };
    for (const theme of ['light', 'dark'] as const) {
      await page.emulateMedia({ colorScheme: theme });
      await reach(`1440-${theme}`);
    }
    await page.setViewportSize({ width: 390, height: 844 });
    await reach('390-dark');
    if (surface === 'annotation') {
      // With room below the selection the whole body and Reload fit the popover at once.
      await page.setViewportSize({ width: 1440, height: 1500 });
      await reach();
      await expect(status.getByText(copy.sessionLimitReached(2))).toBeInViewport();
      await shot('1440-tall');
    }
    await page.setViewportSize({ width: 1440, height: 900 });
    await reach();
    // The card's one action is Reload: once a session frees, it opens the page again.
    f.free();
    await status.getByRole('button', { name: copy.reload, exact: true }).click();
    await expect(
      page.frameLocator('iframe').getByRole('heading', { name: 'Live fixture' }),
    ).toBeVisible();
    await expect(page.getByText(copy.sessionLimitClose)).toHaveCount(0);
  });

// A tab that outlives an upgrade shows one row directly under the fixed header; the page
// moves down by its height, so no control (Chat composer, annotate window) is covered.
test('an upgraded server shows the update row under the header without covering page controls', async ({
  page,
  context,
}, testInfo) => {
  const built = (hash: string) => `/assets/index-${hash}.js`;
  const [oldEntry, newEntry] = [built('OLDOLD12'), built('NEWNEW34')];
  await draftRecoveryWire(context);
  let loaded = false;
  await context.route(`**${mount}`, async (route) => {
    const first = route.request().isNavigationRequest() && !loaded;
    loaded ||= route.request().isNavigationRequest();
    const response = await route.fetch();
    const html = await response.text();
    await route.fulfill({
      response,
      body: html.replace(
        /\/src\/main\.tsx(?:\?[^"']*)?/,
        `${first ? oldEntry : newEntry}?fixture=upgrade`,
      ),
    });
  });
  for (const entry of [oldEntry, newEntry])
    // Vite may retain an HMR timestamp on the rewritten entry URL.
    await context.route(
      (url) => url.pathname === entry,
      (route) => route.fulfill({ contentType: 'text/javascript', body: "import '/src/main.tsx';" }),
    );
  await page.goto(mount);
  await page.locator(`[data-page-id="${v.page}"] a`).click();
  const heading = page.frameLocator('iframe').getByRole('heading', { name: 'Live fixture' });
  await expect(heading).toBeVisible();
  const row = page.locator('[data-update-notice]');
  await expect(row).toHaveText(`${copy.updated}${copy.reload}`);
  await expect(row).toHaveAttribute('role', 'status');
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    const rowBox = (await row.boundingBox())!;
    const headerBox = (await page.locator('header.tmt-ui-header').boundingBox())!;
    // Directly under the header bar, full width, one line, pushing the page down.
    expect(rowBox.y).toBeCloseTo(headerBox.y + headerBox.height, 0);
    expect(rowBox.width).toBeCloseTo(width, 0);
    expect(rowBox.height).toBe(40);
    const mainBox = (await page.locator('main').boundingBox())!;
    expect(mainBox.y).toBeGreaterThanOrEqual(rowBox.y + rowBox.height - 1);
    // Chat open: the composer's actions stay reachable, not under the row.
    const toggle = await pageAction(page, 'Chat');
    if (!(await page.locator('.tmt-ui-drawer[data-panel=chat][open]').isVisible())) {
      await toggle.click();
    }
    const panel = page.getByTestId('chat-panel');
    await expect(panel).toBeVisible();
    // At 390 the drawer is a full-screen modal above header and row alike.
    if (width === 1440)
      expect((await panel.boundingBox())!.y).toBeGreaterThanOrEqual(rowBox.y + rowBox.height - 1);
    await expectUncovered(panel.getByRole('button', { name: /^Send(?: · to .+)?$/, exact: true }));
    for (const theme of ['light', 'dark']) {
      await page.evaluate((value) => (document.documentElement.dataset.theme = value), theme);
      await page.screenshot({ path: testInfo.outputPath(`update-chat-${theme}-${width}.png`) });
    }
    await page.getByRole('button', { name: 'Close Chat', exact: true }).click();
    // The annotate window opens below the row as well.
    await heading.evaluate((node) => {
      const range = document.createRange();
      range.selectNodeContents(node);
      const selection = getSelection()!;
      selection.removeAllRanges();
      selection.addRange(range);
    });
    await page.getByTestId('selection-ask').click();
    const dialog = page.getByRole('dialog', { name: 'Annotate selection' });
    await expect(dialog).toBeVisible();
    if (width === 1440)
      expect((await dialog.boundingBox())!.y).toBeGreaterThanOrEqual(rowBox.y + rowBox.height - 1);
    await expectUncovered(dialog.getByRole('button', { name: /^Send(?: · to .+)?$/, exact: true }));
    for (const theme of ['light', 'dark']) {
      await page.evaluate((value) => (document.documentElement.dataset.theme = value), theme);
      await page.screenshot({ path: testInfo.outputPath(`update-annotate-${theme}-${width}.png`) });
    }
    await page.keyboard.press('Escape');
  }
});

test('same-device tabs stay connected and exchange source edits without reopening', async ({
  page,
  context,
}) => {
  const f = await wire(context);
  const other = await context.newPage();
  const reopens = (tab: typeof page) =>
    tab.evaluate(() => Number(sessionStorage.getItem('test:reopens') ?? 0));
  for (const tab of [page, other]) {
    await tab.goto(mount);
    await tab.locator(`[data-page-id="${v.page}"] a`).click();
    await expect(
      tab.frameLocator('iframe').getByRole('heading', { name: 'Live fixture' }),
    ).toBeVisible();
    await (await pageAction(tab, 'Source')).click();
  }
  await expect.poll(() => f.connections).toBe(2);
  expect(await reopens(page)).toBe(1);
  expect(await reopens(other)).toBe(1);
  await page.getByRole('textbox').fill('<h1>From first tab</h1>');
  await page.getByRole('button', { name: 'Save source' }).click();
  await expect(other.getByRole('textbox')).toHaveValue('<h1>From first tab</h1>');
  await other.getByRole('textbox').fill('<h1>From second tab</h1>');
  await other.getByRole('button', { name: 'Save source' }).click();
  await expect(page.getByRole('textbox')).toHaveValue('<h1>From second tab</h1>');
  await expect(
    page.frameLocator('iframe').getByRole('heading', { name: 'From second tab' }),
  ).toBeVisible();
  await expect(
    other.frameLocator('iframe').getByRole('heading', { name: 'From second tab' }),
  ).toBeVisible();
  expect(await reopens(page)).toBe(1);
  expect(await reopens(other)).toBe(1);
  await f.settled();
  await page.close();
  await expect.poll(() => f.connections).toBe(1);
  await other.getByRole('textbox').fill('<h1>Still connected</h1>');
  await other.getByRole('button', { name: 'Save source' }).click();
  await expect(
    other.frameLocator('iframe').getByRole('heading', { name: 'Still connected' }),
  ).toBeVisible();
});

test('chunked epoch baseline stays live in two tabs through an edit and reload', async ({
  page,
  context,
}) => {
  const source = '<h1>Reset baseline</h1>' + 'x'.repeat(300_000);
  await wire(context, { source });
  const other = await context.newPage();
  for (const tab of [page, other]) {
    await tab.goto(mount);
    await tab.locator(`[data-page-id="${v.page}"] a`).click();
    await expect(
      tab.frameLocator('iframe').getByRole('heading', { name: 'Reset baseline' }),
    ).toBeVisible();
    await (await pageAction(tab, 'Source')).click();
    await expect(tab.getByRole('textbox')).toHaveValue(source);
  }
  const next = source.replace('Reset baseline', 'New epoch edit');
  await other.getByRole('textbox').fill(next);
  await other.getByRole('button', { name: 'Save source' }).click();
  await expect(
    other.frameLocator('iframe').getByRole('heading', { name: 'New epoch edit' }),
  ).toBeVisible();
  await expect(
    page.frameLocator('iframe').getByRole('heading', { name: 'New epoch edit' }),
  ).toBeVisible();
  await other.reload();
  await expect(
    other.frameLocator('iframe').getByRole('heading', { name: 'New epoch edit' }),
  ).toBeVisible();
  await expect(
    page.frameLocator('iframe').getByRole('heading', { name: 'New epoch edit' }),
  ).toBeVisible();
});
/** A page of one line per row, the heading first, so row N is line N + 1. */
const rowsPage = (changes: Record<number, string> = {}) =>
  [
    '<h1>Merged rows</h1>',
    ...Array.from({ length: 30 }, (_, i) => changes[i + 1] ?? `<p>row ${i + 1}</p>`),
    '',
  ].join('\n');

/** Two tabs open on the Source panel of the same page. */
async function twoSourceTabs(page: Page, context: BrowserContext, source: string) {
  const f = await wire(context, { source });
  const other = await context.newPage();
  for (const tab of [page, other]) {
    await tab.goto(mount);
    await tab.locator(`[data-page-id="${v.page}"] a`).click();
    await expect(
      tab.frameLocator('iframe').getByRole('heading', { name: 'Merged rows' }),
    ).toBeVisible();
    await (await pageAction(tab, 'Source')).click();
    await expect(tab.getByRole('textbox')).toHaveValue(source);
  }
  return { f, other };
}

test('a Save of a page that changed elsewhere is merged, says so once, and shows the merged source', async ({
  page,
  context,
}) => {
  const { f, other } = await twoSourceTabs(page, context, rowsPage());
  const mine = rowsPage({ 3: '<p>mine</p>' });
  const both = rowsPage({ 3: '<p>mine</p>', 25: '<p>theirs</p>' });
  await page.getByRole('textbox').fill(mine);
  await other.getByRole('textbox').fill(rowsPage({ 25: '<p>theirs</p>' }));
  await other.getByRole('button', { name: 'Save source' }).click();
  // The page renders their change while this draft, built on the page as opened, stays as typed.
  await expect(page.frameLocator('iframe').getByText('theirs')).toBeVisible();
  await expect(page.getByRole('textbox')).toHaveValue(mine);
  await page.getByRole('button', { name: 'Save source' }).click();
  const merged = page.getByRole('status').filter({ hasText: copy.saveMerged });
  await expect(merged).toBeVisible();
  await expect(page.getByRole('textbox')).toHaveValue(both);
  await expect(other.getByRole('textbox')).toHaveValue(both);
  await expect(page.getByRole('alert')).toHaveCount(0);
  await expect(page.frameLocator('iframe').getByText('mine')).toBeVisible();
  // One hunk with its context, anchored to the page as the editor opened it, and one frame sent.
  const sent = f.saveFrames.at(-1)!;
  expect(sent.hunks).toEqual([
    {
      oldStart: 1,
      old: '<h1>Merged rows</h1>\n<p>row 1</p>\n<p>row 2</p>\n<p>row 3</p>\n<p>row 4</p>\n<p>row 5</p>\n<p>row 6</p>\n',
      new: '<h1>Merged rows</h1>\n<p>row 1</p>\n<p>row 2</p>\n<p>mine</p>\n<p>row 4</p>\n<p>row 5</p>\n<p>row 6</p>\n',
    },
  ]);
  expect(f.saveFrames.filter((frame) => frame.operationId === sent.operationId)).toHaveLength(1);
  for (const [width, theme] of [
    [1440, 'light'],
    [390, 'dark'],
  ] as const) {
    await page.setViewportSize({ width, height: 900 });
    await page.emulateMedia({ colorScheme: theme });
    await page.screenshot({ path: capturePath(`2843-merged-${width}-${theme}.png`) });
  }
  // The line clears with the next edit.
  await page.getByRole('textbox').fill(`${both}<p>next</p>\n`);
  await expect(merged).toHaveCount(0);
});

test('a Save whose anchor changed elsewhere is refused naming it, keeps the draft and can copy it', async ({
  page,
  context,
}) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write']);
  const odd = `<p>${'Long '.repeat(20)}\u202e<b>x</b>\u0007 end</p>`;
  const { f, other } = await twoSourceTabs(page, context, rowsPage({ 10: odd }));
  const mine = rowsPage({ 5: '<p>my five</p>', 20: '<p>my twenty</p>', 28: '<p>my row</p>' });
  await page.getByRole('textbox').fill(mine);
  await other.getByRole('textbox').fill(
    rowsPage({
      5: '<p>their five</p>',
      10: '<p>their ten</p>',
      20: '<p>their twenty</p>',
    }),
  );
  await other.getByRole('button', { name: 'Save source' }).click();
  await expect(page.frameLocator('iframe').getByText('their five')).toBeVisible();
  await page.getByRole('button', { name: 'Save source' }).click();
  // Rows 5 and 20 conflict, row 28 does not: nothing is saved, and the first conflict is named by
  // the line the editor changed, as numbered in the source it opened, and quoted as plain text.
  const notice = page.getByRole('alert');
  await expect(notice).toContainText('Not saved');
  await expect(notice).toContainText(
    'Nothing was saved. 2 of your edits conflict with changes made elsewhere, the first near \u201c<p>row 5</p>\u201d (line 6).',
  );
  await expect(notice).toContainText('Your draft is kept.');
  await expect(page.getByRole('textbox')).toHaveValue(mine);
  expect(f.saveFrames).toHaveLength(2);
  // Pressing Save again is a new request with the same outcome, never an automatic resend.
  await page.getByRole('button', { name: 'Save source' }).click();
  await expect(notice).toContainText('Nothing was saved.');
  expect(f.saveFrames).toHaveLength(3);
  await expect(page.getByRole('textbox')).toHaveValue(mine);
  await page.getByRole('button', { name: 'Copy draft' }).click();
  await expect(page.getByRole('status').filter({ hasText: copy.saveDraftCopied })).toBeVisible();
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(mine);
  for (const [width, theme] of [
    [1440, 'light'],
    [390, 'dark'],
  ] as const) {
    await page.setViewportSize({ width, height: 900 });
    await page.emulateMedia({ colorScheme: theme });
    await page.screenshot({ path: capturePath(`2843-conflicts-${width}-${theme}.png`) });
  }
  // One conflict reads in the singular, and an excerpt with markup, bidi and control characters is
  // inert, clipped text on one line.
  await page.getByRole('textbox').fill(rowsPage({ 10: '<p>my ten</p>' }));
  await page.getByRole('button', { name: 'Save source' }).click();
  const clipped = `<p>${'Long '.repeat(20)}`.slice(0, 60);
  await expect(notice).toContainText(
    `Nothing was saved. Your edit near \u201c${clipped}\u2026\u201d (line 11) conflicts`,
  );
  await expect(notice.locator('b')).toHaveCount(0);
  const shown = await notice.innerText();
  expect(shown.includes('\u202e') || shown.includes('\u0007')).toBe(false);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(
    true,
  );
  for (const [width, theme] of [
    [1440, 'light'],
    [390, 'dark'],
  ] as const) {
    await page.setViewportSize({ width, height: 900 });
    await page.emulateMedia({ colorScheme: theme });
    await page.screenshot({ path: capturePath(`2843-conflict-${width}-${theme}.png`) });
  }
});

for (const invalid of ['commitment', 'source', 'descriptor', 'oldEpoch'] as const)
  test(`signed reset rejects ${invalid} without partial renderer publication`, async ({
    page,
    context,
  }) => {
    await wire(context, { source: '<h1>Never publish</h1>', invalid });
    await page.goto(mount);
    await page.locator(`[data-page-id="${v.page}"] a`).click();
    await expect(page.getByRole('alert')).toBeVisible();
    await expect(page.locator('iframe')).toHaveCount(0);
    await expect(page.locator('.page-header-actions')).toHaveCount(0);
    await expect(page.getByRole('button', { name: 'Export page' })).toHaveCount(0);
    await expect
      .poll(() =>
        page.evaluate(
          async () =>
            (await navigator.locks.query()).held?.filter((l) => l.name?.startsWith('writer:'))
              .length,
        ),
      )
      .toBe(0);
  });

test('paired checkpoints precede an authenticated interleaved tail, preserve edits and reload', async ({
  page,
  context,
}) => {
  const f = await wire(context, undefined, {});
  await page.goto(mount);
  await page.locator(`[data-page-id="${v.page}"] a`).click();
  await expect(page.getByRole('heading', { name: 'Checkpoint', exact: true })).toBeVisible();
  await expect(page.getByTestId('comments-panel')).toContainText('No comments yet.');
  await expect(page.getByTestId('comment-entry')).toHaveCount(0);
  await (await pageAction(page, 'Source')).click();
  await expect(page.getByRole('textbox')).toHaveValue('<p>after tail</p>' + 'x'.repeat(300_000));
  await page.getByRole('textbox').fill('<h1>After compacted reload</h1>');
  await page.getByRole('button', { name: 'Save source' }).click();
  await expect(
    page.frameLocator('iframe').getByRole('heading', { name: 'After compacted reload' }),
  ).toBeVisible();
  expect(f.entries.at(-1)!.seq).toBe('9');
  // The reply to this save is lost: the page reconnects and one status request settles it,
  // with the save committed exactly once and never sent again.
  f.dropNext();
  await page.getByRole('textbox').fill('<h1>Lost reply settled</h1>');
  await page.getByRole('button', { name: 'Save source' }).click();
  await expect(
    page.frameLocator('iframe').getByRole('heading', { name: 'Lost reply settled' }),
  ).toBeVisible();
  await expect(page.getByRole('alert')).toHaveCount(0);
  await f.settled();
  expect(f.statusChecks).toBe(1);
  expect(f.entries.at(-1)!.seq).toBe('10');
  await page.reload();
  await (await pageAction(page, 'Source')).click();
  await expect(page.getByRole('textbox')).toHaveValue('<h1>Lost reply settled</h1>');
  await page.getByRole('textbox').fill('<h1>After compacted reload</h1>');
  await page.getByRole('button', { name: 'Save source' }).click();
  await expect(
    page.frameLocator('iframe').getByRole('heading', { name: 'After compacted reload' }),
  ).toBeVisible();
  await expect(page.getByRole('button', { name: 'Save source' })).toBeDisabled();
  await f.settled();
  expect(f.statusChecks).toBe(1);
  expect(f.entries.at(-1)!.seq).toBe('11');
  const before = f.hellos;
  f.resync();
  await expect.poll(() => f.hellos).toBe(before + 1);
  await page.reload();
  await expect(
    page.frameLocator('iframe').getByRole('heading', { name: 'After compacted reload' }),
  ).toBeVisible();
  await page.getByRole('link', { name: 'Space home' }).click();
  await expect.poll(() => f.connections).toBe(0);
  await expect
    .poll(() =>
      page.evaluate(
        async () =>
          (await navigator.locks.query()).held?.filter((l) => l.name?.startsWith('writer:')).length,
      ),
    )
    .toBe(0);
});
for (const invalid of ['prefix', 'n', 'namespace', 'body', 'gap', 'ownBody', 'ownTail'] as const)
  test(`compacted catchup rejects ${invalid} without a partial content projection`, async ({
    page,
    context,
  }) => {
    await wire(context, undefined, { invalid });
    await page.goto(mount);
    await page.locator(`[data-page-id="${v.page}"] a`).click();
    await expect(page.getByRole('alert')).toBeVisible();
    await expect(page.locator('iframe')).toHaveCount(0);
  });
async function persistedLog(page: import('@playwright/test').Page) {
  return page.evaluate(async (space) => {
    const db = await new Promise<IDBDatabase>((resolve, reject) => {
      const request = indexedDB.open('tmt-colab', 1);
      request.onsuccess = () => resolve(request.result);
      request.onerror = () => reject(request.error);
    });
    try {
      return await new Promise<string[]>((resolve, reject) => {
        const tx = db.transaction('keys'),
          request = tx.objectStore('keys').get(`log:${space}`);
        tx.oncomplete = () => resolve(request.result);
        tx.onabort = () => reject(tx.error);
      });
    } finally {
      db.close();
    }
  }, v.space);
}
test('baseline finishes before a signed large statement; exact owner log survives reload', async ({
  page,
  context,
}) => {
  const source = '<h1>Statement fixture</h1>' + 'x'.repeat(40_000),
    f = await wire(context, { source }, undefined, 'valid');
  await page.goto(mount);
  await page.locator(`[data-page-id="${v.page}"] a`).click();
  await expect(
    page.frameLocator('iframe').getByRole('heading', { name: 'Statement fixture' }),
  ).toBeVisible();
  const log = await persistedLog(page);
  expect(log).toHaveLength(4);
  expect(log[3]).toBe(f.statement);
  expect(f.statementChunks).toBeGreaterThan(8);
  const chunks = f.statementChunks;
  await page.reload();
  await expect(
    page.frameLocator('iframe').getByRole('heading', { name: 'Statement fixture' }),
  ).toBeVisible();
  expect(await persistedLog(page)).toEqual(log);
  expect(f.statementChunks).toBe(chunks);
  await page.getByRole('link', { name: 'Space home' }).click();
  await expect.poll(() => f.connections).toBe(0);
});
test('tampered statement reference blocks home metadata and preserves the verified prefix', async ({
  page,
  context,
}) => {
  const f = await wire(context, undefined, undefined, 'hash');
  await page.goto(mount);
  await expect(page.locator(`[data-page-id="${v.page}"] a`)).toHaveCount(0);
  await expect(page.getByRole('alert')).toBeVisible();
  await expect(page.locator('iframe')).toHaveCount(0);
  const log = await persistedLog(page);
  expect(log).toHaveLength(2);
  expect(log).not.toContain(f.statement);
  await expect.poll(() => f.connections).toBe(0);
});

test('signed catchup publishes detached own maps for two authors while source stays content-only', async ({
  page,
  context,
}) => {
  const f = await wire(context, undefined, {});
  await page.goto(mount);
  const result = await page.evaluate(
    async ({ pageId, device }) => {
      const path = '/src/mounted.ts',
        { mountedTransport } = await import(path);
      const { transport, close } = await mountedTransport();
      const snapshot = await transport.page(pageId);
      try {
        const own = structuredClone(snapshot.own);
        const unsubscribe = snapshot.binding!.subscribe(
          (view: PageView) => {
            if (view.own) view.own[device].threads = {};
          },
          () => {},
        );
        const subscriptionIsDetached = Object.keys(snapshot.own![device].threads).length > 0;
        unsubscribe();
        return { own, subscriptionIsDetached, source: snapshot.source };
      } finally {
        snapshot.binding?.close();
        close();
      }
    },
    { pageId: v.page, device: v.device },
  );
  expect(result.own![v.device]).toEqual(ownVector.expected);
  expect(result.own!['00000000-0000-4000-8000-000000000126']).toEqual(ownVector.expectedPrefix);
  expect(result.subscriptionIsDetached).toBe(true);
  expect(result.source).toBe('<p>after tail</p>' + 'x'.repeat(300000));
  await expect.poll(() => f.connections).toBe(0);
});

test('parent export downloads exact frozen baseline files, ignores drafts and renderer messages, and cleans URLs', async ({
  page,
  context,
}) => {
  const source =
    '<h1>Exact export</h1>\r\n<p>λ 😀\0</p><script>parent.postMessage({type:"export"},"*");</script>';
  const f = await wire(context, { source });
  await page.addInitScript(() => {
    if (window !== window.top) return;
    const active = new Set<string>();
    Object.defineProperty(window, 'exportUrls', { value: active });
    const create = URL.createObjectURL.bind(URL),
      revoke = URL.revokeObjectURL.bind(URL);
    URL.createObjectURL = (blob) => {
      const url = create(blob);
      active.add(url);
      return url;
    };
    URL.revokeObjectURL = (url) => {
      active.delete(url);
      revoke(url);
    };
  });
  let requested = 0;
  page.on('download', () => requested++);
  await page.goto(mount);
  await page.locator(`[data-page-id="${v.page}"] a`).click();
  await expect(
    page.frameLocator('iframe').getByRole('heading', { name: 'Exact export' }),
  ).toBeVisible();
  const exportButton = await pageAction(page, 'Export page');
  await exportButton.evaluate((button: HTMLButtonElement) => button.click());
  await expect(page.getByRole('region', { name: 'Export page' })).toHaveCount(0);
  expect(requested).toBe(0);
  await (await pageAction(page, 'Source')).click();
  await page.getByRole('textbox').fill('<p>Unsaved draft</p>');
  const before = Date.now();
  await (await pageAction(page, 'Export page')).click();
  const panel = page.getByRole('region', { name: 'Export page' });
  await expect(panel).toContainText(
    'This creates an unencrypted copy of the page. Anyone with these files can read it.',
  );
  await expect(panel.getByRole('button', { name: 'Download page.html' })).toBeEnabled();
  await expect(panel).toHaveCSS('background-color', 'rgba(0, 0, 0, 0)');
  await expect(panel.getByRole('button', { name: 'Download page.html' })).toHaveClass(
    /tmt-ui-action/,
  );
  await panel
    .getByRole('button', { name: 'Download page.html' })
    .evaluate((button: HTMLButtonElement) => button.click());
  expect(requested).toBe(0);
  await page.screenshot({ path: capturePath('export-light.png'), fullPage: true });
  await (await pageAction(page, 'Theme: Dark')).click();
  await (await pageAction(page, 'Export page')).click();
  await page.screenshot({ path: capturePath('export-dark.png'), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({ path: capturePath('export-mobile.png'), fullPage: true });
  await page.setViewportSize({ width: 1280, height: 720 });
  async function download(name: string) {
    const pending = page.waitForEvent('download');
    const action = panel.getByRole('button', { name: new RegExp(`Download ${name}`) });
    await action.focus();
    await action.press('Enter');
    const received = await pending;
    expect(received.suggestedFilename()).toBe(name);
    expect(await received.failure()).toBeNull();
    const path = await received.path();
    expect(path).not.toBeNull();
    return readFileSync(path!);
  }
  const html = await download('page.html');
  expect(html).toEqual(Buffer.from(source, 'utf8'));
  await expect(panel.getByRole('status')).toContainText('Some files requested');
  // A separately admitted content update must not replace the frozen download.
  await f.contentUpdate('<h1>New live page</h1>');
  await expect(
    page.frameLocator('iframe').getByRole('heading', { name: 'New live page' }),
  ).toBeVisible();
  const conversationsBytes = await download('conversations.json');
  const conversations = JSON.parse(conversationsBytes.toString('utf8'));
  expect(conversations).toMatchObject({
    format: 'tmt-colab-conversations',
    version: 1,
    spaceId: v.space,
    pageId: v.page,
    title: 'Live fixture',
    epoch: '2',
    threads: [],
    asks: [],
  });
  const readingBytes = await download('conversations.md');
  expect(readingBytes.toString('utf8')).toContain('No threads.');
  const manifestBytes = await download('manifest.json');
  const manifest = JSON.parse(manifestBytes.toString('utf8'));
  expect(manifest.exportedAtMs).toBeGreaterThanOrEqual(before);
  expect(manifest.exportedAtMs).toBeLessThanOrEqual(Date.now());
  expect(manifestBytes.toString('utf8')).toBe(
    JSON.stringify({
      format: 'tmt-colab-page-export',
      version: 1,
      spaceId: v.space,
      pageId: v.page,
      title: 'Live fixture',
      exportedAtMs: manifest.exportedAtMs,
      membershipHead: { revision: '3', statementHash: Buffer.from(f.head.hash).toString('hex') },
      epoch: '2',
      plaintext: true,
      discussions: {
        included: true,
        scope: 'current-epoch',
        format: 'tmt-colab-conversations',
        version: 1,
      },
      attachments: [],
      files: [html, conversationsBytes, readingBytes].map((bytes, index) => ({
        name: ['page.html', 'conversations.json', 'conversations.md'][index],
        sizeBytes: bytes.length,
        sha256: createHash('sha256').update(bytes).digest('hex'),
      })),
    }),
  );
  await expect(panel.getByRole('status')).toContainText('All downloads requested');
  expect(await download('page.html')).toEqual(html);
  expect(await download('manifest.json')).toEqual(manifestBytes);
  const urlCount = () =>
    page.evaluate(() => (window as unknown as { exportUrls: Set<string> }).exportUrls.size);
  await expect.poll(urlCount).toBe(0);
  await download('page.html');
  await expect(panel.getByRole('button', { name: 'Close export', exact: true })).toHaveCount(0);
  const closeExport = page.locator(
    '.tmt-ui-drawer[data-panel="export"] .tmt-ui-drawer-header button',
  );
  await closeExport.focus();
  await closeExport.press('Enter');
  await expect.poll(urlCount).toBe(0);
  await (await pageAction(page, 'Source')).click();
  await expect(page.getByRole('textbox', { name: 'Source', exact: true })).toHaveValue(
    '<p>Unsaved draft</p>',
  );
  await (await pageAction(page, 'Export page')).click();
  await expect(panel.getByRole('button', { name: 'Download page.html' })).toBeEnabled();
  await download('page.html');
  await closeExport.click();
  await page.getByRole('link', { name: 'Space home' }).click();
  await expect.poll(urlCount).toBe(0);
  await expect.poll(() => f.connections).toBe(0);
});

test('failed export preparation requests no files and closing clears pending preparation', async ({
  page,
  context,
}) => {
  await wire(context);
  await page.goto(mount);
  await page.locator(`[data-page-id="${v.page}"] a`).click();
  await expect(page.getByRole('heading', { name: 'Live fixture', exact: true })).toBeVisible();
  let downloads = 0;
  page.on('download', () => downloads++);
  await page.evaluate(() => {
    const original = crypto.subtle.digest.bind(crypto.subtle);
    crypto.subtle.digest = () => {
      crypto.subtle.digest = original;
      return Promise.reject(new Error('Hash failed'));
    };
  });
  await (await pageAction(page, 'Export page')).click();
  const panel = page.getByRole('region', { name: 'Export page' });
  await expect(panel.getByRole('status')).toContainText('Could not prepare');
  await expect(panel.getByRole('button', { name: 'Download page.html' })).toBeDisabled();
  expect(downloads).toBe(0);
  await page.locator('.tmt-ui-drawer[data-panel="export"] .tmt-ui-drawer-header button').click();
  await page.evaluate(() => {
    const original = crypto.subtle.digest.bind(crypto.subtle);
    crypto.subtle.digest = async (...args) => {
      crypto.subtle.digest = original;
      await new Promise<void>((resolve) => {
        Object.defineProperty(window, 'releaseExportHash', { value: resolve });
      });
      return original(...args);
    };
  });
  await (await pageAction(page, 'Export page')).click();
  await expect(panel.getByRole('status')).toContainText('Preparing an exact copy');
  await expect
    .poll(() =>
      page.evaluate(
        () => typeof (window as unknown as { releaseExportHash?: unknown }).releaseExportHash,
      ),
    )
    .toBe('function');
  await page.locator('.tmt-ui-drawer[data-panel="export"] .tmt-ui-drawer-header button').click();
  await page.evaluate(() =>
    (window as unknown as { releaseExportHash(): void }).releaseExportHash(),
  );
  await expect(panel).toHaveCount(0);
  expect(downloads).toBe(0);
  await (await pageAction(page, 'Export page')).click();
  await expect(panel.getByRole('button', { name: 'Download page.html' })).toBeEnabled();
  await page.locator('.tmt-ui-drawer[data-panel="export"] .tmt-ui-drawer-header button').click();
  await page.getByRole('link', { name: 'Space home' }).click();
});

test('accepted fold titles persist as encrypted display hints for home, dialog and the parent tab', async ({
  page,
  context,
}) => {
  const title = '<img src=x onerror="window.titleInjected=true"> Personal notes';
  await wire(context, undefined, undefined, undefined, title);
  await page.goto(mount);
  const row = () => page.locator(`[data-page-id="${v.page}"]`);
  await expect(row().locator('.tmt-ui-list-title')).toHaveText(title);
  await expect(row().getByText(`Page ID: ${v.page}`, { exact: true })).toBeHidden();
  await row().locator('a').click();
  await expect(page.getByRole('heading', { name: title, exact: true })).toBeVisible();
  await expect(page).toHaveTitle(`${title} · Colab`);
  await (await pageAction(page, 'Manage page')).click();
  const dialog = page.getByRole('dialog');
  await expect(dialog.getByRole('heading', { name: title, exact: true })).toBeVisible();
  await expect(dialog.getByText(`Page ID: ${v.page}`, { exact: true })).toBeHidden();
  await dialog.getByText('Details', { exact: true }).click();
  await expect(dialog.getByText(`Page ID: ${v.page}`, { exact: true })).toBeVisible();
  await dialog.getByRole('button', { name: 'Close', exact: true }).click();
  await page.getByRole('link', { name: 'Space home', exact: true }).click();
  await expect(row().locator('.tmt-ui-list-title')).toHaveText(title);
  await expect(page).toHaveTitle('Colab');
  await page.reload();
  await expect(row().locator('.tmt-ui-list-title')).toHaveText(title);
  await expect(page.locator('.home img')).toHaveCount(0);
  const persisted = await page.evaluate(
    async ({ space, device, pageId }) => {
      const path = '/src/storage.ts';
      const { record } = await import(path);
      const key = await record(`title-key:${device}`);
      const value = await record(`title:${space}:${device}:${pageId}`);
      return {
        extractable: key.extractable,
        value,
        injected: (window as unknown as { titleInjected?: boolean }).titleInjected,
      };
    },
    { space: v.space, device: v.device, pageId: v.page },
  );
  expect(persisted.extractable).toBe(false);
  expect(Object.keys(persisted.value).sort()).toEqual(['ciphertext', 'nonce']);
  expect(JSON.stringify(persisted.value)).not.toContain(title);
  expect(persisted.injected).toBeUndefined();
  // Corrupt hints cannot turn a readable owner page into a failed home.
  await page.evaluate(
    async ({ space, device, pageId }) => {
      const path = '/src/storage.ts';
      const { record } = await import(path);
      await record(`title:${space}:${device}:${pageId}`, { nonce: 'AA', ciphertext: 'AA' });
    },
    { space: v.space, device: v.device, pageId: v.page },
  );
  await page.reload();
  await expect(row().locator('.tmt-ui-list-title')).toHaveText(title);
  await row().locator('a').click();
  await expect(page.getByRole('heading', { name: title, exact: true })).toBeVisible();
  await page.getByRole('link', { name: 'Space home', exact: true }).click();
  await expect(row().locator('.tmt-ui-list-title')).toHaveText(title);
});

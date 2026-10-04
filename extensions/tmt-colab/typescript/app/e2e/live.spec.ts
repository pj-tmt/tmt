import { readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { expect, test, type BrowserContext, type WebSocketRoute } from '@playwright/test';
import * as c from '@tmt/colab-client';
import type { PageView } from '../src/transport.js';
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
async function wire(
  context: BrowserContext,
  reset?: { source: string; invalid?: 'commitment' | 'source' | 'descriptor' | 'oldEpoch' },
  compacted?: { invalid?: 'prefix' | 'n' | 'namespace' | 'body' | 'gap' | 'ownBody' | 'ownTail' },
  statementTransfer?: 'valid' | 'hash',
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
      body: `export async function reopenSession(){sessionStorage.setItem('test:reopens',String(Number(sessionStorage.getItem('test:reopens')??0)+1));await window.fixtureKeys;} export async function certifyKey(purpose,bytes){return {publicKey:btoa(String.fromCharCode(...bytes)).replaceAll('+','-').replaceAll('/','_').replace(/=+$/,''),issuedAtMs:Date.now(),signature:'${c.encodeBinary(new Uint8Array(64))}'}}`,
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
        pages: [{ pageId: v.page, epoch, sharing: 'private', history: 'current', archived: false }],
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
  doc.getMap('meta').set('title', 'Live fixture');
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
      title: 'Live fixture',
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
  doc.destroy();
  const entry = async (env: c.Envelope) => ({
    seq: c.decodeHeader(env.header()).context.streamSeq,
    envelopeHash: c.encodeBinary(await env.hash()),
    envelope: c.encodeBinary(env.toJson()),
  });
  const entries = [await entry(initial)],
    peers = new Set<WebSocketRoute>();
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
    statementChunks = 0;
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
    outgoing.set(socket, { frames: [], waiting: false });
    socket.onClose(() => {
      peers.delete(socket);
      outgoing.delete(socket);
    });
    socket.onMessage((message) => {
      const frame = JSON.parse(String(message));
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
    head: head.head,
    entries,
    statement: largeStatement ? c.encodeBinary(largeStatement.toJson()) : null,
    get statementChunks() {
      return statementChunks;
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
  };
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
    export async function certifyKey(purpose,bytes){return {publicKey:btoa(String.fromCharCode(...bytes)).replaceAll('+','-').replaceAll('/','_').replace(/=+$/,''),issuedAtMs:Date.now(),signature:'${c.encodeBinary(new Uint8Array(64))}'}}`,
    }),
  );
}
const privateGuidance = `<!doctype html><html lang="en"><h1>TMT Colab</h1>
<p id="colab-recovery-status">Opening your paired browser…</p>
<p id="colab-guidance" hidden>This colab space is private. Open it from a browser paired with tmt remote pair, or use a share link.</p>
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
  await page.getByRole('link', { name: new RegExp(v.page) }).click();
  await expect(
    page.frameLocator('iframe').getByRole('heading', { name: 'Live fixture' }),
  ).toBeVisible();
  expect(opens).toBe(2); // Guidance recovery, then normal authenticated registration.
  expect(
    await page.evaluate((path) => sessionStorage.getItem(`colab-recovery:${path}`), mount),
  ).toBeNull();
});

for (const mode of ['unpaired', 'failed', 'cookie-lost'] as const) {
  test(`private guidance ${mode} stays visible without an automatic reopen loop`, async ({
    page,
    context,
  }) => {
    await wire(context);
    await recoverySdk(context);
    let opens = 0;
    await context.route('**/test-recovery-open', (route) => {
      opens++;
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

test('a disconnected active tab explicitly reconnects and reloads without background session reopen', async ({
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
  await page.getByRole('link', { name: new RegExp(v.page) }).click();
  const heading = page.frameLocator('iframe').getByRole('heading', { name: 'Live fixture' });
  await expect(heading).toBeVisible();
  f.stopSync();
  await expect(page.getByRole('button', { name: 'Reconnect', exact: true })).toBeVisible();
  expect(opens).toBe(1);
  await page.screenshot({
    path: '/private/tmp/colab-1110-design/colab-reconnect.png',
    fullPage: true,
  });
  await page.getByRole('button', { name: 'Reconnect', exact: true }).click();
  await expect(heading).toBeVisible();
  expect(opens).toBe(3); // Explicit recovery plus normal registration after reload.
  await expect.poll(() => f.connections).toBe(1);
  await page.getByRole('link', { name: 'Space home' }).click();
  await expect.poll(() => f.connections).toBe(0);
});

test('Ask publishes owner own envelopes through production Connection before Remote dispatch', async ({
  page,
  context,
}) => {
  const f = await wire(context),
    agentId = '00000000-0000-4000-8000-000000000006',
    requestId = 'req_00000000-0000-4000-8000-000000000007',
    reply = 'Wire reply <script>inert</script>';
  let sends = 0;
  let deliveredMessage: string | null = null;
  let operationId: string | null = null;
  // Only Remote and the signed sync server are doubles: registration, controller,
  // Worker preparation, Writer, Connection, receipts and projection are production.
  await context.route('**/sdk/remote-v1.js*', (route) =>
    route.fulfill({
      contentType: 'text/javascript',
      body: `export async function reopenSession(){await window.fixtureKeys;return {sessionId:'fixture-session',serverTimeMs:Date.now(),grantRevision:'1',expiresAtMs:null}}
export async function certifyKey(purpose,bytes){return {publicKey:btoa(String.fromCharCode(...bytes)).replaceAll('+','-').replaceAll('/','_').replace(/=+$/,''),issuedAtMs:Date.now(),signature:'${c.encodeBinary(new Uint8Array(64))}'}}
export function operations(){return {
listAgents:async()=>[{id:'${agentId}',name:'Wire agent',presence:'active'}],
send:async(input)=>(await fetch('/test-wire-send',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(input)})).json(),
operation:async(operationId)=>({state:'accepted',operationId,requestId:'${requestId}'}),
result:async()=>({state:'replied',requestId:'${requestId}',message:${JSON.stringify(reply)}})
}}`,
    }),
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
      expect(deliveredMessage).toBe(`[remote: Fixture]\n${input.message}`);
      operationId = input.operationId;
      sends++;
      await route.fulfill({ json: { state: 'accepted', operationId, requestId } });
    } finally {
      own.destroy();
    }
  });
  await page.goto(mount);
  await page.getByRole('link', { name: new RegExp(v.page) }).click();
  const heading = page.frameLocator('iframe').getByRole('heading', { name: 'Live fixture' });
  await expect(heading).toBeVisible();
  await heading.evaluate((node) => {
    const range = document.createRange();
    range.selectNodeContents(node);
    getSelection()!.removeAllRanges();
    getSelection()!.addRange(range);
  });
  await page.getByTestId('ask-action').click();
  await page.getByTestId('ask-agent-option').getByRole('radio').check();
  await page.getByRole('button', { name: 'Ask agent — preview', exact: true }).click();
  deliveredMessage = await page.getByTestId('ask-preview-text').textContent();
  await page.getByTestId('ask-send').click();
  await expect(page.getByTestId('ask-reply')).toHaveText(reply);
  expect(sends).toBe(1);
  await expect(page.getByTestId('ask-entry')).toHaveAttribute('data-operation-id', operationId!);
  await expect(page.getByTestId('ask-state')).toHaveAttribute('data-state', 'accepted');
  await expect(page.getByTestId('ask-reply-attribution')).toContainText('Reply from Wire agent');
  await expect(page.getByTestId('ask-panel').locator('script')).toHaveCount(0);
  await expect(heading).toHaveText('Live fixture');
  await f.settled();
  expect(f.entries.map((row) => row.seq)).toEqual(['1', '2', '3', '4', '5']);
  await page.reload();
  await expect(page.getByTestId('ask-reply')).toHaveText(reply);
  await expect(page.getByTestId('ask-entry')).toHaveAttribute('data-operation-id', operationId!);
  await expect(heading).toHaveText('Live fixture');
  await f.settled();
  expect(sends).toBe(1);
  expect(f.entries).toHaveLength(5);
  await page.getByRole('link', { name: 'Space home' }).click();
  await expect.poll(() => f.connections).toBe(0);
});

test('same-device tabs explicitly take over one durable stream without reopen ping-pong', async ({
  page,
  context,
}) => {
  const f = await wire(context),
    other = await context.newPage();
  const reopens = (tab: typeof page) =>
    tab.evaluate(() => Number(sessionStorage.getItem('test:reopens') ?? 0));
  await page.goto(mount);
  await page.getByRole('link', { name: new RegExp(v.page) }).click();
  await expect(
    page.frameLocator('iframe').getByRole('heading', { name: 'Live fixture' }),
  ).toBeVisible();
  await page.getByRole('button', { name: 'Source', exact: true }).click();
  await page.screenshot({ path: '/tmp/1252-live-light.png', fullPage: true });
  await page.getByRole('button', { name: 'Change color theme' }).click();
  await expect(page.locator('iframe')).toHaveCSS('background-color', 'rgb(255, 255, 255)');
  await expect(page.locator('iframe')).toHaveCSS('color-scheme', 'light');
  await page.screenshot({ path: '/tmp/1252-live-dark.png', fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({ path: '/tmp/1252-live-mobile.png', fullPage: true });
  await page.setViewportSize({ width: 1280, height: 720 });
  await page.getByRole('textbox').fill('<h1>First</h1>');
  await page.getByRole('button', { name: 'Save source' }).click();
  await expect(page.frameLocator('iframe').getByRole('heading', { name: 'First' })).toBeVisible();
  await other.goto(mount);
  await other.getByRole('link', { name: new RegExp(v.page) }).click();
  await expect(page.getByTestId('colab-inactive')).toContainText('Colab is open in another tab.');
  await expect(page.locator('iframe')).toHaveCount(0);
  await expect(other.frameLocator('iframe').getByRole('heading', { name: 'First' })).toBeVisible();
  await expect.poll(() => f.connections).toBe(1);
  expect(await reopens(page)).toBe(1);
  expect(await reopens(other)).toBe(1);
  await page
    .getByRole('button', { name: 'Use here' })
    .evaluate((button: HTMLButtonElement) => button.click());
  expect(await reopens(page)).toBe(1);
  await page.screenshot({
    path: '/private/tmp/colab-1110-design/colab-inactive.png',
    fullPage: true,
  });
  await other.getByRole('button', { name: 'Source', exact: true }).click();
  const large = '<h1>Large</h1>' + 'x'.repeat(50_000);
  await other.getByRole('textbox').fill(large);
  await other.getByRole('button', { name: 'Save source' }).click();
  await expect(other.frameLocator('iframe').getByRole('heading', { name: 'Large' })).toBeVisible();
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
  await expect(
    other.frameLocator('iframe').getByRole('heading', { name: 'After reload' }),
  ).toBeVisible();
  await f.settled();
  expect(f.retries).toBe(1);
  expect(f.entries.map((row) => row.seq)).toEqual(['1', '2', '3', '4', '5']);
  const before = f.hellos;
  f.resync();
  await expect.poll(() => f.hellos).toBe(before + 1);
  await expect(
    other.frameLocator('iframe').getByRole('heading', { name: 'After reload' }),
  ).toBeVisible();
  expect(await reopens(other)).toBe(2);
  expect(await reopens(page)).toBe(1);
  await f.ownUpdate();
  await expect(other.getByRole('status')).toContainText('Comments and activity are not displayed');
  await page.getByRole('button', { name: 'Use here' }).click();
  await expect(other.getByTestId('colab-inactive')).toBeVisible();
  await expect(other.locator('iframe')).toHaveCount(0);
  await expect(
    page.frameLocator('iframe').getByRole('heading', { name: 'After reload' }),
  ).toBeVisible();
  expect(await reopens(page)).toBe(2);
  expect(await reopens(other)).toBe(2);
  await page.getByRole('button', { name: 'Source', exact: true }).click();
  await page.getByRole('textbox').fill('<h1>Across own</h1>');
  await page.getByRole('button', { name: 'Save source' }).click();
  await expect(
    page.frameLocator('iframe').getByRole('heading', { name: 'Across own' }),
  ).toBeVisible();
  expect(f.entries.at(-1)!.seq).toBe('7');
  await other.getByRole('button', { name: 'Use here' }).click();
  await expect(page.getByTestId('colab-inactive')).toBeVisible();
  await expect(
    other.frameLocator('iframe').getByRole('heading', { name: 'Across own' }),
  ).toBeVisible();
  expect(await reopens(other)).toBe(3);
  expect(await reopens(page)).toBe(2);
  await other.getByRole('link', { name: 'Space home' }).click();
  await expect.poll(() => f.connections).toBe(0);
  expect(
    await page.evaluate(
      async () =>
        (await navigator.locks.query()).held?.filter((lock) => lock.name?.startsWith('writer:'))
          .length,
    ),
  ).toBe(0);
  await other.close();
  await expect(page.getByTestId('colab-inactive')).toBeVisible();
  expect(await reopens(page)).toBe(2);
});

test('chunked epoch baseline survives explicit tab takeover, edits and reload', async ({
  page,
  context,
}) => {
  const source = '<h1>Reset baseline</h1>' + 'x'.repeat(300_000);
  await wire(context, { source });
  const other = await context.newPage();
  for (const tab of [page, other]) {
    await tab.goto(mount);
    await tab.getByRole('link', { name: new RegExp(v.page) }).click();
    await expect(
      tab.frameLocator('iframe').getByRole('heading', { name: 'Reset baseline' }),
    ).toBeVisible();
    await tab.getByRole('button', { name: 'Source', exact: true }).click();
    await expect(tab.getByRole('textbox')).toHaveValue(source);
  }
  await expect(page.getByTestId('colab-inactive')).toBeVisible();
  const next = source.replace('Reset baseline', 'New epoch edit');
  await other.getByRole('textbox').fill(next);
  await other.getByRole('button', { name: 'Save source' }).click();
  await expect(
    other.frameLocator('iframe').getByRole('heading', { name: 'New epoch edit' }),
  ).toBeVisible();
  await page.getByRole('button', { name: 'Use here' }).click();
  await expect(
    page.frameLocator('iframe').getByRole('heading', { name: 'New epoch edit' }),
  ).toBeVisible();
  await expect(other.getByTestId('colab-inactive')).toBeVisible();
  await other.reload();
  await expect(
    other.frameLocator('iframe').getByRole('heading', { name: 'New epoch edit' }),
  ).toBeVisible();
  await expect(page.getByTestId('colab-inactive')).toBeVisible();
});
for (const invalid of ['commitment', 'source', 'descriptor', 'oldEpoch'] as const)
  test(`signed reset rejects ${invalid} without partial renderer publication`, async ({
    page,
    context,
  }) => {
    await wire(context, { source: '<h1>Never publish</h1>', invalid });
    await page.goto(mount);
    await page.getByRole('link', { name: new RegExp(v.page) }).click();
    await expect(page.getByRole('alert')).toBeVisible();
    await expect(page.locator('iframe')).toHaveCount(0);
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
  await page.getByRole('link', { name: new RegExp(v.page) }).click();
  await expect(page.getByRole('heading', { name: 'Checkpoint', exact: true })).toBeVisible();
  await expect(page.getByRole('status')).toContainText('Comments and activity are not displayed');
  await page.getByRole('button', { name: 'Source', exact: true }).click();
  await expect(page.getByRole('textbox')).toHaveValue('<p>after tail</p>' + 'x'.repeat(300_000));
  await page.getByRole('textbox').fill('<h1>After compacted reload</h1>');
  await page.getByRole('button', { name: 'Save source' }).click();
  await expect(
    page.frameLocator('iframe').getByRole('heading', { name: 'After compacted reload' }),
  ).toBeVisible();
  expect(f.entries.at(-1)!.seq).toBe('9');
  f.dropNext();
  await page.getByRole('textbox').fill('<h1>Frozen retry after prune</h1>');
  await page.getByRole('button', { name: 'Save source' }).click();
  await expect(page.getByRole('alert')).toContainText('edit was not saved');
  await page.reload();
  await page.getByRole('button', { name: 'Source', exact: true }).click();
  await expect(page.getByRole('textbox')).toHaveValue('<h1>Frozen retry after prune</h1>');
  await page.getByRole('textbox').fill('<h1>After compacted reload</h1>');
  await page.getByRole('button', { name: 'Save source' }).click();
  await expect(
    page.frameLocator('iframe').getByRole('heading', { name: 'After compacted reload' }),
  ).toBeVisible();
  await expect(page.getByRole('button', { name: 'Save source' })).toBeDisabled();
  await f.settled();
  expect(f.retries).toBe(1);
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
    await page.getByRole('link', { name: new RegExp(v.page) }).click();
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
  await page.getByRole('link', { name: new RegExp(v.page) }).click();
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
  await expect(page.getByRole('link', { name: new RegExp(v.page) })).toHaveCount(0);
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
      // This isolated read-admission probe supplies test-owned activation.
      const { transport, close } = await mountedTransport({
        active: true,
        run: <T>(action: () => Promise<T>) => action(),
      });
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
  await page.getByRole('link', { name: new RegExp(v.page) }).click();
  await expect(
    page.frameLocator('iframe').getByRole('heading', { name: 'Exact export' }),
  ).toBeVisible();
  const exportButton = page.getByRole('button', { name: 'Export page', exact: true });
  await exportButton.evaluate((button: HTMLButtonElement) => button.click());
  await expect(page.getByRole('region', { name: 'Export page' })).toHaveCount(0);
  expect(requested).toBe(0);
  await page.getByRole('button', { name: 'Source', exact: true }).click();
  await page.getByRole('textbox').fill('<p>Unsaved draft</p>');
  const before = Date.now();
  await exportButton.click();
  const panel = page.getByRole('region', { name: 'Export page' });
  await expect(panel).toContainText(
    'This creates an unencrypted copy of the page. Anyone with these files can read it.',
  );
  await expect(panel.getByRole('button', { name: 'Download page.html' })).toBeEnabled();
  await panel
    .getByRole('button', { name: 'Download page.html' })
    .evaluate((button: HTMLButtonElement) => button.click());
  expect(requested).toBe(0);
  await page.screenshot({ path: '/private/tmp/1309-export-light.png', fullPage: true });
  await page.getByRole('button', { name: 'Change color theme' }).click();
  await page.screenshot({ path: '/private/tmp/1309-export-dark.png', fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({ path: '/private/tmp/1309-export-mobile.png', fullPage: true });
  await page.setViewportSize({ width: 1280, height: 720 });
  async function download(name: string) {
    const pending = page.waitForEvent('download');
    await panel.getByRole('button', { name: new RegExp(`Download ${name}`) }).click();
    const received = await pending;
    expect(received.suggestedFilename()).toBe(name);
    expect(await received.failure()).toBeNull();
    const path = await received.path();
    expect(path).not.toBeNull();
    return readFileSync(path!);
  }
  const html = await download('page.html');
  expect(html).toEqual(Buffer.from(source, 'utf8'));
  await expect(panel.getByRole('status')).toContainText('One file requested');
  // A committed edit after preparation must not replace the frozen download.
  await page.getByRole('textbox').fill('<h1>New live page</h1>');
  await page.getByRole('button', { name: 'Save source' }).click();
  await expect(
    page.frameLocator('iframe').getByRole('heading', { name: 'New live page' }),
  ).toBeVisible();
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
      discussions: 'not-included',
      files: [
        {
          name: 'page.html',
          sizeBytes: html.length,
          sha256: createHash('sha256').update(html).digest('hex'),
        },
      ],
    }),
  );
  await expect(panel.getByRole('status')).toContainText('Both downloads requested');
  expect(await download('page.html')).toEqual(html);
  expect(await download('manifest.json')).toEqual(manifestBytes);
  const urlCount = () =>
    page.evaluate(() => (window as unknown as { exportUrls: Set<string> }).exportUrls.size);
  await expect.poll(urlCount).toBe(0);
  await download('page.html');
  await panel.getByRole('button', { name: 'Close export' }).click();
  await expect.poll(urlCount).toBe(0);
  await exportButton.click();
  await expect(panel.getByRole('button', { name: 'Download page.html' })).toBeEnabled();
  await download('page.html');
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
  await page.getByRole('link', { name: new RegExp(v.page) }).click();
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
  await page.getByRole('button', { name: 'Export page', exact: true }).click();
  const panel = page.getByRole('region', { name: 'Export page' });
  await expect(panel.getByRole('status')).toContainText('Could not prepare');
  await expect(panel.getByRole('button', { name: 'Download page.html' })).toBeDisabled();
  expect(downloads).toBe(0);
  await panel.getByRole('button', { name: 'Close export' }).click();
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
  await page.getByRole('button', { name: 'Export page', exact: true }).click();
  await expect(panel.getByRole('status')).toContainText('Preparing an exact copy');
  await expect
    .poll(() =>
      page.evaluate(
        () => typeof (window as unknown as { releaseExportHash?: unknown }).releaseExportHash,
      ),
    )
    .toBe('function');
  await panel.getByRole('button', { name: 'Close export' }).click();
  await page.evaluate(() =>
    (window as unknown as { releaseExportHash(): void }).releaseExportHash(),
  );
  await expect(panel).toHaveCount(0);
  expect(downloads).toBe(0);
  await page.getByRole('button', { name: 'Export page', exact: true }).click();
  await expect(panel.getByRole('button', { name: 'Download page.html' })).toBeEnabled();
  await panel.getByRole('button', { name: 'Close export' }).click();
  await page.getByRole('link', { name: 'Space home' }).click();
});

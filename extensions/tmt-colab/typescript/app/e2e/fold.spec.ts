import { expect, test as base } from '@playwright/test';
import { readFileSync } from 'node:fs';

// Opt-in observation only: unchanged assertions, no cache preparation or app patch.
const test = base.extend<{ coldObservation: void }>({
  coldObservation: [
    async ({ page }, use, info) => {
      if (process.env.COLAB_FOLD_COLD_OBSERVE !== '1') {
        await use();
        return;
      }
      const events: unknown[] = [];
      const session = await page.context().newCDPSession(page);
      const record = (event: string, data: unknown) =>
        events.push({ observedAt: new Date().toISOString(), event, data });
      session.on('Network.requestWillBeSent', (event) =>
        record('request', {
          requestId: event.requestId,
          loaderId: event.loaderId,
          frameId: event.frameId,
          timestamp: event.timestamp,
          type: event.type,
          url: event.request.url,
          initiator: event.initiator,
          redirectStatus: event.redirectResponse?.status,
        }),
      );
      session.on('Network.responseReceived', (event) =>
        record('response', {
          requestId: event.requestId,
          timestamp: event.timestamp,
          url: event.response.url,
          status: event.response.status,
          mimeType: event.response.mimeType,
        }),
      );
      session.on('Network.loadingFailed', (event) => record('loadingFailed', event));
      session.on('Network.webSocketFrameReceived', (event) => record('webSocketReceived', event));
      session.on('Page.frameNavigated', (event) => record('frameNavigated', event));
      session.on('Page.frameRequestedNavigation', (event) => record('navigationRequested', event));
      session.on('Runtime.executionContextDestroyed', (event) => record('contextDestroyed', event));
      session.on('Target.attachedToTarget', (event) => record('targetAttached', event));
      page.on('console', (message) =>
        record('console', { type: message.type(), text: message.text() }),
      );
      page.on('pageerror', (error) => record('pageerror', { message: error.message }));
      await session.send('Network.enable');
      await session.send('Page.enable');
      await session.send('Runtime.enable');
      await session.send('Target.setAutoAttach', {
        autoAttach: true,
        waitForDebuggerOnStart: false,
        flatten: true,
      });
      try {
        await use();
      } finally {
        await info.attach('cold-fold-observation', {
          body: Buffer.from(JSON.stringify({ title: info.title, events }, null, 2)),
          contentType: 'application/json',
        });
        await session.detach();
      }
    },
    { auto: true },
  ],
});

test('real Worker admits and refuses the shared editor metadata request and result vectors', async ({
  page,
}) => {
  const vectors = JSON.parse(
    readFileSync(
      new URL('../../../contracts/vectors/editor-metadata-v1.json', import.meta.url),
      'utf8',
    ),
  );
  await page.goto('/');
  const results = await page.evaluate(async (vectors) => {
    const path = '/src/fold.ts';
    const { Fold } = await import(path);
    const results = [];
    for (const c of [...vectors.requests, ...vectors.results]) {
      const fold = new Fold();
      let valid = false;
      let record;
      try {
        const prepared = await fold.run({
          type: 'prepare-own',
          writer: '00000000-0000-4000-8000-000000000001',
          records: [{ root: c.root, key: c.key, value: c.record }],
        });
        record = prepared.own['00000000-0000-4000-8000-000000000001'][c.root][c.key];
        valid = true;
      } catch {
        valid = false;
      } finally {
        fold.close();
      }
      results.push({ name: c.name, valid, record });
    }
    return results;
  }, vectors);
  const cases = [...vectors.requests, ...vectors.results];
  for (let i = 0; i < cases.length; i++) {
    expect(results[i].valid, cases[i].name).toBe(cases[i].valid);
    if (cases[i].valid) expect(results[i].record, cases[i].name).toEqual(cases[i].record);
  }
});

test('attachment object adapter verifies actual browser crypto without replaying an uncertain original', async ({
  page,
}) => {
  const corpus = JSON.parse(
    readFileSync(new URL('../../../contracts/vectors/attachment-v1.json', import.meta.url), 'utf8'),
  );
  await page.goto('/');
  const result = await page.evaluate(
    async ({ corpus, clientPath }) => {
      const adapterPath = '/src/attachment-channel.ts',
        attachmentsPath = '/src/attachments.ts';
      const { AttachmentObjectChannel, FrozenAttachmentUpload } = await import(adapterPath);
      const { attachmentNamespace } = await import(attachmentsPath);
      const c = await import(clientPath);
      const fixture = corpus.cases.find(
        (entry: { name: string }) => entry.name === 'document-asset',
      );
      const descriptor = c.attachment.attachmentDescriptor(
        c.strictJson(c.text(fixture.input), 2048),
      );
      const raw = c.binary(fixture.payload, c.attachment.ATTACHMENT_PAYLOAD_BYTES);
      const original = await FrozenAttachmentUpload.capture(
        descriptor,
        `v1:${'ab'.repeat(32)}`,
        '11111111-1111-4111-8111-111111111111',
        raw,
      );
      const calls: string[] = [];
      let damaged = false;
      const channel = new AttachmentObjectChannel(
        async (request: { method: string; transferId: string; offset: number; count: number }) => {
          calls.push(request.method);
          if (request.method === 'verify')
            return {
              ok: {
                result: 'read',
                offset: request.offset,
                totalBytes: raw.length,
                bytes: c.encodeBinary(
                  damaged
                    ? new Uint8Array(request.count)
                    : raw.slice(request.offset, request.offset + request.count),
                ),
              },
            };
          if (request.transferId !== original.transferId) throw new Error('Original changed');
          return { error: { code: 'unknown' } };
        },
      );
      const deadline = performance.now() + 15_000;
      const uncertain = await channel.begin(original, deadline);
      await channel.status(original, deadline);
      const namespace = await attachmentNamespace(descriptor.space, descriptor.page);
      const key = Uint8Array.from(descriptor.objectId.match(/../g), (byte: string) =>
        parseInt(byte, 16),
      );
      const committed = await channel.verifier(original).readCommitted(namespace, key, deadline);
      const plaintext = await c.attachment.openAttachment(
        descriptor,
        committed,
        c.attachment.attachmentContext(descriptor),
        c.binary(corpus.secret, 32, 32),
        c.binary(corpus.publicKey, 32, 32),
      );
      damaged = true;
      let refused = false;
      try {
        await channel.verifier(original).readCommitted(namespace, key, deadline);
      } catch {
        refused = true;
      }
      return { uncertain, calls, plaintext: c.encodeBinary(plaintext), refused };
    },
    {
      corpus,
      clientPath: `/@fs${new URL('../../colab-client/src/index.ts', import.meta.url).pathname}`,
    },
  );
  expect(result.uncertain).toEqual({ error: { code: 'unknown' } });
  expect(result.calls).toEqual(['begin', 'status', 'verify', 'verify']);
  expect(result.plaintext).toBe(corpus.plaintext);
  expect(result.refused).toBe(true);
});

test('Worker folds concurrent independent writers, reload reconstruction and Unicode minimal edits', async ({
  page,
}) => {
  await page.goto('/');
  const result = await page.evaluate(async () => {
    const path = '/src/fold.ts';
    const { Fold } = await import(path);
    const a = new Fold(),
      b = new Fold(),
      reloaded = new Fold();
    try {
      const base = (r: { source: string; title: string; own: unknown }) => ({
        source: r.source,
        title: r.title,
        own: r.own,
      });
      const one = await a.prepareContent('<p>😀 café</p>', { source: '', title: '', own: {} });
      const baseA = await a.run({ type: 'apply', updates: one.updates });
      const baseB = await b.run({ type: 'apply', updates: one.updates });
      const two = await a.prepareContent('<p>😀 café A</p>', base(baseA));
      const three = await b.prepareContent('<p>😀 café B</p>', base(baseB));
      const all = [...two.updates, ...three.updates];
      const mergedA = await a.run({ type: 'apply', updates: all });
      const mergedB = await b.run({ type: 'apply', updates: all });
      const restored = await reloaded.run({
        type: 'apply',
        updates: [...one.updates, ...all],
      });
      return {
        one: one.projection.source,
        two: two.projection.source,
        a: mergedA.source,
        b: mergedB.source,
        restored: restored.source,
        updateBytes: two.updates.reduce((n: number, u: Uint8Array) => n + u.length, 0),
      };
    } finally {
      a.close();
      b.close();
      reloaded.close();
    }
  });
  expect(result.one).toBe('<p>😀 café</p>');
  expect(result.two).toBe('<p>😀 café A</p>');
  expect(result.a).toBe(result.b);
  expect(result.restored).toBe(result.a);
  expect(result.a).toContain('A');
  expect(result.a).toContain('B');
  expect(result.updateBytes).toBeLessThan(100);
});

test('decoder rejects malformed data and cannot be reused after rejection', async ({ page }) => {
  await page.goto('/');
  const rejected = await page.evaluate(async () => {
    const path = '/src/fold.ts';
    const { Fold } = await import(path);
    const fold = new Fold();
    let errors = 0;
    try {
      await fold.run({ type: 'apply', updates: [new Uint8Array([255])] }).catch(() => errors++);
      await fold.run({ type: 'apply', updates: [] }).catch(() => errors++);
      return errors;
    } finally {
      fold.close();
    }
  });
  expect(rejected).toBe(2);
});

test('read batches admit more than the write limits', async ({ page }) => {
  await page.goto('/');
  const result = await page.evaluate(async () => {
    const path = '/src/fold.ts';
    const { Fold } = await import(path);
    const protocol = '/src/fold-protocol.ts';
    const { WRITE_TAIL_BYTES } = await import(protocol);
    const fold = new Fold();
    const empty = new Uint8Array([0, 0]);
    try {
      await fold.run({ type: 'apply', updates: Array(201).fill(empty) });
      const writeCount = await fold.run({ type: 'check', updates: Array(201).fill(empty) }).then(
        () => false,
        () => true,
      );
      const writeBytes = await fold
        .run({ type: 'check', updates: [new Uint8Array(WRITE_TAIL_BYTES + 1)] })
        .then(
          () => false,
          () => true,
        );
      const readCount = await fold.run({ type: 'apply', updates: Array(5_001).fill(empty) }).then(
        () => false,
        () => true,
      );
      return { writeCount, writeBytes, readCount };
    } finally {
      fold.close();
    }
  });
  expect(result).toEqual({ writeCount: true, writeBytes: true, readCount: true });
});

test('same-device tabs persist one non-extractable signing/encryption binding', async ({
  page,
  context,
}) => {
  const other = await context.newPage();
  await Promise.all([page.goto('/'), other.goto('/')]);
  const read = async (tab: typeof page) =>
    tab.evaluate(async () => {
      const path = '/src/keyring.ts';
      const { deviceKeys } = await import(path);
      const keys = await deviceKeys('00000000-0000-4000-8000-000000000123');
      const denied = await crypto.subtle.exportKey('pkcs8', keys.sign).then(
        () => false,
        () => true,
      );
      const encDenied = await crypto.subtle.exportKey('pkcs8', keys.enc.handle()).then(
        () => false,
        () => true,
      );
      return { sign: [...keys.signPublic], enc: [...keys.enc.publicKey()], denied, encDenied };
    });
  const [a, b] = await Promise.all([read(page), read(other)]);
  expect(a).toEqual(b);
  expect(a.denied).toBe(true);
  expect(a.encDenied).toBe(true);
  await page.reload();
  expect(await read(page)).toEqual(a);
});

test('Worker rejects unknown, mixed and rich-text roots before publishing any projection', async ({
  page,
}) => {
  await page.goto('/');
  const errors = await page.evaluate(async () => {
    const workerPath = '/src/fold.ts',
      fixturePath = '/test/update-fixtures.ts';
    const { Fold } = await import(workerPath),
      { invalidUpdates } = await import(fixturePath);
    let rejected = 0;
    for (const update of invalidUpdates()) {
      const fold = new Fold();
      try {
        const previous = await fold.prepareContent('previous valid source', {
          source: '',
          title: '',
          own: {},
        });
        await fold.run({ type: 'apply', updates: previous.updates });
        await fold.run({ type: 'apply', updates: [update] }).catch(() => rejected++);
      } finally {
        fold.close();
      }
    }
    return rejected;
  });
  expect(errors).toBe(4);
});

test('prepared edits never leak through a later committed projection', async ({ page }) => {
  await page.goto('/');
  const result = await page.evaluate(async () => {
    const path = '/src/fold.ts';
    const { Fold } = await import(path);
    const a = new Fold(),
      b = new Fold();
    try {
      const snapshot = (r: { source: string; title: string; own: unknown }) => ({
        source: r.source,
        title: r.title,
        own: r.own,
      });
      const seed = await a.prepareContent('initial', { source: '', title: '', own: {} });
      const baseA = await a.run({ type: 'apply', updates: seed.updates });
      const baseB = await b.run({ type: 'apply', updates: seed.updates });
      await a.prepareContent('unsaved draft', snapshot(baseA));
      const remote = await b.prepareContent('initial saved', snapshot(baseB));
      return (await a.run({ type: 'apply', updates: remote.updates })).source;
    } finally {
      a.close();
      b.close();
    }
  });
  expect(result).toBe('initial saved');
});

test('baseline vectors reset exact struct identities and reject digest, title and commitment mismatch', async ({
  page,
}) => {
  const { readFileSync } = await import('node:fs');
  const vectors = JSON.parse(
    readFileSync(new URL('../../../contracts/vectors/baseline-v1.json', import.meta.url), 'utf8'),
  );
  await page.goto('/');
  const result = await page.evaluate(async (vectors) => {
    const foldPath = '/src/fold.ts';
    const { Fold } = await import(foldPath);
    const binary = (s: string) =>
      Uint8Array.from(atob(s.replaceAll('-', '+').replaceAll('_', '/')), (c) => c.charCodeAt(0));
    let rejected = 0;
    for (const v of vectors) {
      const command = {
        type: 'baseline',
        update: binary(v.update),
        title: v.title,
        sourceDigest: binary(v.sourceDigest),
        commitment: binary(v.commitment),
      };
      const a = new Fold(),
        b = new Fold();
      try {
        const one = await a.run(command),
          two = await b.run(command);
        if (
          one.source !== v.source ||
          two.source !== v.source ||
          one.title !== v.title ||
          one.originalAuthor !== v.originalAuthor ||
          two.originalAuthor !== v.originalAuthor ||
          one.publisherAgent !== v.publisherAgent ||
          two.publisherAgent !== v.publisherAgent ||
          one.layout !== v.layout ||
          one.layoutReason !== v.layoutReason ||
          two.layout !== v.layout ||
          two.layoutReason !== v.layoutReason
        )
          throw new Error('Baseline vector projection');
        const edit = await a.prepareContent(v.source + 'later', {
          source: one.source,
          title: one.title,
          own: one.own,
          ...(one.originalAuthor === undefined ? {} : { originalAuthor: one.originalAuthor }),
          ...(one.publisherAgent === undefined ? {} : { publisherAgent: one.publisherAgent }),
          ...(one.layout === undefined ? {} : { layout: one.layout }),
          ...(one.layoutReason === undefined ? {} : { layoutReason: one.layoutReason }),
          ...(one.creationRecipient === undefined
            ? {}
            : { creationRecipient: one.creationRecipient }),
        });
        const left = await a.run({ type: 'apply', updates: edit.updates }),
          right = await b.run({ type: 'apply', updates: edit.updates });
        if (
          left.source !== right.source ||
          left.originalAuthor !== v.originalAuthor ||
          right.originalAuthor !== v.originalAuthor ||
          left.publisherAgent !== v.publisherAgent ||
          right.publisherAgent !== v.publisherAgent ||
          left.layout !== v.layout ||
          left.layoutReason !== v.layoutReason ||
          right.layout !== v.layout ||
          right.layoutReason !== v.layoutReason
        )
          throw new Error('Baseline struct identity or publisher metadata differs');
      } finally {
        a.close();
        b.close();
      }
      for (const change of [
        { sourceDigest: new Uint8Array(32) },
        { commitment: new Uint8Array(32) },
        { title: 'mismatch' },
      ]) {
        const fold = new Fold();
        try {
          await fold.run({ ...command, ...change }).then(
            () => {
              throw new Error('Accepted invalid baseline');
            },
            () => rejected++,
          );
        } finally {
          fold.close();
        }
      }
    }
    return rejected;
  }, vectors);
  expect(result).toBe(vectors.length * 3);
});

test('shared raw checkpoint vectors preserve dependencies and delete sets without cross-writer reattribution', async ({
  page,
}) => {
  const { readFileSync } = await import('node:fs');
  const v = JSON.parse(
    readFileSync(new URL('../../../contracts/vectors/checkpoint-v1.json', import.meta.url), 'utf8'),
  );
  await page.goto('/');
  const result = await page.evaluate(async (v) => {
    const path = '/src/fold.ts',
      { Fold } = await import(path);
    const bytes = (s: string) =>
      Uint8Array.from(atob(s.replaceAll('-', '+').replaceAll('_', '/')), (c) => c.charCodeAt(0));
    const compacted = new Fold(),
      original = new Fold();
    try {
      const prefix = await compacted.run({ type: 'apply', updates: [bytes(v.checkpoint)] });
      if (prefix.source !== v.sourceAtPrefix) throw new Error('Wrong checkpoint prefix');
      const a = await compacted.run({ type: 'apply', updates: [bytes(v.tail)] });
      const b = await original.run({
        type: 'apply',
        updates: [...v.contentUpdates.map(bytes), bytes(v.tail)],
      });
      if (a.source !== b.source || a.source !== v.sourceAfterTail || a.title !== v.title)
        throw new Error('Compaction lost dependency/delete-set state');
      for (const raw of [v.tail, ...v.negativeBodies.map((n: { bytes: string }) => n.bytes)]) {
        const reject = new Fold();
        try {
          await reject.run({ type: 'apply', updates: [bytes(raw)] }).then(
            () => {
              throw new Error('Accepted unresolved or malformed checkpoint');
            },
            () => {},
          );
        } finally {
          reject.close();
        }
      }
      return a.source;
    } finally {
      compacted.close();
      original.close();
    }
  }, v);
  expect(result).toBe(v.sourceAfterTail);
});

test('checkpoint steps retain pending dependencies until the final bounded tail resolves them', async ({
  page,
}) => {
  const { readFileSync } = await import('node:fs');
  const v = JSON.parse(
    readFileSync(new URL('../../../contracts/vectors/checkpoint-v1.json', import.meta.url), 'utf8'),
  );
  await page.goto('/');
  const source = await page.evaluate(async (v) => {
    const path = '/src/fold.ts',
      { Fold } = await import(path);
    const bytes = (s: string) =>
      Uint8Array.from(atob(s.replaceAll('-', '+').replaceAll('_', '/')), (c) => c.charCodeAt(0));
    const complete = new Fold(),
      unresolved = new Fold();
    try {
      await complete.run({ type: 'checkpoint', update: bytes(v.tail) });
      await complete.run({ type: 'checkpoint', update: bytes(v.checkpoint) });
      const result = await complete.run({ type: 'apply', updates: [] });
      await unresolved.run({ type: 'checkpoint', update: bytes(v.tail) });
      await unresolved.run({ type: 'apply', updates: [] }).then(
        () => {
          throw new Error('Published unresolved checkpoint dependencies');
        },
        () => {},
      );
      return result.source;
    } finally {
      complete.close();
      unresolved.close();
    }
  }, v);
  expect(source).toBe(v.sourceAfterTail);
});

test('real Worker preserves attributed registry metadata through source edits and rejects claimed answered state', async ({
  page,
}) => {
  const corpus = JSON.parse(
    readFileSync(
      new URL('../../../contracts/vectors/question-metadata-v1.json', import.meta.url),
      'utf8',
    ),
  );
  const creator = corpus.cases.find(
    (entry: { kind: string; valid: boolean }) => entry.kind === 'creator' && entry.valid,
  ).value;
  const questions = corpus.cases.find(
    (entry: { kind: string; valid: boolean; value: unknown[] }) =>
      entry.kind === 'questions' && entry.valid && entry.value.length > 0,
  ).value;
  await page.goto('/');
  const result = await page.evaluate(
    async ({ creator, questions }) => {
      const foldPath = '/src/fold.ts',
        fixturePath = '/test/update-fixtures.ts';
      const { Fold } = await import(foldPath),
        { metadataUpdate } = await import(fixturePath);
      const fold = new Fold();
      try {
        const before = await fold.run({
          type: 'apply',
          updates: [metadataUpdate({ creator, questions })],
        });
        const edit = await fold.prepareContent('<p>Changed source</p>', {
          source: before.source,
          title: before.title,
          own: before.own,
          creator: before.creator,
          questions: before.questions,
        });
        const after = await fold.run({ type: 'apply', updates: edit.updates });
        const invalid = new Fold();
        let rejected = false;
        try {
          rejected = await invalid
            .run({
              type: 'apply',
              updates: [
                metadataUpdate({
                  creator,
                  questions: questions.map((entry: Record<string, unknown>) => ({
                    ...entry,
                    state: 'answered',
                  })),
                }),
              ],
            })
            .then(
              () => false,
              () => true,
            );
        } finally {
          invalid.close();
        }
        return {
          creator: after.creator,
          questions: after.questions,
          source: after.source,
          rejected,
        };
      } finally {
        fold.close();
      }
    },
    { creator, questions },
  );
  expect(result).toEqual({ creator, questions, source: '<p>Changed source</p>', rejected: true });
});

test('real Worker prepares edit locks without committing and admits increasing revisions and refuses stale replacement or removal', async ({
  page,
}) => {
  const fixture = JSON.parse(
    readFileSync(new URL('../../../contracts/vectors/edit-lock-v1.json', import.meta.url), 'utf8'),
  );
  const Y = await import('yjs');
  const records = ['advance', 'replace', 'remove'].map((mode) => {
    const doc = new Y.Doc();
    for (const root of ['threads', 'messages', 'intents', 'replies']) doc.getMap(root);
    const key = `${fixture.record.holder.id}:edit-lock`;
    doc.getMap('messages').set(key, fixture.record);
    const baseline = Y.encodeStateAsUpdate(doc);
    const vector = Y.encodeStateVector(doc);
    if (mode === 'remove') doc.getMap('messages').delete(key);
    else
      doc.getMap('messages').set(key, {
        ...fixture.record,
        holder: { ...fixture.record.holder, name: 'Changed label' },
        revision: mode === 'advance' ? '2' : fixture.record.revision,
      });
    const tail = Y.encodeStateAsUpdate(doc, vector);
    doc.destroy();
    return { baseline: [...baseline], tail: [...tail] };
  });
  await page.goto('/');
  const result = await page.evaluate(
    async ({ fixture, records }) => {
      const path = '/src/fold.ts';
      const { Fold } = await import(path);
      const writer = fixture.record.senderDevice;
      const key = `${fixture.record.holder.id}:edit-lock`;
      const prepared = new Fold();
      let proposal;
      let committed;
      try {
        proposal = (
          await prepared.run({
            type: 'prepare-own',
            writer,
            records: [{ root: 'messages', key, value: fixture.record }],
          })
        ).own[writer].messages[key];
        committed = (await prepared.run({ type: 'apply', updates: [] })).own;
      } finally {
        prepared.close();
      }
      const refused = [];
      let preparedRevision;
      for (const bytes of records) {
        const fold = new Fold();
        try {
          const base = await fold.run({
            type: 'apply',
            updates: [],
            own: [{ writer, update: new Uint8Array(bytes.baseline) }],
          });
          if (JSON.stringify(base.own[writer].messages[key]) !== JSON.stringify(fixture.record))
            throw new Error('Missing initial record');
          const replacement = await fold.run({
            type: 'prepare-own',
            writer,
            records: [
              {
                root: 'messages',
                key,
                value: { ...fixture.record, revision: '2', released: true },
              },
            ],
          });
          preparedRevision = replacement.own[writer].messages[key].revision;
          const unchanged = await fold.run({ type: 'apply', updates: [] });
          if (unchanged.own[writer].messages[key].revision !== fixture.record.revision)
            throw new Error('Prepared revision leaked before commit');
          refused.push(
            await fold
              .run({
                type: 'apply',
                updates: [],
                own: [{ writer, update: new Uint8Array(bytes.tail) }],
              })
              .then(
                () => false,
                () => true,
              ),
          );
        } finally {
          fold.close();
        }
      }
      return { proposal, committed, refused, preparedRevision };
    },
    { fixture, records },
  );
  expect(result.proposal).toEqual(fixture.record);
  expect(result.committed).toEqual({});
  expect(result.refused).toEqual([false, true, true]);
  expect(result.preparedRevision).toBe('2');
});

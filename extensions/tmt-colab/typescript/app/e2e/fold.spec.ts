import { expect, test as base } from '@playwright/test';

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
      const one = await a.run({ type: 'prepare', source: '<p>😀 café</p>' });
      await a.run({ type: 'apply', updates: [one.update] });
      await b.run({ type: 'apply', updates: [one.update] });
      const two = await a.run({ type: 'prepare', source: '<p>😀 café A</p>' });
      const three = await b.run({ type: 'prepare', source: '<p>😀 café B</p>' });
      const mergedA = await a.run({ type: 'apply', updates: [two.update, three.update] });
      const mergedB = await b.run({ type: 'apply', updates: [two.update, three.update] });
      const restored = await reloaded.run({
        type: 'apply',
        updates: [one.update, two.update, three.update],
      });
      return {
        one: one.source,
        two: two.source,
        a: mergedA.source,
        b: mergedB.source,
        restored: restored.source,
        updateBytes: two.update.length,
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
      await fold.run({ type: 'prepare', source: 'must not apply' }).catch(() => errors++);
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
        const previous = await fold.run({ type: 'prepare', source: 'previous valid source' });
        await fold.run({ type: 'apply', updates: [previous.update] });
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
      const seed = await a.run({ type: 'prepare', source: 'initial' });
      await a.run({ type: 'apply', updates: [seed.update] });
      await b.run({ type: 'apply', updates: [seed.update] });
      await a.run({ type: 'prepare', source: 'unsaved draft' });
      const remote = await b.run({ type: 'prepare', source: 'initial saved' });
      return (await a.run({ type: 'apply', updates: [remote.update] })).source;
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
          one.publisherAgent !== v.publisherAgent ||
          two.publisherAgent !== v.publisherAgent
        )
          throw new Error('Baseline vector projection');
        const edit = await a.run({ type: 'prepare', source: v.source + 'later' });
        const left = await a.run({ type: 'apply', updates: [edit.update] }),
          right = await b.run({ type: 'apply', updates: [edit.update] });
        if (
          left.source !== right.source ||
          left.publisherAgent !== v.publisherAgent ||
          right.publisherAgent !== v.publisherAgent
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

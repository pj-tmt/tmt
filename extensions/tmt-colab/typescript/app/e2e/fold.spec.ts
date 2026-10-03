import { expect, test } from '@playwright/test';

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
        if (one.source !== v.source || two.source !== v.source || one.title !== v.title)
          throw new Error('Baseline vector projection');
        const edit = await a.run({ type: 'prepare', source: v.source + 'later' });
        const left = await a.run({ type: 'apply', updates: [edit.update] }),
          right = await b.run({ type: 'apply', updates: [edit.update] });
        if (left.source !== right.source) throw new Error('Baseline struct identity differs');
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

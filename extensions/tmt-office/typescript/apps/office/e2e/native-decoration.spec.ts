import { readFileSync } from 'node:fs';
import { expect } from '@playwright/test';
import { test } from './browser-session.js';
import {
  layoutFile,
  openAssignedBlock,
  rawFile,
  withNativeDecoration,
} from './native-decoration-fixture.js';

test('native block show/apply reaches durable state and visible browser geometry', async ({
  openSession,
}) => {
  test.setTimeout(150_000);
  await withNativeDecoration(openSession, false, async (context) => {
    const missing = await context.office(['block', 'show']);
    expect(missing.status, missing.stdout).toBe(0);
    expect(JSON.parse(missing.stdout)).toMatchObject({
      blockId: context.blockId,
      revision: 0,
      objects: [],
    });
    expect((await context.blockRef.get()).exists).toBe(false);

    const objects = [
      { asset: 'desk', x: 2, y: 3, rotation: 0 },
      { asset: 'plant', x: 8, y: 4, rotation: 0 },
    ] as const;
    const file = layoutFile(context.sandbox, 'native-layout.json', objects);
    const applied = await context.office(
      ['block', 'apply'],
      ['--file', file, '--if-revision', '0']
    );
    expect(applied.status, applied.stdout).toBe(0);
    const result = JSON.parse(applied.stdout);
    expect(result).toMatchObject({ blockId: context.blockId, revision: 1, objects });
    const stored = (await context.blockRef.get()).data()!;
    expect(stored).toMatchObject({ version: 1, revision: 1, objects: ['d023', 'p084'] });
    expect(stored.updatedAt).toBeDefined();

    const retry = await context.office(['block', 'apply'], ['--file', file, '--if-revision', '0']);
    expect(retry.status, retry.stdout).toBe(0);
    expect(JSON.parse(retry.stdout)).toEqual(result);
    expect((await context.blockRef.get()).data()).toEqual(stored);

    await openAssignedBlock(context.page, context.worldId, context.blockId);
    await expect(context.page.getByRole('button', { name: 'Desk 1', exact: true })).toBeVisible();
    await expect(context.page.getByRole('button', { name: 'Plant 2', exact: true })).toBeVisible();
    expect(
      await context.page.locator('svg.block-scene > g > g').evaluateAll((elements) =>
        elements.map((element) => ({
          placement: element.getAttribute('transform'),
          artwork: element.firstElementChild?.getAttribute('transform'),
        }))
      )
    ).toEqual([
      {
        placement: 'translate(2 3)',
        artwork: 'translate(2 1) rotate(0) translate(-2 -1)',
      },
      {
        placement: 'translate(8 4)',
        artwork: 'translate(1 1) rotate(0) translate(-1 -1)',
      },
    ]);
    for (const viewport of [
      { name: 'desktop', width: 1280, height: 900 },
      { name: 'narrow', width: 390, height: 844 },
    ]) {
      await context.page.setViewportSize(viewport);
      await expect(context.page.getByRole('button', { name: 'Desk 1', exact: true })).toBeVisible();
      expect(
        await context.page.evaluate(() => document.documentElement.scrollWidth)
      ).toBeLessThanOrEqual(viewport.width);
      await context.page.locator('.block-editor').screenshot({
        path: test.info().outputPath(`native-decoration-${viewport.name}.png`),
      });
    }
  });
});

test('native block apply preserves conflicts, concurrent winners and invalid-input state', async ({
  openSession,
}) => {
  test.setTimeout(150_000);
  await withNativeDecoration(openSession, false, async (context) => {
    const firstObjects = [{ asset: 'desk', x: 2, y: 2, rotation: 0 }] as const;
    const firstFile = layoutFile(context.sandbox, 'native-first-layout.json', firstObjects);
    const first = await context.office(
      ['block', 'apply'],
      ['--file', firstFile, '--if-revision', '0']
    );
    expect(first.status, first.stdout).toBe(0);
    const beforeConflict = (await context.blockRef.get()).data()!;

    const staleFile = layoutFile(context.sandbox, 'native-stale-layout.json', [
      { asset: 'plant', x: 8, y: 2, rotation: 0 },
    ]);
    const staleFileBytes = readFileSync(staleFile);
    const stale = await context.office(
      ['block', 'apply'],
      ['--file', staleFile, '--if-revision', '0']
    );
    expect(stale.status).toBe(1);
    expect(JSON.parse(stale.stdout).error.code).toBe('OFFICE_REVISION_CONFLICT');
    expect(stale.stderr).toBe('');
    expect((await context.blockRef.get()).data()).toEqual(beforeConflict);
    expect(readFileSync(staleFile)).toEqual(staleFileBytes);

    const leftFile = layoutFile(context.sandbox, 'native-left-layout.json', [
      { asset: 'desk', x: 4, y: 4, rotation: 0 },
    ]);
    const rightFile = layoutFile(context.sandbox, 'native-right-layout.json', [
      { asset: 'plant', x: 10, y: 4, rotation: 0 },
    ]);
    const leftFileBytes = readFileSync(leftFile);
    const rightFileBytes = readFileSync(rightFile);
    // The local lock is fail-fast, not a queue. A loser can be rejected before
    // execution; its later original-intent retry must detect the winner.
    const concurrent = await Promise.all([
      context.office(['block', 'apply'], ['--file', leftFile, '--if-revision', '1']),
      context.office(['block', 'apply'], ['--file', rightFile, '--if-revision', '1']),
    ]);
    expect(concurrent.map((result) => result.status).sort()).toEqual([0, 1]);
    const winnerIndex = concurrent.findIndex((result) => result.status === 0);
    const winner = concurrent.find((result) => result.status === 0)!;
    const loser = concurrent.find((result) => result.status === 1)!;
    expect(['OFFICE_BUSY', 'OFFICE_REVISION_CONFLICT']).toContain(
      JSON.parse(loser.stdout).error.code
    );
    const winnerObjects = JSON.parse(winner.stdout).objects;
    const expectedWinner =
      winnerIndex === 0
        ? { objects: [{ asset: 'desk', x: 4, y: 4, rotation: 0 }], tokens: ['d044'] }
        : { objects: [{ asset: 'plant', x: 10, y: 4, rotation: 0 }], tokens: ['p0a4'] };
    expect(winnerObjects).toEqual(expectedWinner.objects);
    const afterConcurrent = (await context.blockRef.get()).data()!;
    expect(afterConcurrent.revision).toBe(2);
    expect(afterConcurrent.objects).toEqual(expectedWinner.tokens);
    const retryLoser = await context.office(
      ['block', 'apply'],
      ['--file', winnerIndex === 0 ? rightFile : leftFile, '--if-revision', '1']
    );
    expect(retryLoser.status).toBe(1);
    expect(JSON.parse(retryLoser.stdout).error.code).toBe('OFFICE_REVISION_CONFLICT');
    expect((await context.blockRef.get()).data()).toEqual(afterConcurrent);
    expect(readFileSync(leftFile)).toEqual(leftFileBytes);
    expect(readFileSync(rightFile)).toEqual(rightFileBytes);

    const unrelated = await context.office(['block', 'show', crypto.randomUUID()]);
    expect(unrelated.status).toBe(1);
    expect(JSON.parse(unrelated.stdout).error.code).toBe('OFFICE_REMOTE_DENIED');
    expect((await context.blockRef.get()).data()).toEqual(afterConcurrent);

    const malformedFile = rawFile(context.sandbox, 'malformed.json', '{"objects":');
    const malformedBytes = readFileSync(malformedFile);
    const malformed = await context.office(
      ['block', 'apply'],
      ['--file', malformedFile, '--if-revision', '2']
    );
    expect(malformed.status).toBe(1);
    expect(JSON.parse(malformed.stdout).error.code).toBe('OFFICE_LAYOUT_INVALID');
    expect((await context.blockRef.get()).data()).toEqual(afterConcurrent);
    expect(readFileSync(malformedFile)).toEqual(malformedBytes);

    const oversizedFile = rawFile(
      context.sandbox,
      'oversized.json',
      `{"objects":[]}${' '.repeat(65_536)}`
    );
    const oversizedBytes = readFileSync(oversizedFile);
    const oversized = await context.office(
      ['block', 'apply'],
      ['--file', oversizedFile, '--if-revision', '2']
    );
    expect(oversized.status).toBe(1);
    expect(JSON.parse(oversized.stdout).error.code).toBe('OFFICE_LAYOUT_INVALID');
    expect((await context.blockRef.get()).data()).toEqual(afterConcurrent);
    expect(readFileSync(oversizedFile)).toEqual(oversizedBytes);

    const overflowFile = layoutFile(
      context.sandbox,
      'native-overflow-layout.json',
      Array.from({ length: 17 }, (_, index) => ({
        asset: 'plant',
        x: (index % 8) * 2,
        y: Math.floor(index / 8) * 2,
        rotation: 0,
      }))
    );
    const overflow = await context.office(
      ['block', 'apply'],
      ['--file', overflowFile, '--if-revision', '2']
    );
    expect(overflow.status).toBe(1);
    expect(JSON.parse(overflow.stdout).error.code).toBe('OFFICE_LAYOUT_INVALID');
    expect((await context.blockRef.get()).data()).toEqual(afterConcurrent);
  });
});

test('revoked native grant denies show and apply without changing the durable block', async ({
  openSession,
}) => {
  test.setTimeout(120_000);
  await withNativeDecoration(openSession, false, async (context) => {
    const file = layoutFile(context.sandbox, 'native-revocation-layout.json', [
      { asset: 'desk', x: 2, y: 2, rotation: 0 },
    ]);
    const fileBytes = readFileSync(file);
    const applied = await context.office(
      ['block', 'apply'],
      ['--file', file, '--if-revision', '0']
    );
    expect(applied.status, applied.stdout).toBe(0);
    const beforeRevocation = (await context.blockRef.get()).data()!;
    await context.grantRef.update({ enabled: false });

    const deniedShow = await context.office(['block', 'show']);
    expect(deniedShow.status).toBe(1);
    expect(JSON.parse(deniedShow.stdout).error.code).toBe('OFFICE_REMOTE_DENIED');
    const deniedApply = await context.office(
      ['block', 'apply'],
      ['--file', file, '--if-revision', '1']
    );
    expect(deniedApply.status).toBe(1);
    expect(JSON.parse(deniedApply.stdout).error.code).toBe('OFFICE_REMOTE_DENIED');
    expect((await context.blockRef.get()).data()).toEqual(beforeRevocation);
    expect(readFileSync(file)).toEqual(fileBytes);
  });
});

test('read-only native pairing can show its assigned block but cannot apply a layout', async ({
  openSession,
}) => {
  test.setTimeout(120_000);
  await withNativeDecoration(openSession, true, async (context) => {
    const shown = await context.office(['block', 'show']);
    expect(shown.status, shown.stdout).toBe(0);
    expect(JSON.parse(shown.stdout)).toMatchObject({
      blockId: context.blockId,
      revision: 0,
      objects: [],
    });
    const file = layoutFile(context.sandbox, 'native-read-only-layout.json', [
      { asset: 'plant', x: 8, y: 8, rotation: 0 },
    ]);
    const fileBytes = readFileSync(file);
    const denied = await context.office(['block', 'apply'], ['--file', file, '--if-revision', '0']);
    expect(denied.status).toBe(1);
    expect(JSON.parse(denied.stdout).error.code).toBe('OFFICE_REMOTE_DENIED');
    expect((await context.blockRef.get()).exists).toBe(false);
    expect(readFileSync(file)).toEqual(fileBytes);
  });
});

import { expect, test } from '@playwright/test';
import { openReaderLink, startDoor } from './harness/browser.js';
import { createPage, freePort, run } from './harness/ask.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';
import type { AcceptanceWorld } from './harness/world.js';

// #1545 read-only share link. The page, the link and Reset are the real owner commands; the
// reader is a Chromium profile that never paired with the door.

test.afterEach(disposeActiveWorlds);

const colab = (world: AcceptanceWorld, args: string[], input?: string) =>
  JSON.parse(run(world, world.binaries.colab, [...args, '--json'], input)) as Record<
    string,
    never
  > &
    Record<string, unknown>;

/** The seed is the one secret in a reader link; it lives in the fragment only. */
const seedOf = (readerPath: string) => new URLSearchParams(readerPath.split('#')[1]).get('seed')!;

test('reader link: opens unpaired, shows live edits read-only, and ends on Reset', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const created = createPage(world, 'Reader acceptance', '<h1 id="text">First text</h1>');
    colab(world, ['share', 'mode', created.pageId, 'link', '--yes']);
    const added = colab(world, ['share', 'link', 'add', created.pageId, '--yes']);
    const readerPath = added.readerPath as string;
    expect(readerPath.startsWith('x/colab/read#v=1&')).toBe(true);

    // An unpaired profile gets the reader entry but no owner file.
    const owner = await fetch(`${door.address}/x/colab/index.html`);
    expect(owner.status).toBe(403);

    const reader = await openReaderLink(world, door, readerPath, 'reader-one');
    const frame = reader.page.frameLocator('iframe');
    await expect(frame.locator('#text')).toHaveText('First text', { timeout: 30_000 });
    // The fragment left the address bar, and the page offers no write or Ask control.
    expect(await reader.page.evaluate(() => location.hash)).toBe('');
    await expect(reader.page.getByText('Read-only').first()).toBeVisible();
    await expect(reader.page.locator('textarea, [data-testid=ask-action]')).toHaveCount(0);

    // A live edit by the owner's agent reaches the reader.
    const read = colab(world, ['page', 'read', created.pageId]);
    colab(
      world,
      [
        'page',
        'write',
        created.pageId,
        '--file',
        '-',
        '--expected-revision',
        read.revision as string,
      ],
      '<h1 id="text">Second text</h1>',
    );
    await expect(frame.locator('#text')).toHaveText('Second text', { timeout: 30_000 });

    // The seed never left the browser: no request URL or body carries it.
    const seed = seedOf(readerPath);
    expect(
      reader.requests.filter((r) => r.url.includes(seed) || (r.body ?? '').includes(seed)),
    ).toEqual([]);

    // Reset ends the old link, live, and the replacement opens in another profile.
    const reset = colab(world, [
      'share',
      'link',
      'reset',
      created.pageId,
      added.linkId as string,
      '--yes',
    ]);
    await expect(reader.page.getByRole('heading', { name: 'Access ended' })).toBeVisible({
      timeout: 60_000,
    });
    await expect(reader.page.locator('iframe')).toHaveCount(0);
    const stale = await openReaderLink(world, door, readerPath, 'reader-stale');
    await expect(stale.page.getByRole('heading', { name: 'Access ended' })).toBeVisible({
      timeout: 30_000,
    });
    const fresh = await openReaderLink(world, door, reset.readerPath as string, 'reader-fresh');
    await expect(fresh.page.frameLocator('iframe').locator('#text')).toHaveText('Second text', {
      timeout: 30_000,
    });
  });
});

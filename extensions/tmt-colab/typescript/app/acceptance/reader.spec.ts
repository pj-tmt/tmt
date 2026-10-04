import { execFileSync } from 'node:child_process';
import { mkdir } from 'node:fs/promises';
import path from 'node:path';
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

/** Link-device rows in the owner store: one per link, however often it is opened. */
const deviceRows = (world: AcceptanceWorld) =>
  Number(
    execFileSync(
      'sqlite3',
      [path.join(world.dataRoot, 'colab', 'space.db'), 'SELECT count(*) FROM devices'],
      { encoding: 'utf8' },
    ).trim(),
  );

/** The seed is the one secret in a reader link; it lives in the fragment only. */
const seedOf = (readerPath: string) => new URLSearchParams(readerPath.split('#')[1]).get('seed')!;

test('reader link: opens unpaired, shows live edits read-only, and ends on Reset', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const longBody = Array.from(
      { length: 48 },
      (_, index) =>
        `<p style="min-height:54px">Reader paragraph ${index + 1}. The browser window scrolls beneath the Colab bar.</p>`,
    ).join('');
    const created = createPage(
      world,
      'Reader acceptance',
      `<h1 id="text">First text</h1><a href="#target">Jump to reader anchor</a>${longBody}<h2 id="target">Reader anchor</h2><div style="height:900px"></div>`,
    );
    colab(world, ['share', 'mode', created.pageId, 'link', '--yes']);
    const added = colab(world, ['share', 'link', 'add', created.pageId, '--yes']);
    const readerPath = added.readerPath as string;
    expect(readerPath.startsWith('x/colab/read#v=1&')).toBe(true);

    // An unpaired profile gets the reader entry but no owner file.
    const owner = await fetch(`${door.address}/x/colab/index.html`);
    expect(owner.status).toBe(403);

    const baseline = deviceRows(world);
    const reader = await openReaderLink(world, door, readerPath, 'reader-one');
    await expect(reader.page).toHaveTitle('Colab');
    const frame = reader.page.frameLocator('iframe');
    await expect(frame.locator('#text')).toHaveText('First text', { timeout: 30_000 });
    const element = reader.page.locator('iframe');
    await expect.poll(async () => (await element.boundingBox())?.height ?? 0).toBeGreaterThan(3000);
    await expect(element).toHaveAttribute('scrolling', 'no');
    await expect
      .poll(() =>
        frame.locator('html').evaluate((node) => node.scrollHeight <= node.clientHeight + 1),
      )
      .toBe(true);
    const captures = process.env.COLAB_READER_CAPTURE_DIR;
    if (captures) await mkdir(captures, { recursive: true });
    for (const width of [1440, 390]) {
      await reader.page.setViewportSize({ width, height: 900 });
      for (const theme of ['light', 'dark'] as const) {
        await reader.page.evaluate(
          (value) => (document.documentElement.dataset.theme = value),
          theme,
        );
        await expect
          .poll(async () => (await element.boundingBox())?.height ?? 0)
          .toBeGreaterThan(3000);
        const frameBox = await element.boundingBox();
        const contentWidth = await reader.page.evaluate(() => document.documentElement.clientWidth);
        expect(frameBox?.x).toBe(0);
        expect(Math.abs((frameBox?.width ?? 0) - contentWidth)).toBeLessThan(1);
        await reader.page.evaluate(() => window.scrollTo(0, 0));
        await expect.poll(() => reader.page.evaluate(() => window.scrollY)).toBe(0);
        await expect(reader.page.locator('.colab-header')).toBeInViewport();
        if (captures)
          await reader.page.screenshot({ path: `${captures}/reader-${width}-${theme}-top.png` });
        await reader.page.evaluate(() => window.scrollTo(0, 1800));
        await expect.poll(() => reader.page.evaluate(() => window.scrollY)).toBeGreaterThan(1000);
        await expect(reader.page.locator('.colab-header')).toBeInViewport();
        expect(
          Math.abs((await reader.page.locator('.colab-header').boundingBox())?.y ?? Infinity),
        ).toBeLessThan(1);
        expect(
          await frame.locator('html').evaluate((node) => node.ownerDocument.defaultView!.scrollY),
        ).toBe(0);
        if (captures)
          await reader.page.screenshot({
            path: `${captures}/reader-${width}-${theme}-scrolled.png`,
          });
      }
    }
    await reader.page.setViewportSize({ width: 1280, height: 900 });
    await reader.page.evaluate(() => {
      document.documentElement.dataset.theme = 'light';
      window.scrollTo(0, 0);
    });
    const frameWidth = (await element.boundingBox())?.width;
    const pageHeight = await reader.page.evaluate(() => document.documentElement.scrollHeight);
    await reader.page.getByLabel('Info', { exact: true }).click();
    await expect(reader.page.getByText('You are reading a shared page.')).toBeVisible();
    expect((await element.boundingBox())?.width).toBe(frameWidth);
    expect(await reader.page.evaluate(() => document.documentElement.scrollHeight)).toBe(
      pageHeight,
    );
    await reader.page.getByLabel('Info', { exact: true }).click();
    await frame.getByRole('link', { name: 'Jump to reader anchor' }).click();
    await expect.poll(() => reader.page.evaluate(() => window.scrollY)).toBeGreaterThan(2000);
    expect(
      await frame.locator('html').evaluate((node) => node.ownerDocument.defaultView!.scrollY),
    ).toBe(0);
    await reader.page.evaluate(() => window.scrollTo(0, 0));
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
    await expect
      .poll(async () => (await element.boundingBox())?.height ?? Infinity)
      .toBeLessThan(1100);

    // Re-opening the same link in more browsers presents the same derived device: still one row.
    for (const name of ['reader-again-1', 'reader-again-2']) {
      const again = await openReaderLink(world, door, readerPath, name);
      await expect(again.page.frameLocator('iframe').locator('#text')).toHaveText('Second text', {
        timeout: 30_000,
      });
    }
    expect(deviceRows(world)).toBe(baseline + 1);

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
    await expect(reader.page.getByRole('heading', { name: 'Access ended', level: 2 })).toBeVisible({
      timeout: 60_000,
    });
    await expect(reader.page.locator('iframe')).toHaveCount(0);
    const stale = await openReaderLink(world, door, readerPath, 'reader-stale');
    await expect(stale.page.getByRole('heading', { name: 'Access ended', level: 2 })).toBeVisible({
      timeout: 30_000,
    });
    const fresh = await openReaderLink(world, door, reset.readerPath as string, 'reader-fresh');
    await expect(fresh.page.frameLocator('iframe').locator('#text')).toHaveText('Second text', {
      timeout: 30_000,
    });
    // The replacement link is a new identity with its own device; the old one stays revoked.
    expect(deviceRows(world)).toBe(baseline + 2);
  });
});

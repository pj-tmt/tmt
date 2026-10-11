import fs from 'node:fs';
import path from 'node:path';
import { expect, test } from '@playwright/test';
import { openReaderLink, pairBrowser, startDoor } from './harness/browser.js';
import { clientId, createPage, freePort, openPage, run } from './harness/ask.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';

test.afterEach(disposeActiveWorlds);
const PNG = Buffer.from(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/q842iQAAAABJRU5ErkJggg==',
  'base64',
);

test('document image references disclose verified page files to paired and reader views, and end with the ticket', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const author = await pairBrowser(world, 'document-images');
    const created = createPage(world, 'Document images', '<h1>Document images</h1>');
    const foreign = createPage(world, 'Other page', '<h1>Other page</h1>');
    const page = await openPage(door, author, created);
    await expect(
      page.frameLocator('iframe').getByRole('heading', { name: 'Document images' }),
    ).toBeVisible();
    const colab = (args: string[], input?: string) =>
      JSON.parse(run(world, world.binaries.colab, [...args, '--json'], input));
    const image = path.join(world.root, 'image.png');
    const foreignImage = path.join(world.root, 'foreign.png');
    const pdf = path.join(world.root, 'not-an-image.pdf');
    // Valid PNG followed by inert trailing bytes: the object is bigger than the HTML cap,
    // while its raster dimensions remain bounded. Embedding its base64 could not fit.
    const imageBytes = Buffer.concat([PNG, Buffer.alloc(2 * 1024 * 1024, 0x5a)]);
    fs.writeFileSync(image, imageBytes);
    fs.writeFileSync(foreignImage, PNG);
    fs.writeFileSync(pdf, '%PDF-1.7\n');
    const attached = colab(['attachment', 'attach', created.pageId, image]).attachment;
    const other = colab(['attachment', 'attach', foreign.pageId, foreignImage]).attachment;
    const document = colab(['attachment', 'attach', created.pageId, pdf]).attachment;
    const source = `<h1>Document images</h1><img id="full" src="colab-attachment:${attached.attachmentId}" alt="Full ID"><img id="short" src="colab-attachment:${attached.attachmentId.slice(0, 8)}" alt="Short ID"><img src="colab-attachment:${other.attachmentId}" alt="Another page"><img src="colab-attachment:${document.attachmentId}" alt="Not raster">`;
    colab(['page', 'write', created.pageId, '--file', '-'], source);
    const written = colab(['page', 'read', created.pageId]);
    expect(written.source).toBe(source);
    expect(Buffer.byteLength(written.source)).toBeLessThan(2048);
    const check = async (viewer: typeof page) => {
      const frame = viewer.frameLocator('iframe');
      for (const id of ['full', 'short']) {
        await expect(frame.locator(`#${id}`)).toHaveAttribute('src', /^data:image\/png;base64,/, {
          timeout: 30_000,
        });
        const url = (await frame.locator(`#${id}`).getAttribute('src'))!;
        expect(Buffer.from(url.slice(url.indexOf(',') + 1), 'base64')).toEqual(imageBytes);
        await expect
          .poll(() =>
            frame.locator(`#${id}`).evaluate((image: HTMLImageElement) => image.naturalWidth),
          )
          .toBe(1);
      }
      await expect(frame.getByRole('img', { name: 'Another page' })).toContainText(
        'Image unavailable',
      );
      await expect(frame.getByRole('img', { name: 'Not raster' })).toContainText(
        'Not shown: unsupported image type',
      );
    };
    await check(page);
    colab(['share', 'mode', created.pageId, 'link', '--yes']);
    const link = colab(['share', 'link', 'add', created.pageId, '--yes']);
    const reader = await openReaderLink(world, door, link.readerPath, 'document-image-reader');
    await check(reader.page);
    colab(['share', 'link', 'reset', created.pageId, link.linkId, '--yes']);
    await expect(
      reader.page.getByRole('heading', { name: 'Access ended', level: 2 }),
    ).toBeVisible();
    await expect(reader.page.locator('iframe')).toHaveCount(0);
    await expect(reader.page.locator('img[src^="data:"]')).toHaveCount(0);
  });
});

test('page images keep their attachment references after a key change and the second Save', async () => {
  test.setTimeout(180_000);
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const author = await pairBrowser(world, 'image-rekey-author');
    const viewer = await pairBrowser(world, 'image-rekey-viewer');
    const created = createPage(world, 'Rekey images', '<h1>Rekey images</h1>');
    const authorPage = await openPage(door, author, created);
    await expect(
      authorPage.frameLocator('iframe').getByRole('heading', { name: 'Rekey images' }),
    ).toBeVisible();
    const page = await openPage(door, viewer, created);
    await expect(
      page.frameLocator('iframe').getByRole('heading', { name: 'Rekey images' }),
    ).toBeVisible();
    const colab = (args: string[], input?: string) =>
      JSON.parse(run(world, world.binaries.colab, [...args, '--json'], input));
    const file = path.join(world.root, 'image.png');
    fs.writeFileSync(file, PNG);
    const old = colab(['attachment', 'attach', created.pageId, file]).attachment
      .attachmentId as string;
    const html = (id: string, short: string) =>
      `<h1>Rekey images</h1>é<img id="full" src="colab-attachment:${id}" alt="Full"><img id="short" src="colab-attachment:${short}" alt="Short">🦀`;
    colab(['page', 'write', created.pageId, '--file', '-'], html(old, old.slice(0, 8)));
    const check = async () => {
      for (const id of ['full', 'short']) {
        const image = page.frameLocator('iframe').locator(`#${id}`);
        await expect(image).toHaveAttribute('src', /^data:image\/png;base64,/, { timeout: 30_000 });
        const url = (await image.getAttribute('src'))!;
        expect(Buffer.from(url.slice(url.indexOf(',') + 1), 'base64')).toEqual(PNG);
        await expect
          .poll(() => image.evaluate((node: HTMLImageElement) => node.naturalWidth))
          .toBe(1);
      }
    };
    await check();
    run(world, world.binaries.remote, [
      'devices',
      'revoke',
      clientId(world, author.name),
      '--json',
    ]);
    let current = old;
    await expect
      .poll(
        () => {
          const read = colab(['page', 'read', created.pageId]).source as string;
          const match = read.match(/id="full" src="colab-attachment:([0-9a-f-]+)"/);
          current = match?.[1] ?? old;
          return current !== old && read === html(current, current);
        },
        { timeout: 120_000 },
      )
      .toBe(true);
    await check();
    expect(colab(['page', 'read', created.pageId]).source).toBe(html(current, current));
  });
});

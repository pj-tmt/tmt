import { expect, test } from '@playwright/test';
import { createPage, freePort, openPage, run, selectInRenderer } from './harness/ask.js';
import {
  openReaderLink,
  pairBrowser,
  restartColab,
  restartRemote,
  startDoor,
} from './harness/browser.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';

test.afterEach(disposeActiveWorlds);
test('real Colab keeps public short entries across admission, reload and history', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const created = createPage(
      world,
      'Short routing proof',
      '<h1 id="proof">Short route content</h1>',
    );
    const shown = JSON.parse(
      run(world, world.binaries.colab, ['show', created.pageId, '--json']),
    ) as { link: string; shortLink: string; path: string; spaceId: string };
    expect(shown.link).toBe(shown.shortLink);
    expect(new URL(shown.link).pathname).toBe(created.path);
    expect(new URL(shown.link).hash).toBe('');
    const device = await pairBrowser(world, 'short-owner', { talk: true });
    const page = await openPage(door, device, created);
    const proof = () => page.frameLocator('iframe').locator('#proof');
    await expect(proof()).toHaveText('Short route content');
    expect(page.url()).toBe(shown.link);
    await page.reload();
    await expect(proof()).toHaveText('Short route content');
    expect(page.url()).toBe(shown.link);
    await selectInRenderer(page, '#proof');
    await page.getByTestId('selection-ask').click();
    const compose = page.getByRole('dialog', { name: 'Annotate selection' });
    await compose
      .getByRole('combobox', { name: 'Message', exact: true })
      .fill('Short thread proof');
    await compose.getByRole('button', { name: 'Send', exact: true }).click();
    const thread = compose.getByTestId('comment-thread');
    await expect(thread.locator('.comment-body')).toHaveText('Short thread proof');
    const threadId = await thread.getAttribute('data-thread-id');
    expect(threadId).not.toBeNull();
    const threadUrl = `${shown.link}#t=${threadId}`;
    await page.goto(threadUrl);
    await expect(page.getByTestId('comment-thread').locator('.comment-body')).toHaveText(
      'Short thread proof',
    );
    expect(page.url()).toBe(threadUrl);
    await page.goto(shown.link);
    await expect(proof()).toHaveText('Short route content');
    await page.getByRole('link', { name: 'Space home', exact: true }).click();
    await expect(
      page.locator(`[data-page-id="${created.pageId}"]`).getByRole('link'),
    ).toBeVisible();
    expect(new URL(page.url()).pathname).toBe('/colab/');
    await page.goBack();
    await expect(proof()).toHaveText('Short route content');
    expect(page.url()).toBe(shown.link);
    await page.goForward();
    await expect(
      page.locator(`[data-page-id="${created.pageId}"]`).getByRole('link'),
    ).toBeVisible();
    expect(new URL(page.url()).pathname).toBe('/colab/');
    await page.goto(
      `${door.mounts}colab/#space=${shown.spaceId}&path=%2Fpages%2F${created.pageId}`,
    );
    await expect(proof()).toHaveText('Short route content');
    expect(page.url()).toBe(shown.link);
    await page.reload();
    await expect(proof()).toHaveText('Short route content');
    expect(page.url()).toBe(shown.link);
    for (const route of ['/colab', '/colab/']) {
      await page.goto(new URL(route, door.origin).href);
      await expect(
        page.locator(`[data-page-id="${created.pageId}"]`).getByRole('link'),
      ).toBeVisible();
      expect(new URL(page.url()).pathname).toBe(route);
    }
    // A fresh public response supplies the current private mount after either process restarts.
    for (const restart of [restartColab, restartRemote]) {
      await restart(world, door);
      await page.goto(shown.link);
      await expect(proof()).toHaveText('Short route content');
      expect(page.url()).toBe(shown.link);
      await page.reload();
      await expect(proof()).toHaveText('Short route content');
      expect(page.url()).toBe(shown.link);
    }
    const colab = (args: string[]) =>
      JSON.parse(run(world, world.binaries.colab, [...args, '--json'])) as Record<string, unknown>;
    colab(['share', 'mode', created.pageId, 'link', '--yes']);
    const added = colab(['share', 'link', 'add', created.pageId, '--yes']);
    const readerPath = added.readerPath as string;
    const capability = new URL(readerPath, door.origin);
    expect(capability.pathname).toBe(`/read/${added.linkId}`);
    expect(added.readerUrl).toBe(capability.href);
    const reader = await openReaderLink(world, door, readerPath, 'short-reader');
    await expect(reader.page.frameLocator('iframe').locator('#proof')).toHaveText(
      'Short route content',
    );
    expect(reader.page.url()).toBe(`${door.origin}${capability.pathname}`);
    expect(
      reader.requests.every(
        (request) => !request.url.includes('#') && !request.url.includes('seed='),
      ),
    ).toBe(true);
    expect((await fetch(`${door.mounts}colab/index.html`)).status).toBe(403);
    colab(['share', 'link', 'remove', created.pageId, String(added.linkId), '--yes']);
    await expect(
      reader.page.getByRole('heading', { name: 'Access ended', level: 2 }),
    ).toBeVisible();
    const requestCount = reader.requests.length;
    await reader.page.goto(shown.link);
    await expect(
      reader.page.getByRole('heading', { name: 'Pair this browser first', level: 2 }),
    ).toBeVisible();
    expect(reader.page.url()).toBe(shown.link);
    await expect(reader.page.locator('iframe')).toHaveCount(0);
    expect(
      reader.requests.slice(requestCount).some((request) => request.url.includes('/api/pages')),
    ).toBe(false);
    expect(world.coreCalls().filter((call) => call.operation === 'dispatch.create')).toHaveLength(
      0,
    );
  });
});

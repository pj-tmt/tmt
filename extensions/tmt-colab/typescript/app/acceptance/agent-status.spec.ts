import fs from 'node:fs';
import { createHash } from 'node:crypto';
import { expect, test } from '@playwright/test';
import { pairBrowser, startDoor } from './harness/browser.js';
import { createPage, openPage } from './harness/ask.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';

test.afterEach(disposeActiveWorlds);

test('the served Agents view reads the admitted directory without sending or reopening', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world);
    const agent = await world.startAgent('status-agent');
    const created = createPage(
      world,
      'Admitted agent status',
      '<h1>Admitted agent status</h1>',
      agent.pane,
    );
    const browser = await pairBrowser(world, 'status-viewer');
    const operations: string[] = [];
    browser.context.on('request', (request) => {
      if (
        request.method() !== 'POST' ||
        !new URL(request.url()).pathname.endsWith('/append') ||
        !request.headers()['content-type']?.includes('application/json')
      )
        return;
      // Retain only the public operation name, never credentials or signed payloads.
      const body = request.postDataJSON() as { operation?: string };
      if (typeof body.operation === 'string') operations.push(body.operation);
    });
    const page = await openPage(door, browser, created);
    const content = page
      .frameLocator('iframe')
      .getByRole('heading', { name: 'Admitted agent status' });
    await expect(content).toBeVisible();
    const response = await browser.context.request.get(page.url());
    expect(response.status()).toBe(200);
    const policy = response.headers()['content-security-policy'];
    expect(policy).toContain("script-src 'self'; style-src 'self'");
    expect(policy).not.toContain('unsafe-inline');
    expect(policy).not.toContain('unsafe-eval');
    const manifestPath = process.env.COLAB_STATUS_ASSET_MANIFEST;
    if (!manifestPath)
      throw new Error('Set COLAB_STATUS_ASSET_MANIFEST to the hashed build report.');
    const manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf8')) as {
      distAssets: { path: string; sha256: string }[];
    };
    expect(manifest.distAssets).toHaveLength(11);
    const mount = new URL('./', page.url());
    for (const asset of manifest.distAssets) {
      const served = await browser.context.request.get(new URL(asset.path, mount).href);
      expect(served.status(), asset.path).toBe(200);
      expect(
        createHash('sha256')
          .update(await served.body())
          .digest('hex'),
        asset.path,
      ).toBe(asset.sha256);
    }
    const opened = operations.filter((value) => value === 'session.open').length;
    await page.getByTestId('agents-toggle').click();
    const panel = page.locator('.page-drawer[data-panel=agents]');
    const row = panel.locator('.agent-status-list li').filter({ hasText: agent.id });
    await expect(row).toContainText(agent.name);
    await expect(row).toHaveAttribute('data-presence', /^(active|offline|unknown)$/);
    await expect(panel.getByText('Read succeeded', { exact: true })).toHaveCount(2);
    await panel.getByRole('button', { name: 'Recheck status' }).click();
    await expect(panel.getByRole('button', { name: 'Recheck status' })).toBeEnabled();
    await expect(row).toBeVisible();
    const captures = process.env.COLAB_STATUS_NATIVE_CAPTURE_DIR;
    if (captures) {
      fs.mkdirSync(captures, { recursive: true });
      for (const width of [1440, 390]) {
        await page.setViewportSize({ width, height: 844 });
        for (const theme of ['light', 'dark']) {
          await page.evaluate((value) => (document.documentElement.dataset.theme = value), theme);
          await page.screenshot({ path: `${captures}/${width}-${theme}-ready.png` });
        }
      }
    }
    // Deliberately refuse just the Colab context GET. This is a labelled injected
    // read failure, not evidence about Ben's existing tab or #1822's root cause.
    await page.route('**/api/session', (route) =>
      route.fulfill({ status: 403, contentType: 'application/json', body: '{}' }),
    );
    await panel.getByRole('button', { name: 'Recheck status' }).click();
    await expect(panel.locator('.agent-status-list li')).toHaveCount(0);
    await expect(panel.getByText('No successful directory read yet.')).toBeVisible();
    await expect(panel.getByText('Read unavailable', { exact: true })).toHaveCount(2);
    await expect(content).toBeVisible();
    await page.unroute('**/api/session');
    await panel.getByRole('button', { name: 'Recheck status' }).click();
    await expect(row).toBeVisible();
    expect(operations.filter((value) => value === 'session.open')).toHaveLength(opened);
    expect(agent.received()).toHaveLength(0);
    expect(world.coreCalls().filter((value) => value.operation === 'dispatch.create')).toHaveLength(
      0,
    );
  });
});

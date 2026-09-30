import { expect, test, chromium } from '@playwright/test';
import type { BrowserContext, Page } from '@playwright/test';
import { mkdtemp, rm, readFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve, join } from 'node:path';
import { createServer } from 'node:http';
let context: BrowserContext, popup: Page, source: Page, profile: string;
let server: ReturnType<typeof createServer>;
let extensionId: string, tabId: number;
test.beforeEach(async () => {
  profile = await mkdtemp(join(tmpdir(), 'tmt-addon-'));
  server = createServer((_request, response) => {
    response.setHeader('Content-Type', 'text/html; charset=utf-8');
    response.end(
      '<!doctype html><title>Fixture title</title><p id="selected">exact &lt;script&gt;\t\u202E\u200B bytes</p>',
    );
  });
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
  const address = server.address();
  if (!address || typeof address === 'string') throw new Error('Fixture did not bind.');
  const url = `http://127.0.0.1:${address.port}/selection?literal=%0A`;
  context = await chromium.launchPersistentContext(profile, {
    channel: 'chromium',
    viewport: { width: 390, height: 600 },
    headless: true,
    args: [
      `--disable-extensions-except=${resolve('dist')}`,
      `--load-extension=${resolve('dist')}`,
      '--enable-unsafe-extension-debugging',
    ],
  });
  let worker = context.serviceWorkers()[0];
  if (!worker) worker = await context.waitForEvent('serviceworker');
  extensionId = new URL(worker.url()).host;
  source = context.pages()[0] ?? (await context.newPage());
  await source.goto(url);
  await source.evaluate(() => {
    const range = document.createRange();
    range.selectNodeContents(document.getElementById('selected')!);
    window.getSelection()!.removeAllRanges();
    window.getSelection()!.addRange(range);
  });
  popup = await context.newPage();
  await popup.goto(`chrome-extension://${extensionId}/popup.html`);
  tabId = await popup.evaluate(
    async (url) => (await chrome.tabs.query({})).find((tab) => tab.url === url)?.id ?? -1,
    url,
  );
  // Only the fixture page and our extension page exist; the fixture URL is hidden before activeTab.
  if (tabId === -1)
    tabId = await popup.evaluate(
      async () =>
        (await chrome.tabs.query({})).filter((tab) => tab.id !== undefined && !tab.url)[0]?.id ??
        -1,
    );
});
test.afterEach(async () => {
  try {
    const browser = context?.browser();
    await context?.close();
    expect(browser?.isConnected()).toBe(false);
  } finally {
    await new Promise<void>((resolve, reject) =>
      server?.close((error) => (error ? reject(error) : resolve())),
    );
    expect(server.listening).toBe(false);
    await rm(profile, { recursive: true, force: true });
  }
});
async function grantThroughBrowserAction(): Promise<void> {
  await source.bringToFront();
  const browserSession = await context.browser()!.newBrowserCDPSession();
  const { targetInfos } = await browserSession.send('Target.getTargets', {
    filter: [{ type: 'tab', exclude: false }],
  });
  const target = targetInfos.find((info) => info.url === source.url());
  if (!target) throw new Error(`No tab target for fixture: ${JSON.stringify(targetInfos)}`);
  await browserSession.send('Extensions.triggerAction', {
    id: extensionId,
    targetId: target.targetId,
  });
  await browserSession.detach();
}
test('packaged permissions and no-gesture/page-message paths cannot capture or send', async () => {
  const manifest = JSON.parse(await readFile('dist/manifest.json', 'utf8'));
  expect(manifest.permissions).toEqual(['activeTab', 'scripting', 'contextMenus']);
  for (const key of [
    'host_permissions',
    'optional_host_permissions',
    'externally_connectable',
    'content_scripts',
  ])
    expect(manifest[key]).toBeUndefined();
  expect(manifest.content_security_policy.extension_pages).toContain("connect-src 'none'");
  await expect(popup.locator('#send')).toBeDisabled();
  await source.evaluate(() =>
    window.postMessage({ type: 'tmt-send', message: 'page request' }, '*'),
  );
  await popup.evaluate(() => {
    document.getElementById('capture')!.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    (document.getElementById('composer') as HTMLFormElement).requestSubmit();
    document.getElementById('send')!.click();
  });
  await expect(popup.locator('#preview')).toHaveText('Capture a selection to begin.');
  const denied = await popup.evaluate(async (tabId) => {
    try {
      await chrome.scripting.executeScript({ target: { tabId }, func: () => 'denied' });
      return 'allowed';
    } catch {
      return 'denied';
    }
  }, tabId);
  expect(tabId).toBeGreaterThan(0);
  expect(denied).toBe('denied');
  await grantThroughBrowserAction();
  const capture = await popup.evaluate(
    async (tabId) =>
      chrome.scripting.executeScript({
        target: { tabId },
        func: () => window.getSelection()?.toString(),
      }),
    tabId,
  );
  expect(capture[0].result).toContain('exact <script>');
});
test('explicit capture, exact preview/send, restart recovery and inert reply', async () => {
  await grantThroughBrowserAction();
  await popup.locator('#capture').click();
  await expect(popup.locator('#preview')).toContainText('exact <script>');
  await popup.locator('#note').fill('note\u202E');
  const exact = await popup.locator('#preview').textContent();
  await popup.evaluate(() => {
    (document.getElementById('composer') as HTMLFormElement).requestSubmit();
    document.getElementById('send')!.click();
  });
  await expect(popup.locator('#status')).toHaveText('Review the exact message, then Send.');
  await expect(popup.locator('#send')).toBeEnabled();
  await expect(popup.locator('#escaped')).toContainText('\\u{202E}');
  await expect(popup.locator('#escaped')).toContainText('\\u{200B}');
  await popup.locator('#send').click();
  await expect(popup.locator('#status')).toContainText('Held');
  await expect(popup.locator('#agent')).toBeDisabled();
  const workerSession = await context.newCDPSession(source);
  await workerSession.send('ServiceWorker.enable');
  const stopped = new Promise<void>((resolve, reject) => {
    const deadline = setTimeout(() => reject(new Error('Extension worker did not stop.')), 5000);
    workerSession.on('ServiceWorker.workerVersionUpdated', ({ versions }) => {
      if (
        versions.some(
          (version) =>
            version.scriptURL === `chrome-extension://${extensionId}/background.js` &&
            version.runningStatus === 'stopped',
        )
      ) {
        clearTimeout(deadline);
        resolve();
      }
    });
  });
  await workerSession.send('ServiceWorker.stopAllWorkers');
  await stopped;
  await workerSession.detach();
  await popup.reload();
  await expect(popup.locator('#preview')).toHaveText(exact!);
  await expect(popup.locator('#status')).toContainText('Uncertain');
  await popup.locator('#recover').click();
  await expect(popup.locator('#status')).toHaveText('Replied');
  await expect(popup.locator('#reply')).toHaveText(
    '<demo reply> Selection received. Nothing was sent to TMT.',
  );
  expect(await popup.locator('#reply').locator('*').count()).toBe(0);
  await popup.screenshot({ path: test.info().outputPath('reply.png'), fullPage: true });
});
test('uncertain recovery, empty reply and unavailable result stay distinct', async () => {
  await grantThroughBrowserAction();
  await popup.locator('#capture').click();
  await popup.locator('#agent').selectOption('demo-uncertain');
  await popup.locator('#send').click();
  await expect(popup.locator('#status')).toContainText('Uncertain');
  await popup.locator('#recover').click();
  await expect(popup.locator('#reply')).toHaveText('(Empty reply)');
  popup.once('dialog', (dialog) => dialog.accept());
  await popup.locator('#reset').click();
  await source.bringToFront();
  await popup.locator('#capture').click();
  await popup.locator('#agent').selectOption('demo-unavailable');
  await popup.locator('#send').click();
  await popup.locator('#recover').click();
  await expect(popup.locator('#status')).toContainText('Result unavailable');
  await expect(popup.locator('#retry')).toBeHidden();
});

test('restored credentialed menu capture never enters preview or frozen intent', async () => {
  await popup.evaluate(async () => {
    await new Promise<void>((resolve, reject) => {
      const request = indexedDB.open('tmt-addon-shell', 1);
      request.onsuccess = () => {
        const db = request.result;
        const tx = db.transaction('values', 'readwrite');
        tx.objectStore('values').put(
          {
            selection: 'secret selection',
            title: 'title',
            url: 'https://user:password@example.test/',
          },
          'capture',
        );
        tx.oncomplete = () => {
          db.close();
          resolve();
        };
        tx.onabort = () => {
          db.close();
          reject(new Error('Fixture storage failed'));
        };
      };
    });
  });
  await popup.reload();
  await expect(popup.locator('#status')).toContainText('unavailable');
  await expect(popup.locator('#preview')).toHaveText('Capture a selection to begin.');
  await expect(popup.locator('#send')).toBeDisabled();
  expect(
    await popup.evaluate(async () => {
      return await new Promise((resolve) => {
        const request = indexedDB.open('tmt-addon-shell', 1);
        request.onsuccess = () => {
          const db = request.result,
            tx = db.transaction('values', 'readonly');
          const intent = tx.objectStore('values').get('intent');
          tx.oncomplete = () => {
            resolve(intent.result);
            db.close();
          };
        };
      });
    }),
  ).toBeUndefined();
});
